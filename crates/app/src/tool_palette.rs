use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use crate::Grooph;
use crate::pickers::{Picker, time_signature_label};
use crate::shell::centered_column;
use crate::tools::{BeatTemplate, DeleteOp, MetaOp, Modifier, NavOp, Tool, ToolId, ToolKind};
use crate::{Mode, tools};
use eframe::egui;
use eframe::egui::scroll_area::ScrollBarVisibility;
use eframe::egui::{
    Align2, Atom, Button, Color32, FontFamily, FontId, IdSalt, Pos2, Rect, Response, Sense,
    StrokeKind, Ui, Vec2, WidgetInfo,
};
use grooph_layout::glyphs::{Glyph, head_glyph_for, technique_sign};
use grooph_layout::pixel_layout::{
    LayoutOpts, MeasureLayout, PlacedGlyph, build_measure_layout, build_time_sig_layout, compute_em,
};
use grooph_measure::BeatKind::{Note, Rest};
use grooph_measure::duration::{Duration, NoteValue, TupletKind, TupletSpec, human_readable};
use grooph_measure::{Cursor, Instrument, Measure, StaffKind, Sticking, Technique, VoiceBar};
use grooph_render::draw_glyph;
use grooph_render::measure::{draw_notes, draw_time_signature};
use log::info;
use tools::EditOp;

pub(crate) const TOOL_PALETTE_BUTTON_SIZE: f32 = 70.0;
const TOOL_PALETTE_BUTTON_CORNER_RADIUS: f32 = 2.0;

enum ButtonKind {
    /// Renders a miniature notation.
    Note,
    /// Renders a stacked time-signature glyph.
    TimeSignature,
    /// Plain text/symbol button.
    Plain,
}

fn button_kind(kind: &ToolKind) -> ButtonKind {
    match kind {
        ToolKind::InsertBeat(..)
        | ToolKind::Modify(Modifier::ToggleDotted { .. })
        | ToolKind::Modify(Modifier::ToggleAccent | Modifier::ToggleSoft) => ButtonKind::Note,
        ToolKind::Meta(MetaOp::ChangeTimeSignature) => ButtonKind::TimeSignature,
        _ => ButtonKind::Plain,
    }
}

/// Room between the grid's cells.
const GRID_GAP: f32 = 4.0;
/// The narrowest a cell gets before the grid takes fewer columns.
const MIN_CELL: f32 = 54.0;
/// The widest a cell gets: on a wide screen the grid stays in the middle.
const MAX_CELL: f32 = 64.0;
/// The share of a value button's height its tuplet number takes, over the
/// value, while the cursor is in a tuplet, below a margin of
/// [`TUPLET_NUMBER_MARGIN`] of it.
const TUPLET_NUMBER_BAND: f32 = 0.25;
const TUPLET_NUMBER_MARGIN: f32 = 0.16;
/// The tallest a cell gets; narrower ones are square.
const MAX_CELL_HEIGHT: f32 = 64.0;
/// The most columns the grid has, on a wide screen.
const MAX_COLUMNS: usize = 16;
/// The share of the screen's height the grid takes at most; past it the
/// grid scrolls, and the staff above stays in view.
const MAX_GRID_SHARE: f32 = 0.45;
/// The most lines of cells the grid shows; past them it scrolls. The half
/// line cut off shows there is more, as the scroll bar is hidden.
const MAX_GRID_LINES: f32 = 3.5;

/// The kit's instruments in the grid's order, cymbals and drums from left to
/// right as they stand, then the feet.
const KIT_ORDER: [Instrument; 9] = [
    Instrument::HiHat,
    Instrument::Ride,
    Instrument::Crash,
    Instrument::Snare,
    Instrument::HighTom,
    Instrument::MidTom,
    Instrument::FloorTom,
    Instrument::Kick,
    Instrument::HiHatFoot,
];

/// The tools in the editors' bars, next to play, not in the grid.
pub(crate) const BAR_TOOLS: [ToolId; 2] = [ToolId::EditUndo, ToolId::EditRedo];
/// Which head accent and ghost go to, of several on a beat, if the picked
/// instrument has none there.
const AIM_ORDER: [Instrument; 9] = [
    Instrument::Snare,
    Instrument::Kick,
    Instrument::HighTom,
    Instrument::MidTom,
    Instrument::FloorTom,
    Instrument::HiHat,
    Instrument::Ride,
    Instrument::Crash,
    Instrument::HiHatFoot,
];
/// The tools in one row, going on in the next line where it does not fit,
/// the most used first: the note values, short to long, the rests in the
/// same order under them, the head modifiers, the stickings, the tuplets,
/// the meter last, as it is set once per exercise.
const TOOLS: [ToolId; 25] = [
    ToolId::InsertNoteQ,
    ToolId::InsertNoteE,
    ToolId::InsertNoteS,
    ToolId::InsertNoteTh,
    ToolId::InsertNoteH,
    ToolId::InsertNoteW,
    ToolId::InsertRestQ,
    ToolId::InsertRestE,
    ToolId::InsertRestS,
    ToolId::InsertRestTh,
    ToolId::InsertRestH,
    ToolId::InsertRestW,
    ToolId::ModifyToggleAccent,
    ToolId::ModifyToggleSoft,
    ToolId::ModifyToggleDotted,
    ToolId::ModifyStickingRight,
    ToolId::ModifyStickingLeft,
    ToolId::ModifyStickingBoth,
    ToolId::InsertTupletT8,
    ToolId::InsertTupletT16,
    ToolId::InsertTupletQt16,
    ToolId::InsertTupletSt16,
    ToolId::InsertTupletSpt16,
    ToolId::InsertTupletNt16,
    ToolId::MetaChangeTimeSignature,
];

/// One cell of the tool grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Cell {
    Tool(&'static Tool),
    /// Picks the instrument the tools write and change (the kit's); a tap
    /// steps its head on the cursor's beat through its techniques
    /// (`Grooph::tap_instrument`), a long press takes it away.
    Instrument(Instrument),
}

/// What `technique` is, on its button's hover.
fn technique_name(technique: Technique) -> &'static str {
    match technique {
        Technique::Normal => "Plain",
        Technique::Open => "Open",
        Technique::HalfOpen => "Half open",
        Technique::Closed => "Closed",
        Technique::CrossStick => "Cross-stick",
        Technique::Rimshot => "Rimshot",
        Technique::Bell => "Bell",
        Technique::Choke => "Choke",
    }
}

/// The grid's rows, top to bottom; a row wider than the grid goes on in the
/// next line. On the kit its instruments come first, always the same nine
/// cells; on the one line there are none. The stickings only where they
/// are written (`sticking`): not on the metronome's clicks.
pub(crate) fn grid_rows(kit: bool, sticking: bool) -> Vec<Vec<Cell>> {
    let tools = |ids: &[ToolId]| {
        ids.iter()
            .filter(|id| {
                sticking || !matches!(id.tool().kind, ToolKind::Modify(Modifier::ToggleSticking(_)))
            })
            .map(|&id| Cell::Tool(id.tool()))
            .collect::<Vec<_>>()
    };
    let mut rows = Vec::new();
    if kit {
        rows.push(KIT_ORDER.iter().map(|&i| Cell::Instrument(i)).collect());
    }
    rows.push(tools(&TOOLS));
    rows
}

/// `row` as lines of at most `columns` cells.
fn row_lines(row: &[Cell], columns: usize) -> Vec<&[Cell]> {
    row.chunks(columns).collect()
}

/// The grid's columns and the size of its cells, `width` wide.
fn grid_metrics(width: f32) -> (usize, Vec2) {
    let columns =
        (((width + GRID_GAP) / (MIN_CELL + GRID_GAP)).floor() as usize).clamp(1, MAX_COLUMNS);
    let cell = ((width - GRID_GAP * (columns - 1) as f32) / columns as f32).min(MAX_CELL);
    (columns, Vec2::new(cell, cell.min(MAX_CELL_HEIGHT)))
}

/// The height the grid of `lines` lines of cells `cell` high takes on a
/// screen `screen` high: all of them, but at most [`MAX_GRID_LINES`] and
/// [`MAX_GRID_SHARE`] of the screen.
fn grid_height(lines: usize, cell: f32, screen: f32) -> f32 {
    let all = lines as f32 * (cell + GRID_GAP) - GRID_GAP;
    let most = MAX_GRID_LINES * (cell + GRID_GAP) - GRID_GAP;
    all.min(most).min(MAX_GRID_SHARE * screen)
}

impl Grooph {
    /// The editor's tools as a grid in the thumb's reach, the same in every
    /// editor (Practice, Score, the metronome's click bar): on the kit its
    /// instruments, then one row of the note values, the rests, the head
    /// modifiers, the tuplets and the meter. Undo and redo are in
    /// the editor's bar ([`BAR_TOOLS`]).
    pub(super) fn tool_palette_panel(&mut self, ui: &mut Ui) {
        egui::Panel::bottom("tool_palette")
            .show_separator_line(false)
            .resizable(false)
            .show_collapsible(ui, &mut (self.ui.mode == Mode::Edit), |ui| {
                // In the page's column, as the editor's bar below it: on a
                // wide screen as wide as the cards, not the window.
                centered_column(ui, |ui| self.tool_grid_scroll(ui));
            });
    }

    fn tool_grid_scroll(&mut self, ui: &mut Ui) {
        // Sized up front: a bottom panel offers the height it had.
        let rows = grid_rows(!self.staff_model().one_line(), !self.metronome_mode());
        let (columns, cell) = grid_metrics(ui.available_width());
        let lines: usize = rows.iter().map(|r| row_lines(r, columns).len()).sum();
        let height = grid_height(lines, cell.y, ui.ctx().content_rect().height());
        egui::ScrollArea::vertical()
            .id_salt("tool_grid")
            .min_scrolled_height(height)
            .max_height(height)
            .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| self.tool_grid(ui, &rows));
    }

    fn tool_grid(&mut self, ui: &mut Ui, rows: &[Vec<Cell>]) {
        let width = ui.available_width();
        let (columns, cell) = grid_metrics(width);
        let used = columns as f32 * cell.x + (columns - 1) as f32 * GRID_GAP;
        let left = ui.max_rect().left() + (width - used) / 2.0;
        let mut top = ui.cursor().top();
        let mut clicked = None;
        for row in rows {
            for line in row_lines(row, columns) {
                for (col, &c) in line.iter().enumerate() {
                    let min = Pos2::new(left + col as f32 * (cell.x + GRID_GAP), top);
                    let response = self.grid_cell(ui, Rect::from_min_size(min, cell), c);
                    // A long press (a right click) on an instrument takes
                    // its head away; it is no click as well.
                    if response.secondary_clicked() && matches!(c, Cell::Instrument(_)) {
                        clicked = Some((c, true));
                    } else if response.clicked() {
                        clicked = Some((c, false));
                    }
                }
                top += cell.y + GRID_GAP;
            }
        }
        ui.allocate_rect(
            Rect::from_min_max(
                Pos2::new(ui.max_rect().left(), ui.cursor().top()),
                Pos2::new(ui.max_rect().right(), top - GRID_GAP),
            ),
            Sense::hover(),
        );
        match clicked {
            Some((Cell::Tool(t), _)) => self.apply_tool(t),
            Some((Cell::Instrument(i), false)) => self.tap_instrument(i),
            Some((Cell::Instrument(i), true)) => self.hold_instrument(i),
            None => {}
        }
    }

    /// Whether `cell` shows as picked: an instrument with a head on the
    /// cursor's beat, a toggle that is on, or the tuplet the beat is in.
    fn cell_selected(&self, cell: Cell) -> bool {
        match cell {
            Cell::Instrument(i) => self.heads_at_cursor().iter().any(|&(h, _)| h == i),
            Cell::Tool(Tool { kind: ToolKind::Modify(Modifier::ToggleTechnique(t)), .. }) => {
                let (pen, at) = self.aimed(Modifier::ToggleTechnique(*t));
                let score = &self.editor.doc().score;
                score
                    .voice_at(at)
                    .beats()
                    .get(at.beat_idx)
                    .and_then(|b| b.heads().get(pen))
                    .is_some_and(|h| h.technique() == *t)
            }
            // The toggles show whether they are on: the accent and the ghost
            // note of the head they mean, the dot of the beat.
            Cell::Tool(Tool {
                kind: ToolKind::Modify(m @ (Modifier::ToggleAccent | Modifier::ToggleSoft)),
                ..
            }) => {
                let (pen, at) = self.aimed(*m);
                let score = &self.editor.doc().score;
                score
                    .voice_at(at)
                    .beats()
                    .get(at.beat_idx)
                    .and_then(|b| b.heads().get(pen))
                    .is_some_and(|h| match m {
                        Modifier::ToggleAccent => h.accented(),
                        _ => h.soft(),
                    })
            }
            Cell::Tool(Tool { kind: ToolKind::Modify(Modifier::ToggleDotted { .. }), .. }) => {
                let cursor = self.editor.doc().score.clamp_cursor(self.editor.doc().cursor);
                self.current_voice()
                    .beats()
                    .get(cursor.beat_idx)
                    .is_some_and(|b| matches!(b.duration, Duration::Dotted { .. }))
            }
            // The sticking the beat has: a tap takes it away.
            Cell::Tool(Tool { kind: ToolKind::Modify(Modifier::ToggleSticking(s)), .. }) => {
                let cursor = self.editor.doc().score.clamp_cursor(self.editor.doc().cursor);
                self.current_voice()
                    .beats()
                    .get(cursor.beat_idx)
                    .is_some_and(|b| b.sticking() == Some(*s))
            }
            // The tuplet the cursor is in: a tap on it dissolves the group.
            Cell::Tool(t) => self.in_tuplet_of(t),
        }
    }

    /// How the head of `instrument` on the cursor's beat is played, `None`
    /// without one there.
    pub(crate) fn technique_at_cursor(&self, instrument: Instrument) -> Option<Technique> {
        let score = &self.editor.doc().score;
        self.heads_at_cursor().into_iter().find(|&(i, _)| i == instrument).and_then(|(_, at)| {
            score
                .voice_at(at)
                .beats()
                .get(at.beat_idx)
                .and_then(|b| b.heads().get(instrument))
                .map(|h| h.technique())
        })
    }

    /// The heads on the beat the cursor is on, whichever voice has them
    /// (the beat sounding at the cursor's time in each), with where each is.
    pub(super) fn heads_at_cursor(&self) -> Vec<(Instrument, Cursor)> {
        let score = &self.editor.doc().score;
        let cursor = score.clamp_cursor(self.editor.doc().cursor);
        (0..self.current_measure().voices().len())
            .flat_map(|voice| {
                let at = score.cursor_in_voice(cursor, voice);
                let beat = score.voice_at(at).beats().get(at.beat_idx).copied();
                let heads: Vec<_> = beat.iter().flat_map(|b| b.heads().iter()).collect();
                heads.into_iter().map(|h| (h.instrument(), at)).collect::<Vec<_>>()
            })
            .collect()
    }

    /// The head and its place that `modifier` changes on the kit. A head
    /// mark means a head on the cursor's beat the mark fits: a technique's
    /// is of an instrument played that way, so no instrument is picked
    /// first. Of several, the picked instrument's, else the snare, kick,
    /// toms, hi-hat, ride, crash. Anything else, and the one line, edits the
    /// picked instrument at the cursor.
    fn aimed(&self, modifier: Modifier) -> (Instrument, Cursor) {
        let score = &self.editor.doc().score;
        let at = score.clamp_cursor(self.editor.doc().cursor);
        let pen = self.edit_instrument();
        if self.staff_model().one_line() {
            return (pen, at);
        }
        let fits: fn(Modifier, Instrument) -> bool = |modifier, i| match modifier {
            Modifier::ToggleAccent | Modifier::ToggleSoft => true,
            Modifier::ToggleTechnique(t) => i.allows(t),
            Modifier::CycleTechnique => i.techniques().len() > 1,
            _ => false,
        };
        if !matches!(
            modifier,
            Modifier::ToggleAccent
                | Modifier::ToggleSoft
                | Modifier::ToggleTechnique(_)
                | Modifier::CycleTechnique
        ) {
            return (pen, at);
        }
        let heads: Vec<_> =
            self.heads_at_cursor().into_iter().filter(|&(i, _)| fits(modifier, i)).collect();
        let rank = |i: Instrument| AIM_ORDER.iter().position(|&o| o == i).unwrap_or(usize::MAX);
        heads
            .iter()
            .find(|&&(i, _)| i == pen)
            .or_else(|| heads.iter().min_by_key(|&&(i, _)| rank(i)))
            .copied()
            .unwrap_or((pen, at))
    }

    /// Whether the picked instrument's cell is the one accent and ghost mean,
    /// of several heads on the beat: it gets a ring.
    fn is_aimed(&self, instrument: Instrument) -> bool {
        self.heads_at_cursor().len() > 1 && self.aimed(Modifier::ToggleAccent).0 == instrument
    }

    /// Whether the cursor's beat is in a group of `tool`'s tuplet.
    fn in_tuplet_of(&self, tool: &Tool) -> bool {
        let ToolKind::InsertBeat(BeatTemplate { duration: Duration::Tuplet(spec), .. }) = tool.kind
        else {
            return false;
        };
        let cursor = self.editor.doc().score.clamp_cursor(self.editor.doc().cursor);
        self.current_voice()
            .beats()
            .get(cursor.beat_idx)
            .is_some_and(|b| b.duration == Duration::Tuplet(spec))
    }

    /// Whether `cell` does something now: undo and redo with a step to take,
    /// a technique the picked instrument is played with.
    fn cell_enabled(&self, cell: Cell) -> bool {
        match cell {
            Cell::Tool(t) => match t.kind {
                ToolKind::Edit(EditOp::Undo) => self.can_undo(),
                ToolKind::Edit(EditOp::Redo) => self.can_redo(),
                ToolKind::Modify(modifier) => {
                    self.applies(Trial::Modify(modifier), || self.modifier_applies(modifier))
                }
                ToolKind::InsertBeat(
                    template @ BeatTemplate { duration: Duration::Simple(_), .. },
                ) => self.applies(Trial::Value(template), || self.value_applies(template)),
                _ => true,
            },
            Cell::Instrument(_) => true,
        }
    }

    /// Whether the edit `trial` stands for changes the cursor's beat, by
    /// `check` (which tries it on a copy of the cursor's bar), remembered
    /// until the bar, the cursor, the picked instrument or the staff change.
    /// The grid asks for each of its cells every frame, and the answers only
    /// change with an edit or a move.
    fn applies(&self, trial: Trial, check: impl FnOnce() -> bool) -> bool {
        type Kept = Option<(TrialState, Vec<(Trial, bool)>)>;
        thread_local! {
            static KEPT: RefCell<Kept> = const { RefCell::new(None) };
        }
        let doc = self.editor.doc();
        let cursor = doc.score.clamp_cursor(doc.cursor);
        let now = |state: &TrialState| {
            state.cursor == cursor
                && state.instrument == self.editor.instrument
                && state.staff == doc.score.staff
                && state.measure == *self.current_measure()
        };
        let known = KEPT.with_borrow(|kept| match kept {
            Some((state, answers)) if now(state) => {
                answers.iter().find(|(t, _)| *t == trial).map(|&(_, a)| a)
            }
            _ => None,
        });
        if let Some(answer) = known {
            return answer;
        }
        let answer = check();
        KEPT.with_borrow_mut(|kept| {
            if !kept.as_ref().is_some_and(|(state, _)| now(state)) {
                let state = TrialState {
                    measure: self.current_measure().clone(),
                    cursor,
                    instrument: self.editor.instrument,
                    staff: doc.score.staff,
                };
                *kept = Some((state, Vec::new()));
            }
            if let Some((_, answers)) = kept {
                answers.push((trial, answer));
            }
        });
        answer
    }

    /// Whether `modifier` changes the cursor's beat: an accent needs a head
    /// of the picked instrument there, not a rest; the dot a value it fits
    /// in the bar; a technique a head its instrument plays that way; "+/−"
    /// a beat with room, without a drum struck by both voices. Tried on a
    /// copy of the cursor's bar.
    fn modifier_applies(&self, modifier: Modifier) -> bool {
        let (pen, cursor) = self.aimed(modifier);
        let mut measure = self.current_measure().clone();
        let Some(voice) = measure.voice_mut(cursor.voice) else {
            return false;
        };
        if voice.beats().is_empty() {
            return false;
        }
        let idx = cursor.beat_idx.min(voice.beats().len() - 1);
        if !modify(voice, idx, pen, modifier) {
            return false;
        }
        // The kit refuses an edit that has two voices strike one drum.
        let before = self.current_measure().doubled_instruments();
        self.staff_model().one_line()
            || measure.doubled_instruments().iter().all(|d| before.contains(d))
    }

    /// Whether a value button changes the cursor's beat: the value fits
    /// there (in a tuplet, in the group from the cursor on), without a drum
    /// struck by both voices. Tried on a copy of the cursor's bar.
    fn value_applies(&self, template: BeatTemplate) -> bool {
        let cursor = self.editor.doc().score.clamp_cursor(self.editor.doc().cursor);
        let mut measure = self.current_measure().clone();
        let Some(voice) = measure.voice_mut(cursor.voice) else {
            return false;
        };
        if voice.beats().is_empty() {
            return false;
        }
        let idx = cursor.beat_idx.min(voice.beats().len() - 1);
        let base = template.duration.base_note();
        let pen = self.edit_instrument();
        if voice.modify_beat(idx, base, Some(template.kind), pen).is_none() {
            return false;
        }
        let before = self.current_measure().doubled_instruments();
        self.staff_model().one_line()
            || measure.doubled_instruments().iter().all(|d| before.contains(d))
    }

    /// The tuplet value a value button writes on the cursor's beat: its
    /// value in the ratio of the tuplet the beat is in ([`VoiceBar::modify_beat`]).
    /// `None` for the other tools and off a tuplet.
    fn value_in_tuplet(&self, tool: &Tool) -> Option<TupletSpec> {
        let ToolKind::InsertBeat(BeatTemplate { duration: Duration::Simple(base), .. }) = tool.kind
        else {
            return None;
        };
        let cursor = self.editor.doc().score.clamp_cursor(self.editor.doc().cursor);
        let spec = self.current_voice().beats().get(cursor.beat_idx)?.duration.as_tuplet_spec()?;
        Some(TupletSpec { base, ..spec })
    }

    /// A modifier's cell shows the cursor's beat as it modifies it: its value
    /// with the accent, the ghost note's parentheses or the dot. `None` for
    /// the other tools, and for a value the dot cannot show (a dotted whole
    /// or 32nd: the cell keeps its own label).
    fn modifier_preview(&self, tool: &Tool) -> Option<Measure> {
        let (accented, soft, dot) = match tool.kind {
            ToolKind::Modify(Modifier::ToggleAccent) => (true, false, false),
            ToolKind::Modify(Modifier::ToggleSoft) => (false, true, false),
            ToolKind::Modify(Modifier::ToggleDotted { .. }) => (false, false, true),
            _ => return None,
        };
        let cursor = self.editor.doc().score.clamp_cursor(self.editor.doc().cursor);
        let beat = self.current_voice().beats().get(cursor.beat_idx)?;
        let base = beat.duration.base_note();
        // Accent and ghost mean the note, whatever its dot; the dot's cell
        // is the dotted value, a rest where the beat is one.
        let duration = match dot {
            true => Duration::Dotted { base, dots: 1 },
            false => Duration::Simple(base),
        };
        let kind = if dot { beat.kind() } else { Note };
        // A button's measure holds a dotted half, quarter, eighth or
        // sixteenth.
        if matches!(duration, Duration::Dotted { .. })
            && !matches!(
                base,
                NoteValue::Half | NoteValue::Quarter | NoteValue::Eighth | NoteValue::Sixteenth
            )
        {
            return None;
        }
        let template = BeatTemplate { duration, kind, accented, soft };
        Some(Grooph::build_button_measure(template))
    }

    /// One cell of the grid in `rect`, framed like a button.
    fn grid_cell(&self, ui: &mut Ui, rect: Rect, cell: Cell) -> Response {
        let enabled = self.cell_enabled(cell);
        let selected = self.cell_selected(cell);
        let (key, label, hover) = match cell {
            Cell::Tool(t) => {
                let hover = match t.kind {
                    ToolKind::Modify(Modifier::ToggleTechnique(technique)) => {
                        technique_name(technique).to_owned()
                    }
                    ToolKind::Modify(Modifier::ToggleHead) => {
                        "Add the instrument here, or take it away".to_owned()
                    }
                    ToolKind::Modify(Modifier::ToggleSticking(_)) if selected => {
                        format!("{}: tap to take it away", t.label)
                    }
                    ToolKind::InsertBeat(_) if selected => "Dissolve the tuplet".to_owned(),
                    // In a tuplet a value button writes the value of its ratio.
                    ToolKind::InsertBeat(BeatTemplate { kind, .. })
                        if let Some(spec) = self.value_in_tuplet(t) =>
                    {
                        let value = human_readable(&Duration::Tuplet(spec));
                        match kind {
                            Rest => format!("{value} rest"),
                            Note => value,
                        }
                    }
                    // The meter with its grouping, which the cell leaves out.
                    _ if matches!(button_kind(&t.kind), ButtonKind::TimeSignature) => {
                        format!(
                            "Meter {}",
                            time_signature_label(self.current_measure().time_signature())
                        )
                    }
                    _ => t.label.to_owned(),
                };
                let hover = match t.shortcuts.first() {
                    Some(sc) => format!("{hover} ({})", sc.label()),
                    None => hover,
                };
                let label = match t.kind {
                    ToolKind::Modify(Modifier::ToggleHead) => "+/−",
                    // The letters the staff writes; both hands R above L.
                    ToolKind::Modify(Modifier::ToggleSticking(s)) => match s {
                        Sticking::Right => "R",
                        Sticking::Left => "L",
                        Sticking::Both => "R\nL",
                    },
                    _ => t.label,
                };
                (t.id.name(), label, hover)
            }
            Cell::Instrument(i) => (i.short_label(), i.short_label(), self.instrument_hover(i)),
        };
        let id = ui.make_persistent_id(("tool_grid", key));
        let sense = if enabled { Sense::click() } else { Sense::hover() };
        let response = ui.interact(rect, id, sense);
        response
            .widget_info(|| WidgetInfo::selected(egui::Role::Button, enabled, selected, &hover));
        // Framed like the edit bar's buttons; a disabled one keeps its frame
        // and fades, as an egui button does.
        let visuals = if enabled {
            ui.style().interact_selectable(&response, selected)
        } else {
            ui.visuals().widgets.inactive
        };
        let painter = ui.painter();
        let fill = if enabled {
            visuals.weak_bg_fill
        } else {
            visuals.weak_bg_fill.gamma_multiply(ui.visuals().disabled_alpha())
        };
        painter.rect(rect, visuals.corner_radius, fill, visuals.bg_stroke, StrokeKind::Inside);
        // The instrument the note values write, picked or not on the beat;
        // with several heads on the beat, the one accent and ghost go to.
        if let Cell::Instrument(i) = cell {
            let aimed = self.is_aimed(i);
            if aimed || i == self.edit_instrument() {
                let color = ui.visuals().selection.stroke.color;
                let ring =
                    egui::Stroke::new(2.0, color.gamma_multiply(if aimed { 1.0 } else { 0.6 }));
                painter.rect_stroke(rect, visuals.corner_radius, ring, StrokeKind::Inside);
            }
        }
        let color = if enabled { visuals.text_color() } else { ui.visuals().weak_text_color() };
        let inner = rect.shrink(NOTATION_PADDING);
        match cell {
            Cell::Instrument(i) => {
                let technique = self.technique_at_cursor(i);
                let family = self.ui.music_font_id.family.clone();
                paint_instrument(ui, rect, i, technique, family, color);
            }
            // In a tuplet a value button shows its value with the tuplet's
            // number over it, as the group is written.
            Cell::Tool(t) if let Some(spec) = self.value_in_tuplet(t) => {
                let family = self.ui.music_font_id.family.clone();
                let band = inner.height() * TUPLET_NUMBER_BAND;
                let margin = inner.height() * TUPLET_NUMBER_MARGIN;
                let (_, rest) = inner.split_top_bottom_at_y(inner.top() + margin);
                let (top, below) = rest.split_top_bottom_at_y(rest.top() + band);
                // The digits of the staff's tuplet numbers.
                let digits: String =
                    grooph_layout::glyphs::tuplet_glyphs(spec.n).iter().map(|g| g.ch).collect();
                painter.text(
                    top.center(),
                    Align2::CENTER_CENTER,
                    digits,
                    FontId::new(2.0 * band, family.clone()),
                    color,
                );
                paint_notation(ui, below, &self.editor.button_measures[&t.id], family, color);
            }
            Cell::Tool(t) if matches!(button_kind(&t.kind), ButtonKind::Note) => {
                let family = self.ui.music_font_id.family.clone();
                match self.modifier_preview(t) {
                    Some(measure) => paint_notation(ui, inner, &measure, family, color),
                    None => paint_notation(
                        ui,
                        inner,
                        &self.editor.button_measures[&t.id],
                        family,
                        color,
                    ),
                }
            }
            Cell::Tool(t) if matches!(button_kind(&t.kind), ButtonKind::TimeSignature) => {
                self.paint_time_signature(ui, inner, color);
            }
            Cell::Tool(Tool { kind: ToolKind::Modify(Modifier::ToggleSticking(_)), .. }) => {
                // As on the staff, two letters stacked where both hands play.
                let lines = label.lines().count() as f32;
                let size = 0.42 / lines.max(1.0).sqrt() * rect.height();
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    label,
                    FontId::proportional(size),
                    color,
                );
            }
            _ => {
                // Words smaller than marks and short labels.
                let size = if label.chars().count() > 3 { 0.3 } else { 0.42 } * rect.height();
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    label,
                    FontId::proportional(size),
                    color,
                );
            }
        }
        response.on_hover_text(hover)
    }

    /// What a tap and a long press on `instrument`'s cell do now.
    fn instrument_hover(&self, instrument: Instrument) -> String {
        let name = instrument.label();
        let techniques: Vec<_> = instrument.techniques().collect();
        let Some(technique) = self.technique_at_cursor(instrument) else {
            return format!("Add the {}", name.to_lowercase());
        };
        let at = techniques.iter().position(|&t| t == technique).unwrap_or(0);
        match techniques.get(at + 1) {
            Some(&next) if techniques.len() > 1 => format!(
                "{name}, {}: tap for {}, hold to take it away",
                technique_name(technique).to_lowercase(),
                technique_name(next).to_lowercase()
            ),
            _ if techniques.len() > 1 => {
                format!("{name}, {}: tap to take it away", technique_name(technique).to_lowercase())
            }
            _ => format!("Take the {} away", name.to_lowercase()),
        }
    }

    /// The bar's meter as on the staff, stacked, in `rect`.
    fn paint_time_signature(&self, ui: &Ui, rect: Rect, color: Color32) {
        let painter = &ui.painter_at(rect);
        let em = compute_em(&rect, 0.7, ui);
        let font_id = FontId::new(em, self.ui.music_font_id.family.clone());
        let opts = LayoutOpts {
            rect,
            font_id: font_id.clone(),
            pixels_per_point: ui.ctx().pixels_per_point(),
            em,
            layout_clef: false,
            layout_time_signature: true,
            y_offset: 0.0,
            stem_length_factor: 0.9,
            stem_thickness_factor: 0.03,
            accent_displacement: 0.0,
            stem_down: false,
            shared_line: false,
            staff: StaffKind::Line,
        };
        let ts = self.current_measure().time_signature();
        let layout = build_time_sig_layout(&ts, rect.center().x, &opts);
        draw_time_signature(painter, &layout, &font_id, color);
    }

    pub(super) fn apply_tool(&mut self, tool: &Tool) {
        if !self.tool_applicable(tool) {
            return;
        }

        // Non-mutating tools have dedicated paths that manage their own state.
        match tool.kind {
            ToolKind::Edit(EditOp::Undo) => {
                self.undo();
                return;
            }
            ToolKind::Edit(EditOp::Redo) => {
                self.redo();
                return;
            }
            // The click bar's meter is the metronome's: pick it there.
            ToolKind::Meta(MetaOp::ChangeTimeSignature) if self.metronome_mode() => {
                self.ui.picker = Some(Picker::MetronomeMeter);
                return;
            }
            ToolKind::Meta(MetaOp::ChangeTimeSignature) => {
                self.ui.picker = Some(Picker::BarMeter);
                return;
            }
            ToolKind::Navigate(op) => {
                self.execute_navigation(op);
                return;
            }
            _ => {}
        }

        debug_assert!(tool.kind.is_mutating());

        let committed = self.with_undo_snapshot(|g| {
            // A drum is struck by one voice at a time: an edit that has two
            // voices strike it together is refused. One there already (from
            // a link, say) leaves the other edits of its measure alone.
            let kit = !g.staff_model().one_line();
            let before = kit.then(|| (g.editor.doc().cursor, g.current_measure().clone()));
            let changed = g.execute_mutating_tool(tool);
            if let Some((cursor, measure)) = before
                && changed
                && g.current_measure()
                    .doubled_instruments()
                    .iter()
                    .any(|d| !measure.doubled_instruments().contains(d))
            {
                let doc = g.editor.doc_mut();
                doc.score.measures_mut()[cursor.measure_idx] = measure;
                doc.cursor = cursor;
                return false;
            }
            changed
        });
        if committed {
            info!("applied tool: {}", tool.id.name());
        }
    }

    /// Gate for any tool dispatch — single source of truth, regardless of trigger (palette / keyboard / panel).
    fn tool_applicable(&self, tool: &Tool) -> bool {
        // The heads of one instrument and its techniques are the kit's: on
        // the one line a note is its one head.
        let kit_only = matches!(
            tool.kind,
            ToolKind::Modify(
                Modifier::ToggleHead | Modifier::CycleTechnique | Modifier::ToggleTechnique(_)
            )
        );
        // The metronome's clicks are played by no hand.
        let sticking = matches!(tool.kind, ToolKind::Modify(Modifier::ToggleSticking(_)));
        // Tools only run in edit mode.
        matches!(self.ui.mode, Mode::Edit)
            && !(kit_only && self.staff_model().one_line())
            && !(sticking && self.metronome_mode())
    }

    /// Moves the cursor according to `op`. Non-mutating; never produces an undo snapshot.
    /// Left/Right cross measure boundaries; Home/End jump to the very first/last beat in the score.
    /// The cursor stays in its voice, or the last one of a measure with fewer.
    fn execute_navigation(&mut self, op: NavOp) {
        let measure_count = self.editor.doc().score.len();
        let cursor = self.editor.doc().cursor;
        let to = |measure_idx, beat_idx| Cursor { measure_idx, beat_idx, ..cursor };
        let moved = match op {
            NavOp::Left if cursor.beat_idx > 0 => to(cursor.measure_idx, cursor.beat_idx - 1),
            // The clamp puts it on the last beat.
            NavOp::Left if cursor.measure_idx > 0 => to(cursor.measure_idx - 1, usize::MAX),
            NavOp::Left => cursor,
            NavOp::Right if cursor.beat_idx + 1 < self.current_voice().beats().len() => {
                to(cursor.measure_idx, cursor.beat_idx + 1)
            }
            NavOp::Right if cursor.measure_idx + 1 < measure_count => to(cursor.measure_idx + 1, 0),
            NavOp::Right => cursor,
            NavOp::Start => to(0, 0),
            NavOp::End => to(measure_count.saturating_sub(1), usize::MAX),
        };
        let doc = self.editor.doc_mut();
        doc.cursor = doc.score.clamp_cursor(moved);
    }

    /// Executes a tool that is known to be mutating. Returns whether a change was committed.
    /// Must only be called from within `with_undo_snapshot`.
    fn execute_mutating_tool(&mut self, tool: &Tool) -> bool {
        let beats_len = self.current_voice().beats().len();
        match tool.kind {
            ToolKind::InsertBeat(template) => {
                if beats_len == 0 {
                    return false;
                }
                let idx = self.editor.doc().cursor.beat_idx.min(beats_len - 1);
                let result = match template.duration {
                    Duration::Simple(_) => {
                        self.set_beat(idx, template.duration.base_note(), Some(template.kind))
                    }
                    Duration::Tuplet(spec) => self.set_tuplet(idx, spec),
                    _ => None,
                };
                result.is_some()
            }
            ToolKind::Meta(MetaOp::ResetMeasure) => {
                // The voice being edited; the others stay.
                let cursor = self.editor.doc().score.clamp_cursor(self.editor.doc().cursor);
                self.editor.doc_mut().score.measures_mut()[cursor.measure_idx]
                    .clear_voice(cursor.voice, Rest);
                self.editor.doc_mut().cursor = Cursor { beat_idx: 0, ..cursor };
                true
            }
            ToolKind::Modify(modifier) => {
                if beats_len == 0 {
                    return false;
                }
                // A head mark goes to the head it means, which becomes the
                // picked one, so the next mark follows it.
                let (aimed, at) = self.aimed(modifier);
                if !self.staff_model().one_line() {
                    self.editor.instrument = aimed;
                    self.editor.doc_mut().cursor = at;
                }
                let beats_len = self.current_voice().beats().len();
                if beats_len == 0 {
                    return false;
                }
                let idx = self.editor.doc().cursor.beat_idx.min(beats_len - 1);
                let pen = self.edit_instrument();
                modify(self.current_voice_mut(), idx, pen, modifier)
            }
            ToolKind::Delete(op) => {
                if beats_len == 0 {
                    return false;
                }
                let idx = self.editor.doc().cursor.beat_idx.min(beats_len - 1);
                self.current_voice_mut().remove(idx);
                let new_len = self.current_voice().beats().len();
                self.editor.doc_mut().cursor.beat_idx = if new_len == 0 {
                    0
                } else {
                    let last = new_len - 1;
                    match op {
                        DeleteOp::Forward => (self.editor.doc().cursor.beat_idx + 1).min(last),
                        DeleteOp::Backward => {
                            self.editor.doc().cursor.beat_idx.saturating_sub(1).min(last)
                        }
                    }
                };
                true
            }
            ToolKind::Edit(_)
            | ToolKind::Meta(MetaOp::ChangeTimeSignature)
            | ToolKind::Navigate(_) => {
                debug_assert!(false, "non-mutating tool reached execute_mutating_tool");
                false
            }
        }
    }
}

/// Apply `modifier` to the beat at `idx` of `voice`, for the head of
/// `pen`; whether it changed anything.
fn modify(voice: &mut VoiceBar, idx: usize, pen: Instrument, modifier: Modifier) -> bool {
    match modifier {
        Modifier::ToggleDotted { dots: _ } => voice.toggle_dotted(idx),
        Modifier::ToggleAccent => voice.toggle_accent(idx, pen),
        Modifier::ToggleSoft => voice.toggle_soft(idx, pen),
        Modifier::ToggleRestNote => voice.toggle_beat_kind(idx, pen),
        Modifier::ToggleHead => voice.toggle_head(idx, pen),
        Modifier::CycleTechnique => voice.cycle_technique(idx, pen),
        Modifier::ToggleTechnique(technique) => voice.toggle_technique(idx, pen, technique),
        Modifier::ToggleSticking(sticking) => voice.toggle_sticking(idx, sticking),
    }
    .is_some()
}

/// Padding (px) between a [`notation_button`]'s frame and its notes.
const NOTATION_PADDING: f32 = 4.0;
/// Height of a quarter note's ink (stem and head) in em, as drawn here.
const QUARTER_INK_EM: f32 = 0.85;
/// Margin around the notes inside their tile, as a share of its height.
const NOTATION_MARGIN: f32 = 0.08;

/// A button `size` high showing `measure` as miniature notation (tool
/// palette, grouping picker). `selected` renders it in the highlighted state.
pub(crate) fn notation_button(
    ui: &mut Ui,
    symbol_id: IdSalt,
    measure: &Measure,
    family: FontFamily,
    selected: bool,
    size: f32,
) -> Response {
    // The notes take most of the button's padding, the button keeps its
    // size.
    let padding = ui.spacing().button_padding;
    let room = 2.0 * (padding - Vec2::splat(NOTATION_PADDING)).max(Vec2::ZERO);
    let symbol = Atom::custom(symbol_id, notation_size(measure, size) + room);
    ui.spacing_mut().button_padding = padding.min(Vec2::splat(NOTATION_PADDING));
    let button = Button::new(symbol)
        .corner_radius(TOOL_PALETTE_BUTTON_CORNER_RADIUS)
        .selected(selected)
        .atom_ui(ui);
    ui.spacing_mut().button_padding = padding;
    if let Some(rect) = button.rect(symbol_id) {
        let color = ui.style().visuals.text_color();
        paint_notation(ui, rect, measure, family, color);
    }
    button.response
}

/// A button like a [`notation_button`] showing "none": a circle struck
/// through, `size` high.
pub(crate) fn none_button(ui: &mut Ui, symbol_id: IdSalt, selected: bool, size: f32) -> Response {
    let padding = ui.spacing().button_padding;
    let room = 2.0 * (padding - Vec2::splat(NOTATION_PADDING)).max(Vec2::ZERO);
    let symbol = Atom::custom(symbol_id, Vec2::splat(size) + room);
    ui.spacing_mut().button_padding = padding.min(Vec2::splat(NOTATION_PADDING));
    let button = Button::new(symbol)
        .corner_radius(TOOL_PALETTE_BUTTON_CORNER_RADIUS)
        .selected(selected)
        .atom_ui(ui);
    ui.spacing_mut().button_padding = padding;
    if let Some(rect) = button.rect(symbol_id) {
        paint_none(ui.painter(), rect, ui.style().visuals.text_color());
    }
    button.response
}

/// "None" in `rect`: a circle struck through, as on a [`none_button`].
pub(crate) fn paint_none(painter: &egui::Painter, rect: Rect, color: Color32) {
    let radius = 0.3 * rect.width().min(rect.height());
    let stroke = egui::Stroke::new((0.12 * radius).max(1.5), color);
    let center = rect.center();
    painter.circle_stroke(center, radius, stroke);
    // Lower left to upper right.
    let d = radius * std::f32::consts::FRAC_1_SQRT_2 * Vec2::new(1.0, -1.0);
    painter.line_segment([center - d, center + d], stroke);
}

/// An instrument's cell of the kit in `rect`: its short name, bold, in the
/// corner, the head of its note on the cursor's beat as on the staff (with
/// the technique's sign), faded where it has none there and the head a tap
/// writes, and a dot for each of its techniques, the one played lit.
fn paint_instrument(
    ui: &Ui,
    rect: Rect,
    instrument: Instrument,
    technique: Option<Technique>,
    family: FontFamily,
    color: Color32,
) {
    let painter = ui.painter_at(rect);
    let (w, h) = (rect.width(), rect.height());
    // Bold: egui's font has no bold face, so the name is drawn twice, whole
    // pixels apart (text is snapped to them).
    let size = 0.23 * h;
    let ppp = ui.ctx().pixels_per_point();
    let bold = (0.07 * size * ppp).round().max(1.0) / ppp;
    let at = rect.left_top() + Vec2::new(0.1 * w, 0.06 * h);
    let font = FontId::proportional(size);
    let name = instrument.short_label();
    for dx in [0.0, bold] {
        painter.text(at + Vec2::new(dx, 0.0), Align2::LEFT_TOP, name, font.clone(), color);
    }
    let ink = if technique.is_some() { color } else { color.gamma_multiply(0.45) };
    let played = technique.unwrap_or_default();
    let head = head_glyph_for(instrument, played, Duration::Simple(NoteValue::Quarter)).glyph;
    let head_size = 0.8 * h;
    let head_center = Pos2::new(rect.center().x, rect.top() + 0.64 * h);
    paint_glyph_centered(&painter, head, head_center, head_size, &family, ink);
    if let Some(sign) = technique_sign(played) {
        let sign_size = 0.6 * h;
        let ss = sign_size / 4.0;
        let head_top = head_center.y - head.ne[1] * head_size / 4.0;
        let sign_center =
            Pos2::new(rect.center().x, head_top - 0.02 * h - 0.5 * (sign.ne[1] - sign.sw[1]) * ss);
        paint_glyph_centered(&painter, sign, sign_center, sign_size, &family, ink);
    }
    // One dot per technique, where there are several.
    let techniques = instrument.techniques();
    if techniques.len() > 1 {
        let radius = (0.035 * h).max(1.5);
        let step = 0.11 * h;
        let y = rect.bottom() - 0.09 * h;
        let first = rect.center().x - step * (techniques.len() - 1) as f32 / 2.0;
        for (n, t) in techniques.enumerate() {
            let lit = technique == Some(t);
            let dot = if lit { color } else { color.gamma_multiply(0.35) };
            painter.circle_filled(Pos2::new(first + step * n as f32, y), radius, dot);
        }
    }
}

/// `glyph` of the music font at font size `size`, its ink centered on
/// `center`.
fn paint_glyph_centered(
    painter: &egui::Painter,
    glyph: Glyph,
    center: Pos2,
    size: f32,
    family: &FontFamily,
    color: Color32,
) {
    let ss = size / 4.0;
    let origin = Pos2::new(
        center.x - 0.5 * (glyph.sw[0] + glyph.ne[0]) * ss,
        center.y + 0.5 * (glyph.sw[1] + glyph.ne[1]) * ss,
    );
    draw_glyph(painter, &PlacedGlyph { glyph, origin, size }, family, color);
}

/// Size of a [`notation_button`]'s notes `size` high: tuplets and long rows
/// of short notes get a wider tile.
pub(crate) fn notation_size(measure: &Measure, size: f32) -> Vec2 {
    let wide = measure.first_voice().beats().len() > 6
        || measure
            .first_voice()
            .beats()
            .iter()
            .any(|b| b.is_note() && b.duration.as_tuplet_spec().is_some());
    Vec2::new(if wide { 1.5 * size } else { size }, size)
}

/// Draw `measure` as miniature notation fitted into `rect`, as on a
/// [`notation_button`].
pub(crate) fn paint_notation(
    ui: &Ui,
    rect: Rect,
    measure: &Measure,
    family: FontFamily,
    color: Color32,
) {
    let Some(laid_out) = notation_layout(ui, rect, measure, family) else {
        return;
    };
    let (opts, measure_layout) = &*laid_out;
    let painter = &ui.painter_at(rect);
    draw_notes(painter, measure_layout, color, None, opts);
    // A half rest sits on the staff's line, a whole rest hangs from it:
    // without the line the two look alike, so the cell draws it under
    // the one and over the other.
    let stroke = egui::Stroke::new(opts.staff_line_thickness(), color);
    let reach = 0.4 * opts.staff_space();
    for note in &measure_layout.notes {
        if note.kind == Rest
            && matches!(note.duration.base_note(), NoteValue::Half | NoteValue::Whole)
        {
            let (ink, y) = (note.head.ink(), opts.y_of(0));
            painter.hline(ink.left() - reach..=ink.right() + reach, y, stroke);
        }
    }
}

/// The layout [`paint_notation`] draws `measure` with in `rect`, and its
/// options. Fitting it to the tile takes three layouts, and the palette
/// draws dozens of tiles every frame that only change with the window, the
/// zoom or the cursor's bar, so the last few hundred are kept.
fn notation_layout(
    ui: &Ui,
    rect: Rect,
    measure: &Measure,
    family: FontFamily,
) -> Option<Rc<(LayoutOpts, MeasureLayout)>> {
    /// More than the palette, the bar's buttons and the kit's tiles show
    /// at once.
    const KEPT: usize = 256;
    type Kept = VecDeque<(Rect, f32, FontFamily, Measure, Rc<(LayoutOpts, MeasureLayout)>)>;
    thread_local! {
        static KEPT_NOTATION: RefCell<Kept> = const { RefCell::new(VecDeque::new()) };
    }
    let ppp = ui.ctx().pixels_per_point();
    let found = KEPT_NOTATION.with_borrow(|kept| {
        kept.iter()
            .find(|(r, p, f, m, _)| *r == rect && *p == ppp && *f == family && m == measure)
            .map(|(.., laid_out)| Rc::clone(laid_out))
    });
    if found.is_some() {
        return found;
    }
    let laid_out = Rc::new(fit_notation(ui, rect, measure, family.clone())?);
    KEPT_NOTATION.with_borrow_mut(|kept| {
        kept.push_front((rect, ppp, family, measure.clone(), Rc::clone(&laid_out)));
        kept.truncate(KEPT);
    });
    Some(laid_out)
}

/// Lays `measure` out to fill `rect` ([`notation_layout`]).
fn fit_notation(
    ui: &Ui,
    rect: Rect,
    measure: &Measure,
    family: FontFamily,
) -> Option<(LayoutOpts, MeasureLayout)> {
    // Size the glyphs by the first note; rest-only measures use rest sizing.
    let first = measure.first_voice().beats().first()?;
    let template = measure.first_voice().beats().iter().find(|b| b.is_note()).unwrap_or(first);
    let cap_factor = match (template.kind(), template.duration) {
        (Rest, _) => 0.8,
        (Note, Duration::Tuplet(spec)) if matches!(spec.kind(), TupletKind::Nonuplet) => 0.35,
        (Note, Duration::Tuplet(..)) => 0.4,
        _ => 0.7,
    };

    let em = compute_em(&rect, cap_factor, ui);

    let y_offset = match (template.kind(), template.duration) {
        (Rest, _) => 2.0,
        (Note, Duration::Tuplet(..)) => 22.0,
        _ => 20.0,
    };

    let make_opts = |em: f32, rect: Rect| -> LayoutOpts {
        LayoutOpts {
            rect,
            font_id: FontId::new(em, family.clone()),
            pixels_per_point: ui.ctx().pixels_per_point(),
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset,
            stem_length_factor: 0.8,
            stem_thickness_factor: 0.06,
            accent_displacement: 0.1,
            stem_down: false,
            shared_line: false,
            staff: StaffKind::Line,
        }
    };

    // Probe pass: lay the measure out at the size chosen from the
    // template beat, then take the ink of what it draws (note glyphs
    // incl. flags, and tuplet brackets/numbers) instead of
    // trusting the fixed factors above. Scale it to fill the tile
    // along whichever axis it reaches first, and center it: a
    // trailing unbeamed flag or a tuplet number stays inside, a single
    // note grows to the tile's height.
    let content = notes_ink(&build_measure_layout(measure.first_voice(), &make_opts(em, rect)));
    let avail = rect.shrink(NOTATION_MARGIN * rect.height());
    let content_size = Vec2::new((content.width()).max(1.0), (content.height()).max(1.0));
    let scale = (avail.width() / content_size.x).min(avail.height() / content_size.y);
    // No glyph larger than a quarter note filling the tile would be:
    // rests and flags keep their size next to the notes.
    let fitted_em = (em * scale).min(avail.height() / QUARTER_INK_EM);
    // Re-measure at the new size (glyph widths don't scale perfectly
    // linearly) and center the content in the tile.
    let fitted =
        notes_ink(&build_measure_layout(measure.first_voice(), &make_opts(fitted_em, rect)));
    let (final_em, final_rect) = (fitted_em, rect.translate(rect.center() - fitted.center()));

    let opts = make_opts(final_em, final_rect);
    let measure_layout = build_measure_layout(measure.first_voice(), &opts);
    Some((opts, measure_layout))
}

/// Bounding rect (in the same pixel space as `LayoutOpts::rect`) of the ink
/// of everything a measure draws. Used to detect when the fixed sizing
/// factors in [`notation_button`] under- or over-shoot the tile, e.g. a
/// trailing unbeamed flag overhanging the right edge, or a tuplet number
/// close enough to the top of the tile to be clipped.
fn notes_ink(layout: &MeasureLayout) -> Rect {
    layout.notes_ink().unwrap_or(Rect::ZERO)
}

/// An edit a grid cell would make, tried to tell whether the cell does
/// something now ([`Grooph::applies`]).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Trial {
    Modify(Modifier),
    Value(BeatTemplate),
}

/// What the answers of [`Grooph::applies`] depend on. Every state that
/// `modifier_applies`, `value_applies` and `aimed` read must be a field
/// here, or the grid shows an answer from before it changed.
struct TrialState {
    measure: Measure,
    cursor: Cursor,
    instrument: Instrument,
    staff: StaffKind,
}

#[cfg(test)]
mod tests {
    use crate::shell::Tab;
    use crate::tools::{ToolId, all_tools};
    use crate::{Grooph, Mode};
    use grooph_measure::Cursor;
    use grooph_measure::notation::{format_score, parse_score};

    fn editing(score: &str) -> Grooph {
        let cc = crate::test_support::creation_context(eframe::egui::Context::default());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Score);
        app.editor.doc_mut().score = parse_score(score).unwrap();
        app.ui.mode = Mode::Edit;
        app
    }

    /// The grid's answers are kept between frames, and change with the
    /// cursor's bar, the cursor and the staff.
    #[test]
    fn a_cells_answer_follows_the_cursor_the_bar_and_the_staff() {
        use super::Cell;
        let dotted = || Cell::Tool(ToolId::ModifyToggleDotted.tool());
        let mut app = editing("4/4 q q q q");
        app.editor.doc_mut().cursor = Cursor::at(0, 0);
        let first = app.cell_enabled(dotted());
        assert_eq!(app.cell_enabled(dotted()), first);
        // The last beat has no room for a dot.
        app.editor.doc_mut().cursor = Cursor::at(0, 3);
        assert!(!app.cell_enabled(dotted()));
        // A shorter last beat has.
        app.editor.doc_mut().score = parse_score("4/4 q q q e e").unwrap();
        app.editor.doc_mut().cursor = Cursor::at(0, 3);
        assert!(app.cell_enabled(dotted()));
        // The kit, with no head on the beat, takes no accent; the line does.
        let accent = || Cell::Tool(ToolId::ModifyToggleAccent.tool());
        app.editor.doc_mut().score = parse_score("4/4 q q q q").unwrap();
        let on_line = app.cell_enabled(accent());
        app.editor.doc_mut().score = parse_score("kit 4/4 r:q r:q r:q r:q").unwrap();
        assert_ne!(app.cell_enabled(accent()), on_line);
    }

    #[test]
    fn a_tiles_notation_is_kept_until_its_tile_or_bar_changes() {
        use super::notation_layout;
        use eframe::egui::{self, FontFamily, Pos2, Rect};
        use std::rc::Rc;
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let eighths = parse_score("4/4 e e q h").unwrap().measures()[0].clone();
            let quarter = parse_score("4/4 q q q q").unwrap().measures()[0].clone();
            let tile = Rect::from_min_size(Pos2::new(10.0, 10.0), egui::vec2(60.0, 40.0));
            let family = FontFamily::Proportional;
            let at = |rect: Rect, m: &grooph_measure::Measure| {
                notation_layout(ui, rect, m, family.clone()).unwrap()
            };
            let first = at(tile, &eighths);
            assert!(Rc::ptr_eq(&first, &at(tile, &eighths.clone())));
            assert!(!Rc::ptr_eq(&first, &at(tile, &quarter)));
            assert!(!Rc::ptr_eq(&first, &at(tile.translate(egui::vec2(64.0, 0.0)), &eighths)));
            // What is kept is what laying it out again gives.
            let again = super::fit_notation(ui, tile, &eighths, family.clone()).unwrap();
            assert_eq!(format!("{:?}", first.1), format!("{:?}", again.1));
        });
        output.textures_delta.clear();
    }

    #[test]
    fn the_grid_shows_three_lines_and_a_half_at_most() {
        use super::grid_height;
        // Three lines fit whole.
        assert_eq!(grid_height(3, 60.0, 1000.0), 3.0 * 64.0 - 4.0);
        // More are cut half a line below the third.
        assert_eq!(grid_height(6, 60.0, 1000.0), 3.5 * 64.0 - 4.0);
        // A low screen keeps its share for the staff.
        assert_eq!(grid_height(3, 60.0, 300.0), 0.45 * 300.0);
    }

    /// Clean up writes only what the palette can enter, by the measure
    /// crate's lists of the palette's values; they stay the same.
    #[test]
    fn the_palette_offers_the_values_clean_up_writes() {
        use crate::tools::{BeatTemplate, ToolKind};
        use grooph_measure::duration::{Duration, NoteValue};
        use grooph_measure::editing::{TUPLET_TOOLS, VALUE_BUTTONS};
        let templates = || {
            super::TOOLS.iter().filter_map(|&id| match id.tool().kind {
                ToolKind::InsertBeat(BeatTemplate { duration, kind, .. }) => Some((duration, kind)),
                _ => None,
            })
        };
        for kind in [grooph_measure::BeatKind::Note, grooph_measure::BeatKind::Rest] {
            let values: Vec<_> = templates()
                .filter_map(|(d, k)| match d {
                    // The whole is entered by hand only; Clean up never writes it.
                    Duration::Simple(v) if k == kind && v != NoteValue::Whole => Some(v),
                    _ => None,
                })
                .collect();
            // The palette's order is its own: the same values.
            assert_eq!(values.len(), VALUE_BUTTONS.len(), "{kind:?}");
            assert!(VALUE_BUTTONS.iter().all(|v| values.contains(v)), "{kind:?}");
        }
        let tuplets: Vec<_> = templates()
            .filter_map(|(d, _)| match d {
                Duration::Tuplet(spec) => Some(spec),
                _ => None,
            })
            .collect();
        assert_eq!(tuplets, TUPLET_TOOLS);
    }

    fn apply(app: &mut Grooph, id: ToolId) {
        let tool = id.tool();
        app.apply_tool(tool);
    }

    fn text(app: &Grooph) -> String {
        format_score(&app.editor.doc().score).unwrap()
    }

    #[test]
    fn the_tools_edit_the_voice_of_the_cursor() {
        let mut app = editing("4/4 H:q H:q H:q H:q ; K:h K:h");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 1);
        apply(&mut app, ToolId::InsertNoteQ);
        // The kick keeps its head in a new duration; the hands stay.
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; K:h K:q r:q");
        apply(&mut app, ToolId::ModifyToggleRestNote);
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 1, 2));
        assert!(app.editor.doc().score.measures()[0].voice(1).unwrap().beats()[2].is_note());
        apply(&mut app, ToolId::EditUndo);
        apply(&mut app, ToolId::MetaResetMeasure);
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; r:q r:q r:q r:q");
        apply(&mut app, ToolId::EditUndo);
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; K:h K:q r:q");
    }

    #[test]
    fn value_buttons_show_their_value_in_the_cursors_tuplet() {
        use super::Cell;
        use grooph_measure::duration::{NoteValue, TupletSpec};
        let cell = |id: ToolId| Cell::Tool(id.tool());
        let value = |app: &Grooph, id: ToolId| app.value_in_tuplet(id.tool());
        let mut app = editing("4/4 qt16 qt16 qt16 qt16 qt16 r:q r:h");
        app.editor.doc_mut().cursor = Cursor::at(0, 0);
        // In a quintuplet of sixteenths "8" writes a quintuplet eighth.
        let eighth = TupletSpec { n: 5, m: 4, base: NoteValue::Eighth };
        assert_eq!(value(&app, ToolId::InsertNoteE), Some(eighth));
        assert_eq!(value(&app, ToolId::InsertRestE), Some(eighth));
        assert!(app.cell_enabled(cell(ToolId::InsertNoteE)));
        // A quintuplet quarter takes four of its five places.
        assert!(app.cell_enabled(cell(ToolId::InsertNoteQ)));
        assert!(!app.cell_enabled(cell(ToolId::InsertNoteH)));
        // On its last place only a sixteenth or shorter fits.
        app.editor.doc_mut().cursor = Cursor::at(0, 4);
        assert!(!app.cell_enabled(cell(ToolId::InsertNoteE)));
        assert!(app.cell_enabled(cell(ToolId::InsertNoteS)));
        assert!(app.cell_enabled(cell(ToolId::InsertNoteTh)));
        // The tuplet tools stay as they are.
        assert_eq!(value(&app, ToolId::InsertTupletT8), None);
        // Off a tuplet the buttons are plain values, greyed where they do
        // not fit in the bar.
        app.editor.doc_mut().cursor = Cursor::at(0, 5);
        assert_eq!(value(&app, ToolId::InsertNoteE), None);
        assert!(app.cell_enabled(cell(ToolId::InsertNoteH)));
        app.editor.doc_mut().cursor = Cursor::at(0, 6);
        assert!(app.cell_enabled(cell(ToolId::InsertNoteH)));
        let mut last = editing("4/4 q q q q");
        last.editor.doc_mut().cursor = Cursor::at(0, 3);
        assert!(!last.cell_enabled(cell(ToolId::InsertNoteH)));
        assert!(last.cell_enabled(cell(ToolId::InsertNoteQ)));
    }

    #[test]
    fn wholes_are_entered_by_hand_and_never_dotted() {
        let mut app = editing("4/4 q q q q");
        app.editor.doc_mut().cursor = Cursor::at(0, 0);
        apply(&mut app, ToolId::InsertNoteW);
        assert_eq!(text(&app), "4/4 w");
        apply(&mut app, ToolId::InsertRestW);
        assert_eq!(text(&app), "4/4 r:w");
        // The dot has no whole to make.
        apply(&mut app, ToolId::ModifyToggleDotted);
        assert_eq!(text(&app), "4/4 r:w");
        // A whole rest is no value of a bar of another length.
        let mut app = editing("3/4 q q q");
        app.editor.doc_mut().cursor = Cursor::at(0, 0);
        apply(&mut app, ToolId::InsertRestW);
        apply(&mut app, ToolId::InsertNoteW);
        assert_eq!(text(&app), "3/4 q q q");
    }

    #[test]
    fn halves_are_entered_and_dotted() {
        let mut app = editing("4/4 q q q q");
        app.editor.doc_mut().cursor = Cursor::at(0, 0);
        apply(&mut app, ToolId::InsertNoteH);
        assert_eq!(text(&app), "4/4 h q q");
        app.editor.doc_mut().cursor = Cursor::at(0, 1);
        apply(&mut app, ToolId::InsertRestH);
        assert_eq!(text(&app), "4/4 h r:h");
        // And dotted.
        app.editor.doc_mut().cursor = Cursor::at(0, 0);
        apply(&mut app, ToolId::ModifyToggleDotted);
        assert_eq!(text(&app), "4/4 h. r:q");
    }

    #[test]
    fn the_cursor_keeps_its_voice_from_measure_to_measure() {
        let mut app = editing("4/4 H:h H:h ; K:q K:q K:q K:q | H:h H:h ; K:h K:h | q q q q");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 3);
        apply(&mut app, ToolId::NavRight);
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(1, 1, 0));
        apply(&mut app, ToolId::NavLeft);
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 1, 3));
        apply(&mut app, ToolId::NavStart);
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 1, 0));
        // The last measure has one voice.
        apply(&mut app, ToolId::NavEnd);
        assert_eq!(app.editor.doc().cursor, Cursor::at(2, 3));
        // Into a last measure with two, End keeps the voice.
        let mut two = editing("4/4 H:h H:h ; K:q K:q K:q K:q | H:h H:h ; K:h K:h");
        two.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 1);
        apply(&mut two, ToolId::NavEnd);
        assert_eq!(two.editor.doc().cursor, Cursor::in_voice(1, 1, 1));
        // A new measure has the voices of the one before.
        two.push_measure();
        assert_eq!(two.editor.doc().cursor, Cursor::in_voice(2, 1, 0));
        assert_eq!(two.editor.doc().score.measures()[2].voices().len(), 2);
        two.remove_current_measure();
        assert_eq!(two.editor.doc().cursor.voice, 1);
    }

    #[test]
    fn a_line_score_edits_as_before() {
        let mut app = editing("4/4 >q (q) q q");
        app.editor.doc_mut().cursor = Cursor::at(0, 1);
        apply(&mut app, ToolId::InsertNoteE);
        assert_eq!(text(&app), "4/4 >q e r:e q q");
        assert_eq!(app.editor.doc().cursor, Cursor::at(0, 2));
        apply(&mut app, ToolId::MetaResetMeasure);
        assert_eq!(text(&app), "4/4 r:q r:q r:q r:q");
    }

    #[test]
    fn a_sticking_is_set_on_beats_played_by_hand_and_undone() {
        use super::Cell;
        let cell = |id: ToolId| Cell::Tool(id.tool());
        for (score, set) in [
            ("4/4 q q q q", "4/4 q q@RL q q"),
            ("4/4 S-H:q H:q r:h ; K:q r:q K:q r:q", "4/4 S-H:q H:q@RL r:h ; K:q r:q K:q r:q"),
        ] {
            let mut app = editing(score);
            app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 1);
            assert!(app.cell_enabled(cell(ToolId::ModifyStickingRight)), "{score}");
            apply(&mut app, ToolId::ModifyStickingRight);
            apply(&mut app, ToolId::ModifyStickingBoth);
            assert_eq!(text(&app), set);
            assert!(app.cell_selected(cell(ToolId::ModifyStickingBoth)));
            assert!(!app.cell_selected(cell(ToolId::ModifyStickingRight)));
            app.undo();
            assert!(text(&app).contains("q@R"), "{}", text(&app));
            // Tapped again, it goes.
            apply(&mut app, ToolId::ModifyStickingRight);
            assert_eq!(text(&app), score);
        }
        // Not on the kick or a rest.
        let mut app = editing("4/4 S-H:q H:q r:h ; K:q r:q K:q r:q");
        for at in [Cursor::in_voice(0, 1, 0), Cursor::in_voice(0, 1, 1)] {
            app.editor.doc_mut().cursor = at;
            assert!(!app.cell_enabled(cell(ToolId::ModifyStickingLeft)), "{at:?}");
            apply(&mut app, ToolId::ModifyStickingLeft);
            assert!(!text(&app).contains('@'));
        }
    }

    #[test]
    fn the_rock_groove_can_be_entered_on_the_kit() {
        use grooph_measure::Instrument::{HiHat, Kick, Snare};
        let mut app = editing("4/4 q q q q");
        app.new_kit_score();
        assert_eq!(text(&app), "kit 4/4 r:q r:q r:q r:q ; r:q r:q r:q r:q");
        // The hi-hat's eighths, step by step.
        app.pick_instrument(HiHat);
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 0, 0));
        for _ in 0..8 {
            apply(&mut app, ToolId::InsertNoteE);
        }
        // The snare on two and four, on the hi-hat's stem.
        app.pick_instrument(Snare);
        for beat in [1, 5] {
            app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, beat);
            apply(&mut app, ToolId::ModifyToggleHead);
        }
        // The hi-hat opens on the last but one eighth and closes on the last.
        app.pick_instrument(HiHat);
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 6);
        apply(&mut app, ToolId::ModifyCycleTechnique);
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 7);
        for _ in 0..3 {
            apply(&mut app, ToolId::ModifyCycleTechnique);
        }
        // The kick goes below, to the feet's voice, on the beat sounding.
        app.pick_instrument(Kick);
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 1, 3));
        for beat in [0, 2] {
            app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, beat);
            apply(&mut app, ToolId::ModifyToggleRestNote);
        }
        assert_eq!(text(&app), "4/4 H:e S-H:e H:e H:e H:e S-H:e Ho:e Hc:e ; K:q r:q K:q r:q");
        // Accent and ghost change the picked instrument's head alone.
        app.pick_instrument(Snare);
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 1);
        apply(&mut app, ToolId::ModifyToggleAccent);
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 5);
        apply(&mut app, ToolId::ModifyToggleSoft);
        assert!(text(&app).starts_with("4/4 H:e >S-H:e H:e H:e H:e (S)-H:e"), "{}", text(&app));
        // Taking the snare away leaves the hi-hat; the hi-hat's head away
        // on a beat of its own leaves a rest.
        apply(&mut app, ToolId::ModifyToggleHead);
        app.pick_instrument(HiHat);
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 2);
        apply(&mut app, ToolId::ModifyToggleHead);
        assert!(text(&app).starts_with("4/4 H:e >S-H:e r:e H:e H:e H:e"), "{}", text(&app));
        // Every step undoes, back to the empty kit bar.
        while app.can_undo() && app.editor.doc().score.staff == grooph_measure::StaffKind::Kit {
            apply(&mut app, ToolId::EditUndo);
        }
        assert_eq!(text(&app), "4/4 q q q q");
    }

    #[test]
    fn a_drum_is_not_struck_by_both_voices_at_once() {
        use grooph_measure::Instrument::Snare;
        let mut app = editing("4/4 S-H:q H:q H:q H:q ; K:q r:q K:q r:q");
        app.editor.instrument = Snare;
        // The snare is above on the first beat: not below as well.
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 0);
        apply(&mut app, ToolId::ModifyToggleHead);
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 1);
        apply(&mut app, ToolId::ModifyToggleRestNote);
        assert_eq!(text(&app), "4/4 S-H:q H:q H:q H:q ; K:q S:q K:q r:q");
        // Below on the second beat, so not above there.
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 1);
        apply(&mut app, ToolId::ModifyToggleHead);
        assert_eq!(text(&app), "4/4 S-H:q H:q H:q H:q ; K:q S:q K:q r:q");
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 0, 1));
        // A refused edit leaves nothing to undo.
        apply(&mut app, ToolId::EditUndo);
        assert_eq!(text(&app), "4/4 S-H:q H:q H:q H:q ; K:q r:q K:q r:q");

        // A snare in both voices already (links and patterns are loaded
        // without, see `Score::undouble_instruments`): the other edits of
        // the measure go through, a second doubling does not.
        let mut app = editing("4/4 S-H:q H:q H:q H:q ; S:q r:q K:h");
        app.editor.instrument = Snare;
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 3);
        // The accent goes to the head on the beat, the kick's here.
        apply(&mut app, ToolId::ModifyToggleAccent);
        assert_eq!(text(&app), "4/4 S-H:q H:q H:q H:q ; S:q r:q >K:h");
        app.editor.instrument = Snare;
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 3);
        apply(&mut app, ToolId::ModifyToggleHead);
        assert_eq!(text(&app), "4/4 S-H:q H:q H:q S-H:q ; S:q r:q >K:h");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 2);
        apply(&mut app, ToolId::ModifyToggleHead);
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 1);
        apply(&mut app, ToolId::ModifyToggleHead);
        assert_eq!(text(&app), "4/4 S-H:q H:q S-H:q S-H:q ; S:q S:q >K:h");
    }

    #[test]
    fn an_instrument_cell_adds_or_takes_away_its_head() {
        use super::Cell;
        use grooph_measure::Instrument::{HiHat, Kick, Snare};
        let mut app = editing("4/4 H:q H:q H:q H:q ; K:q r:q K:q r:q");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 1);
        let lit = |app: &Grooph, i| app.cell_selected(Cell::Instrument(i));
        assert!(lit(&app, HiHat) && !lit(&app, Snare) && !lit(&app, Kick));
        // The snare joins the hi-hat; the cursor stays.
        app.tap_instrument(Snare);
        assert_eq!(text(&app), "4/4 H:q S-H:q H:q H:q ; K:q r:q K:q r:q");
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 0, 1));
        assert!(lit(&app, HiHat) && lit(&app, Snare) && !lit(&app, Kick));
        // The next taps step it through its techniques, the last takes it away.
        let mut played = vec![];
        for _ in 0..3 {
            played.push(app.technique_at_cursor(Snare));
            app.tap_instrument(Snare);
        }
        use grooph_measure::Technique::{CrossStick, Normal, Rimshot};
        assert_eq!(played, [Some(Normal), Some(CrossStick), Some(Rimshot)]);
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; K:q r:q K:q r:q");
        assert!(!lit(&app, Snare));
        // A long press takes it away however it is played.
        app.tap_instrument(Snare);
        app.tap_instrument(Snare);
        app.hold_instrument(Snare);
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; K:q r:q K:q r:q");
        for _ in 0..3 {
            apply(&mut app, ToolId::EditUndo);
        }
        // The kick goes below, to the feet's beat sounding then, and the
        // cursor goes with it.
        app.tap_instrument(Kick);
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; K:q K:q K:q r:q");
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 1, 1));
        assert!(lit(&app, Kick) && lit(&app, HiHat));
        // Every tap undoes.
        for _ in 0..5 {
            apply(&mut app, ToolId::EditUndo);
        }
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; K:q r:q K:q r:q");
        // A bar without the kick's voice gets it with the kick, in one step.
        let mut app = editing("4/4 r:q r:q r:q r:q");
        app.editor.doc_mut().score.staff = grooph_measure::StaffKind::Kit;
        app.tap_instrument(Kick);
        assert!(text(&app).contains(';'));
        apply(&mut app, ToolId::EditUndo);
        assert_eq!(text(&app), "kit 4/4 r:q r:q r:q r:q");
        // A bar of one voice with notes stays one voice.
        let mut app = editing("4/4 H:q H:q H:q H:q");
        app.tap_instrument(Kick);
        assert_eq!(text(&app), "4/4 K-H:q H:q H:q H:q");
    }

    #[test]
    fn a_mark_goes_to_the_head_it_means() {
        use grooph_measure::Instrument::{HiHat, Kick, Ride, Snare};
        let mut app = editing("4/4 S-H:q H:q H:q H:q ; K:q r:q K:q r:q");
        app.editor.instrument = Kick;
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 0);
        // A technique goes to the head played that way: the hi-hat opens,
        // the snare takes a rimshot, whichever instrument is picked.
        apply(&mut app, ToolId::ModifyTechniqueOpen);
        assert_eq!(text(&app), "4/4 S-Ho:q H:q H:q H:q ; K:q r:q K:q r:q");
        assert_eq!(app.editor.instrument, HiHat);
        apply(&mut app, ToolId::ModifyTechniqueRimshot);
        assert_eq!(text(&app), "4/4 Sr-Ho:q H:q H:q H:q ; K:q r:q K:q r:q");
        assert_eq!(app.editor.instrument, Snare);
        // An accent goes to the picked instrument's head if it is on the
        // beat, else to the snare first.
        app.editor.instrument = HiHat;
        apply(&mut app, ToolId::ModifyToggleAccent);
        assert_eq!(text(&app), "4/4 Sr->Ho:q H:q H:q H:q ; K:q r:q K:q r:q");
        app.editor.instrument = Ride;
        apply(&mut app, ToolId::ModifyToggleSoft);
        assert_eq!(app.editor.instrument, Snare);
        // The cell of the head accent and ghost go to has a ring when there
        // are several.
        assert!(app.is_aimed(Snare) && !app.is_aimed(HiHat));
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 1);
        assert!(!app.is_aimed(HiHat));
    }

    #[test]
    fn the_instruments_row_is_the_same_on_every_beat() {
        use super::{Cell, grid_rows, row_lines};
        // Nine cells, whatever the beat holds: none move when a head comes or goes.
        let row = grid_rows(true, true).remove(0);
        assert_eq!(row, super::KIT_ORDER.map(Cell::Instrument).to_vec());
        for columns in [4, 5, 6, 9] {
            let lines = row_lines(&row, columns);
            assert_eq!(lines.iter().map(|l| l.len()).sum::<usize>(), row.len());
            assert!(lines.iter().all(|l| l.len() <= columns));
        }
    }

    #[test]
    fn the_toggles_show_whether_they_are_on() {
        use super::Cell;
        let mut app = editing("4/4 S:q H:q r:q r:q ; K:q r:q K:q r:q");
        let cell = |id: ToolId| Cell::Tool(id.tool());
        for id in [ToolId::ModifyToggleAccent, ToolId::ModifyToggleSoft, ToolId::ModifyToggleDotted]
        {
            assert!(!app.cell_selected(cell(id)), "{id}");
        }
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 0);
        apply(&mut app, ToolId::ModifyToggleAccent);
        assert!(app.cell_selected(cell(ToolId::ModifyToggleAccent)));
        assert!(!app.cell_selected(cell(ToolId::ModifyToggleSoft)));
        apply(&mut app, ToolId::ModifyToggleSoft);
        assert!(app.cell_selected(cell(ToolId::ModifyToggleSoft)));
        assert!(!app.cell_selected(cell(ToolId::ModifyToggleAccent)));
        apply(&mut app, ToolId::ModifyToggleSoft);
        assert!(!app.cell_selected(cell(ToolId::ModifyToggleSoft)));
    }

    #[test]
    fn the_meter_cell_opens_the_meter_picker_of_the_bar() {
        use crate::pickers::Picker;
        let mut app = editing("4/4 S:q H:q r:q r:q ; K:q r:q K:q r:q");
        apply(&mut app, ToolId::MetaChangeTimeSignature);
        assert_eq!(app.ui.picker, Some(Picker::BarMeter));
        assert_eq!(app.ui.mode, crate::Mode::Edit);
    }

    #[test]
    fn the_grid_offers_every_tool_of_the_palette() {
        use super::{Cell, grid_rows};
        use crate::tools::{Modifier, ToolKind};
        let tools = |kit| {
            grid_rows(kit, true)
                .into_iter()
                .flatten()
                .filter_map(|c| match c {
                    Cell::Tool(t) => Some(t.id),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        // Undo and redo are in the editor's bar instead, clear in the bar menu.
        for t in all_tools().iter().filter(|t| t.show_in_palette) {
            let in_grid = !super::BAR_TOOLS.contains(&t.id);
            let count = tools(true).iter().filter(|&&id| id == t.id).count();
            assert_eq!(count, usize::from(in_grid), "{}", t.id);
            assert_eq!(tools(false).contains(&t.id), in_grid, "{}", t.id);
        }
        // The kit adds its instruments, whose cells add and take away a
        // head: no cell for that tool.
        assert!(!tools(true).contains(&ToolId::ModifyToggleHead));
        assert!(!tools(false).contains(&ToolId::ModifyToggleHead));
        // The techniques are the instruments' cells' too: no cells of their own.
        assert!(
            !tools(true).iter().any(|&id| matches!(
                id.tool().kind,
                ToolKind::Modify(Modifier::ToggleTechnique(_))
            ))
        );
        let cells = grid_rows(true, true).into_iter().flatten().collect::<Vec<_>>();
        let instruments = cells.iter().filter(|c| matches!(c, Cell::Instrument(_))).count();
        assert_eq!(instruments, 9);
        assert!(grid_rows(false, true).iter().flatten().all(|c| matches!(c, Cell::Tool(_))));
        // The metronome's clicks are played by no hand: no stickings there.
        let sticking = |c: &Cell| matches!(c, Cell::Tool(t) if matches!(t.kind, ToolKind::Modify(Modifier::ToggleSticking(_))));
        assert_eq!(grid_rows(false, true).iter().flatten().filter(|c| sticking(c)).count(), 3);
        assert!(!grid_rows(false, false).iter().flatten().any(sticking));
    }

    #[test]
    fn a_technique_is_offered_for_the_instruments_that_play_it() {
        use super::Cell;
        use grooph_measure::Instrument::{HiHat, Kick, Snare};
        let open = all_tools().iter().find(|t| t.id == ToolId::ModifyTechniqueOpen).unwrap();
        let mut app = editing("4/4 S-H:q H:q H:q H:q ; K:q r:q K:q r:q");
        app.editor.instrument = HiHat;
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 1);
        assert!(app.cell_enabled(Cell::Tool(open)));
        assert!(!app.cell_selected(Cell::Tool(open)));
        apply(&mut app, ToolId::ModifyTechniqueOpen);
        assert!(app.cell_selected(Cell::Tool(open)));
        apply(&mut app, ToolId::ModifyTechniqueClosed);
        assert_eq!(text(&app), "4/4 S-H:q Hc:q H:q H:q ; K:q r:q K:q r:q");
        apply(&mut app, ToolId::ModifyTechniqueClosed);
        assert_eq!(text(&app), "4/4 S-H:q H:q H:q H:q ; K:q r:q K:q r:q");
        // A technique is offered where a head played that way is on the
        // beat, whichever instrument is picked: the snare and the kick are
        // not played open, the hi-hat on the beat is.
        for instrument in [Snare, Kick, HiHat] {
            app.editor.instrument = instrument;
            assert!(app.cell_enabled(Cell::Tool(open)));
        }
        let mut bare = editing("4/4 S:q H:q r:q r:q ; K:q r:q K:q r:q");
        for (beat, enabled) in [(0, false), (1, true)] {
            bare.editor.doc_mut().cursor = Cursor::in_voice(0, 0, beat);
            assert_eq!(bare.cell_enabled(Cell::Tool(open)), enabled, "beat {beat}");
        }
        app.editor.instrument = Kick;
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 0);
        apply(&mut app, ToolId::ModifyTechniqueCrossStick);
        assert_eq!(text(&app), "4/4 Sx-H:q H:q H:q H:q ; K:q r:q K:q r:q");
        // The head it went to is the picked one now, and shows as picked.
        assert_eq!(app.editor.instrument, Snare);
        assert!(app.cell_selected(Cell::Instrument(Snare)));
    }

    #[test]
    fn a_modifier_is_offered_where_it_changes_the_beat() {
        use super::Cell;
        use grooph_measure::Instrument::{HiHat, Kick, Snare};
        let cell = |id: ToolId| Cell::Tool(id.tool());
        let mut app = editing("4/4 q r:q q q");
        // An accent, a ghost and a dot on a note, none on a rest.
        for id in [ToolId::ModifyToggleAccent, ToolId::ModifyToggleSoft, ToolId::ModifyToggleDotted]
        {
            assert!(app.cell_enabled(cell(id)), "{id}");
            app.editor.doc_mut().cursor = Cursor::at(0, 1);
            assert_eq!(app.cell_enabled(cell(id)), id == ToolId::ModifyToggleDotted, "{id}");
            app.editor.doc_mut().cursor = Cursor::at(0, 0);
        }
        // No room for a dot on the last beat.
        app.editor.doc_mut().cursor = Cursor::at(0, 3);
        assert!(!app.cell_enabled(cell(ToolId::ModifyToggleDotted)));

        let mut app = editing("4/4 S-H:q H:q H:q H:q ; K:q r:q K:q r:q");
        app.editor.instrument = Snare;
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 1);
        // No snare on the beat, the accent goes to the hi-hat there; no head
        // that plays a rimshot.
        assert!(app.cell_enabled(cell(ToolId::ModifyToggleAccent)));
        assert!(!app.cell_enabled(cell(ToolId::ModifyTechniqueRimshot)));
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 0);
        assert!(app.cell_enabled(cell(ToolId::ModifyTechniqueRimshot)));
        // The hi-hat on the beat opens, whichever instrument is picked.
        app.editor.instrument = Kick;
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 1);
        assert!(app.cell_enabled(cell(ToolId::ModifyTechniqueOpen)));
        assert!(app.cell_enabled(cell(ToolId::ModifyToggleAccent)));
        let _ = HiHat;
    }

    #[test]
    fn a_modifier_shows_the_cursors_beat() {
        use grooph_measure::duration::{Duration, NoteValue};
        let tool = |id: ToolId| id.tool();
        let mut app = editing("4/4 e e e. s h");
        let shown = |app: &Grooph, id| {
            let m = app.modifier_preview(tool(id)).unwrap();
            let b = m.first_voice().beats()[0];
            (b.duration, b.accented(), b.soft())
        };
        let e = Duration::Simple(NoteValue::Eighth);
        let e_dot = Duration::Dotted { base: NoteValue::Eighth, dots: 1 };
        assert_eq!(shown(&app, ToolId::ModifyToggleAccent), (e, true, false));
        assert_eq!(shown(&app, ToolId::ModifyToggleSoft), (e, false, true));
        assert_eq!(shown(&app, ToolId::ModifyToggleDotted), (e_dot, false, false));
        // Accent and ghost mean the note: the beat's dot is left out.
        app.editor.doc_mut().cursor = Cursor::at(0, 2);
        assert_eq!(shown(&app, ToolId::ModifyToggleAccent), (e, true, false));
        assert_eq!(shown(&app, ToolId::ModifyToggleSoft), (e, false, true));
        assert_eq!(shown(&app, ToolId::ModifyToggleDotted), (e_dot, false, false));
        assert!(app.modifier_preview(ToolId::InsertNoteQ.tool()).is_none());
        // A half note shows its accent, and the dot a dotted half.
        app.editor.doc_mut().cursor = Cursor::at(0, 4);
        let h = Duration::Simple(NoteValue::Half);
        let h_dot = Duration::Dotted { base: NoteValue::Half, dots: 1 };
        assert_eq!(shown(&app, ToolId::ModifyToggleAccent), (h, true, false));
        assert_eq!(shown(&app, ToolId::ModifyToggleDotted), (h_dot, false, false));
    }

    #[test]
    fn the_dot_shows_a_rest_where_the_cursor_is_on_one() {
        use grooph_measure::BeatKind;
        let mut app = editing("4/4 e r:e e e q q");
        let preview = |app: &Grooph| {
            let m = app.modifier_preview(ToolId::ModifyToggleDotted.tool()).unwrap();
            m.first_voice().beats()[0].kind()
        };
        app.editor.doc_mut().cursor = Cursor::at(0, 0);
        assert_eq!(preview(&app), BeatKind::Note);
        app.editor.doc_mut().cursor = Cursor::at(0, 1);
        assert_eq!(preview(&app), BeatKind::Rest);
    }

    #[test]
    fn the_cursors_tuplet_is_marked_and_a_tap_dissolves_it() {
        let mut app = editing("4/4 t8 t8 t8 q q q");
        use super::Cell;
        let cell = |id: ToolId| Cell::Tool(id.tool());
        assert!(app.cell_selected(cell(ToolId::InsertTupletT8)));
        assert!(!app.cell_selected(cell(ToolId::InsertTupletT16)));
        assert!(!app.cell_selected(cell(ToolId::InsertNoteE)));
        apply(&mut app, ToolId::InsertTupletT8);
        assert!(!text(&app).contains("t8"), "{}", text(&app));
        assert!(!app.cell_selected(cell(ToolId::InsertTupletT8)));
        // Outside a group, nothing is marked.
        app.editor.doc_mut().cursor = Cursor::at(0, app.current_voice().beats().len() - 1);
        assert!(!app.cell_selected(cell(ToolId::InsertTupletT8)));
    }

    #[test]
    fn the_ghost_bar_adds_a_bar_at_the_end() {
        let mut app = editing("4/4 H:q H:q H:q H:q ; K:h K:h | 3/4 q q q");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 1);
        app.push_measure();
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; K:h K:h | 3/4 q q q | r:q r:q r:q");
        // The cursor goes there, in the voice it can.
        assert_eq!(app.editor.doc().cursor, Cursor::at(2, 0));
        apply(&mut app, ToolId::EditUndo);
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; K:h K:h | 3/4 q q q");
        // Not in the metronome's one bar.
        app.select_tab(Tab::Metronome);
        let bars = app.editor.doc().score.len();
        app.push_measure();
        assert_eq!(app.editor.doc().score.len(), bars);
    }

    #[test]
    fn the_bar_menu_duplicates_clears_and_deletes_the_cursors_bar() {
        let mut app = editing("4/4 H:q H:q H:q H:q ; K:h K:h | 3/4 q q q");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 1);
        app.duplicate_current_measure();
        assert_eq!(
            text(&app),
            "4/4 H:q H:q H:q H:q ; K:h K:h | H:q H:q H:q H:q ; K:h K:h | 3/4 q q q"
        );
        // The cursor goes to the copy, to the same beat.
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(1, 1, 1));
        // Clear empties every voice of the bar.
        app.clear_current_measure();
        assert_eq!(
            text(&app),
            "4/4 H:q H:q H:q H:q ; K:h K:h | r:q r:q r:q r:q ; r:q r:q r:q r:q | 3/4 q q q"
        );
        app.remove_current_measure();
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; K:h K:h | 3/4 q q q");
        apply(&mut app, ToolId::EditUndo);
        apply(&mut app, ToolId::EditUndo);
        apply(&mut app, ToolId::EditUndo);
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; K:h K:h | 3/4 q q q");
        // The metronome's one bar is not duplicated.
        app.select_tab(Tab::Metronome);
        let bars = app.editor.doc().score.len();
        app.duplicate_current_measure();
        assert_eq!(app.editor.doc().score.len(), bars);
    }

    #[test]
    fn the_bar_menu_cleans_up_the_cursors_bar() {
        let mut app = editing("4/4 r:q S:h S:q ; K:h r:e K:e r:q | 3/4 q q q");
        // The cursor on the snare of beat 4 stays on it.
        app.editor.doc_mut().cursor = Cursor::at(0, 2);
        assert!(app.can_clean_up_current_measure());
        app.clean_up_current_measure();
        assert_eq!(text(&app), "4/4 r:q S:q r:q S:q ; K:q r:q r:e K:e r:q | 3/4 q q q");
        assert_eq!(app.editor.doc().cursor, Cursor::at(0, 3));
        assert!(app.editor.doc().dirty);
        // Clean already: nothing to do, nothing to undo.
        assert!(!app.can_clean_up_current_measure());
        app.clean_up_current_measure();
        apply(&mut app, ToolId::EditUndo);
        assert_eq!(text(&app), "4/4 r:q S:h S:q ; K:h r:e K:e r:q | 3/4 q q q");
    }

    #[test]
    fn the_one_line_has_no_instruments_to_pick() {
        let mut app = editing("4/4 q (q) r:q q");
        app.pick_instrument(grooph_measure::Instrument::Kick);
        assert_eq!(app.editor.doc().cursor, Cursor::at(0, 0));
        apply(&mut app, ToolId::ModifyToggleHead);
        apply(&mut app, ToolId::ModifyCycleTechnique);
        apply(&mut app, ToolId::ModifyTechniqueOpen);
        assert_eq!(text(&app), "4/4 q (q) r:q q");
        // The tools write the line's rhythm, as before.
        app.editor.doc_mut().cursor = Cursor::at(0, 2);
        apply(&mut app, ToolId::ModifyToggleRestNote);
        apply(&mut app, ToolId::ModifyToggleAccent);
        assert_eq!(text(&app), "4/4 q (q) >q q");
    }

    #[test]
    fn kit_input_follows_the_voicing_of_the_score() {
        use grooph_measure::Instrument::{HiHat, Kick, Snare};
        use grooph_measure::Voicing;
        // Cymbals above: the snare goes below, with the feet.
        let mut app = editing("4/4 H:q H:q H:q H:q ; K:q r:q K:q r:q");
        app.editor.doc_mut().score.voicing = Voicing::CYMBALS_ABOVE;
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 1);
        app.tap_instrument(Snare);
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; K:q S:q K:q r:q");
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 1, 1));
        // One voice: the kick goes above, into the hands' voice.
        let mut app = editing("4/4 H:q H:q H:q H:q");
        app.editor.doc_mut().score.voicing = Voicing::ONE_VOICE;
        app.tap_instrument(Kick);
        assert_eq!(text(&app), "4/4 K-H:q H:q H:q H:q");
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 0, 0));
        app.pick_instrument(HiHat);
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 0, 0));
        // A new groove has the voices of its rule.
        let groove = crate::kit_palette::empty_groove(
            grooph_measure::TimeSignature::FOUR_FOUR,
            Voicing::ONE_VOICE,
        );
        assert_eq!(groove.measures()[0].voices().len(), 1);
        assert_eq!(groove.voicing, Voicing::ONE_VOICE);
    }

    #[test]
    fn kit_input_follows_the_style_of_the_measure() {
        use grooph_measure::Instrument::{Kick, Snare};
        // The snare is down in this measure, though the rule has it up.
        let mut app = editing("4/4 H:q H:q H:q H:q ; S:q r:q S:q r:q");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 1);
        app.tap_instrument(Snare);
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; S:q S:q S:q r:q");
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 1, 1));
        // So does picking it on a beat where it is not.
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 3);
        app.pick_instrument(Snare);
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 1, 3));
        // A single-voice fill stays one voice: the kick goes above.
        let mut app = editing("4/4 S:s S:s T1:s T1:s T2:s T2:s FT:s FT:s S:q r:q");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 8);
        app.tap_instrument(Kick);
        assert_eq!(app.current_measure().voices().len(), 1);
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 0, 8));
        // An empty measure follows the rule.
        let mut app = editing("4/4 r:q r:q r:q r:q");
        app.editor.doc_mut().score.staff = grooph_measure::StaffKind::Kit;
        app.tap_instrument(Kick);
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 1, 0));
    }

    #[test]
    fn the_rule_of_a_link_comes_from_its_content() {
        use grooph_measure::Voicing;
        let rule = |notation: &str| {
            let mut app = editing(notation);
            app.replace_score_from_link(app.editor.doc().score.clone());
            app.editor.doc().score.voicing
        };
        let hands_feet = "4/4 H:e H:e S-H:e H:e H:e H:e S-H:e H:e ; K:q r:q K:e K:e r:q";
        let one = "4/4 K-H:e H:e S-H:e H:e K-H:e K-H:e S-H:e H:e";
        let cymbals = "4/4 H:e H:e H:e H:e H:e H:e H:e H:e ; K:q S:q K:e K:e S:q";
        assert_eq!(rule(hands_feet), Voicing::HANDS_FEET);
        assert_eq!(rule(one), Voicing::ONE_VOICE);
        assert_eq!(rule(cymbals), Voicing::CYMBALS_ABOVE);
        // The same when the link opens on the practice page (score tab hidden).
        let mut app = editing(cymbals);
        app.replace_practice_from_link(app.editor.doc().score.clone());
        assert_eq!(app.editor.doc().score.voicing, Voicing::CYMBALS_ABOVE);
        // A fill in one voice does not change the rule.
        let fill = format!("{cymbals} | S:s S:s T1:s T1:s T2:s T2:s FT:s FT:s K-CR:q r:q");
        assert_eq!(rule(&fill), Voicing::CYMBALS_ABOVE);
    }
}
