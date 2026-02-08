use crate::Grooph;
use crate::{CountingBase, Mode};
use eframe::egui;
use eframe::egui::{Align, Direction, Layout, Widget, global_theme_preference_buttons};
use grooph_audio::Waveform;

impl Grooph {
    pub(super) fn settings_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("settings").resizable(true).show_animated(
            ctx,
            self.mode == Mode::Settings,
            |ui| {
                ui.set_min_height(300.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::CollapsingHeader::new("Click").default_open(false).show(ui, |ui| {
                        ui.label("Decay:");
                        ui.add(
                            egui::DragValue::new(&mut self.audio_settings.decay)
                                .range(0.01..=0.5)
                                .speed(0.001)
                                .suffix("s"),
                        );
                        ui.separator();
                        egui::ComboBox::from_label("Waveform")
                            .selected_text(match self.audio_settings.waveform {
                                Waveform::Sine => "Sine",
                                Waveform::Triangle => "Triangle",
                                Waveform::Square => "Square",
                                Waveform::Sawtooth => "Sawtooth",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.audio_settings.waveform,
                                    Waveform::Sine,
                                    "Sine",
                                );
                                ui.selectable_value(
                                    &mut self.audio_settings.waveform,
                                    Waveform::Triangle,
                                    "Triangle",
                                );
                                ui.selectable_value(
                                    &mut self.audio_settings.waveform,
                                    Waveform::Square,
                                    "Square",
                                );
                                ui.selectable_value(
                                    &mut self.audio_settings.waveform,
                                    Waveform::Sawtooth,
                                    "Sawtooth",
                                );
                            });
                        ui.separator();
                        ui.label("Noise Mix:");
                        ui.add(
                            egui::Slider::new(&mut self.audio_settings.noise_mix, 0.0..=1.0)
                                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                        );
                        if self.audio_settings.noise_mix > 0.0 {
                            ui.separator();
                            ui.label("Noise High-Pass Cutoff:");
                            ui.add(
                                egui::DragValue::new(&mut self.audio_settings.noise_hpf_hz)
                                    .range(2000.0..=6000.0)
                                    .speed(10.0)
                                    .suffix(" Hz"),
                            );
                            ui.separator();
                            ui.label("Noise Decay:");
                            ui.add(
                                egui::DragValue::new(&mut self.audio_settings.noise_decay)
                                    .range(0.01..=0.5)
                                    .speed(0.001)
                                    .suffix("s"),
                            );
                        }
                        ui.separator();
                        ui.label("Base Frequency (PrimaryBeat):");
                        ui.add(
                            egui::DragValue::new(&mut self.audio_settings.base_frequency)
                                .range(220.0..=880.0)
                                .speed(0.1)
                                .suffix("Hz"),
                        );
                    });

                    egui::CollapsingHeader::new("Audio Latency").default_open(false).show(
                        ui,
                        |ui| {
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
                        },
                    );

                    egui::CollapsingHeader::new("Developer settings").default_open(false).show(
                        ui,
                        |ui| {
                            ui.label("Size:");
                            ui.add(
                                egui::DragValue::new(&mut self.layout_width_cap_factor)
                                    .speed(0.01)
                                    .range(0.05..=0.5),
                            );
                            ui.separator();
                            ui.label("Stem Length Factor:");
                            ui.add(
                                egui::DragValue::new(&mut self.layout_stem_length_factor)
                                    .speed(0.01)
                                    .range(0.7..=1.3),
                            );
                            ui.separator();
                            ui.checkbox(&mut self.layout_debug_bbox, "Show bounding boxes");
                            ui.separator();
                            ui.label("Accents Position:");
                            ui.horizontal(|ui| {
                                ui.radio_value(&mut self.layout_accent_below, true, "Below");
                                ui.radio_value(&mut self.layout_accent_below, false, "Above");
                            });
                            ui.separator();
                            ui.checkbox(
                                &mut self.layout_proportional_spacing,
                                "Proportional Spacing",
                            );
                        },
                    );

                    egui::CollapsingHeader::new("Counting").default_open(false).show(ui, |ui| {
                        ui.checkbox(&mut self.counting.enabled, "Enable counting overlay");
                        ui.add_enabled_ui(self.counting.enabled, |ui| {
                            ui.checkbox(&mut self.counting.show_colors, "Show underlay colors");
                            ui.checkbox(&mut self.counting.show_labels, "Show labels");
                            ui.checkbox(&mut self.counting.show_tuplets, "Tuplet overlay");
                            egui::ComboBox::from_label("Subdivision")
                                .selected_text(match self.counting.base {
                                    CountingBase::Off => "Off",
                                    CountingBase::Primary => "Primary",
                                    CountingBase::Ands => "Ands",
                                    CountingBase::Sixteenth => "16th",
                                    CountingBase::Triplet => "Triplet",
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut self.counting.base,
                                        CountingBase::Off,
                                        "Off",
                                    );
                                    ui.selectable_value(
                                        &mut self.counting.base,
                                        CountingBase::Primary,
                                        "Primary",
                                    );
                                    ui.selectable_value(
                                        &mut self.counting.base,
                                        CountingBase::Ands,
                                        "1 & 2 & ...",
                                    );
                                    ui.selectable_value(
                                        &mut self.counting.base,
                                        CountingBase::Sixteenth,
                                        "1 e & a ...",
                                    );
                                    ui.selectable_value(
                                        &mut self.counting.base,
                                        CountingBase::Triplet,
                                        "1 trip let ...",
                                    );
                                });
                        });
                    });

                    egui::CollapsingHeader::new("MIDI Input").default_open(false).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if ui.button("🔄").on_hover_text("Refresh MIDI input ports").clicked()
                            {
                                self.refresh_midi_input_ports();
                            }

                            let Some(midi_input) = self.midi_input.as_mut() else {
                                ui.label("MIDI input unavailable.");
                                return;
                            };

                            let connected = midi_input.is_connected();
                            let selected_idx = self
                                .midi_selected_port_id
                                .as_ref()
                                .and_then(|id| midi_input.find_port_index_by_id(id));
                            let selected_text = if connected {
                                selected_idx
                                    .and_then(|idx| self.midi_input_ports.get(idx))
                                    .cloned()
                                    .unwrap_or_else(|| "Connected".to_string())
                            } else {
                                "Disconnected".to_string()
                            };

                            let mut should_disconnect = false;
                            let mut connect_port = None;

                            egui::ComboBox::from_id_salt("midi_input_port")
                                .selected_text(selected_text)
                                .show_ui(ui, |ui| {
                                    if connected {
                                        if ui.selectable_label(false, "Disconnect").clicked() {
                                            should_disconnect = true;
                                        }
                                    } else {
                                        let _ = ui.selectable_label(false, "Disconnected");
                                    }

                                    for (idx, name) in self.midi_input_ports.iter().enumerate() {
                                        if ui
                                            .selectable_label(
                                                selected_idx == Some(idx),
                                                name,
                                            )
                                            .clicked()
                                        {
                                            connect_port = Some(idx);
                                        }
                                    }
                                });

                            if should_disconnect {
                                let _ = midi_input.disconnect();
                                self.midi_selected_port_id = None;
                            } else if let Some(port_index) = connect_port
                                && midi_input.connect(port_index).is_ok()
                            {
                                self.midi_selected_port_id = midi_input.port_id(port_index);
                            }
                        });
                        ui.separator();
                        ui.label("Input Offset:");
                        ui.add(
                            egui::DragValue::new(&mut self.midi_input_offset_ms)
                                .range(-200.0..=200.0)
                                .speed(0.1)
                                .suffix(" ms"),
                        );
                    });

                    ui.separator();
                    ui.label("Theme:");
                    global_theme_preference_buttons(ui);
                });
            },
        );
    }

    fn refresh_midi_input_ports(&mut self) {
        if let Some(ref mut input) = self.midi_input {
            if let Err(e) = input.refresh_ports() {
                log::warn!("Failed to refresh MIDI input ports: {}", e);
            }
            match input.available_ports() {
                Ok(ports) => {
                    self.midi_input_ports = ports;
                }
                Err(e) => {
                    log::warn!("Failed to get MIDI input ports: {}", e);
                }
            }

            if let Some(ref port_id) = self.midi_selected_port_id {
                if let Some(idx) = input.find_port_index_by_id(port_id) {
                    if !input.is_connected() {
                        let _ = input.connect(idx);
                    }
                }
            }
        }
    }
}
