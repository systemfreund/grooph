use crate::Grooph;
use crate::Mode;
use eframe::egui;
use eframe::egui::{Align, Align2, Layout};
use grooph_measure::TimeSignature;

impl Grooph {
    pub(super) fn time_signature_dialog(&mut self, ui: &mut egui::Ui) {
        egui::Window::new("Change time signature")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ui.ctx(), |ui| {
                let layout = Layout::top_down(Align::Center).with_cross_align(Align::Center);
                ui.with_layout(layout, |ui| {
                    let l2 = Layout::left_to_right(Align::Min);
                    ui.with_layout(l2, |ui| {
                        if let Mode::TimeSignature(TimeSignature {
                            beats,
                            beat_unit: unit,
                            grouping,
                        }) = &mut self.ui.mode
                        {
                            let range = TimeSignature::beat_range(*unit)
                                .unwrap_or(1..=TimeSignature::MAX_BEATS);
                            egui::ComboBox::from_id_salt("beats")
                                .selected_text(format!("{}", *beats))
                                .show_ui(ui, |ui| {
                                    for v in range {
                                        ui.selectable_value(beats, v, format!("{}", v));
                                    }
                                });
                            ui.label(" / ");
                            let before = (*beats, *unit);
                            egui::ComboBox::from_id_salt("beat_unit")
                                .selected_text(format!("{}", *unit))
                                .show_ui(ui, |ui| {
                                    for v in TimeSignature::BEAT_UNITS {
                                        ui.selectable_value(unit, v, format!("{}", v));
                                    }
                                });
                            // Keep the beats within what the new unit offers.
                            if *unit != before.1
                                && let Some(range) = TimeSignature::beat_range(*unit)
                            {
                                *beats = (*beats).clamp(*range.start(), *range.end());
                            }
                            // Another meter starts in its usual grouping.
                            if (*beats, *unit) != before {
                                *grouping = 0;
                            }
                            let meter = TimeSignature::new(*beats, *unit);
                            let count = meter.groupings().len() as u8;
                            if count > 1 {
                                ui.label("  ");
                                egui::ComboBox::from_id_salt("grouping")
                                    .selected_text(meter.with_grouping(*grouping).groups_label())
                                    .show_ui(ui, |ui| {
                                        for i in 0..count {
                                            let label = meter.with_grouping(i).groups_label();
                                            ui.selectable_value(grouping, i, label);
                                        }
                                    })
                                    .response
                                    .on_hover_text("Beat grouping: beams, clicks and counting");
                            }
                        }
                    });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.ui.mode = Mode::Edit;
                        }
                        if ui.button("Done").clicked() {
                            // Prevent no-op undo entries
                            let current = self.current_measure().time_signature();
                            let new_ts = match &self.ui.mode {
                                Mode::TimeSignature(ts) => *ts,
                                _ => current,
                            };
                            if new_ts == current {
                                self.ui.mode = Mode::Edit;
                                return;
                            }

                            let committed = self.with_undo_snapshot(|g| {
                                if g.current_measure_mut().set_time_signature(new_ts).is_err() {
                                    return false;
                                }
                                let new_len = g.current_measure().beats().len();
                                if new_len > 0 {
                                    g.editor.cursor.beat_idx =
                                        g.editor.cursor.beat_idx.min(new_len - 1);
                                } else {
                                    g.editor.cursor.beat_idx = 0;
                                }
                                true
                            });
                            if committed {
                                self.follow_meter(new_ts);
                                self.ui.mode = Mode::Edit;
                            }
                        }
                    });
                });
            });
    }
}
