use crate::audio::Waveform;
use crate::Grooph;
use crate::app::Mode;
use eframe::egui;
use eframe::egui::{Align, Direction, Layout, Widget, global_theme_preference_buttons};

impl Grooph {
    pub(super) fn settings_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("settings").show_animated(
            ctx,
            self.mode == Mode::Settings,
            |ui| {
                ui.set_min_height(300.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::CollapsingHeader::new("Click").default_open(false).show(ui, |ui| {
                        ui.label("Decay:");
                        ui.add(
                            egui::DragValue::new(&mut self.mixer.decay)
                                .range(0.01..=0.5)
                                .speed(0.001)
                                .suffix("s"),
                        );
                        ui.separator();
                        egui::ComboBox::from_label("Waveform")
                            .selected_text(match self.mixer.waveform {
                                Waveform::Sine => "Sine",
                                Waveform::Triangle => "Triangle",
                                Waveform::Square => "Square",
                                Waveform::Sawtooth => "Sawtooth",
                                Waveform::Noise => "Noise",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.mixer.waveform, Waveform::Sine, "Sine");
                                ui.selectable_value(&mut self.mixer.waveform, Waveform::Triangle, "Triangle");
                                ui.selectable_value(&mut self.mixer.waveform, Waveform::Square, "Square");
                                ui.selectable_value(&mut self.mixer.waveform, Waveform::Sawtooth, "Sawtooth");
                                ui.selectable_value(&mut self.mixer.waveform, Waveform::Noise, "Noise");
                            });
                        if matches!(self.mixer.waveform, Waveform::Noise) {
                            ui.separator();
                            ui.label("Noise High-Pass Cutoff:");
                            ui.add(
                                egui::DragValue::new(&mut self.mixer.noise_hpf_hz)
                                    .range(2000.0..=6000.0)
                                    .speed(10.0)
                                    .suffix("Hz"),
                            );
                        }
                        ui.separator();
                        ui.label("Base Frequency (PrimaryBeat):");
                        ui.add(
                            egui::DragValue::new(&mut self.mixer.base_frequency)
                                .range(220.0..=880.0)
                                .speed(0.1)
                                .suffix("Hz"),
                        );
                    });

                    egui::CollapsingHeader::new("Audio Latency").default_open(false).show(ui, |ui| {
                        ui.checkbox(&mut self.audio_latency_enabled, "Enabled");
                        ui.add_enabled_ui(self.audio_latency_enabled, |ui| {
                            ui.label("Offset:");
                            ui.add(
                                egui::DragValue::new(&mut self.audio_offset)
                                    .range(-0.5..=0.5)
                                    .speed(0.001)
                                    .suffix("s"),
                            );
                        });
                        ui.label("Adjust if audio is out of sync with cursor.");
                    });

                    egui::CollapsingHeader::new("Developer settings").default_open(false).show(
                        ui,
                        |ui| {
                            ui.label("Size:");
                            ui.add(
                                egui::DragValue::new(&mut self.layout_width_cap_factor)
                                    .range(0.05..=0.5),
                            );
                            ui.separator();
                            ui.label("Stem Length Factor:");
                            ui.add(egui::DragValue::new(&mut self.layout_stem_length_factor).range(0.1..=2.0));
                            ui.separator();
                            ui.horizontal(|ui| {
                                ui.label("Accents Position:");
                                ui.radio_value(&mut self.layout_accent_below, true, "Below");
                                ui.radio_value(&mut self.layout_accent_below, false, "Above");
                            });
                        },
                    );

                    ui.separator();
                    ui.label("Theme:");
                    global_theme_preference_buttons(ui);
                });

            },
        );
    }
}
