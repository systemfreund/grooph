use crate::Grooph;
use crate::Mode;
use eframe::egui;
use eframe::egui::{Align, Align2, Layout};
use grooph_measure::TimeSignature;

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
                        if let Mode::TimeSignature { beats, unit } = &mut self.mode {
                            egui::ComboBox::from_id_salt("beats")
                                .selected_text(format!("{}", *beats))
                                .show_ui(ui, |ui| {
                                    for v in 1u8..=17u8 {
                                        ui.selectable_value(beats, v, format!("{}", v));
                                    }
                                });
                            ui.label(" / ");
                            egui::ComboBox::from_id_salt("beat_unit")
                                .selected_text(format!("{}", *unit))
                                .show_ui(ui, |ui| {
                                    for v in [4u8, 8, 16] {
                                        ui.selectable_value(unit, v, format!("{}", v));
                                    }
                                });
                        }
                    });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.mode = Mode::Edit;
                        }
                        if ui.button("Done").clicked() {
                            // Prevent no-op undo entries
                            let current = self.current_measure().time_signature();
                            let (beats, unit) = match &self.mode {
                                Mode::TimeSignature { beats, unit } => (*beats, *unit),
                                _ => (current.beats, current.beat_unit),
                            };
                            let new_ts = TimeSignature { beats, beat_unit: unit };
                            if new_ts == current {
                                self.mode = Mode::Edit;
                                return;
                            }

                            let committed = self.with_undo_snapshot(|g| {
                                if g.current_measure_mut().set_time_signature(new_ts).is_err() {
                                    return false;
                                }
                                let new_len = g.current_measure().beats().len();
                                if new_len > 0 {
                                    g.cursor.beat_idx = g.cursor.beat_idx.min(new_len - 1);
                                } else {
                                    g.cursor.beat_idx = 0;
                                }
                                true
                            });
                            if committed {
                                self.mode = Mode::Edit;
                            }
                        }
                    });
                });
            });
    }
}
