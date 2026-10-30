use std::collections::HashMap;

use grooph_measure::Score;
use grooph_measure::tempo::ScoreTiming;

use super::progress::update_progress;
use super::recorder::record_hit;
use super::session::{AccuracyMark, RecordingSession, RunSummary, shift_marks};

/// Coordinates the accuracy-recording session. Holds a `RecordingSession`
/// that is either `Idle` (no playback) or `Recording` (playback active with
/// accumulated hit/miss data). Lifecycle methods drive the state transitions;
/// `record_hit` and `update_progress` are no-ops while idle.
#[derive(Default)]
pub(crate) struct AccuracyTracker {
    session: RecordingSession,
    /// Evaluation of the last finished run with at least one hit.
    last_summary: Option<RunSummary>,
    /// Marks of that run, kept on the staff after stopping until the next
    /// run starts or the score is edited.
    last_marks: HashMap<u64, AccuracyMark>,
}

impl AccuracyTracker {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn has_start_time(&self) -> bool {
        self.session.is_recording()
    }

    /// Keep the session running while `active` (playing with an input
    /// source), end it otherwise.
    pub(crate) fn update_state(&mut self, active: bool, now_seconds: f64) -> bool {
        if active {
            if !self.session.is_recording() {
                self.session.start(now_seconds, Some(0.0));
            }
            true
        } else {
            self.end_session();
            false
        }
    }

    pub(crate) fn on_playback_start(&mut self, now_seconds: Option<f64>) {
        self.last_summary = None;
        self.last_marks.clear();
        match now_seconds {
            Some(t) => self.session.start(t, Some(0.0)),
            None => self.session.stop(),
        }
    }

    /// (Re)start the session anchored at `start_time`. Drops any data of a
    /// session already running, e.g. one re-anchored during the count-in.
    pub(crate) fn on_playback_start_at(&mut self, start_time: f64, last_tick: f64) {
        self.session.start(start_time, Some(last_tick));
    }

    pub(crate) fn realign_start_time(&mut self, start_time: f64, last_tick: f64) {
        self.session.realign(start_time, last_tick);
    }

    pub(crate) fn on_playback_stop(&mut self) {
        self.end_session();
    }

    /// Stop recording without keeping a summary (calibration runs).
    pub(crate) fn discard_session(&mut self) {
        self.session.stop();
    }

    fn end_session(&mut self) {
        if let Some(data) = self.session.data()
            && let Some(summary) = data.summary()
        {
            self.last_summary = Some(summary);
            self.last_marks = data.marks_by_onset.clone();
        }
        self.session.stop();
    }

    /// Running evaluation of the current session.
    pub(crate) fn live_summary(&self) -> Option<RunSummary> {
        self.session.data()?.summary()
    }

    pub(crate) fn last_summary(&self) -> Option<RunSummary> {
        self.last_summary
    }

    pub(crate) fn dismiss_summary(&mut self) {
        self.last_summary = None;
    }

    pub(crate) fn clear_for_edit(&mut self) {
        self.session.clear_marks();
        self.last_marks.clear();
    }

    /// The score lost `ticks` from its front (endless mode dropped a played
    /// measure): the marks of the notes still there move back with them.
    pub(crate) fn drop_leading_ticks(&mut self, ticks: u64) {
        self.session.drop_leading_ticks(ticks);
        self.last_marks = shift_marks(std::mem::take(&mut self.last_marks), ticks);
    }

    /// Whether there are marks to draw: a running session, or the marks a
    /// finished run left behind.
    pub(crate) fn has_marks(&self) -> bool {
        self.session.is_recording() || !self.last_marks.is_empty()
    }

    /// Look up the accuracy mark for a score-global onset tick: the running
    /// session's, else the last finished run's.
    pub(crate) fn mark_for_onset(&self, global_onset_tick: u64) -> Option<AccuracyMark> {
        let marks = match self.session.data() {
            Some(data) => &data.marks_by_onset,
            None => &self.last_marks,
        };
        marks.get(&global_onset_tick).copied()
    }

    /// Returns the hit's deviation in ms if it was matched to a note.
    pub(crate) fn record_hit(
        &mut self,
        timestamp: f64,
        timing: &ScoreTiming,
        score: &Score,
    ) -> Option<f64> {
        record_hit(self.session.data_mut()?, timestamp, timing, score)
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
        Score::new(
            ts_list
                .iter()
                .map(|ts| {
                    let mut m = Measure::new(*ts);
                    for i in 0..(ts.beats as usize) {
                        m.set_beat(i, Beat::note(q())).unwrap();
                    }
                    m
                })
                .collect(),
        )
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
            AccuracyMark::Hit { diff_ticks: diff, .. } => assert!(diff.abs() < 1e-6, "diff={diff}"),
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
            AccuracyMark::Hit { diff_ticks: diff, .. } => assert!(diff.abs() < 1e-6, "diff={diff}"),
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

    #[test]
    fn record_hit_measures_against_swung_onset() {
        use grooph_measure::duration::e;
        use grooph_measure::swing::{Swing, SwingUnit};
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        for i in 0..8 {
            m.set_beat(i, Beat::note(e())).unwrap();
        }
        let score = Score::single(m);
        let swing = Swing { unit: SwingUnit::Eighths, percent: 75 };
        let timing = ScoreTiming::from_score(&score, 120).with_swing(swing);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);

        // Hit exactly where the first "&" sounds (dotted-eighth position).
        let written = DEFAULT_GRID.ticks_of(&e()).unwrap();
        let performed = timing.performed_global_tick(0, written);
        assert_ne!(performed, written as u64);
        tracker.record_hit(timing.global_tick_to_seconds(performed as f64), &timing, &score);

        // The mark belongs to the written note and is dead on.
        match tracker.mark_for_onset(written as u64).expect("hit on the swung note") {
            AccuracyMark::Hit { diff_ticks: diff, .. } => assert!(diff.abs() < 1e-6, "diff={diff}"),
            _ => panic!("expected hit"),
        }
    }

    #[test]
    fn summary_counts_hits_misses_and_extra_hits() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let tpb = DEFAULT_GRID.ticks_per_beat(&TimeSignature::FOUR_FOUR) as f64;
        let beat_s = timing.global_tick_to_seconds(tpb);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        assert!(tracker.live_summary().is_none());

        // Beat 1 dead on, beat 2 played twice (10 ms late, then again),
        // beats 3 and 4 skipped.
        tracker.record_hit(0.0, &timing, &score);
        assert_eq!(tracker.record_hit(beat_s + 0.010, &timing, &score).map(f64::round), Some(10.0));
        assert_eq!(tracker.record_hit(beat_s + 0.050, &timing, &score), None);
        tracker.update_progress(beat_s * 3.9, &timing, &score);

        let live = tracker.live_summary().expect("hits recorded");
        assert_eq!(live.hits, 2);
        assert_eq!(live.perfect, 2);
        assert_eq!(live.extra, 1);
        assert_eq!(live.misses, 2);
        assert!((live.mean_ms - 5.0).abs() < 1e-6);

        tracker.on_playback_stop();
        assert_eq!(tracker.last_summary(), Some(live));
        tracker.dismiss_summary();
        assert!(tracker.last_summary().is_none());
    }

    #[test]
    fn marks_stay_after_stop_until_next_run_or_edit() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let tpb = DEFAULT_GRID.ticks_per_beat(&TimeSignature::FOUR_FOUR) as u64;
        let beat_s = timing.global_tick_to_seconds(tpb as f64);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        tracker.record_hit(0.0, &timing, &score);
        tracker.update_progress(beat_s * 1.9, &timing, &score);
        tracker.on_playback_stop();

        assert!(tracker.has_marks());
        assert!(matches!(tracker.mark_for_onset(0), Some(AccuracyMark::Hit { .. })));
        assert!(matches!(tracker.mark_for_onset(tpb), Some(AccuracyMark::Miss)));
        // Closing the summary keeps the marks on the staff.
        tracker.dismiss_summary();
        assert!(tracker.has_marks());

        tracker.clear_for_edit();
        assert!(!tracker.has_marks());
        assert!(tracker.mark_for_onset(0).is_none());
    }

    #[test]
    fn dropping_leading_ticks_moves_the_marks_back() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR, TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let m1_start = timing.measure_start_tick(1);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        tracker.record_hit(0.0, &timing, &score);
        tracker.record_hit(timing.global_tick_to_seconds(m1_start as f64), &timing, &score);

        // Measure 0 is dropped: its mark goes, measure 1's is now at 0.
        tracker.drop_leading_ticks(m1_start);
        assert!(tracker.mark_for_onset(0).is_some());
        assert!(tracker.mark_for_onset(m1_start).is_none());
        assert_eq!(tracker.live_summary().map(|s| s.hits), Some(2));
        // The same for the marks a finished run leaves on the staff.
        tracker.on_playback_stop();
        tracker.drop_leading_ticks(1);
        assert!(tracker.mark_for_onset(0).is_none());
    }

    #[test]
    fn next_run_starts_without_old_marks() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        tracker.record_hit(0.0, &timing, &score);
        tracker.on_playback_stop();
        assert!(tracker.mark_for_onset(0).is_some());

        tracker.on_playback_start(None);
        assert!(!tracker.has_marks());
        assert!(tracker.mark_for_onset(0).is_none());
    }

    #[test]
    fn fast_notes_grade_with_narrower_windows() {
        use super::super::session::HitGrade;
        use grooph_measure::duration::s;
        // 16ths at 160 BPM are ~94 ms apart; a 40 ms late hit is no longer "good".
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        for i in 0..16 {
            m.set_beat(i, Beat::note(s())).unwrap();
        }
        let score = Score::single(m);
        let timing = ScoreTiming::from_score(&score, 160);
        let sixteenth = DEFAULT_GRID.ticks_of(&s()).unwrap() as f64;
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        tracker.record_hit(0.040, &timing, &score);
        let second = timing.global_tick_to_seconds(sixteenth);
        tracker.record_hit(second + 0.030, &timing, &score);

        let grade = |tick: u64| match tracker.mark_for_onset(tick) {
            Some(AccuracyMark::Hit { grade, .. }) => grade,
            _ => panic!("expected a hit at {tick}"),
        };
        assert_eq!(grade(0), HitGrade::Off);
        assert_eq!(grade(sixteenth as u64), HitGrade::Good);
        let summary = tracker.live_summary().unwrap();
        assert_eq!((summary.perfect, summary.good, summary.off()), (0, 1, 1));
    }

    #[test]
    fn run_without_hits_leaves_no_summary() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        tracker.update_progress(1.9, &timing, &score);
        tracker.on_playback_stop();
        assert!(tracker.last_summary().is_none());
    }

    #[test]
    fn discarded_session_leaves_no_summary() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        tracker.record_hit(0.0, &timing, &score);
        tracker.discard_session();
        assert!(tracker.last_summary().is_none());
    }
}
