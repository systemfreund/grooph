use crate::Grooph;
use crate::kit_palette::{instrument_label, instrument_short};
use crate::pickers::Picker;
use crate::shell::centered_column;
use crate::tools::{BeatTemplate, DeleteOp, MetaOp, Modifier, NavOp, Tool, ToolKind, all_tools};
use crate::{Mode, tools};
use eframe::egui;
use eframe::egui::scroll_area::ScrollBarVisibility;
use eframe::egui::{
    Align2, Atom, Button, Color32, FontFamily, FontId, IdSalt, Pos2, Rect, Response, Sense,
    StrokeKind, Ui, Vec2, WidgetInfo,
};
use grooph_layout::pixel_layout::{
    LayoutOpts, MeasureLayout, build_measure_layout, build_time_sig_layout, compute_em,
};
use grooph_measure::BeatKind::{Note, Rest};
use grooph_measure::duration::{Duration, NoteValue, TupletKind};
use grooph_measure::{Cursor, Instrument, Measure, StaffKind, Technique, VoiceBar};
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
/// How long a technique group takes to open out of its instrument.
const OPEN_SECONDS: f32 = 0.15;
const GRID_GAP: f32 = 4.0;
/// The narrowest a cell gets before the grid takes fewer columns.
const MIN_CELL: f32 = 54.0;
/// The widest a cell gets: on a wide screen the grid stays in the middle.
const MAX_CELL: f32 = 64.0;
/// The tallest a cell gets; narrower ones are square.
const MAX_CELL_HEIGHT: f32 = 64.0;
/// The most columns the grid has, on a wide screen.
const MAX_COLUMNS: usize = 16;
/// The share of the screen's height the grid takes at most; past it the
/// grid scrolls, and the staff above stays in view.
const MAX_GRID_SHARE: f32 = 0.45;

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
pub(crate) const BAR_TOOLS: [&str; 2] = ["edit.undo", "edit.redo"];
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
/// The meter, the head modifiers and the rests: the first row of tools,
/// the kit's modifiers after them.
const HEAD_TOOLS: [&str; 8] = [
    "meta.change_time_signature",
    "modify.toggle.accent",
    "modify.toggle.soft",
    "modify.toggle.dotted",
    "insert.rest.q",
    "insert.rest.e",
    "insert.rest.s",
    "insert.rest.th",
];
/// The note values, then the tuplets, in one row.
const VALUE_TOOLS: [&str; 10] = [
    "insert.note.q",
    "insert.note.e",
    "insert.note.s",
    "insert.note.th",
    "insert.tuplet.t8",
    "insert.tuplet.t16",
    "insert.tuplet.qt16",
    "insert.tuplet.st16",
    "insert.tuplet.spt16",
    "insert.tuplet.nt16",
];

/// One cell of the tool grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Cell {
    Tool(&'static Tool),
    /// Picks the instrument the tools write and change (the kit's).
    Instrument(Instrument),
    /// A technique of the instrument before it, opened out to its right
    /// while that instrument has a head on the beat; a tap aims it there.
    Technique(Instrument, &'static Tool),
}

/// The tool with `id`.
pub(crate) fn tool(id: &str) -> &'static Tool {
    all_tools().iter().find(|t| t.id == id).expect("a tool of the palette")
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
/// next line. On the kit the instruments come first, each followed by the
/// techniques `technique` shows for it; on the one line there are neither.
pub(crate) fn grid_rows(
    kit: bool,
    technique: &dyn Fn(Instrument, &'static Tool) -> bool,
) -> Vec<Vec<Cell>> {
    let tools = |ids: &[&str]| ids.iter().map(|id| Cell::Tool(tool(id))).collect::<Vec<_>>();
    let mut rows = Vec::new();
    if kit {
        let mut instruments = Vec::new();
        for &i in &KIT_ORDER {
            instruments.push(Cell::Instrument(i));
            for t in all_tools() {
                if let ToolKind::Modify(Modifier::ToggleTechnique(tech)) = t.kind
                    && i.allows(tech)
                    && technique(i, t)
                {
                    instruments.push(Cell::Technique(i, t));
                }
            }
        }
        rows.push(instruments);
    }
    rows.push(tools(&HEAD_TOOLS));
    rows.push(tools(&VALUE_TOOLS));
    rows
}

/// `row` as lines of at most `columns` cells: an instrument and its
/// techniques stay in one line unless they are more than a line.
fn row_lines(row: &[Cell], columns: usize) -> Vec<&[Cell]> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < row.len() {
        let mut end = i + 1;
        while end < row.len() && matches!(row[end], Cell::Technique(..)) {
            end += 1;
        }
        if end - start > columns && i > start {
            lines.push(&row[start..i]);
            start = i;
        }
        while end - start > columns {
            lines.push(&row[start..start + columns]);
            start += columns;
        }
        i = end;
    }
    if start < row.len() {
        lines.push(&row[start..]);
    }
    lines
}

/// The grid's columns and the size of its cells, `width` wide.
fn grid_metrics(width: f32) -> (usize, Vec2) {
    let columns =
        (((width + GRID_GAP) / (MIN_CELL + GRID_GAP)).floor() as usize).clamp(1, MAX_COLUMNS);
    let cell = ((width - GRID_GAP * (columns - 1) as f32) / columns as f32).min(MAX_CELL);
    (columns, Vec2::new(cell, cell.min(MAX_CELL_HEIGHT)))
}

impl Grooph {
    /// The editor's tools as a grid in the thumb's reach, the same in every
    /// editor (Practice, Score, the metronome's click bar): on the kit its
    /// instruments, then the meter, the head modifiers and the rests, and
    /// the note values with the tuplets. Undo and redo are in
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
        // The techniques the cursor's beat can take, no others.
        let heads = self.heads_at_cursor();
        let rows =
            grid_rows(!self.staff_model().one_line(), &|i, _| heads.iter().any(|&(h, _)| h == i));
        let (columns, cell) = grid_metrics(ui.available_width());
        let lines: usize = rows.iter().map(|r| row_lines(r, columns).len()).sum();
        let height = lines as f32 * (cell.y + GRID_GAP) - GRID_GAP;
        let height = height.min(MAX_GRID_SHARE * ui.ctx().content_rect().height());
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
        let ctx = ui.ctx().clone();
        let tint = ui.visuals().selection.bg_fill.gamma_multiply(0.6);
        let clip = ui.clip_rect();
        for row in rows {
            for line in row_lines(row, columns) {
                let rect_of = |col: usize| {
                    let min = Pos2::new(left + col as f32 * (cell.x + GRID_GAP), top);
                    Rect::from_min_size(min, cell)
                };
                let mut open = 1.0;
                let mut group_start = 0;
                for (col, &c) in line.iter().enumerate() {
                    let rect = rect_of(col);
                    if let Cell::Instrument(i) = c {
                        group_start = col;
                        // The techniques open out to the right, from their
                        // instrument; the ones to the right just move.
                        let has = matches!(line.get(col + 1), Some(Cell::Technique(..)));
                        open = ctx.animate_bool_with_time(
                            ui.make_persistent_id(("tool_grid_open", i)),
                            has,
                            OPEN_SECONDS,
                        );
                        if has {
                            let last = line[col + 1..]
                                .iter()
                                .take_while(|c| matches!(c, Cell::Technique(..)))
                                .count();
                            let end = rect_of(col + last).right();
                            let shown = rect.right() + (end - rect.right()) * open;
                            let group = Rect::from_min_max(rect.min, Pos2::new(shown, rect.max.y))
                                .expand(GRID_GAP / 2.0);
                            ui.painter().rect_filled(group, 8.0, tint);
                        }
                    }
                    if let Cell::Technique(..) = c {
                        let start = rect_of(group_start).right();
                        let shown = start + (rect.right() - start) * open;
                        ui.set_clip_rect(clip.intersect(Rect::everything_left_of(shown)));
                    }
                    if self.grid_cell(ui, rect, c).clicked() {
                        clicked = Some(c);
                    }
                    ui.set_clip_rect(clip);
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
            Some(Cell::Tool(t)) => self.apply_tool(t),
            Some(Cell::Instrument(i)) => self.tap_instrument(i),
            Some(Cell::Technique(i, t)) => {
                self.editor.instrument = i;
                self.apply_tool(t);
            }
            None => {}
        }
    }

    /// Whether `cell` shows as picked: an instrument with a head on the
    /// cursor's beat, a toggle that is on, the technique of the head a technique's cell means
    /// there, or the tuplet the beat is in.
    fn cell_selected(&self, cell: Cell) -> bool {
        match cell {
            Cell::Instrument(i) => self.heads_at_cursor().iter().any(|&(h, _)| h == i),
            Cell::Technique(i, t) => {
                let ToolKind::Modify(Modifier::ToggleTechnique(tech)) = t.kind else {
                    return false;
                };
                let score = &self.editor.doc().score;
                self.heads_at_cursor().into_iter().any(|(h, at)| {
                    h == i
                        && score
                            .voice_at(at)
                            .beats()
                            .get(at.beat_idx)
                            .and_then(|b| b.heads().get(i))
                            .is_some_and(|h| h.technique() == tech)
                })
            }
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
            // The tuplet the cursor is in: a tap on it dissolves the group.
            Cell::Tool(t) => self.in_tuplet_of(t),
        }
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
                ToolKind::Modify(Modifier::CycleTuplet) => true,
                ToolKind::Modify(modifier) => self.modifier_applies(modifier),
                _ => true,
            },
            Cell::Instrument(_) | Cell::Technique(..) => true,
        }
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

    /// A modifier's cell shows the cursor's beat as it modifies it: its value
    /// with the accent, the ghost note's parentheses or the dot. `None` for
    /// the other tools, and for a value the dot cannot show (a dotted half,
    /// whole or 32nd: the cell keeps its own label).
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
        let duration = match beat.duration {
            _ if dot => Duration::Dotted { base, dots: 1 },
            Duration::Dotted { dots: 1, .. } => Duration::Dotted { base, dots: 1 },
            _ => Duration::Simple(base),
        };
        // A button's measure holds a dotted quarter, eighth or sixteenth.
        if matches!(duration, Duration::Dotted { .. })
            && !matches!(base, NoteValue::Quarter | NoteValue::Eighth | NoteValue::Sixteenth)
        {
            return None;
        }
        let template = BeatTemplate { duration, kind: Note, accented, soft };
        Some(Grooph::build_button_measure(template))
    }

    /// One cell of the grid in `rect`, framed like a button.
    fn grid_cell(&self, ui: &mut Ui, rect: Rect, cell: Cell) -> Response {
        let enabled = self.cell_enabled(cell);
        let selected = self.cell_selected(cell);
        let (key, label, hover) = match cell {
            Cell::Tool(t) | Cell::Technique(_, t) => {
                let hover = match t.kind {
                    ToolKind::Modify(Modifier::ToggleTechnique(technique)) => {
                        technique_name(technique).to_owned()
                    }
                    ToolKind::Modify(Modifier::ToggleHead) => {
                        "Add the instrument here, or take it away".to_owned()
                    }
                    ToolKind::InsertBeat(_) if selected => "Dissolve the tuplet".to_owned(),
                    _ => t.label.to_owned(),
                };
                let hover = match t.shortcuts.first() {
                    Some(sc) => format!("{hover} ({})", sc.label()),
                    None => hover,
                };
                let label = match t.kind {
                    ToolKind::Modify(Modifier::ToggleHead) => "+/−",
                    _ => t.label,
                };
                (t.id, label, hover)
            }
            Cell::Instrument(i) => {
                let hover = match selected {
                    true => format!("Take the {} away", instrument_label(i).to_lowercase()),
                    false => format!("Add the {}", instrument_label(i).to_lowercase()),
                };
                (instrument_short(i), instrument_short(i), hover)
            }
        };
        let parent = match cell {
            Cell::Technique(i, _) => Some(i),
            _ => None,
        };
        let id = ui.make_persistent_id(("tool_grid", key, parent));
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
            Cell::Tool(t) if matches!(button_kind(&t.kind), ButtonKind::Note) => {
                let family = self.ui.music_font_id.family.clone();
                match self.modifier_preview(t) {
                    Some(measure) => paint_notation(ui, inner, &measure, family, color),
                    None => {
                        paint_notation(ui, inner, &self.editor.button_measures[t.id], family, color)
                    }
                }
            }
            Cell::Tool(t) if matches!(button_kind(&t.kind), ButtonKind::TimeSignature) => {
                self.paint_time_signature(ui, inner, color);
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
        draw_time_signature(painter, &layout, ts, &font_id, color);
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
                self.ui.mode = Mode::TimeSignature(self.current_measure().time_signature());
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
            info!("applied tool: {}", tool.id);
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
        // Tools only run in edit mode, and not while a modal dialog is open.
        // Mode::TimeSignature is a sub-state of editing but blocks tool actions.
        matches!(self.ui.mode, Mode::Edit) && !(kit_only && self.staff_model().one_line())
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
                    Duration::Tuplet(spec) => self.set_tuplet(idx, Some(spec)),
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
                match modifier {
                    Modifier::CycleTuplet => self.set_tuplet(idx, None).is_some(),
                    _ => modify(self.current_voice_mut(), idx, pen, modifier),
                }
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
/// `pen`; whether it changed anything. Not for [`Modifier::CycleTuplet`],
/// which needs the score.
fn modify(voice: &mut VoiceBar, idx: usize, pen: Instrument, modifier: Modifier) -> bool {
    match modifier {
        Modifier::ToggleDotted { dots: _ } => voice.toggle_dotted(idx),
        Modifier::ToggleAccent => voice.toggle_accent(idx, pen),
        Modifier::ToggleSoft => voice.toggle_soft(idx, pen),
        Modifier::ToggleRestNote => voice.toggle_beat_kind(idx, pen),
        Modifier::ToggleHead => voice.toggle_head(idx, pen),
        Modifier::CycleTechnique => voice.cycle_technique(idx, pen),
        Modifier::ToggleTechnique(technique) => voice.toggle_technique(idx, pen, technique),
        Modifier::CycleTuplet => None,
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
    // Size the glyphs by the first note; rest-only measures use rest sizing.
    let Some(first) = measure.first_voice().beats().first() else {
        return;
    };
    let template = measure.first_voice().beats().iter().find(|b| b.is_note()).unwrap_or(first);
    {
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
        let painter = &ui.painter_at(rect);
        draw_notes(painter, &measure_layout, color, None, &opts);
    }
}

/// Bounding rect (in the same pixel space as `LayoutOpts::rect`) of the ink
/// of everything a measure draws. Used to detect when the fixed sizing
/// factors in [`notation_button`] under- or over-shoot the tile, e.g. a
/// trailing unbeamed flag overhanging the right edge, or a tuplet number
/// close enough to the top of the tile to be clipped.
fn notes_ink(layout: &MeasureLayout) -> Rect {
    layout.notes_ink().unwrap_or(Rect::ZERO)
}

#[cfg(test)]
mod tests {
    use crate::shell::Tab;
    use crate::tools::all_tools;
    use crate::{Grooph, Mode};
    use grooph_measure::Cursor;
    use grooph_measure::notation::{format_score, parse_score};

    fn editing(score: &str) -> Grooph {
        let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Score);
        app.editor.doc_mut().score = parse_score(score).unwrap();
        app.ui.mode = Mode::Edit;
        app
    }

    fn apply(app: &mut Grooph, id: &str) {
        let tool = all_tools().iter().find(|t| t.id == id).unwrap();
        app.apply_tool(tool);
    }

    fn text(app: &Grooph) -> String {
        format_score(&app.editor.doc().score).unwrap()
    }

    #[test]
    fn the_tools_edit_the_voice_of_the_cursor() {
        let mut app = editing("4/4 H:q H:q H:q H:q ; K:h K:h");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 1);
        apply(&mut app, "insert.note.q");
        // The kick keeps its head in a new duration; the hands stay.
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; K:h K:q r:q");
        apply(&mut app, "modify.toggle.rest_note");
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 1, 2));
        assert!(app.editor.doc().score.measures()[0].voice(1).unwrap().beats()[2].is_note());
        apply(&mut app, "edit.undo");
        apply(&mut app, "meta.reset_measure");
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; r:q r:q r:q r:q");
        apply(&mut app, "edit.undo");
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; K:h K:q r:q");
    }

    #[test]
    fn the_cursor_keeps_its_voice_from_measure_to_measure() {
        let mut app = editing("4/4 H:h H:h ; K:q K:q K:q K:q | H:h H:h ; K:h K:h | q q q q");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 3);
        apply(&mut app, "nav.right");
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(1, 1, 0));
        apply(&mut app, "nav.left");
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 1, 3));
        apply(&mut app, "nav.start");
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 1, 0));
        // The last measure has one voice.
        apply(&mut app, "nav.end");
        assert_eq!(app.editor.doc().cursor, Cursor::at(2, 3));
        // Into a last measure with two, End keeps the voice.
        let mut two = editing("4/4 H:h H:h ; K:q K:q K:q K:q | H:h H:h ; K:h K:h");
        two.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 1);
        apply(&mut two, "nav.end");
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
        apply(&mut app, "insert.note.e");
        assert_eq!(text(&app), "4/4 >q e r:e q q");
        assert_eq!(app.editor.doc().cursor, Cursor::at(0, 2));
        apply(&mut app, "meta.reset_measure");
        assert_eq!(text(&app), "4/4 r:q r:q r:q r:q");
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
            apply(&mut app, "insert.note.e");
        }
        // The snare on two and four, on the hi-hat's stem.
        app.pick_instrument(Snare);
        for beat in [1, 5] {
            app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, beat);
            apply(&mut app, "modify.toggle.head");
        }
        // The hi-hat opens on the last but one eighth and closes on the last.
        app.pick_instrument(HiHat);
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 6);
        apply(&mut app, "modify.cycle_technique");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 7);
        for _ in 0..3 {
            apply(&mut app, "modify.cycle_technique");
        }
        // The kick goes below, to the feet's voice, on the beat sounding.
        app.pick_instrument(Kick);
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 1, 3));
        for beat in [0, 2] {
            app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, beat);
            apply(&mut app, "modify.toggle.rest_note");
        }
        assert_eq!(text(&app), "4/4 H:e S-H:e H:e H:e H:e S-H:e Ho:e Hc:e ; K:q r:q K:q r:q");
        // Accent and ghost change the picked instrument's head alone.
        app.pick_instrument(Snare);
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 1);
        apply(&mut app, "modify.toggle.accent");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 5);
        apply(&mut app, "modify.toggle.soft");
        assert!(text(&app).starts_with("4/4 H:e >S-H:e H:e H:e H:e (S)-H:e"), "{}", text(&app));
        // Taking the snare away leaves the hi-hat; the hi-hat's head away
        // on a beat of its own leaves a rest.
        apply(&mut app, "modify.toggle.head");
        app.pick_instrument(HiHat);
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 2);
        apply(&mut app, "modify.toggle.head");
        assert!(text(&app).starts_with("4/4 H:e >S-H:e r:e H:e H:e H:e"), "{}", text(&app));
        // Every step undoes, back to the empty kit bar.
        while app.can_undo() && app.editor.doc().score.staff == grooph_measure::StaffKind::Kit {
            apply(&mut app, "edit.undo");
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
        apply(&mut app, "modify.toggle.head");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 1);
        apply(&mut app, "modify.toggle.rest_note");
        assert_eq!(text(&app), "4/4 S-H:q H:q H:q H:q ; K:q S:q K:q r:q");
        // Below on the second beat, so not above there.
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 1);
        apply(&mut app, "modify.toggle.head");
        assert_eq!(text(&app), "4/4 S-H:q H:q H:q H:q ; K:q S:q K:q r:q");
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 0, 1));
        // A refused edit leaves nothing to undo.
        apply(&mut app, "edit.undo");
        assert_eq!(text(&app), "4/4 S-H:q H:q H:q H:q ; K:q r:q K:q r:q");

        // A snare in both voices already (links and patterns are loaded
        // without, see `Score::undouble_instruments`): the other edits of
        // the measure go through, a second doubling does not.
        let mut app = editing("4/4 S-H:q H:q H:q H:q ; S:q r:q K:h");
        app.editor.instrument = Snare;
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 3);
        // The accent goes to the head on the beat, the kick's here.
        apply(&mut app, "modify.toggle.accent");
        assert_eq!(text(&app), "4/4 S-H:q H:q H:q H:q ; S:q r:q >K:h");
        app.editor.instrument = Snare;
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 3);
        apply(&mut app, "modify.toggle.head");
        assert_eq!(text(&app), "4/4 S-H:q H:q H:q S-H:q ; S:q r:q >K:h");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 2);
        apply(&mut app, "modify.toggle.head");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 1);
        apply(&mut app, "modify.toggle.head");
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
        // A second tap takes it away again.
        app.tap_instrument(Snare);
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; K:q r:q K:q r:q");
        assert!(!lit(&app, Snare));
        // The kick goes below, to the feet's beat sounding then, and the
        // cursor goes with it.
        app.tap_instrument(Kick);
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; K:q K:q K:q r:q");
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 1, 1));
        assert!(lit(&app, Kick) && lit(&app, HiHat));
        // Every tap undoes.
        for _ in 0..3 {
            apply(&mut app, "edit.undo");
        }
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; K:q r:q K:q r:q");
    }

    #[test]
    fn a_mark_goes_to_the_head_it_means() {
        use grooph_measure::Instrument::{HiHat, Kick, Ride, Snare};
        let mut app = editing("4/4 S-H:q H:q H:q H:q ; K:q r:q K:q r:q");
        app.editor.instrument = Kick;
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 0);
        // A technique goes to the head played that way: the hi-hat opens,
        // the snare takes a rimshot, whichever instrument is picked.
        apply(&mut app, "modify.technique.open");
        assert_eq!(text(&app), "4/4 S-Ho:q H:q H:q H:q ; K:q r:q K:q r:q");
        assert_eq!(app.editor.instrument, HiHat);
        apply(&mut app, "modify.technique.rimshot");
        assert_eq!(text(&app), "4/4 Sr-Ho:q H:q H:q H:q ; K:q r:q K:q r:q");
        assert_eq!(app.editor.instrument, Snare);
        // An accent goes to the picked instrument's head if it is on the
        // beat, else to the snare first.
        app.editor.instrument = HiHat;
        apply(&mut app, "modify.toggle.accent");
        assert_eq!(text(&app), "4/4 Sr->Ho:q H:q H:q H:q ; K:q r:q K:q r:q");
        app.editor.instrument = Ride;
        apply(&mut app, "modify.toggle.soft");
        assert_eq!(app.editor.instrument, Snare);
        // The cell of the head accent and ghost go to has a ring when there
        // are several.
        assert!(app.is_aimed(Snare) && !app.is_aimed(HiHat));
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 1);
        assert!(!app.is_aimed(HiHat));
    }

    #[test]
    fn the_techniques_in_the_instruments_row_are_the_beats() {
        use super::{Cell, grid_rows};
        let app_at = |beat| {
            let mut app = editing("4/4 S:q H:q r:q r:q ; K:q r:q K:q r:q");
            app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, beat);
            app
        };
        let techniques = |app: &Grooph| {
            let heads = app.heads_at_cursor();
            let rows = grid_rows(true, &|i, _| heads.iter().any(|&(h, _)| h == i));
            // The first row: the instruments, each with its techniques after it.
            rows[0]
                .iter()
                .filter_map(|c| match c {
                    Cell::Technique(i, t) => Some((*i, t.id)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        use grooph_measure::Instrument::{HiHat, Snare};
        assert_eq!(
            techniques(&app_at(0)),
            [(Snare, "modify.technique.cross_stick"), (Snare, "modify.technique.rimshot")]
        );
        assert_eq!(
            techniques(&app_at(1)),
            [
                (HiHat, "modify.technique.open"),
                (HiHat, "modify.technique.half_open"),
                (HiHat, "modify.technique.closed")
            ]
        );
        // The row right after the snare's cell is the snare's.
        let rows = grid_rows(true, &|i, _| i == Snare);
        let at = rows[0].iter().position(|c| *c == Cell::Instrument(Snare)).unwrap();
        assert!(matches!(rows[0][at + 1], Cell::Technique(Snare, _)));
        // A rest has none.
        assert!(techniques(&app_at(2)).is_empty());
    }

    #[test]
    fn the_toggles_show_whether_they_are_on() {
        use super::Cell;
        let mut app = editing("4/4 S:q H:q r:q r:q ; K:q r:q K:q r:q");
        let cell = |id| Cell::Tool(super::tool(id));
        for id in ["modify.toggle.accent", "modify.toggle.soft", "modify.toggle.dotted"] {
            assert!(!app.cell_selected(cell(id)), "{id}");
        }
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 0);
        apply(&mut app, "modify.toggle.accent");
        assert!(app.cell_selected(cell("modify.toggle.accent")));
        assert!(!app.cell_selected(cell("modify.toggle.soft")));
        apply(&mut app, "modify.toggle.soft");
        assert!(app.cell_selected(cell("modify.toggle.soft")));
        assert!(!app.cell_selected(cell("modify.toggle.accent")));
        apply(&mut app, "modify.toggle.soft");
        assert!(!app.cell_selected(cell("modify.toggle.soft")));
    }

    #[test]
    fn an_instrument_and_its_techniques_stay_in_a_line() {
        use super::{Cell, grid_rows, row_lines};
        let row = grid_rows(true, &|_, _| true).remove(0);
        for columns in [4, 5, 6, 9] {
            let lines = row_lines(&row, columns);
            assert_eq!(lines.iter().map(|l| l.len()).sum::<usize>(), row.len());
            for line in &lines {
                assert!(line.len() <= columns);
                // Never starts on a technique unless its group is longer than a line.
                if matches!(line[0], Cell::Technique(..)) {
                    assert!(columns < 5);
                }
            }
        }
    }

    #[test]
    fn the_grid_offers_every_tool_of_the_palette() {
        use super::{Cell, grid_rows};
        use crate::tools::{Modifier, ToolKind};
        let tools = |kit| {
            grid_rows(kit, &|_, _| true)
                .into_iter()
                .flatten()
                .filter_map(|c| match c {
                    Cell::Tool(t) | Cell::Technique(_, t) => Some(t.id),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let kit_only = |t: &crate::tools::Tool| {
            matches!(t.kind, ToolKind::Modify(Modifier::ToggleTechnique(_)))
        };
        // Undo and redo are in the editor's bar instead, clear in the bar menu.
        for t in all_tools().iter().filter(|t| t.show_in_palette) {
            let in_grid = !super::BAR_TOOLS.contains(&t.id);
            let count = tools(true).iter().filter(|&&id| id == t.id).count();
            // A technique has a cell for every instrument that plays it.
            if kit_only(t) {
                assert!(count >= 1, "{}", t.id);
            } else {
                assert_eq!(count, usize::from(in_grid), "{}", t.id);
            }
            assert_eq!(tools(false).contains(&t.id), in_grid && !kit_only(t), "{}", t.id);
        }
        // The kit adds its instruments, whose cells add and take away a
        // head: no cell for that tool.
        assert!(!tools(true).contains(&"modify.toggle.head"));
        assert!(!tools(false).contains(&"modify.toggle.head"));
        let cells = grid_rows(true, &|_, _| true).into_iter().flatten().collect::<Vec<_>>();
        let instruments = cells.iter().filter(|c| matches!(c, Cell::Instrument(_))).count();
        assert_eq!(instruments, 9);
        assert!(
            grid_rows(false, &|_, _| true).iter().flatten().all(|c| matches!(c, Cell::Tool(_)))
        );
    }

    #[test]
    fn a_technique_is_offered_for_the_instruments_that_play_it() {
        use super::Cell;
        use grooph_measure::Instrument::{HiHat, Kick, Snare};
        let open = all_tools().iter().find(|t| t.id == "modify.technique.open").unwrap();
        let mut app = editing("4/4 S-H:q H:q H:q H:q ; K:q r:q K:q r:q");
        app.editor.instrument = HiHat;
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 1);
        assert!(app.cell_enabled(Cell::Tool(open)));
        assert!(!app.cell_selected(Cell::Tool(open)));
        apply(&mut app, "modify.technique.open");
        assert!(app.cell_selected(Cell::Tool(open)));
        apply(&mut app, "modify.technique.closed");
        assert_eq!(text(&app), "4/4 S-H:q Hc:q H:q H:q ; K:q r:q K:q r:q");
        apply(&mut app, "modify.technique.closed");
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
        apply(&mut app, "modify.technique.cross_stick");
        assert_eq!(text(&app), "4/4 Sx-H:q H:q H:q H:q ; K:q r:q K:q r:q");
        // The head it went to is the picked one now, and shows as picked.
        assert_eq!(app.editor.instrument, Snare);
        assert!(app.cell_selected(Cell::Instrument(Snare)));
    }

    #[test]
    fn a_modifier_is_offered_where_it_changes_the_beat() {
        use super::Cell;
        use grooph_measure::Instrument::{HiHat, Kick, Snare};
        let cell = |id| Cell::Tool(all_tools().iter().find(|t| t.id == id).unwrap());
        let mut app = editing("4/4 q r:q q q");
        // An accent, a ghost and a dot on a note, none on a rest.
        for id in ["modify.toggle.accent", "modify.toggle.soft", "modify.toggle.dotted"] {
            assert!(app.cell_enabled(cell(id)), "{id}");
            app.editor.doc_mut().cursor = Cursor::at(0, 1);
            assert_eq!(app.cell_enabled(cell(id)), id == "modify.toggle.dotted", "{id}");
            app.editor.doc_mut().cursor = Cursor::at(0, 0);
        }
        // No room for a dot on the last beat.
        app.editor.doc_mut().cursor = Cursor::at(0, 3);
        assert!(!app.cell_enabled(cell("modify.toggle.dotted")));

        let mut app = editing("4/4 S-H:q H:q H:q H:q ; K:q r:q K:q r:q");
        app.editor.instrument = Snare;
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 1);
        // No snare on the beat, the accent goes to the hi-hat there; no head
        // that plays a rimshot.
        assert!(app.cell_enabled(cell("modify.toggle.accent")));
        assert!(!app.cell_enabled(cell("modify.technique.rimshot")));
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 0);
        assert!(app.cell_enabled(cell("modify.technique.rimshot")));
        // The hi-hat on the beat opens, whichever instrument is picked.
        app.editor.instrument = Kick;
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 1);
        assert!(app.cell_enabled(cell("modify.technique.open")));
        assert!(app.cell_enabled(cell("modify.toggle.accent")));
        let _ = HiHat;
    }

    #[test]
    fn a_modifier_shows_the_cursors_beat() {
        use grooph_measure::duration::{Duration, NoteValue};
        let tool = |id| all_tools().iter().find(|t| t.id == id).unwrap();
        let mut app = editing("4/4 e e e. s h");
        let shown = |app: &Grooph, id| {
            let m = app.modifier_preview(tool(id)).unwrap();
            let b = m.first_voice().beats()[0];
            (b.duration, b.accented(), b.soft())
        };
        let e = Duration::Simple(NoteValue::Eighth);
        let e_dot = Duration::Dotted { base: NoteValue::Eighth, dots: 1 };
        assert_eq!(shown(&app, "modify.toggle.accent"), (e, true, false));
        assert_eq!(shown(&app, "modify.toggle.soft"), (e, false, true));
        assert_eq!(shown(&app, "modify.toggle.dotted"), (e_dot, false, false));
        // A dotted beat keeps its dot under the accent.
        app.editor.doc_mut().cursor = Cursor::at(0, 2);
        assert_eq!(shown(&app, "modify.toggle.accent"), (e_dot, true, false));
        assert!(app.modifier_preview(tool("insert.note.q")).is_none());
        // A half note shows its accent; the dot keeps its own label, as a
        // button cannot hold a dotted half.
        app.editor.doc_mut().cursor = Cursor::at(0, 4);
        let h = Duration::Simple(NoteValue::Half);
        assert_eq!(shown(&app, "modify.toggle.accent"), (h, true, false));
        assert!(app.modifier_preview(tool("modify.toggle.dotted")).is_none());
    }

    #[test]
    fn the_cursors_tuplet_is_marked_and_a_tap_dissolves_it() {
        let mut app = editing("4/4 t8 t8 t8 q q q");
        use super::{Cell, tool};
        let cell = |id| Cell::Tool(tool(id));
        assert!(app.cell_selected(cell("insert.tuplet.t8")));
        assert!(!app.cell_selected(cell("insert.tuplet.t16")));
        assert!(!app.cell_selected(cell("insert.note.e")));
        apply(&mut app, "insert.tuplet.t8");
        assert!(!text(&app).contains("t8"), "{}", text(&app));
        assert!(!app.cell_selected(cell("insert.tuplet.t8")));
        // Outside a group, nothing is marked.
        app.editor.doc_mut().cursor = Cursor::at(0, app.current_voice().beats().len() - 1);
        assert!(!app.cell_selected(cell("insert.tuplet.t8")));
    }

    #[test]
    fn the_ghost_bar_adds_a_bar_at_the_end() {
        let mut app = editing("4/4 H:q H:q H:q H:q ; K:h K:h | 3/4 q q q");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 1);
        app.push_measure();
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; K:h K:h | 3/4 q q q | r:q r:q r:q");
        // The cursor goes there, in the voice it can.
        assert_eq!(app.editor.doc().cursor, Cursor::at(2, 0));
        apply(&mut app, "edit.undo");
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
        apply(&mut app, "edit.undo");
        apply(&mut app, "edit.undo");
        apply(&mut app, "edit.undo");
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; K:h K:h | 3/4 q q q");
        // The metronome's one bar is not duplicated.
        app.select_tab(Tab::Metronome);
        let bars = app.editor.doc().score.len();
        app.duplicate_current_measure();
        assert_eq!(app.editor.doc().score.len(), bars);
    }

    #[test]
    fn the_one_line_has_no_instruments_to_pick() {
        let mut app = editing("4/4 q (q) r:q q");
        app.pick_instrument(grooph_measure::Instrument::Kick);
        assert_eq!(app.editor.doc().cursor, Cursor::at(0, 0));
        apply(&mut app, "modify.toggle.head");
        apply(&mut app, "modify.cycle_technique");
        apply(&mut app, "modify.technique.open");
        assert_eq!(text(&app), "4/4 q (q) r:q q");
        // The tools write the line's rhythm, as before.
        app.editor.doc_mut().cursor = Cursor::at(0, 2);
        apply(&mut app, "modify.toggle.rest_note");
        apply(&mut app, "modify.toggle.accent");
        assert_eq!(text(&app), "4/4 q (q) >q q");
    }
}
