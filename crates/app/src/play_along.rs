//! Play-along input: hits from MIDI, the microphone and taps (keyboard or
//! touch on the staff) feed the accuracy tracker on the shared [`crate::clock`] timeline.
//! Also the run summary shown after playback.

use crate::accuracy::{GOOD_WINDOW_MS, PERFECT_WINDOW_MS, RunSummary};
use crate::calibration::Calibration;
use crate::platform::{TapKind, tap_time};
use crate::{Grooph, TransportState, clock};
use eframe::egui;
use grooph_mic::{DEFAULT_THRESHOLD_DB, MicInput};
use grooph_midi::MidiInputEvent;
use log::debug;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Where a hit came from. Each source has its own input latency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HitSource {
    Midi,
    Tap,
    Mic,
}

/// Keys offered for tapping. Space is play/stop and the tool shortcuts are
/// taken, so only free letters are offered.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TapKey {
    #[default]
    J,
    K,
    F,
    D,
}

impl TapKey {
    pub(crate) const ALL: [TapKey; 4] = [TapKey::J, TapKey::K, TapKey::F, TapKey::D];

    pub(crate) fn key(self) -> egui::Key {
        match self {
            TapKey::J => egui::Key::J,
            TapKey::K => egui::Key::K,
            TapKey::F => egui::Key::F,
            TapKey::D => egui::Key::D,
        }
    }

    /// Lowercased DOM `KeyboardEvent.key`.
    fn dom_key(self) -> &'static str {
        match self {
            TapKey::J => "j",
            TapKey::K => "k",
            TapKey::F => "f",
            TapKey::D => "d",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            TapKey::J => "J",
            TapKey::K => "K",
            TapKey::F => "F",
            TapKey::D => "D",
        }
    }
}

/// Persisted play-along settings.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub(crate) struct PlayAlongSettings {
    /// Count key presses and touches on the staff as hits during playback.
    pub(crate) tap_enabled: bool,
    pub(crate) tap_key: TapKey,
    /// Input latency subtracted from taps (ms).
    pub(crate) tap_offset_ms: f32,
    /// Input latency subtracted from MIDI hits (ms).
    pub(crate) midi_offset_ms: f32,
    /// Listen for hits (sticks, a pad, claps) on the microphone.
    pub(crate) mic_enabled: bool,
    /// Sounds louder than this count as hits (dBFS).
    pub(crate) mic_threshold_db: f32,
    /// Input latency subtracted from microphone hits (ms).
    pub(crate) mic_offset_ms: f32,
}

impl Default for PlayAlongSettings {
    fn default() -> Self {
        Self {
            tap_enabled: true,
            tap_key: TapKey::J,
            tap_offset_ms: 0.0,
            midi_offset_ms: 0.0,
            mic_enabled: false,
            mic_threshold_db: DEFAULT_THRESHOLD_DB,
            mic_offset_ms: 0.0,
        }
    }
}

impl PlayAlongSettings {
    pub(crate) fn offset_ms(&self, source: HitSource) -> f32 {
        match source {
            HitSource::Midi => self.midi_offset_ms,
            HitSource::Tap => self.tap_offset_ms,
            HitSource::Mic => self.mic_offset_ms,
        }
    }

    pub(crate) fn offset_ms_mut(&mut self, source: HitSource) -> &mut f32 {
        match source {
            HitSource::Midi => &mut self.midi_offset_ms,
            HitSource::Tap => &mut self.tap_offset_ms,
            HitSource::Mic => &mut self.mic_offset_ms,
        }
    }
}

/// Input latency offsets are limited to this range (ms).
pub(crate) const MAX_INPUT_OFFSET_MS: f32 = 300.0;

#[derive(Default)]
pub(crate) struct PlayAlongState {
    pub(crate) settings: PlayAlongSettings,
    /// A tap or microphone hit arrived during this run: these keep the
    /// accuracy session alive even without MIDI. Without it, a run nobody plays along to would only
    /// collect misses.
    pub(crate) engaged: bool,
    /// Taps on the staff collected by the measure panel this frame.
    pub(crate) pending_taps: Vec<f64>,
    pub(crate) calibration: Option<Calibration>,
}

impl Grooph {
    /// Whether taps are captured right now. Touches on the staff then count
    /// as hits instead of moving the cursor.
    pub(crate) fn tap_capture_active(&self) -> bool {
        if self.playback_ctl.transport_state != TransportState::Playing {
            return false;
        }
        match self.calibration_source() {
            Some(source) => source == HitSource::Tap,
            // In the metronome there is no rhythm to play along with; taps
            // edit the ring.
            None => {
                !self.metronome_mode()
                    && self.playback_ctl.accuracy.enabled
                    && self.playback_ctl.play_along.settings.tap_enabled
            }
        }
    }

    pub(crate) fn push_pointer_tap(&mut self) {
        self.playback_ctl.play_along.pending_taps.push(tap_time(TapKind::Pointer));
    }

    fn midi_connected(&self) -> bool {
        self.playback_ctl.midi.input.as_ref().is_some_and(|input| input.is_connected())
    }

    /// MIDI NoteOns, stamped on the shared clock.
    fn drain_midi_hits(&mut self, now: f64) -> Vec<f64> {
        let Some(input) = self.playback_ctl.midi.input.as_ref() else {
            return Vec::new();
        };
        let midi_now = input.now_seconds();
        let mut hits = Vec::new();
        for event in input.drain_events() {
            match event {
                MidiInputEvent::NoteOn { channel, note, velocity, timestamp } => {
                    // Keep the event's age, move it onto the shared timeline.
                    hits.push(now - (midi_now - timestamp).max(0.0));
                    debug!("MIDI NoteOn ch={} note={} vel={}", channel, note, velocity);
                }
                MidiInputEvent::NoteOff { channel, note, velocity, .. } => {
                    debug!("MIDI NoteOff ch={} note={} vel={}", channel, note, velocity);
                }
                MidiInputEvent::ControlChange { .. } => {}
            }
        }
        hits
    }

    /// Whether the microphone should listen: during a run that uses it, or
    /// while the settings show its level.
    fn mic_wanted(&self, now: f64) -> bool {
        let playing = self.playback_ctl.transport_state == TransportState::Playing;
        match self.calibration_source() {
            Some(source) => playing && source == HitSource::Mic,
            None => {
                self.playback_ctl.play_along.settings.mic_enabled
                    && (now < self.playback_ctl.mic.monitor_until
                        || playing && self.playback_ctl.accuracy.enabled && !self.metronome_mode())
            }
        }
    }

    /// Open or close the microphone as needed and keep its threshold current.
    fn sync_mic(&mut self, ctx: &egui::Context, now: f64) {
        let wanted = self.mic_wanted(now);
        let threshold_db = self.playback_ctl.play_along.settings.mic_threshold_db;
        let mic = &mut self.playback_ctl.mic;
        match (&mic.input, wanted) {
            (None, true) => {
                // On Android the user has to allow the microphone first.
                if let Some(host) = &self.ui.host
                    && !host.mic_permitted()
                {
                    if !mic.permission_asked {
                        host.request_mic();
                        mic.permission_asked = true;
                    }
                    return;
                }
                let ctx = ctx.clone();
                let notifier = Arc::new(move || ctx.request_repaint());
                mic.input = Some(MicInput::open(threshold_db, Some(notifier)));
                mic.level_db = f32::NEG_INFINITY;
            }
            (Some(_), false) => {
                mic.input = None;
                mic.permission_asked = false;
            }
            (Some(input), true) => input.set_threshold_db(threshold_db),
            (None, false) => mic.permission_asked = false,
        }
    }

    /// Microphone hits, stamped on the shared clock.
    fn drain_mic_hits(&mut self, now: f64) -> Vec<f64> {
        let Some(input) = self.playback_ctl.mic.input.as_ref() else {
            return Vec::new();
        };
        let mic_now = input.now_seconds();
        input.drain_hits().into_iter().map(|t| now - (mic_now - t).max(0.0)).collect()
    }

    fn drain_key_taps(&mut self, ui: &egui::Ui) -> Vec<f64> {
        let tap_key = self.playback_ctl.play_along.settings.tap_key;
        if !self.tap_capture_active() || ui.ctx().egui_wants_keyboard_input() {
            return Vec::new();
        }
        let presses = ui.input(|i| {
            i.events
                .iter()
                .filter(|e| {
                    matches!(e, egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. }
                        if *key == tap_key.key() && modifiers.is_none())
                })
                .count()
        });
        (0..presses).map(|_| tap_time(TapKind::Key(tap_key.dom_key()))).collect()
    }

    /// Collect this frame's hits from all sources and feed them to the
    /// accuracy tracker; drives the session lifecycle and calibration.
    pub(crate) fn handle_play_along_input(&mut self, ui: &egui::Ui) {
        let now = clock::now_seconds();
        self.sync_mic(ui.ctx(), now);
        let midi_hits = self.drain_midi_hits(now);
        let mic_hits = self.drain_mic_hits(now);
        let mut tap_hits = std::mem::take(&mut self.playback_ctl.play_along.pending_taps);
        tap_hits.extend(self.drain_key_taps(ui));

        let calibrating = self.calibration_source();
        if !self.playback_ctl.accuracy.enabled && calibrating.is_none() {
            return;
        }
        if self.playback_ctl.transport_state != TransportState::Playing {
            return;
        }

        let settings = self.playback_ctl.play_along.settings;
        let midi_connected = self.midi_connected();
        let use_midi = calibrating.is_none_or(|s| s == HitSource::Midi);
        let use_taps = calibrating.map_or(settings.tap_enabled, |s| s == HitSource::Tap);
        let use_mic = calibrating.map_or(settings.mic_enabled, |s| s == HitSource::Mic);
        let mut hits: Vec<(HitSource, f64)> = Vec::new();
        if use_midi {
            hits.extend(midi_hits.into_iter().map(|t| (HitSource::Midi, t)));
        }
        if use_taps {
            hits.extend(tap_hits.into_iter().map(|t| (HitSource::Tap, t)));
        }
        if use_mic {
            hits.extend(mic_hits.into_iter().map(|t| (HitSource::Mic, t)));
        }
        if hits.iter().any(|(s, _)| *s != HitSource::Midi) {
            self.playback_ctl.play_along.engaged = true;
        }
        let active = calibrating.is_some()
            || (midi_connected && use_midi)
            || self.playback_ctl.play_along.engaged;

        // The score hasn't started yet: keep re-anchoring the session at "now"
        // and ignore hits; `realign_accuracy_clock` fixes the anchor exactly
        // when the count-in ends.
        if self.playback_ctl.playback.counting_in() {
            if active {
                self.playback_ctl.accuracy.tracker.on_playback_start_at(now, 0.0);
            }
            return;
        }

        let timing = self.score_timing();
        let total_loop_seconds = timing.total_loop_seconds();
        if active && !self.playback_ctl.accuracy.tracker.has_start_time() {
            let (start_time, last_tick) = if total_loop_seconds > 0.0 {
                let tick = self.playback_ctl.playback.smooth_tick;
                (now - timing.global_tick_to_seconds(tick), tick)
            } else {
                (now, 0.0)
            };
            self.playback_ctl.accuracy.tracker.on_playback_start_at(start_time, last_tick);
        }
        if !self.playback_ctl.accuracy.tracker.update_state(active, now) {
            return;
        }
        let score = self.played_score().into_owned();
        if total_loop_seconds <= 0.0 || score.measures().is_empty() {
            return;
        }

        hits.sort_by(|a, b| a.1.total_cmp(&b.1));
        for (source, time) in hits {
            // A calibration measures the raw latency, so it ignores the offset.
            let offset_ms = if calibrating.is_some() { 0.0 } else { settings.offset_ms(source) };
            let time = time - offset_ms as f64 / 1000.0;
            let delta = self.playback_ctl.accuracy.tracker.record_hit(time, &timing, &score);
            if let (Some(delta), Some(cal)) =
                (delta, self.playback_ctl.play_along.calibration.as_mut())
            {
                cal.push(delta);
            }
        }
        self.playback_ctl.accuracy.tracker.update_progress(now, &timing, &score);
        self.advance_calibration();
    }

    /// Evaluation of the last run, shown once playback has stopped.
    pub(crate) fn run_summary_window(&mut self, ui: &mut egui::Ui) {
        if self.playback_ctl.transport_state != TransportState::Stopped
            || self.playback_ctl.play_along.calibration.is_some()
        {
            return;
        }
        let Some(summary) = self.playback_ctl.accuracy.tracker.last_summary() else {
            return;
        };
        let mut close = false;
        egui::Window::new("Your run")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 24.0))
            .show(ui.ctx(), |ui| {
                ui.heading(format!("{:.0}%", summary.score_percent()));
                ui.add_space(4.0);
                summary_grid(ui, &summary);
                ui.add_space(8.0);
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        if close {
            self.playback_ctl.accuracy.tracker.dismiss_summary();
        }
    }
}

fn summary_grid(ui: &mut egui::Ui, summary: &RunSummary) {
    egui::Grid::new("run_summary").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
        ui.label("Hits");
        ui.label(summary.hits.to_string());
        ui.end_row();
        ui.label("  Perfect").on_hover_text(format!(
            "Within {PERFECT_WINDOW_MS:.0} ms of the note; fast notes get a fifth of the gap to their neighbour"
        ));
        ui.label(summary.perfect.to_string());
        ui.end_row();
        ui.label("  Good").on_hover_text(format!(
            "Within {GOOD_WINDOW_MS:.0} ms of the note; fast notes get two fifths of the gap to their neighbour"
        ));
        ui.label(summary.good.to_string());
        ui.end_row();
        ui.label("  Off");
        ui.label(summary.off().to_string());
        ui.end_row();
        ui.label("Missed notes");
        ui.label(summary.misses.to_string());
        ui.end_row();
        ui.label("Extra hits");
        ui.label(summary.extra.to_string());
        ui.end_row();
        ui.label("Average");
        ui.label(early_late(summary.mean_ms));
        ui.end_row();
        ui.label("Spread");
        ui.label(format!("± {:.0} ms", summary.spread_ms));
        ui.end_row();
    });
}

fn early_late(ms: f64) -> String {
    let rounded = ms.round();
    if rounded == 0.0 {
        "on time".to_owned()
    } else if rounded < 0.0 {
        format!("{:.0} ms early", -rounded)
    } else {
        format!("{rounded:.0} ms late")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn early_late_rounds_and_names_direction() {
        assert_eq!(early_late(0.3), "on time");
        assert_eq!(early_late(-12.4), "12 ms early");
        assert_eq!(early_late(7.6), "8 ms late");
    }

    #[test]
    fn settings_default_enables_taps_without_offsets() {
        let s = PlayAlongSettings::default();
        assert!(s.tap_enabled);
        assert_eq!(s.offset_ms(HitSource::Tap), 0.0);
        assert_eq!(s.offset_ms(HitSource::Midi), 0.0);
    }
}
