use grooph_measure::Score;
use grooph_measure::tempo::ScoreTiming;

use super::progress::update_progress;
use super::recorder::record_hit;
use super::session::{AccuracyMark, RecordingSession};

/// Coordinates the accuracy-recording session. Holds a `RecordingSession`
/// that is either `Idle` (no playback) or `Recording` (playback active with
/// accumulated hit/miss data). Lifecycle methods drive the state transitions;
/// `record_hit` and `update_progress` are no-ops while idle.
#[derive(Default)]
pub(crate) struct AccuracyTracker {
    session: RecordingSession,
}

impl AccuracyTracker {
    pub(crate) fn new() -> Self { Self::default() }

    pub(crate) fn has_start_time(&self) -> bool { self.session.is_recording() }

    pub(crate) fn update_state(
        &mut self,
        playing: bool,
        is_connected: bool,
        now_seconds: f64,
    ) -> bool {
        if playing && is_connected {
            if !self.session.is_recording() {
                self.session.start(now_seconds, Some(0.0));
            }
            true
        } else {
            if self.session.is_recording() {
                self.session.stop();
            }
            false
        }
    }

    pub(crate) fn on_playback_start(&mut self, now_seconds: Option<f64>) {
        match now_seconds {
            Some(t) => self.session.start(t, Some(0.0)),
            None => self.session.stop(),
        }
    }

    pub(crate) fn on_playback_start_at(&mut self, start_time: f64, last_tick: f64) {
        self.session.start(start_time, Some(last_tick));
    }

    pub(crate) fn realign_start_time(&mut self, start_time: f64, last_tick: f64) {
        self.session.realign(start_time, last_tick);
    }

    pub(crate) fn on_playback_stop(&mut self) { self.session.stop(); }

    pub(crate) fn clear_for_edit(&mut self) { self.session.clear_marks(); }

    /// Look up the accuracy mark for a score-global onset tick.
    pub(crate) fn mark_for_onset(&self, global_onset_tick: u64) -> Option<AccuracyMark> {
        self.session.data()?.marks_by_onset.get(&global_onset_tick).copied()
    }

    pub(crate) fn record_hit(&mut self, timestamp: f64, timing: &ScoreTiming, score: &Score) {
        if let Some(data) = self.session.data_mut() {
            record_hit(data, timestamp, timing, score);
        }
    }

    pub(crate) fn update_progress(
        &mut self,
        now_seconds: f64,
        timing: &ScoreTiming,
        score: &Score,
    ) {
        if let Some(data) = self.session.data_mut() {
            update_progress(data, now_seconds, timing, score);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grooph_measure::duration::q;
    use grooph_measure::grid::DEFAULT_GRID;
    use grooph_measure::{Beat, Measure, Score, TimeSignature};

    use super::super::session::compute_global_beat_onsets;

    fn score_of_quarters(ts_list: &[TimeSignature]) -> Score {
        Score {
            measures: ts_list
                .iter()
                .map(|ts| {
                    let mut m = Measure::new(*ts);
                    for i in 0..(ts.beats as usize) {
                        m.set_beat(i, Beat::note(q())).unwrap();
                    }
                    m
                })
                .collect(),
        }
    }

    #[test]
    fn compute_global_beat_onsets_offsets_correctly() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR, TimeSignature::THREE_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let onsets = compute_global_beat_onsets(&score, &timing);
        assert_eq!(onsets.len(), 7); // 4 + 3 beats
        let m1_start = timing.measure_start_tick(1);
        let local_q = DEFAULT_GRID.ticks_per_beat(&TimeSignature::THREE_FOUR) as u64;
        // The first beat of measure 1 should sit at m1_start.
        assert_eq!(onsets[4].onset_tick, m1_start);
        // Within measure 1, beat 1 sits one quarter later.
        assert_eq!(onsets[5].onset_tick, m1_start + local_q);
    }

    #[test]
    fn record_hit_matches_note_in_second_measure() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR, TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = AccuracyTracker::new();
        let start_time = 0.0;
        tracker.on_playback_start_at(start_time, 0.0);

        // Aim exactly at the first beat of measure 1.
        let m1_start_tick = timing.measure_start_tick(1);
        let m1_start_seconds = timing.global_tick_to_seconds(m1_start_tick as f64);
        tracker.record_hit(start_time + m1_start_seconds, &timing, &score);

        let mark = tracker.mark_for_onset(m1_start_tick).expect("expected a hit");
        match mark {
            AccuracyMark::Hit(diff) => assert!(diff.abs() < 1e-6, "diff={diff}"),
            _ => panic!("expected a hit"),
        }
    }

    #[test]
    fn record_hit_ignores_rest_beats() {
        // Measure with a rest in the middle. Use a 2/4 measure: beat 0 = note,
        // beat 1 = rest (default Rest in Measure::new). After set_beat(0, q),
        // beat 1 stays Rest because set_beat fills.
        let mut m = Measure::new(TimeSignature::TWO_FOUR);
        m.set_beat(0, Beat::note(q())).unwrap();
        let score = Score::single(m);
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);

        // Hit aimed at the rest's onset (tick = ticks_per_beat).
        let tpb = DEFAULT_GRID.ticks_per_beat(&TimeSignature::TWO_FOUR) as f64;
        let secs = timing.global_tick_to_seconds(tpb);
        tracker.record_hit(secs, &timing, &score);

        // Match should fall back to the only Note beat (onset 0), not the rest.
        assert!(tracker.mark_for_onset(0).is_some());
        assert!(tracker.mark_for_onset(tpb as u64).is_none());
    }

    #[test]
    fn record_hit_is_idempotent_per_loop() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);

        let onset_seconds = timing.global_tick_to_seconds(0.0);
        tracker.record_hit(onset_seconds, &timing, &score);
        // Second hit at slightly different time but still closest to onset 0:
        // should not overwrite, but also not panic.
        tracker.record_hit(onset_seconds + 0.001, &timing, &score);
        let mark = tracker.mark_for_onset(0).expect("first hit recorded");
        match mark {
            AccuracyMark::Hit(diff) => assert!(diff.abs() < 1e-6, "diff={diff}"),
            _ => panic!("expected hit"),
        }
    }

    #[test]
    fn update_progress_marks_miss_in_later_measure() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR, TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);

        // Advance time past the first note of measure 1's hit window without
        // recording any hits. That first note's window-end is between its
        // onset and the next note (½ quarter later).
        let m1_first_onset = timing.measure_start_tick(1);
        let tpb = DEFAULT_GRID.ticks_per_beat(&TimeSignature::FOUR_FOUR) as f64;
        let window_end_tick = m1_first_onset as f64 + 0.75 * tpb;
        let window_end_secs = timing.global_tick_to_seconds(window_end_tick);
        tracker.update_progress(window_end_secs, &timing, &score);

        let mark = tracker
            .mark_for_onset(m1_first_onset)
            .expect("expected miss mark at first note of measure 1");
        assert!(matches!(mark, AccuracyMark::Miss));
    }
}
