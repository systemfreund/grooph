use crate::app::Mode;
use crate::layout::pixel_layout::{LayoutOpts, build_measure_layout, build_time_sig_layout};
use crate::measure::BeatKind::{Note, Rest};
use crate::measure::Measure;
use crate::measure::duration::{Duration, TupletSpec};
use crate::measure::editing::Modification;
use crate::render::glyphs;
use crate::render::measure::{compute_em, draw_notes};
use crate::{BeatTemplate, Grooph, MetaOp, Tool, ToolGroup, ToolKind, all_tools};
use eframe::egui;
use eframe::egui::scroll_area::{ScrollBarVisibility, ScrollSource};
use eframe::egui::{
    Align, Align2, Atom, Button, Context, Direction, FontId, Id, Label, Layout, Response, RichText,
    Ui, Vec2, Widget,
};
use log::info;

const TOOL_PALETTE_BUTTON_SIZE: f32 = 70.0;
const TOOL_PALETTE_BUTTON_CORNER_RADIUS: f32 = 2.0;

impl Grooph {
    pub(super) fn tool_palette_panel(&mut self, ctx: &Context) {
        egui::TopBottomPanel::bottom("tool_palette")
            .show_separator_line(false)
            .resizable(true)
            .show_animated(ctx, self.mode == Mode::Edit, |ui| {
                let tools = all_tools();
                let groups = [
                    ToolGroup::Edit,
                    ToolGroup::Meta,
                    ToolGroup::Notes,
                    ToolGroup::Tuplets,
                    ToolGroup::Rests,
                ];

                egui::ScrollArea::horizontal()
                    .scroll_source(ScrollSource::ALL)
                    .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        let layout = Layout::from_main_dir_and_cross_align(
                            Direction::LeftToRight,
                            Align::Center,
                        )
                        .with_cross_justify(false);

                        ui.with_layout(layout, |ui| {
                            self.tool_palette(tools, groups.as_slice(), ui);
                        })
                    });
            });
    }

    fn tool_palette(&mut self, tools: &[Tool], groups: &[ToolGroup], ui: &mut Ui) {
        for g in groups {
            let group_tools: Vec<_> = tools.iter().filter(|t| &t.group == g).collect();
            if group_tools.is_empty() {
                continue;
            }

            for t in group_tools {
                match t.kind {
                    ToolKind::InsertBeat(template) => {
                        let button = self.note_button(ui, template, t.id);
                        if button.clicked() {
                            self.apply_tool(t);
                        }
                    }
                    ToolKind::Meta(MetaOp::ChangeTimeSignature) => {
                        let button = self.time_signature_button(ui, t.id);
                        if button.clicked() {
                            self.apply_tool(t);
                        }
                    }
                    _ => {
                        // Generic button
                        // let symbol_id = Id::new(t.id);
                        // let symbol = Atom::custom(symbol_id, Vec2::splat(TOOL_PALETTE_BUTTON_SIZE));
                        // let button =
                        //     Button::new(symbol).corner_radius(TOOL_PALETTE_BUTTON_CORNER_RADIUS).atom_ui(ui);
                        //
                        // if let Some(rect) = button.rect(symbol_id) {
                        //     let painter = ui.painter_at(rect);
                        //     painter.text(rect.center(), Align2::CENTER_CENTER, t.label, )
                        //     add_sized(rect.size(), Label::new(t.label));
                        // }

                        let button = ui.add_sized(
                            Vec2::splat(TOOL_PALETTE_BUTTON_SIZE),
                            Button::new(RichText::new(t.label).size(24.0))
                                .corner_radius(TOOL_PALETTE_BUTTON_CORNER_RADIUS),
                        );
                        if button.clicked() {
                            self.apply_tool(t);
                        }
                    }
                }
            }
        }
    }

    pub(super) fn apply_tool(&mut self, tool: &Tool) {
        // If edit mode is disabled, ignore all tool interactions
        if self.mode != Mode::Edit {
            return;
        }
        // Block palette/tool actions while a modal dialog is open
        if matches!(self.mode, Mode::TimeSignature { .. }) {
            return;
        }
        // Take a snapshot for tools that change state; Meta tools like dialogs drop it.
        self.push_undo();
        let result = match tool.kind {
            ToolKind::InsertBeat(template) => {
                let beats_len = self.measure.beats().len();
                if beats_len == 0 {
                    // No state change, drop the snapshot and return
                    let _ = self.undo_stack.pop();
                    return;
                }
                let idx = self.cursor_idx.min(beats_len - 1);

                match template.duration {
                    Duration::Simple(_) => {
                        self.set_beat(idx, template.duration.base_note(), Some(template.kind))
                    }
                    Duration::Tuplet(spec) => self.set_tuplet(idx, Some(spec)),
                    _ => None,
                }
            }
            ToolKind::Meta(MetaOp::ChangeTimeSignature) => {
                // Opening a dialog is not a state mutation: drop snapshot, open dialog
                let _ = self.undo_stack.pop();
                let ts = self.measure.time_signature();
                self.mode = Mode::TimeSignature { beats: ts.beats, unit: ts.beat_unit };
                return;
            }
            ToolKind::Meta(MetaOp::ResetMeasure) => {
                // Keep the snapshot we took before calling apply_tool
                let ts = self.measure.time_signature();
                self.measure = Measure::new_init(ts, Rest);
                self.cursor_idx = 0;
                Some(Modification::ChangeTimeSignature(ts, ts))
            }
            ToolKind::Edit(crate::tools::EditOp::Undo) => {
                // Undo should not create a new snapshot; drop the one we took and perform undo
                let _ = self.undo_stack.pop();
                self.undo();
                return;
            }
            ToolKind::Edit(crate::tools::EditOp::Redo) => {
                // Redo should not create a new snapshot; drop the one we took and perform redo
                let _ = self.undo_stack.pop();
                self.redo();
                return;
            }
            ToolKind::Modify(modifier) => {
                let beats_len = self.measure.beats().len();
                if beats_len == 0 {
                    let _ = self.undo_stack.pop();
                    return;
                }
                let idx = self.cursor_idx.min(beats_len - 1);
                match modifier {
                    crate::tools::Modifier::ToggleDotted { dots: _ } => {
                        self.measure.toggle_dotted(idx)
                    }
                    crate::tools::Modifier::ToggleAccent => self.measure.toggle_accent(idx),
                    crate::tools::Modifier::ToggleRestNote => self.measure.toggle_beat_kind(idx),
                }
            }
        };

        // Clear redo for real changes; otherwise drop the snapshot we took before
        if result.is_some() {
            self.clear_redo();
        } else {
            let _ = self.undo_stack.pop();
        }

        info!("edited: {:?}", result);
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
            let font_id = FontId::new(em, self.music_font_id.family.clone());

            // Build a minimal layout area for the time signature only
            let opts = LayoutOpts {
                rect,
                font_id: font_id.clone(),
                em,
                layout_clef: false,
                layout_time_signature: true,
                y_offset: 0.0,
                stem_length_factor: 0.9,
                stem_thickness_factor: 0.03,
            };

            // Use a temporary measure just for layout width and positions
            let ts = self.measure.time_signature();
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

    fn note_button(&self, ui: &mut Ui, template: BeatTemplate, id: &str) -> Response {
        let w_factor = match template {
            BeatTemplate { kind: Note, duration: Duration::Tuplet(..) } => 1.5,
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
            // Use a prebuilt measure for this button
            let measure = self.button_measures.get(id).unwrap();

            let cap_factor = match template {
                BeatTemplate { kind: Rest, .. } => 0.8,
                BeatTemplate { kind: Note, duration: Duration::Simple(..) } => 0.7,
                // BeatTemplate {
                //     kind: Note,
                //     duration: Duration::Tuplet(TupletSpec { n: 9, .. }),
                // } => 0.35,
                _ => 0.4,
            };

            let em = compute_em(&rect, cap_factor, ui);

            let y_offset = match template {
                BeatTemplate { kind: Note, duration: Duration::Simple(..) } => 20.0,
                // BeatTemplate {
                //     kind: Note,
                //     duration: Duration::Tuplet(TupletSpec { n: 9, .. }),
                // } => 18.0,
                BeatTemplate { kind: Note, duration: Duration::Tuplet(..) } => 22.0,
                _ => 2.0,
            };

            let opts = LayoutOpts {
                rect,
                font_id: FontId::new(em, self.music_font_id.family.clone()),
                em,
                layout_clef: false,
                layout_time_signature: false,
                y_offset,
                stem_length_factor: 0.8,
                stem_thickness_factor: 0.03,
            };
            let measure_layout = build_measure_layout(measure, &opts);
            let painter = &ui.painter_at(rect);
            draw_notes(painter, &measure_layout, ui.style().visuals.text_color(), &opts);
        }

        button.response
    }
}
