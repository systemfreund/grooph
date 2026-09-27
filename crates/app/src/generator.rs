//! App-side glue for the rhythm generator: settings, "generate" action and
//! the endless mode that keeps replacing already-played measures with fresh
//! ones while the transport runs.

use crate::platform::random_seed;
use crate::{Grooph, TransportState};
use grooph_measure::generator::{GeneratorSettings, Rng, generate_measure, generate_score};
use grooph_measure::tempo::ScoreTiming;
use grooph_measure::{Cursor, MeasureIdx};
use log::warn;

/// Minimum score length for endless mode: one measure is read while the other
/// one is being replaced.
pub(crate) const ENDLESS_MIN_BARS: usize = 2;

pub(crate) struct GeneratorState {
    pub(crate) settings: GeneratorSettings,
    /// Replace each measure with a new one right after it has been played.
    pub(crate) endless: bool,
    /// Play quiet ghost notes on the free slots of the generator subdivision.
    pub(crate) ghost_notes: bool,
    rng: Rng,
    /// Measure the playback cursor was in during the last frame (endless mode).
    last_playing_measure: Option<MeasureIdx>,
    /// Measure the visible cursor has left but the audio engine may still be
    /// sounding (latency offset); replaced once audio has left it too.
    pending_regeneration: Option<MeasureIdx>,
}

impl GeneratorState {
    pub(crate) fn new(settings: GeneratorSettings, endless: bool, ghost_notes: bool) -> Self {
        Self {
            settings,
            endless,
            ghost_notes,
            rng: Rng::new(random_seed()),
            last_playing_measure: None,
            pending_regeneration: None,
        }
    }

    /// Bars to generate: endless mode needs at least [`ENDLESS_MIN_BARS`].
    pub(crate) fn effective_bars(&self) -> usize {
        if self.endless {
            self.settings.bars.max(ENDLESS_MIN_BARS)
        } else {
            self.settings.bars.max(1)
        }
    }
}

impl Grooph {
    /// Replace the working score with a freshly generated one (undoable).
    pub(crate) fn generate_new_score(&mut self) {
        let settings = GeneratorSettings {
            bars: self.editor.generator.effective_bars(),
            ..self.editor.generator.settings
        };
        let score = match generate_score(&settings, &mut self.editor.generator.rng) {
            Ok(score) => score,
            Err(err) => {
                warn!("Rhythm generation failed: {err:?}");
                return;
            }
        };
        self.with_undo_snapshot(|app| {
            app.editor.score = score;
            app.editor.cursor = Cursor::start();
            true
        });
        self.editor.active_pattern_id = None;
        self.editor.generator.last_playing_measure = None;
        self.editor.generator.pending_regeneration = None;
    }

    /// Endless mode: once the playback cursor leaves a measure, regenerate
    /// that measure so it is new by the time the loop comes back to it.
    ///
    /// The replacement keeps the measure's time signature, so the loop length
    /// and the global tick of the playing measure stay unchanged; changed
    /// generator settings for bars/time signature apply on the next
    /// [`Self::generate_new_score`].
    pub(crate) fn update_endless(&mut self) {
        let playing = self.playback_ctl.transport_state == TransportState::Playing;
        if !self.editor.generator.endless || !playing || self.editor.score.len() < ENDLESS_MIN_BARS
        {
            self.editor.generator.last_playing_measure = None;
            self.editor.generator.pending_regeneration = None;
            return;
        }

        let timing = ScoreTiming::from_score(&self.editor.score, self.playback_ctl.bpm);
        let current = timing.measure_at_global_tick(self.playback_ctl.playback.smooth_tick);
        let previous = self.editor.generator.last_playing_measure.replace(current);
        if let Some(left) = previous
            && left != current
        {
            // A measure still pending from before is flushed first so it is
            // never skipped (only happens with very short measures).
            if let Some(pending) = self.editor.generator.pending_regeneration.replace(left)
                && pending != left
            {
                self.regenerate_measure(pending);
            }
        }

        if let Some(pending) = self.editor.generator.pending_regeneration
            && pending != current
            && self.audio_has_left_measure(&timing, pending)
        {
            self.editor.generator.pending_regeneration = None;
            self.regenerate_measure(pending);
        }
    }

    /// Whether the audio engine's cursor is outside measure `idx`. Without a
    /// running audio engine only the visible cursor counts.
    fn audio_has_left_measure(&self, timing: &ScoreTiming, idx: MeasureIdx) -> bool {
        let Some(audio) = self.playback_ctl.audio.as_ref().filter(|a| a.is_running()) else {
            return true;
        };
        match audio.playback_position() {
            Some((tick, total)) if total > 0 => timing.measure_at_global_tick(tick) != idx,
            // Engine busy or not started yet: try again next frame.
            _ => false,
        }
    }

    fn regenerate_measure(&mut self, idx: MeasureIdx) {
        let settings = GeneratorSettings {
            time_signature: self.editor.score.current(idx).time_signature(),
            ..self.editor.generator.settings
        };
        let measure = match generate_measure(&settings, &mut self.editor.generator.rng) {
            Ok(measure) => measure,
            Err(err) => {
                warn!("Endless mode: cannot regenerate measure {idx}: {err:?}");
                return;
            }
        };
        self.editor.score.measures[idx] = measure;

        let cursor = &mut self.editor.cursor;
        if cursor.measure_idx == idx {
            let len = self.editor.score.current(idx).beats().len();
            cursor.beat_idx = cursor.beat_idx.min(len.saturating_sub(1));
        }
        self.editor.active_pattern_id = None;
        self.editor.dirty = true;
    }
}
