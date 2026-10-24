//! App side of the metronome trainer: follows the bar the audio engine plays
//! to apply the tempo ramp to the UI timing and to show muted bars.

use crate::{Grooph, TransportState};
use eframe::egui;
use grooph_audio::{GapMode, GapMute, RampEnd, RampUnit, TempoRamp};
use grooph_measure::tempo::{MAX_BPM, MIN_BPM};

impl Grooph {
    /// Track the ramp tempo and gap state of the bar being played. A tempo
    /// change re-anchors the accuracy session, whose timeline assumes one tempo.
    pub(crate) fn update_metronome_trainer(&mut self) {
        let playing = self.playback_ctl.transport_state == TransportState::Playing;
        let bar = match self.playback_ctl.audio.as_mut() {
            Some(audio) if playing => audio.bar_state(),
            _ => None,
        };
        let ramp = self.played_ramp();
        let bpm = self.playback_ctl.bpm;
        let bars = self.played_bars();
        let ramp_bpm = (playing && ramp.enabled).then(|| match bar {
            Some(b) => ramp.bpm_for_bar(bpm, b.bar, bars),
            None => bpm,
        });
        self.playback_ctl.playback.bar_muted = bar.is_some_and(|b| b.muted);
        if self.playback_ctl.playback.ramp_bpm != ramp_bpm {
            self.playback_ctl.playback.ramp_bpm = ramp_bpm;
            self.realign_accuracy_clock();
        }
    }
}

impl Grooph {
    /// Endless mode never loops: a ramp counted in loops counts in bars.
    fn endless_running(&self) -> bool {
        self.editor.generator.endless && !self.metronome_mode()
    }

    /// The tempo ramp as played: see [`Self::endless_running`].
    pub(crate) fn played_ramp(&self) -> TempoRamp {
        let mut ramp = self.playback_ctl.audio_cfg.ramp;
        if self.endless_running() {
            ramp.unit = RampUnit::Bars;
        }
        ramp
    }

    pub(crate) fn gap_click_settings(&mut self, ui: &mut egui::Ui) {
        let gap = &mut self.playback_ctl.audio_cfg.gap;
        ui.checkbox(&mut gap.enabled, "Mute bars")
            .on_hover_text("Practise inner time: the metronome drops out for some bars");
        ui.add_enabled_ui(gap.enabled, |ui| {
            ui.horizontal(|ui| {
                ui.radio_value(&mut gap.mode, GapMode::Pattern, "Pattern");
                ui.radio_value(&mut gap.mode, GapMode::Random, "Random");
            });
            match gap.mode {
                GapMode::Pattern => {
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut gap.on_bars).range(1..=16).speed(0.05));
                        ui.label("bars on,");
                        ui.add(egui::DragValue::new(&mut gap.off_bars).range(1..=16).speed(0.05));
                        ui.label("bars off");
                    });
                }
                GapMode::Random => {
                    ui.add(
                        egui::Slider::new(&mut gap.random_percent, 5..=90)
                            .text("chance per bar")
                            .suffix("%"),
                    );
                }
            }
            ui.horizontal(|ui| {
                ui.label("Mute:");
                ui.radio_value(&mut gap.mute, GapMute::Metronome, "Click only");
                ui.radio_value(&mut gap.mute, GapMute::Everything, "Click and notes");
            });
        });
    }

    pub(crate) fn tempo_ramp_settings(&mut self, ui: &mut egui::Ui) {
        let start = self.playback_ctl.bpm;
        let endless = self.endless_running();
        let ramp = &mut self.playback_ctl.audio_cfg.ramp;
        ui.checkbox(&mut ramp.enabled, "Change the tempo while playing")
            .on_hover_text("Starts at the BPM setting and steps towards the target tempo");
        ui.add_enabled_ui(ramp.enabled, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("From {start} to"));
                ui.add(
                    egui::DragValue::new(&mut ramp.target_bpm)
                        .range(MIN_BPM..=MAX_BPM)
                        .speed(0.1)
                        .suffix(" BPM"),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Step");
                ui.add(egui::DragValue::new(&mut ramp.step_bpm).range(1..=50).speed(0.05));
                ui.label("BPM every");
                ui.add(egui::DragValue::new(&mut ramp.every).range(1..=64).speed(0.05));
                if endless {
                    ui.label("bars");
                } else {
                    egui::ComboBox::from_id_salt("ramp_unit")
                        .selected_text(match ramp.unit {
                            RampUnit::Bars => "bars",
                            RampUnit::Loops => "loops",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut ramp.unit, RampUnit::Bars, "bars");
                            ui.selectable_value(&mut ramp.unit, RampUnit::Loops, "loops");
                        });
                }
            });
            ui.horizontal(|ui| {
                ui.label("At the target:");
                ui.radio_value(&mut ramp.end, RampEnd::Hold, "Hold");
                ui.radio_value(&mut ramp.end, RampEnd::Restart, "Start over");
                ui.radio_value(&mut ramp.end, RampEnd::Cycle, "Back down");
            });
            ui.checkbox(&mut ramp.keep_reached, "Keep the reached tempo after stopping");
        });
    }
}
