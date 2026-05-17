use crate::Grooph;
use crate::midi_input_widget::midi_input_widget;
use crate::{CountingBase, Mode};
use eframe::egui;
use eframe::egui::global_theme_preference_buttons;
use grooph_audio::Waveform;

impl Grooph {
    pub(super) fn settings_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("settings").resizable(true).show_animated_inside(
            ui,
            self.ui.mode == Mode::Settings,
            |ui| {
                ui.set_min_height(300.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::CollapsingHeader::new("Click").default_open(false).show(ui, |ui| {
                        ui.label("Decay:");
                        ui.add(
                            egui::DragValue::new(&mut self.playback_ctl.audio_cfg.settings.decay)
                                .range(0.01..=0.5)
                                .speed(0.001)
                                .suffix("s"),
                        );
                        ui.separator();
                        egui::ComboBox::from_label("Waveform")
                            .selected_text(match self.playback_ctl.audio_cfg.settings.waveform {
                                Waveform::Sine => "Sine",
                                Waveform::Triangle => "Triangle",
                                Waveform::Square => "Square",
                                Waveform::Sawtooth => "Sawtooth",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.playback_ctl.audio_cfg.settings.waveform,
                                    Waveform::Sine,
                                    "Sine",
                                );
                                ui.selectable_value(
                                    &mut self.playback_ctl.audio_cfg.settings.waveform,
                                    Waveform::Triangle,
                                    "Triangle",
                                );
                                ui.selectable_value(
                                    &mut self.playback_ctl.audio_cfg.settings.waveform,
                                    Waveform::Square,
                                    "Square",
                                );
                                ui.selectable_value(
                                    &mut self.playback_ctl.audio_cfg.settings.waveform,
                                    Waveform::Sawtooth,
                                    "Sawtooth",
                                );
                            });
                        ui.separator();
                        ui.label("Noise Mix:");
                        ui.add(
                            egui::Slider::new(&mut self.playback_ctl.audio_cfg.settings.noise_mix, 0.0..=1.0)
                                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                        );
                        if self.playback_ctl.audio_cfg.settings.noise_mix > 0.0 {
                            ui.separator();
                            ui.label("Noise High-Pass Cutoff:");
                            ui.add(
                                egui::DragValue::new(&mut self.playback_ctl.audio_cfg.settings.noise_hpf_hz)
                                    .range(2000.0..=6000.0)
                                    .speed(10.0)
                                    .suffix(" Hz"),
                            );
                            ui.separator();
                            ui.label("Noise Decay:");
                            ui.add(
                                egui::DragValue::new(&mut self.playback_ctl.audio_cfg.settings.noise_decay)
                                    .range(0.01..=0.5)
                                    .speed(0.001)
                                    .suffix("s"),
                            );
                        }
                        ui.separator();
                        ui.label("Base Frequency (PrimaryBeat):");
                        ui.add(
                            egui::DragValue::new(&mut self.playback_ctl.audio_cfg.settings.base_frequency)
                                .range(220.0..=880.0)
                                .speed(0.1)
                                .suffix("Hz"),
                        );
                    });

                    egui::CollapsingHeader::new("Audio Latency").default_open(false).show(
                        ui,
                        |ui| {
                            ui.checkbox(&mut self.playback_ctl.audio_cfg.latency_enabled, "Enabled");
                            ui.add_enabled_ui(self.playback_ctl.audio_cfg.latency_enabled, |ui| {
                                ui.label("Offset:");
                                ui.add(
                                    egui::DragValue::new(&mut self.playback_ctl.audio_cfg.offset)
                                        .range(-0.5..=0.5)
                                        .speed(0.001)
                                        .suffix("s"),
                                );
                            });
                            ui.label("Adjust if audio is out of sync with cursor.");
                        },
                    );

                    egui::CollapsingHeader::new("Developer settings").default_open(false).show(
                        ui,
                        |ui| {
                            ui.label("Size:");
                            ui.add(
                                egui::DragValue::new(&mut self.ui.layout.width_cap_factor)
                                    .speed(0.01)
                                    .range(0.05..=0.5),
                            );
                            ui.separator();
                            ui.label("Stem Length Factor:");
                            ui.add(
                                egui::DragValue::new(&mut self.ui.layout.stem_length_factor)
                                    .speed(0.01)
                                    .range(0.7..=1.3),
                            );
                            ui.separator();
                            ui.checkbox(&mut self.ui.layout.debug_bbox, "Show bounding boxes");
                            ui.separator();
                            ui.label("Accents Position:");
                            ui.horizontal(|ui| {
                                ui.radio_value(&mut self.ui.layout.accent_below, true, "Below");
                                ui.radio_value(&mut self.ui.layout.accent_below, false, "Above");
                            });
                            ui.separator();
                            ui.checkbox(
                                &mut self.ui.layout.proportional_spacing,
                                "Proportional Spacing",
                            );
                        },
                    );

                    egui::CollapsingHeader::new("Counting").default_open(false).show(ui, |ui| {
                        ui.checkbox(&mut self.ui.counting.enabled, "Enable counting overlay");
                        ui.add_enabled_ui(self.ui.counting.enabled, |ui| {
                            ui.checkbox(&mut self.ui.counting.show_colors, "Show underlay colors");
                            ui.checkbox(&mut self.ui.counting.show_labels, "Show labels");
                            ui.checkbox(&mut self.ui.counting.show_tuplets, "Tuplet overlay");
                            egui::ComboBox::from_label("Subdivision")
                                .selected_text(match self.ui.counting.base {
                                    CountingBase::Off => "Off",
                                    CountingBase::Primary => "Primary",
                                    CountingBase::Ands => "Ands",
                                    CountingBase::Sixteenth => "16th",
                                    CountingBase::Triplet => "Triplet",
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut self.ui.counting.base,
                                        CountingBase::Off,
                                        "Off",
                                    );
                                    ui.selectable_value(
                                        &mut self.ui.counting.base,
                                        CountingBase::Primary,
                                        "Primary",
                                    );
                                    ui.selectable_value(
                                        &mut self.ui.counting.base,
                                        CountingBase::Ands,
                                        "1 & 2 & ...",
                                    );
                                    ui.selectable_value(
                                        &mut self.ui.counting.base,
                                        CountingBase::Sixteenth,
                                        "1 e & a ...",
                                    );
                                    ui.selectable_value(
                                        &mut self.ui.counting.base,
                                        CountingBase::Triplet,
                                        "1 trip let ...",
                                    );
                                });
                        });
                    });

                    egui::CollapsingHeader::new("MIDI Input").default_open(false).show(ui, |ui| {
                        midi_input_widget(
                            ui,
                            "settings_midi_input",
                            "🔄",
                            &mut self.playback_ctl.midi.input,
                            &mut self.playback_ctl.midi.available_ports,
                            &mut self.playback_ctl.midi.selected_port_id,
                        );
                    });

                    ui.separator();
                    ui.label("Theme:");
                    global_theme_preference_buttons(ui);
                });
            },
        );
    }
}
