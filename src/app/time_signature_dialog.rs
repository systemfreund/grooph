use crate::Grooph;
use crate::measure::TimeSignature;
use eframe::egui;
use eframe::egui::{Align, Align2, Layout};

impl Grooph {
    pub(super) fn time_signature_dialog(&mut self, ctx: &egui::Context) {
        egui::Window::new("Change time signature")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                let layout = Layout::top_down(Align::Center).with_cross_align(Align::Center);
                ui.with_layout(layout, |ui| {
                    let l2 = Layout::left_to_right(Align::Min);
                    ui.with_layout(l2, |ui| {
                        egui::ComboBox::from_id_salt("beats")
                            .selected_text(format!("{}", self.ts_beats))
                            .show_ui(ui, |ui| {
                                for v in 1u8..=16u8 {
                                    ui.selectable_value(&mut self.ts_beats, v, format!("{}", v));
                                }
                            });
                        ui.label(" / ");
                        egui::ComboBox::from_id_salt("beat_unit")
                            .selected_text(format!("{}", self.ts_unit))
                            .show_ui(ui, |ui| {
                                for v in [4u8, 8, 16] {
                                    ui.selectable_value(&mut self.ts_unit, v, format!("{}", v));
                                }
                            });
                    });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.show_ts_dialog = false;
                        }
                        if ui.button("Done").clicked() {
                            // Prevent no-op undo entries
                            let current = self.measure.time_signature();
                            let new_ts =
                                TimeSignature { beats: self.ts_beats, beat_unit: self.ts_unit };
                            if new_ts == current {
                                self.show_ts_dialog = false;
                                return;
                            }

                            // Snapshot before change
                            self.push_undo();
                            let res = self.measure.set_time_signature(new_ts);
                            match res {
                                Ok(_) => {
                                    self.clear_redo();
                                    // Clamp cursor within bounds
                                    let new_len = self.measure.beats().len();
                                    if new_len > 0 {
                                        self.cursor_idx = self.cursor_idx.min(new_len - 1);
                                    } else {
                                        self.cursor_idx = 0;
                                    }
                                    self.show_ts_dialog = false;
                                }
                                Err(_) => {
                                    // Roll back the snapshot if failed
                                    let _ = self.undo_stack.pop();
                                }
                            }
                        }
                    });
                });
            });
    }
}
