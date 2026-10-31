//! Input latency calibration: play a plain quarter-note click, the user taps
//! (or plays MIDI) along, and the median deviation becomes that source's
//! input offset.
//!
//! The run plays a click bar of its own (quarter notes in 4/4) at its own
//! tempo and with plain options ([`Grooph::played_score`],
//! [`Grooph::played_bpm`], [`Grooph::playback_options`]): no page's
//! document, the tempo or an option is touched, and the page shown stays.

use crate::play_along::{HitSource, MAX_INPUT_OFFSET_MS};
use crate::{Grooph, TransportState};
use eframe::egui;
use grooph_measure::Score;

/// Tempo of the calibration click.
pub(crate) const CALIBRATION_BPM: u32 = 100;
/// First hits are ignored while the user settles into the pulse.
pub(crate) const CALIBRATION_WARMUP: usize = 4;
/// Hits measured after the warm-up.
pub(crate) const CALIBRATION_HITS: usize = 12;
/// Above this spread the result is flagged as unreliable.
const UNSTEADY_SPREAD_MS: f64 = 30.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CalibrationPhase {
    Running,
    Done { offset_ms: f32, spread_ms: f32 },
}

pub(crate) struct Calibration {
    pub(crate) source: HitSource,
    pub(crate) phase: CalibrationPhase,
    deltas: Vec<f64>,
    /// The click bar the run plays: quarter notes in 4/4.
    score: Score,
}

impl Calibration {
    pub(crate) fn push(&mut self, delta_ms: f64) {
        if self.phase == CalibrationPhase::Running {
            self.deltas.push(delta_ms);
        }
    }

    pub(crate) fn hits(&self) -> usize {
        self.deltas.len()
    }

    /// `(offset_ms, spread_ms)` once enough hits are in.
    fn result(&self) -> Option<(f64, f64)> {
        if self.deltas.len() < CALIBRATION_WARMUP + CALIBRATION_HITS {
            return None;
        }
        evaluate(&self.deltas[CALIBRATION_WARMUP..])
    }
}

/// Median deviation (robust against a stray tap) and the median absolute
/// deviation around it as spread.
pub(crate) fn evaluate(deltas: &[f64]) -> Option<(f64, f64)> {
    let offset = median(deltas.to_vec())?;
    let spread = median(deltas.iter().map(|d| (d - offset).abs()).collect())?;
    Some((offset, spread))
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    Some(if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) * 0.5
    } else {
        values[mid]
    })
}

impl Grooph {
    /// The calibration whose run plays: the one place that checks the phase.
    fn running_calibration(&self) -> Option<&Calibration> {
        let cal = self.playback_ctl.play_along.calibration.as_ref()?;
        (cal.phase == CalibrationPhase::Running).then_some(cal)
    }

    /// Source being calibrated while the calibration click runs.
    pub(crate) fn calibration_source(&self) -> Option<HitSource> {
        self.running_calibration().map(|c| c.source)
    }

    /// Whether a calibration run plays: the transport plays its click bar.
    pub(crate) fn calibrating(&self) -> bool {
        self.running_calibration().is_some()
    }

    /// The score the transport plays and hits are measured against: the
    /// calibration's click bar during a run, else the shown page's document.
    /// The metronome's is its click bar, played as clicks
    /// ([`grooph_audio::Played::Clicks`]).
    pub(crate) fn played_score(&self) -> &Score {
        self.running_calibration().map_or(&self.editor.doc().score, |cal| &cal.score)
    }

    /// The tempo setting played: the calibration's own during a run.
    pub(crate) fn played_bpm(&self) -> u32 {
        if self.calibrating() { CALIBRATION_BPM } else { self.playback_ctl.bpm }
    }

    pub(crate) fn start_calibration(&mut self, source: HitSource) {
        self.stop_transport();
        self.playback_ctl.play_along.calibration = Some(Calibration {
            source,
            phase: CalibrationPhase::Running,
            deltas: Vec::new(),
            score: Score::single(Self::default_measure()),
        });
        self.start_playback();
    }

    /// Finish once enough hits are in: store the offset and stop.
    pub(crate) fn advance_calibration(&mut self) {
        let Some(cal) = self.playback_ctl.play_along.calibration.as_mut() else {
            return;
        };
        if cal.phase != CalibrationPhase::Running {
            return;
        }
        let Some((offset, spread)) = cal.result() else {
            return;
        };
        let max = MAX_INPUT_OFFSET_MS as f64;
        let offset_ms = offset.round().clamp(-max, max) as f32;
        cal.phase = CalibrationPhase::Done { offset_ms, spread_ms: spread.round() as f32 };
        *self.playback_ctl.play_along.settings.offset_ms_mut(cal.source) = offset_ms;
        self.stop_transport();
    }

    /// Called when the transport stops: a run stopped before it finished is
    /// a cancel.
    pub(crate) fn end_calibration_run(&mut self) {
        if self.calibrating() {
            self.playback_ctl.play_along.calibration = None;
        }
    }

    pub(crate) fn calibration_window(&mut self, ui: &mut egui::Ui) {
        let Some(cal) = self.playback_ctl.play_along.calibration.as_ref() else {
            return;
        };
        let source = cal.source;
        let phase = cal.phase;
        let hits = cal.hits();
        let tap_key = self.playback_ctl.play_along.settings.tap_key;
        let total = CALIBRATION_WARMUP + CALIBRATION_HITS;

        enum Action {
            Cancel,
            Close,
            Again,
            Tap,
        }
        let mut action = None;
        egui::Modal::new(egui::Id::unique("calibration")).show(ui.ctx(), |ui| {
            ui.set_width(300.0);
            let title = match source {
                HitSource::Tap => "Calibrate tap latency",
                HitSource::Midi => "Calibrate MIDI latency",
                HitSource::Mic => "Calibrate microphone latency",
            };
            ui.heading(title);
            ui.add_space(8.0);
            match phase {
                CalibrationPhase::Running => {
                    let how = match source {
                        HitSource::Tap => format!(
                            "Tap along with the click on every beat: press {} or tap the pad.",
                            tap_key.label()
                        ),
                        HitSource::Midi => {
                            "Play along with the click on every beat on your MIDI device.".into()
                        }
                        HitSource::Mic => {
                            "Hit along with the click on every beat; the microphone listens.".into()
                        }
                    };
                    ui.label(how);
                    ui.add_space(8.0);
                    ui.add(egui::ProgressBar::new(hits as f32 / total as f32).text(format!(
                        "{} / {}",
                        hits.min(total),
                        total
                    )));
                    if source == HitSource::Tap {
                        ui.add_space(8.0);
                        // React on press, not on release like a button click.
                        let (rect, resp) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), 120.0),
                            egui::Sense::click(),
                        );
                        let pressed =
                            resp.contains_pointer() && ui.input(|i| i.pointer.any_pressed());
                        let visuals = ui.visuals();
                        let fill = if resp.is_pointer_button_down_on() {
                            visuals.selection.bg_fill
                        } else {
                            visuals.widgets.inactive.bg_fill
                        };
                        ui.painter().rect_filled(rect, 8.0, fill);
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "Tap",
                            egui::FontId::proportional(24.0),
                            visuals.text_color(),
                        );
                        if pressed {
                            action = Some(Action::Tap);
                        }
                    }
                    ui.add_space(8.0);
                    if ui.button("Cancel").clicked() {
                        action = Some(Action::Cancel);
                    }
                }
                CalibrationPhase::Done { offset_ms, spread_ms } => {
                    ui.label(format!("Input latency set to {offset_ms:+.0} ms."));
                    if spread_ms as f64 > UNSTEADY_SPREAD_MS {
                        ui.label(format!(
                            "Your hits were uneven (± {spread_ms:.0} ms). Try again for a more \
                             reliable value."
                        ));
                    } else {
                        ui.label(format!("Spread: ± {spread_ms:.0} ms."));
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Done").clicked() {
                            action = Some(Action::Close);
                        }
                        if ui.button("Again").clicked() {
                            action = Some(Action::Again);
                        }
                    });
                }
            }
        });

        match action {
            Some(Action::Cancel) => self.stop_transport(),
            Some(Action::Close) => self.playback_ctl.play_along.calibration = None,
            Some(Action::Again) => self.start_calibration(source),
            Some(Action::Tap) if self.playback_ctl.transport_state == TransportState::Playing => {
                self.push_pointer_tap();
            }
            Some(Action::Tap) => {}
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluate_uses_median_and_ignores_outlier() {
        let (offset, spread) = evaluate(&[20.0, 22.0, 18.0, 21.0, 150.0]).unwrap();
        assert_eq!(offset, 21.0);
        assert_eq!(spread, 1.0);
    }

    #[test]
    fn evaluate_even_count_averages_middle_values() {
        let (offset, _) = evaluate(&[-10.0, 10.0, 20.0, 30.0]).unwrap();
        assert_eq!(offset, 15.0);
    }

    #[test]
    fn evaluate_empty_is_none() {
        assert!(evaluate(&[]).is_none());
    }
}
