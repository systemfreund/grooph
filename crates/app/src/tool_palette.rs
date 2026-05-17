use crate::Grooph;
use crate::tools::{DeleteOp, MetaOp, Modifier, NavOp, Tool, ToolGroup, ToolKind, all_tools};
use crate::{Mode, tools};
use eframe::egui;
use eframe::egui::scroll_area::{ScrollBarVisibility, ScrollSource};
use eframe::egui::{
    Align, Align2, Atom, Button, Direction, FontId, Id, Label, Layout, Response, RichText, Ui,
    Vec2, Widget,
};
use grooph_layout::glyphs;
use grooph_layout::pixel_layout::{
    LayoutOpts, build_measure_layout, build_time_sig_layout, compute_em,
};
use grooph_measure::BeatKind::{Note, Rest};
use grooph_measure::duration::{Duration, TupletKind};
use grooph_measure::{Beat, Measure};
use grooph_render::measure::draw_notes;
use grooph_render::measure_glyph_metrics;
use log::info;
use tools::EditOp;

const TOOL_PALETTE_BUTTON_SIZE: f32 = 70.0;
const TOOL_PALETTE_BUTTON_CORNER_RADIUS: f32 = 2.0;

enum ButtonKind {
    /// Renders a miniature notation using `note_button`.
    Note,
    /// Renders a stacked time-signature glyph.
    TimeSignature,
    /// Plain text/symbol button with optional disabled state.
    Plain,
}

fn button_kind(kind: &ToolKind) -> ButtonKind {
    match kind {
        ToolKind::InsertBeat(..)
        | ToolKind::Modify(Modifier::ToggleDotted { .. })
        | ToolKind::Modify(Modifier::ToggleAccent) => ButtonKind::Note,
        ToolKind::Meta(MetaOp::ChangeTimeSignature) => ButtonKind::TimeSignature,
        _ => ButtonKind::Plain,
    }
}

impl Grooph {
    pub(super) fn tool_palette_panel(&mut self, ui: &mut Ui) {
        egui::Panel::bottom("tool_palette")
            .show_separator_line(false)
            .resizable(false)
            .show_animated_inside(ui, self.ui.mode == Mode::Edit, |ui| {
                let tools = all_tools().iter().filter(|t| t.show_in_palette).collect::<Vec<_>>();
                let groups = [
                    ToolGroup::Edit,
                    ToolGroup::Meta,
                    ToolGroup::Notes,
                    ToolGroup::Modifiers,
                    ToolGroup::Rests,
                    ToolGroup::Tuplets,
                ];

                egui::ScrollArea::horizontal()
                    .scroll_source(ScrollSource::ALL)
                    .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        let layout = Layout::from_main_dir_and_cross_align(
                            Direction::LeftToRight,
                            Align::Center,
                        )
                        .with_cross_justify(true);

                        ui.with_layout(layout, |ui| {
                            self.tool_palette(tools.to_vec(), groups.as_slice(), ui);
                        })
                    });
            });
    }

    fn tool_palette(&mut self, tools: Vec<&Tool>, groups: &[ToolGroup], ui: &mut Ui) {
        for g in groups {
            for t in tools.iter().filter(|t| &t.group == g) {
                let response = match button_kind(&t.kind) {
                    ButtonKind::Note => self.note_button(ui, t.id),
                    ButtonKind::TimeSignature => self.time_signature_button(ui, t.id),
                    ButtonKind::Plain => self.plain_tool_button(ui, t),
                };
                if response.clicked() {
                    self.apply_tool(t);
                }
            }
        }
    }

    fn plain_tool_button(&self, ui: &mut Ui, tool: &Tool) -> Response {
        let enabled = match tool.kind {
            ToolKind::Edit(EditOp::Undo) => self.can_undo(),
            ToolKind::Edit(EditOp::Redo) => self.can_redo(),
            _ => true,
        };
        ui.add_enabled_ui(enabled, |ui| {
            ui.add_sized(
                Vec2::splat(TOOL_PALETTE_BUTTON_SIZE),
                Button::new(RichText::new(tool.label).size(24.0))
                    .corner_radius(TOOL_PALETTE_BUTTON_CORNER_RADIUS),
            )
        })
        .inner
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
            ToolKind::Meta(MetaOp::ChangeTimeSignature) => {
                let ts = self.current_measure().time_signature();
                self.ui.mode = Mode::TimeSignature { beats: ts.beats, unit: ts.beat_unit };
                return;
            }
            ToolKind::Navigate(op) => {
                self.execute_navigation(op);
                return;
            }
            _ => {}
        }

        debug_assert!(tool.kind.is_mutating());

        let committed = self.with_undo_snapshot(|g| g.execute_mutating_tool(tool));
        if committed {
            info!("applied tool: {}", tool.id);
        }
    }

    /// Gate for any tool dispatch — single source of truth, regardless of trigger (palette / keyboard / panel).
    fn tool_applicable(&self, _tool: &Tool) -> bool {
        // Tools only run in edit mode, and not while a modal dialog is open.
        // Mode::TimeSignature is a sub-state of editing but blocks tool actions.
        matches!(self.ui.mode, Mode::Edit)
    }

    /// Moves the cursor according to `op`. Non-mutating; never produces an undo snapshot.
    /// Left/Right cross measure boundaries; Home/End jump to the very first/last beat in the score.
    fn execute_navigation(&mut self, op: NavOp) {
        let measure_count = self.editor.score.len();
        match op {
            NavOp::Left => {
                if self.editor.cursor.beat_idx > 0 {
                    self.editor.cursor.beat_idx -= 1;
                } else if self.editor.cursor.measure_idx > 0 {
                    self.editor.cursor.measure_idx -= 1;
                    let prev_len = self.current_measure().beats().len();
                    self.editor.cursor.beat_idx = prev_len.saturating_sub(1);
                }
            }
            NavOp::Right => {
                let beats_len = self.current_measure().beats().len();
                let max_idx = beats_len.saturating_sub(1);
                if self.editor.cursor.beat_idx < max_idx {
                    self.editor.cursor.beat_idx += 1;
                } else if self.editor.cursor.measure_idx + 1 < measure_count {
                    self.editor.cursor.measure_idx += 1;
                    self.editor.cursor.beat_idx = 0;
                }
            }
            NavOp::Start => {
                self.editor.cursor.measure_idx = 0;
                self.editor.cursor.beat_idx = 0;
            }
            NavOp::End => {
                self.editor.cursor.measure_idx = measure_count.saturating_sub(1);
                let beats_len = self.current_measure().beats().len();
                self.editor.cursor.beat_idx = beats_len.saturating_sub(1);
            }
        }
    }

    /// Executes a tool that is known to be mutating. Returns whether a change was committed.
    /// Must only be called from within `with_undo_snapshot`.
    fn execute_mutating_tool(&mut self, tool: &Tool) -> bool {
        let beats_len = self.current_measure().beats().len();
        match tool.kind {
            ToolKind::InsertBeat(template) => {
                if beats_len == 0 {
                    return false;
                }
                let idx = self.editor.cursor.beat_idx.min(beats_len - 1);
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
                let ts = self.current_measure().time_signature();
                *self.current_measure_mut() = Measure::new_init(ts, Rest);
                self.editor.cursor.beat_idx = 0;
                true
            }
            ToolKind::Modify(modifier) => {
                if beats_len == 0 {
                    return false;
                }
                let idx = self.editor.cursor.beat_idx.min(beats_len - 1);
                let result = match modifier {
                    Modifier::ToggleDotted { dots: _ } => {
                        self.current_measure_mut().toggle_dotted(idx)
                    }
                    Modifier::ToggleAccent => self.current_measure_mut().toggle_accent(idx),
                    Modifier::ToggleRestNote => self.current_measure_mut().toggle_beat_kind(idx),
                    Modifier::CycleTuplet => self.set_tuplet(idx, None),
                };
                result.is_some()
            }
            ToolKind::Delete(op) => {
                if beats_len == 0 {
                    return false;
                }
                let idx = self.editor.cursor.beat_idx.min(beats_len - 1);
                self.current_measure_mut().remove(idx);
                let new_len = self.current_measure().beats().len();
                self.editor.cursor.beat_idx = if new_len == 0 {
                    0
                } else {
                    let last = new_len - 1;
                    match op {
                        DeleteOp::Forward => (self.editor.cursor.beat_idx + 1).min(last),
                        DeleteOp::Backward => self.editor.cursor.beat_idx.saturating_sub(1).min(last),
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

    fn time_signature_button(&self, ui: &mut Ui, id: &str) -> Response {
        let symbol_id = Id::new(id);
        let symbol = Atom::custom(symbol_id, Vec2::splat(TOOL_PALETTE_BUTTON_SIZE));
        let button =
            Button::new(symbol).corner_radius(TOOL_PALETTE_BUTTON_CORNER_RADIUS).atom_ui(ui);

        if let Some(rect) = button.rect(symbol_id) {
            // Render a stacked 4/4 symbol using Bravura, similar to measure rendering
            let painter = &ui.painter_at(rect);
            let em = compute_em(&rect, 0.7, ui);
            let font_id = FontId::new(em, self.ui.music_font_id.family.clone());

            // Build a minimal layout area for the time signature only
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
                accent_below: false,
                proportional_spacing: true,
                debug_bbox: false,
                metrics: measure_glyph_metrics(ui, &font_id),
            };

            // Use a temporary measure just for layout width and positions
            let ts = self.current_measure().time_signature();
            let layout = build_time_sig_layout(&ts, rect.center().x, &opts);
            let top = glyphs::ts_glyphs(ts.beats);
            let bot = glyphs::ts_glyphs(ts.beat_unit);
            for (p, ch) in layout.beats.iter().zip(top.iter()) {
                painter.text(
                    *p,
                    Align2::CENTER_CENTER,
                    ch.to_string(),
                    font_id.clone(),
                    ui.style().visuals.text_color(),
                );
            }
            for (p, ch) in layout.beat_unit.iter().zip(bot.iter()) {
                painter.text(
                    *p,
                    Align2::CENTER_CENTER,
                    ch.to_string(),
                    font_id.clone(),
                    ui.style().visuals.text_color(),
                );
            }
        }

        button.response
    }

    fn note_button(&self, ui: &mut Ui, id: &str) -> Response {
        let measure = self.editor.button_measures.get(id).unwrap();
        let template = measure.beats().first().unwrap();
        let w_factor = match template {
            Beat { kind: Note, duration: Duration::Tuplet(..), .. } => 1.5,
            _ => 1.0,
        };
        let symbol_id = Id::new(id);
        let symbol = Atom::custom(
            symbol_id,
            Vec2::new(TOOL_PALETTE_BUTTON_SIZE * w_factor, TOOL_PALETTE_BUTTON_SIZE),
        );
        let button =
            Button::new(symbol).corner_radius(TOOL_PALETTE_BUTTON_CORNER_RADIUS).atom_ui(ui);

        if let Some(rect) = button.rect(symbol_id) {
            let cap_factor = match template {
                Beat { kind: Rest, .. } => 0.8,
                Beat { kind: Note, duration: Duration::Tuplet(spec), .. }
                    if matches!(spec.kind(), TupletKind::Nonuplet) =>
                {
                    0.35
                }
                Beat { kind: Note, duration: Duration::Tuplet(..), .. } => 0.4,
                _ => 0.7,
            };

            let em = compute_em(&rect, cap_factor, ui);

            let y_offset = match template {
                Beat { kind: Rest, .. } => 2.0,
                Beat { kind: Note, duration: Duration::Tuplet(..), .. } => 22.0,
                _ => 20.0,
            };

            let font_id = FontId::new(em, self.ui.music_font_id.family.clone());

            let opts = LayoutOpts {
                rect,
                font_id: font_id.clone(),
                pixels_per_point: ui.ctx().pixels_per_point(),
                em,
                layout_clef: false,
                layout_time_signature: false,
                y_offset,
                stem_length_factor: 0.8,
                stem_thickness_factor: 0.06,
                accent_displacement: 0.1,
                accent_below: false,
                proportional_spacing: true,
                debug_bbox: false,
                metrics: measure_glyph_metrics(ui, &font_id),
            };
            let measure_layout = build_measure_layout(measure, &opts);
            let painter = &ui.painter_at(rect);
            draw_notes(painter, &measure_layout, ui.style().visuals.text_color(), &opts);
        }

        button.response
    }
}
