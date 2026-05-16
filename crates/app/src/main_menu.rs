use crate::Grooph;
use crate::{Mode, TransportState};
use eframe::egui;
use eframe::egui::scroll_area::{ScrollBarVisibility, ScrollSource};
use eframe::egui::{Align, Button, Direction, Frame, Layout, Margin};
use egui::Widget;

impl Grooph {
    pub(super) fn main_menu(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("menu")
            .frame(Frame::side_top_panel(&ctx.style()).inner_margin(Margin::same(15)))
            .show_separator_line(false)
            .show(ctx, |ui| {
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
                            // Playback controls
                            let is_running = self.transport_state != TransportState::Stopped;
                            let button_label = if is_running { "⏹" } else { "⏵" };
                            if Button::new(button_label).selected(is_running).ui(ui).clicked() {
                                self.toggle_playback();
                            }
                            let bpm_editor = egui::DragValue::new(&mut self.bpm)
                                .prefix("BPM: ")
                                .range(20..=300)
                                .speed(0.03);
                            let bpm_editor_resp = bpm_editor.ui(ui);
                            if bpm_editor_resp.clicked() {
                                ui.memory_mut(|mem| mem.surrender_focus(bpm_editor_resp.id))
                            }
                            if bpm_editor_resp.changed() {
                                self.handle_bpm_change();
                            }

                            ui.separator();
                            if ui.selectable_label(self.mode == Mode::Edit, "🖊").clicked() {
                                self.toggle_mode(Mode::Edit);
                            }
                            // ui.selectable_label(
                            //     false,
                            //     Image::new(include_image!("../assets/metronome_dark.svg"))
                            //         .tint(ui.style().visuals.text_color()),
                            // )
                            // .clicked();
                            if ui.selectable_label(self.mode == Mode::Mixer, "🔈").clicked() {
                                self.toggle_mode(Mode::Mixer);
                            }
                            if ui.selectable_label(self.mode == Mode::Settings, "⚙").clicked() {
                                self.toggle_mode(Mode::Settings);
                            }
                            if ui.selectable_label(self.accuracy.enabled, "🎯").clicked() {
                                self.set_accuracy_enabled(!self.accuracy.enabled);
                            }
                            if ui.selectable_label(self.mode == Mode::Help, "?").clicked() {
                                self.toggle_mode(Mode::Help);
                            }

                            ui.separator();
                            let measure_label = format!(
                                "{}/{}",
                                self.cursor.measure_idx + 1,
                                self.score.len()
                            );
                            ui.label(measure_label);
                            if Button::new("➕").ui(ui).clicked() {
                                self.append_measure();
                            }
                            if Button::new("➖").ui(ui).clicked() {
                                self.remove_current_measure();
                            }
                        });
                    });
            });
    }
}
