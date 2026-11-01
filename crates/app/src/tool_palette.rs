use crate::Grooph;
use crate::kit_palette::{instrument_label, instrument_short};
use crate::pickers::Picker;
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
pub(crate) const BAR_TOOLS: [&str; 3] = ["edit.undo", "edit.redo", "meta.reset_measure"];
/// The head modifiers' tools, after the meter and before the kit's.
const HEAD_TOOLS: [&str; 4] = [
    "meta.change_time_signature",
    "modify.toggle.accent",
    "modify.toggle.soft",
    "modify.toggle.dotted",
];
/// The note values, then their rests.
const VALUE_TOOLS: [&str; 8] = [
    "insert.note.q",
    "insert.note.e",
    "insert.note.s",
    "insert.note.th",
    "insert.rest.q",
    "insert.rest.e",
    "insert.rest.s",
    "insert.rest.th",
];
/// The tuplets.
const TUPLET_TOOLS: [&str; 6] = [
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
/// next line. On the kit the instruments come first and the
/// head modifiers get the kit's; on the one line they are left out.
pub(crate) fn grid_rows(kit: bool) -> Vec<Vec<Cell>> {
    let tools = |ids: &[&str]| ids.iter().map(|id| Cell::Tool(tool(id))).collect::<Vec<_>>();
    let mut heads = tools(&HEAD_TOOLS);
    let mut rows = Vec::new();
    if kit {
        rows.push(KIT_ORDER.iter().map(|&i| Cell::Instrument(i)).collect());
        heads.push(Cell::Tool(tool("modify.toggle.head")));
        heads.extend(
            all_tools()
                .iter()
                .filter(|t| matches!(t.kind, ToolKind::Modify(Modifier::ToggleTechnique(_))))
                .map(Cell::Tool),
        );
    }
    rows.push(heads);
    rows.push(tools(&VALUE_TOOLS));
    rows.push(tools(&TUPLET_TOOLS));
    rows
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
    /// instruments, then the meter and the head modifiers, the
    /// note values and rests, and the tuplets. Undo, redo and clear are in
    /// the editor's bar ([`BAR_TOOLS`]).
    pub(super) fn tool_palette_panel(&mut self, ui: &mut Ui) {
        egui::Panel::bottom("tool_palette")
            .show_separator_line(false)
            .resizable(false)
            .show_collapsible(ui, &mut (self.ui.mode == Mode::Edit), |ui| {
                // Sized up front: a bottom panel offers the height it had.
                let rows = grid_rows(!self.staff_model().one_line());
                let (columns, cell) = grid_metrics(ui.available_width());
                let lines: usize = rows.iter().map(|r| r.len().div_ceil(columns)).sum();
                let height = lines as f32 * (cell.y + GRID_GAP) - GRID_GAP;
                let height = height.min(MAX_GRID_SHARE * ui.ctx().content_rect().height());
                egui::ScrollArea::vertical()
                    .id_salt("tool_grid")
                    .min_scrolled_height(height)
                    .max_height(height)
                    .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| self.tool_grid(ui, &rows));
            });
    }

    fn tool_grid(&mut self, ui: &mut Ui, rows: &[Vec<Cell>]) {
        let width = ui.available_width();
        let (columns, cell) = grid_metrics(width);
        let used = columns as f32 * cell.x + (columns - 1) as f32 * GRID_GAP;
        let left = ui.max_rect().left() + (width - used) / 2.0;
        let mut top = ui.cursor().top();
        let mut clicked = None;
        for row in rows {
            for line in row.chunks(columns) {
                for (col, &c) in line.iter().enumerate() {
                    let min = Pos2::new(left + col as f32 * (cell.x + GRID_GAP), top);
                    if self.grid_cell(ui, Rect::from_min_size(min, cell), c).clicked() {
                        clicked = Some(c);
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
            Some(Cell::Tool(t)) => self.apply_tool(t),
            Some(Cell::Instrument(i)) => self.pick_instrument(i),
            None => {}
        }
    }

    /// Whether `cell` shows as picked: the instrument, the voice, or the
    /// technique of the picked instrument's head on the cursor's beat.
    fn cell_selected(&self, cell: Cell) -> bool {
        match cell {
            Cell::Instrument(i) => i == self.edit_instrument(),
            Cell::Tool(Tool { kind: ToolKind::Modify(Modifier::ToggleTechnique(t)), .. }) => {
                let cursor = self.editor.doc().cursor;
                self.current_voice()
                    .beats()
                    .get(cursor.beat_idx)
                    .and_then(|b| b.heads().get(self.edit_instrument()))
                    .is_some_and(|h| h.technique() == *t)
            }
            Cell::Tool(_) => false,
        }
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
            Cell::Instrument(_) => true,
        }
    }

    /// Whether `modifier` changes the cursor's beat: an accent needs a head
    /// of the picked instrument there, not a rest; the dot a value it fits
    /// in the bar; a technique a head its instrument plays that way; "+/−"
    /// a beat with room, without a drum struck by both voices. Tried on a
    /// copy of the cursor's bar.
    fn modifier_applies(&self, modifier: Modifier) -> bool {
        let cursor = self.editor.doc().score.clamp_cursor(self.editor.doc().cursor);
        let mut measure = self.current_measure().clone();
        let Some(voice) = measure.voice_mut(cursor.voice) else {
            return false;
        };
        if voice.beats().is_empty() {
            return false;
        }
        let idx = cursor.beat_idx.min(voice.beats().len() - 1);
        if !modify(voice, idx, self.edit_instrument(), modifier) {
            return false;
        }
        // The kit refuses an edit that has two voices strike one drum.
        let before = self.current_measure().doubled_instruments();
        self.staff_model().one_line()
            || measure.doubled_instruments().iter().all(|d| before.contains(d))
    }

    /// A modifier's cell shows the cursor's beat as it modifies it: its value
    /// with the accent, the ghost note's parentheses or the dot. `None` for
    /// the other tools, and for a value the dot cannot show (a dotted 32nd).
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
        if matches!(duration, Duration::Dotted { .. }) && base == NoteValue::ThirtySecond {
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
            Cell::Tool(t) => {
                let hover = match t.kind {
                    ToolKind::Modify(Modifier::ToggleTechnique(technique)) => {
                        technique_name(technique).to_owned()
                    }
                    ToolKind::Modify(Modifier::ToggleHead) => {
                        "Add the instrument here, or take it away".to_owned()
                    }
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
                (instrument_short(i), instrument_short(i), instrument_label(i).to_owned())
            }
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
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 0);
        app.append_measure();
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(1, 1, 0));
        assert_eq!(app.editor.doc().score.measures()[1].voices().len(), 2);
        app.remove_current_measure();
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(1, 1, 0));
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
        apply(&mut app, "modify.toggle.accent");
        assert_eq!(text(&app), "4/4 S-H:q H:q H:q H:q ; S:q r:q K:h");
        apply(&mut app, "modify.toggle.head");
        assert_eq!(text(&app), "4/4 S-H:q H:q H:q S-H:q ; S:q r:q K:h");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 2);
        apply(&mut app, "modify.toggle.head");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 1);
        apply(&mut app, "modify.toggle.head");
        assert_eq!(text(&app), "4/4 S-H:q H:q S-H:q S-H:q ; S:q S:q K:h");
    }

    #[test]
    fn the_grid_offers_every_tool_of_the_palette() {
        use super::{Cell, grid_rows};
        use crate::tools::{Modifier, ToolKind};
        let tools = |kit| {
            grid_rows(kit)
                .into_iter()
                .flatten()
                .filter_map(|c| match c {
                    Cell::Tool(t) => Some(t.id),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let kit_only = |t: &crate::tools::Tool| {
            matches!(t.kind, ToolKind::Modify(Modifier::ToggleTechnique(_)))
        };
        // Undo, redo and clear are in the editor's bar instead.
        for t in all_tools().iter().filter(|t| t.show_in_palette) {
            let in_grid = !super::BAR_TOOLS.contains(&t.id);
            let count = tools(true).iter().filter(|&&id| id == t.id).count();
            assert_eq!(count, usize::from(in_grid), "{}", t.id);
            assert_eq!(tools(false).contains(&t.id), in_grid && !kit_only(t), "{}", t.id);
        }
        // The kit adds its instruments and "+/−".
        assert!(tools(true).contains(&"modify.toggle.head"));
        assert!(!tools(false).contains(&"modify.toggle.head"));
        let cells = grid_rows(true).into_iter().flatten().collect::<Vec<_>>();
        let instruments = cells.iter().filter(|c| matches!(c, Cell::Instrument(_))).count();
        assert_eq!(instruments, 9);
        assert!(grid_rows(false).iter().flatten().all(|c| matches!(c, Cell::Tool(_))));
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
        // The snare and the kick are not played open.
        for instrument in [Snare, Kick] {
            app.editor.instrument = instrument;
            assert!(!app.cell_enabled(Cell::Tool(open)));
        }
        app.editor.instrument = Snare;
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 0);
        apply(&mut app, "modify.technique.cross_stick");
        assert_eq!(text(&app), "4/4 Sx-H:q H:q H:q H:q ; K:q r:q K:q r:q");
        // The picked instrument shows as picked.
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
        // No snare on the beat to accent, but one to add.
        assert!(!app.cell_enabled(cell("modify.toggle.accent")));
        assert!(app.cell_enabled(cell("modify.toggle.head")));
        assert!(!app.cell_enabled(cell("modify.technique.rimshot")));
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 0);
        assert!(app.cell_enabled(cell("modify.technique.rimshot")));
        // Not the snare below where it is above already.
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 0);
        assert!(!app.cell_enabled(cell("modify.toggle.head")));
        // The hi-hat opens where it is; the kick has no such technique.
        app.editor.instrument = HiHat;
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 1);
        assert!(app.cell_enabled(cell("modify.technique.open")));
        app.editor.instrument = Kick;
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 0);
        assert!(!app.cell_enabled(cell("modify.technique.open")));
        assert!(app.cell_enabled(cell("modify.toggle.accent")));
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
