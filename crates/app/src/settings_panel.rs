use crate::Grooph;
use crate::pickers::{choice_grid, dialog_scroll_height};
use crate::voicing_picker::voicing_label;
use eframe::egui;
use grooph_audio::Waveform;
use grooph_measure::Voicing;

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
                ui.checkbox(&mut self.playback_ctl.devices.latency_enabled, "Enabled");
                ui.add_enabled_ui(self.playback_ctl.devices.latency_enabled, |ui| {
                    ui.label("Offset:");
                    ui.add(
                        egui::DragValue::new(&mut self.playback_ctl.devices.offset)
                            .range(-0.5..=0.5)
                            .speed(0.001)
                            .suffix("s"),
                    );
                });
                ui.label("Adjust if audio is out of sync with cursor.");
            });

            egui::CollapsingHeader::new("Playback").default_open(false).show(ui, |ui| {
                ui.checkbox(
                    &mut self.playback_ctl.feedback.flash_enabled,
                    "Flash staff on every beat",
                );
            });

            egui::CollapsingHeader::new("New grooves").default_open(false).show(ui, |ui| {
                self.new_groove_settings(ui);
            });

            egui::CollapsingHeader::new("Developer settings").default_open(false).show(ui, |ui| {
                ui.checkbox(&mut self.ui.view.show_score_tab, "Show the Score tab (beta)");
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

    /// "Write new grooves as": a preset, or the rule of the score tab's
    /// score, copied as it is now.
    fn new_groove_settings(&mut self, ui: &mut egui::Ui) {
        let setting = self.editor.settings.new_groove_voicing;
        ui.horizontal(|ui| {
            ui.label("Write new grooves as");
            ui.strong(voicing_label(setting));
        });
        let mut options: Vec<_> =
            Voicing::PRESETS.map(|v| (Some(v), voicing_label(v).to_owned())).into();
        options.push((None, "Use this score's".to_owned()));
        // A custom rule is none of the tiles; "Use this score's" is an action.
        match choice_grid(ui, &options, Some(setting), 2) {
            Some(Some(voicing)) => self.editor.settings.new_groove_voicing = voicing,
            Some(None) => self.use_score_voicing_for_new_grooves(),
            None => {}
        }
    }

    /// "Use this score's": new grooves get the score tab's rule as it is
    /// now; later changes to that score leave the setting alone.
    pub(crate) fn use_score_voicing_for_new_grooves(&mut self) {
        self.editor.settings.new_groove_voicing = self.editor.docs.score.score.voicing;
    }
}
