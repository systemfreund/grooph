//! App-side glue for the rhythm generator: settings, "generate" action and
//! the endless mode that keeps adding fresh measures below the playing one
//! while the transport runs.

use crate::platform::random_seed;
use crate::{Grooph, TransportState, clock};
use grooph_measure::generator::{GeneratorSettings, Rng, generate_measure, generate_score};
use grooph_measure::tempo::ScoreTiming;
use grooph_measure::{Cursor, MeasureIdx};
use log::warn;

/// Measures endless mode starts with: the one being played and the next.
pub(crate) const ENDLESS_START_BARS: usize = 2;

pub(crate) struct GeneratorState {
    pub(crate) settings: GeneratorSettings,
    /// Keep adding measures while playing instead of looping the score.
    pub(crate) endless: bool,
    /// Play quiet ghost notes on the free slots of the generator subdivision.
    pub(crate) ghost_notes: bool,
    rng: Rng,
}

impl GeneratorState {
    pub(crate) fn new(settings: GeneratorSettings, endless: bool, ghost_notes: bool) -> Self {
        Self { settings, endless, ghost_notes, rng: Rng::new(random_seed()) }
    }

    /// Bars to generate: endless mode starts with [`ENDLESS_START_BARS`].
    pub(crate) fn effective_bars(&self) -> usize {
        if self.endless { ENDLESS_START_BARS } else { self.settings.bars.max(1) }
    }
}

impl Grooph {
    /// Regenerate the score after the generator settings were shown: on
    /// "New rhythm" (`roll`), on any changed setting, and when endless mode
    /// was turned on or off.
    pub(crate) fn finish_generator_change(
        &mut self,
        before: GeneratorSettings,
        endless_before: bool,
        roll: bool,
    ) {
        if roll
            || self.editor.generator.settings != before
            || endless_before != self.editor.generator.endless
        {
            self.generate_new_score();
        }
    }

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
    }

    /// Like [`Self::generate_new_score`], but reseeds the generator first so
    /// the same settings and seed always give the same rhythm (shared links).
    pub(crate) fn generate_new_score_with_seed(&mut self, seed: Option<u64>) {
        self.editor.generator.rng = Rng::new(seed.unwrap_or_else(random_seed));
        self.generate_new_score();
    }

    /// Endless mode: a running score. Once the playback cursor reaches the
    /// last measure a new one is added below it, and measures already played
    /// are removed from the top once they have scrolled out of view, so the
    /// score scrolls down for as long as it plays and never loops.
    ///
    /// Added measures follow the current generator settings.
    pub(crate) fn update_endless(&mut self) {
        let playing = self.playback_ctl.transport_state == TransportState::Playing;
        if !self.editor.generator.endless
            || !playing
            || self.metronome_mode()
            || self.editor.score.is_empty()
        {
            return;
        }

        let timing = self.score_timing();
        let current = timing.measure_at_global_tick(self.playback_ctl.playback.smooth_tick);
        if current + 1 >= self.editor.score.len() {
            self.append_endless_measure(&timing);
        }

        // Rows scrolled out of view above the playing one: the first goes once
        // the audio engine has left it too (latency offset).
        if self.playback_ctl.playback.rows_above_view > 0
            && current > 0
            && self.audio_has_left_measure(&timing, 0)
        {
            self.drop_first_measure(&timing);
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

    fn append_endless_measure(&mut self, timing: &ScoreTiming) {
        let measure =
            match generate_measure(&self.editor.generator.settings, &mut self.editor.generator.rng)
            {
                Ok(measure) => measure,
                Err(err) => {
                    warn!("Endless mode: cannot add a measure: {err:?}");
                    return;
                }
            };
        // Settle the hit windows on the old timing: the loop gets longer, so
        // the accuracy clock is anchored again below.
        let tracker = &mut self.playback_ctl.accuracy.tracker;
        tracker.update_progress(clock::now_seconds(), timing, &self.editor.score);
        self.editor.score.measures.push(measure);
        self.after_endless_change();
    }

    fn drop_first_measure(&mut self, timing: &ScoreTiming) {
        let tracker = &mut self.playback_ctl.accuracy.tracker;
        tracker.update_progress(clock::now_seconds(), timing, &self.editor.score);
        self.editor.score.measures.remove(0);
        let ticks = u64::from(timing.ticks_per_measure(0));
        tracker.drop_leading_ticks(ticks);
        if let Some(audio) = self.playback_ctl.audio.as_mut() {
            audio.drop_leading_ticks(ticks);
        }

        // Everything moves back by one measure: the cursors by its ticks, the
        // view by one row, so nothing on screen jumps.
        let playback = &mut self.playback_ctl.playback;
        playback.smooth_tick = (playback.smooth_tick - ticks as f64).max(0.0);
        playback.heard_tick = (playback.heard_tick - ticks as f64).max(0.0);
        playback.scroll.shift(-playback.row_pitch);
        playback.rows_removed = true;
        playback.rows_above_view -= 1;
        if let Some((m_idx, _)) = &mut playback.last_primary_beat {
            *m_idx = m_idx.saturating_sub(1);
        }
        if let Some((m_idx, _)) = &mut playback.guide_bar {
            *m_idx = m_idx.saturating_sub(1);
        }
        if let Some(anchor) = &mut playback.anchor {
            anchor.measure_idx = anchor.measure_idx.saturating_sub(1);
        }
        let cursor = &mut self.editor.cursor;
        if cursor.measure_idx == 0 {
            *cursor = Cursor::start();
        } else {
            cursor.measure_idx -= 1;
        }
        self.after_endless_change();
    }

    fn after_endless_change(&mut self) {
        self.realign_accuracy_clock();
        self.editor.active_pattern_id = None;
        self.editor.dirty = true;
    }
}
