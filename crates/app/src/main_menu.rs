use crate::Grooph;
use crate::{Mode, TransportState};
use eframe::egui;
use eframe::egui::scroll_area::{ScrollBarVisibility, ScrollSource};
use eframe::egui::{Align, Button, Direction, Frame, Layout, Margin};
use egui::Widget;
use grooph_measure::swing::{MAX_SWING_PERCENT, MIN_SWING_PERCENT, SwingUnit};

impl Grooph {
    pub(super) fn main_menu(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("menu")
            .frame(Frame::side_top_panel(ui.style()).inner_margin(Margin::same(15)))
            .show_separator_line(false)
            .show_inside(ui, |ui| {
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
                            let is_running =
                                self.playback_ctl.transport_state != TransportState::Stopped;
                            let button_label = if is_running { "⏹" } else { "⏵" };
                            if Button::new(button_label).selected(is_running).ui(ui).clicked() {
                                self.toggle_playback();
                            }
                            if ui
                                .selectable_label(self.playback_ctl.audio_cfg.count_in, "🔢")
                                .on_hover_text("Count one bar in before playback starts")
                                .clicked()
                            {
                                self.playback_ctl.audio_cfg.count_in =
                                    !self.playback_ctl.audio_cfg.count_in;
                            }
                            let bpm_editor = egui::DragValue::new(&mut self.playback_ctl.bpm)
                                .prefix("BPM: ")
                                .range(20..=300)
                                .speed(0.03);
                            let bpm_editor_resp = bpm_editor.ui(ui);
                            if bpm_editor_resp.clicked() {
                                ui.memory_mut(|mem| mem.surrender_focus(bpm_editor_resp.id))
                            }
                            if bpm_editor_resp.changed() {
                                self.editor.dirty = true;
                                self.handle_bpm_change();
                            }
                            self.swing_editor(ui);

                            ui.separator();
                            if ui.selectable_label(self.ui.mode == Mode::Edit, "🖊").clicked() {
                                self.toggle_mode(Mode::Edit);
                            }
                            // ui.selectable_label(
                            //     false,
                            //     Image::new(include_image!("../assets/metronome_dark.svg"))
                            //         .tint(ui.style().visuals.text_color()),
                            // )
                            // .clicked();
                            if ui.selectable_label(self.ui.mode == Mode::Mixer, "🔈").clicked() {
                                self.toggle_mode(Mode::Mixer);
                            }
                            if ui.selectable_label(self.ui.mode == Mode::Settings, "⚙").clicked()
                            {
                                self.toggle_mode(Mode::Settings);
                            }
                            if ui
                                .selectable_label(self.ui.mode == Mode::Generator, "🎲")
                                .on_hover_text("Rhythm generator")
                                .clicked()
                            {
                                self.toggle_mode(Mode::Generator);
                            }
                            if ui.selectable_label(self.ui.mode == Mode::Library, "📁").clicked()
                            {
                                self.toggle_mode(Mode::Library);
                            }
                            if ui
                                .selectable_label(self.playback_ctl.accuracy.enabled, "🎯")
                                .clicked()
                            {
                                self.set_accuracy_enabled(!self.playback_ctl.accuracy.enabled);
                            }
                            if ui.selectable_label(self.ui.mode == Mode::Help, "?").clicked() {
                                self.toggle_mode(Mode::Help);
                            }

                            ui.separator();
                            let measure_label = format!(
                                "{}/{}",
                                self.editor.cursor.measure_idx + 1,
                                self.editor.score.len()
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

    /// Swing amount and the swung note value. Applies to all playback, the
    /// editor as well as the generator.
    fn swing_editor(&mut self, ui: &mut egui::Ui) {
        let swing = &mut self.playback_ctl.audio_cfg.swing;
        let resp = egui::DragValue::new(&mut swing.percent)
            .range(MIN_SWING_PERCENT..=MAX_SWING_PERCENT)
            .speed(0.05)
            .custom_formatter(|v, _| {
                if v <= MIN_SWING_PERCENT as f64 {
                    "Swing: off".to_owned()
                } else {
                    format!("Swing: {v:.0}%")
                }
            })
            .custom_parser(|s| {
                let s = s.trim().trim_start_matches("Swing:").trim().trim_end_matches('%');
                if s.eq_ignore_ascii_case("off") {
                    Some(MIN_SWING_PERCENT as f64)
                } else {
                    s.trim().parse().ok()
                }
            })
            .ui(ui)
            .on_hover_text(
                "Delays every second note of a pair: 50% is straight, 66% triplet feel, 75% dotted",
            );
        if resp.clicked() {
            ui.memory_mut(|mem| mem.surrender_focus(resp.id));
        }
        if !swing.is_straight() {
            let next = match swing.unit {
                SwingUnit::Eighths => SwingUnit::Sixteenths,
                SwingUnit::Sixteenths => SwingUnit::Eighths,
            };
            if Button::new(swing.unit.label())
                .ui(ui)
                .on_hover_text("Swung note value (click to switch)")
                .clicked()
            {
                swing.unit = next;
            }
        }
    }
}
