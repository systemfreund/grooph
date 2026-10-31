use crate::Grooph;
use crate::pickers::dialog_scroll_height;
use eframe::egui;
use grooph_audio::Waveform;

impl Grooph {
    pub(super) fn settings_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().max_height(dialog_scroll_height(ui)).show(ui, |ui| {
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
                    egui::Slider::new(
                        &mut self.playback_ctl.audio_cfg.settings.noise_mix,
                        0.0..=1.0,
                    )
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                );
                if self.playback_ctl.audio_cfg.settings.noise_mix > 0.0 {
                    ui.separator();
                    ui.label("Noise High-Pass Cutoff:");
                    ui.add(
                        egui::DragValue::new(
                            &mut self.playback_ctl.audio_cfg.settings.noise_hpf_hz,
                        )
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

            egui::CollapsingHeader::new("Audio Latency").default_open(false).show(ui, |ui| {
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
            });

            egui::CollapsingHeader::new("Playback").default_open(false).show(ui, |ui| {
                ui.checkbox(&mut self.ui.flash_enabled, "Flash staff on every beat");
            });

            egui::CollapsingHeader::new("Tempo Ramp").default_open(false).show(ui, |ui| {
                self.tempo_ramp_settings(ui);
            });

            egui::CollapsingHeader::new("Developer settings").default_open(false).show(ui, |ui| {
                ui.checkbox(&mut self.ui.show_score_tab, "Show the Score tab (beta)");
                ui.separator();
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
            });
        });
    }
}
