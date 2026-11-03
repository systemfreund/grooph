use std::collections::HashMap;

use grooph_measure::Score;
use grooph_measure::tempo::ScoreTiming;

use super::drift::Drift;
use super::positions::BeatPositions;
use super::progress::update_progress;
use super::recorder::record_hit;
use super::session::{AccuracyMark, RecordingSession, RunSummary, TRACKED_VOICES, shift_marks};

/// Coordinates the accuracy-recording session. Holds a `RecordingSession`
/// that is either `Idle` (no playback) or `Recording` (playback active with
/// accumulated hit/miss data). Lifecycle methods drive the state transitions;
/// `record_hit` and `update_progress` are no-ops while idle.
#[derive(Default)]
pub(crate) struct AccuracyTracker {
    session: RecordingSession,
    /// Evaluation of the last finished run with at least one hit.
    last_summary: Option<RunSummary>,
    /// That run per tracked voice.
    last_voice_summaries: [Option<RunSummary>; TRACKED_VOICES],
    /// That run's bars with the click and silent bars, if the gap click
    /// silenced any.
    last_bar_summaries: [Option<RunSummary>; 2],
    /// That run over time.
    last_drift: Option<Drift>,
    /// That run by position in the beat.
    last_positions: Option<BeatPositions>,
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
        self.last_drift = None;
        self.last_positions = None;
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
        if let Some(data) = self.session.data_mut() {
            data.bars.settle();
        }
        if let Some(data) = self.session.data()
            && let Some(summary) = data.summary()
        {
            self.last_summary = Some(summary);
            self.last_voice_summaries = data.voice_summaries();
            self.last_bar_summaries = data.bar_summaries();
            self.last_drift = data.drift.finish(data.bars.silent_spans());
            self.last_positions = data.positions.finish(summary.mean_ms);
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

    /// Running evaluation of the current session per tracked voice.
    pub(crate) fn live_voice_summaries(&self) -> [Option<RunSummary>; TRACKED_VOICES] {
        self.session.data().map_or([None; TRACKED_VOICES], |d| d.voice_summaries())
    }

    /// The last finished run per tracked voice, as [`Self::last_summary`].
    pub(crate) fn last_voice_summaries(&self) -> [Option<RunSummary>; TRACKED_VOICES] {
        self.last_voice_summaries
    }

    /// The last finished run in bars with the click and in silent bars, as
    /// [`Self::last_summary`]; both `None` unless a bar was silent.
    pub(crate) fn last_bar_summaries(&self) -> [Option<RunSummary>; 2] {
        self.last_bar_summaries
    }

    /// The audio engine plays a bar the gap click silences (`Some(true)`)
    /// or one with the click (`Some(false)`) at `now_seconds`; `None` while
    /// the gap click is off. Only a frame well inside a bar tells which bar
    /// it is: around its edges, the engine's bar and the session's clock may
    /// be a frame apart.
    pub(crate) fn observe_bar(
        &mut self,
        now_seconds: f64,
        muted: Option<bool>,
        timing: &ScoreTiming,
    ) {
        let Some(data) = self.session.data_mut() else {
            return;
        };
        let Some(muted) = muted else {
            data.bars.observe(None);
            return;
        };
        let elapsed = now_seconds - data.start_time;
        if timing.total_loop_seconds() <= 0.0 || elapsed < 0.0 {
            return;
        }
        let (into, length) = timing.measure_span_at(elapsed);
        if length <= 0.0 || !(0.25..=0.75).contains(&(into / length)) {
            return;
        }
        let start_s = now_seconds - into;
        data.bars.observe(Some((start_s, start_s + length, muted)));
    }

    /// The last finished run over time, as [`Self::last_summary`]; `None`
    /// with fewer than two hits.
    pub(crate) fn last_drift(&self) -> Option<&Drift> {
        self.last_drift.as_ref()
    }

    /// The last finished run by position in the beat, as
    /// [`Self::last_summary`]; `None` unless its notes fell on two
    /// positions.
    pub(crate) fn last_positions(&self) -> Option<&BeatPositions> {
        self.last_positions.as_ref()
    }

    pub(crate) fn dismiss_summary(&mut self) {
        self.last_summary = None;
        self.last_drift = None;
        self.last_positions = None;
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
        self.update_progress_behind(now_seconds, 0.0, timing, score);
    }

    /// [`Self::update_progress`], judging misses `delay_s` behind the clock
    /// so that hits delayed by input latency are in.
    pub(crate) fn update_progress_behind(
        &mut self,
        now_seconds: f64,
        delay_s: f64,
        timing: &ScoreTiming,
        score: &Score,
    ) {
        if let Some(data) = self.session.data_mut() {
            update_progress(data, now_seconds, delay_s, timing, score);
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
                        m.first_voice_mut().set_beat(i, Beat::note(q())).unwrap();
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

    /// Plays three bars of quarters at 120 BPM, frame by frame, the second
    /// bar silent: `hits` (seconds) in order, the engine's bar at each
    /// frame's time as `muted` says.
    fn play_bars(hits: &[f64], muted: impl Fn(f64) -> Option<bool>) -> AccuracyTracker {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        let mut hits = hits.iter().copied().peekable();
        for frame in 1..=360 {
            let now = frame as f64 / 60.0;
            while let Some(hit) = hits.next_if(|&h| h <= now) {
                tracker.record_hit(hit, &timing, &score);
            }
            tracker.update_progress(now, &timing, &score);
            tracker.observe_bar(now, muted(now), &timing);
        }
        tracker.on_playback_stop();
        tracker
    }

    #[test]
    fn silent_bars_are_scored_apart() {
        // On time with the click; 30 ms late and one note missed in the
        // silent bar; the click's return hit 10 ms early, a bar later.
        let hits = [0.0, 0.5, 1.0, 1.5, 2.03, 2.53, 3.03, 3.99, 4.5, 5.0, 5.5];
        // Bars of 2 s.
        let tracker = play_bars(&hits, |now| Some((now / 2.0) as u64 == 1));
        let [Some(audible), Some(silent)] = tracker.last_bar_summaries() else {
            panic!("a bar was silent");
        };
        assert_eq!((silent.hits, silent.misses), (3, 1));
        assert!((silent.mean_ms - 30.0).abs() < 1e-6, "{}", silent.mean_ms);
        // The early hit on the next downbeat belongs to the bar it is for.
        assert_eq!((audible.hits, audible.misses), (8, 0));
        assert!((audible.mean_ms + 1.25).abs() < 1e-6, "{}", audible.mean_ms);
        assert_eq!(tracker.last_summary().map(|s| (s.hits, s.misses)), Some((11, 1)));
        // Over time: every hit and the missed note at its note's time, and
        // the silent bar shaded.
        let drift = tracker.last_drift().expect("hits recorded");
        assert_eq!(drift.hits.len(), 11);
        assert!(
            (drift.hits[4].t - 2.0).abs() < 1e-6 && (drift.hits[4].delta_ms - 30.0).abs() < 1e-6
        );
        assert_eq!(drift.misses.len(), 1);
        assert!((drift.misses[0] - 3.5).abs() < 1e-6, "{:?}", drift.misses);
        assert_eq!(drift.silent.len(), 1);
        let (start, end) = drift.silent[0];
        assert!((start - 2.0).abs() < 1e-6 && (end - 4.0).abs() < 1e-6, "{start} {end}");

        // Gone with the summary, and when the next run starts.
        let mut dismissed = play_bars(&hits, |_| None);
        dismissed.dismiss_summary();
        assert!(dismissed.last_drift().is_none());
        let mut restarted = play_bars(&hits, |_| None);
        assert!(restarted.last_drift().is_some());
        restarted.on_playback_start(None);
        assert!(restarted.last_drift().is_none());

        // The engine a little ahead of the session's clock, or behind it,
        // does not move a note to another bar.
        for skew in [-0.05, 0.05] {
            let skewed = play_bars(&hits, |now| Some(((now + skew) / 2.0) as u64 == 1));
            assert_eq!(skewed.last_bar_summaries(), tracker.last_bar_summaries());
        }
    }

    #[test]
    fn no_bar_scores_without_a_silent_bar() {
        let hits = [0.0, 0.5, 1.0, 1.5, 2.0];
        assert_eq!(play_bars(&hits, |_| Some(false)).last_bar_summaries(), [None, None]);
        // The gap click off.
        assert_eq!(play_bars(&hits, |_| None).last_bar_summaries(), [None, None]);
    }

    #[test]
    fn hits_and_misses_count_for_each_voice_with_a_note_there() {
        // Click quarters above, counter eighths on the first beat below.
        let score = grooph_measure::notation::parse_score("2/4 q q ; e e r:q").unwrap();
        let timing = ScoreTiming::from_score(&score, 120);
        let q = DEFAULT_GRID.ticks_per_beat(&TimeSignature::TWO_FOUR) as f64;
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        // The shared downbeat and the counter's second eighth are hit; the
        // click's second quarter is missed.
        tracker.record_hit(timing.global_tick_to_seconds(0.0), &timing, &score);
        tracker.record_hit(timing.global_tick_to_seconds(q / 2.0), &timing, &score);
        tracker.update_progress(timing.global_tick_to_seconds(1.9 * q), &timing, &score);

        let [Some(click), Some(counter)] = tracker.live_voice_summaries() else {
            panic!("both voices have notes");
        };
        assert_eq!((click.hits, click.misses), (1, 1));
        assert_eq!((counter.hits, counter.misses), (2, 0));
        let total = tracker.live_summary().unwrap();
        assert_eq!((total.hits, total.misses), (2, 1));

        // Tight counts per voice, each graded around its own voice's
        // average: the click's one hit is perfect around itself.
        assert_eq!(click.tight, (1, 0));

        tracker.on_playback_stop();
        assert_eq!(tracker.last_voice_summaries(), [Some(click), Some(counter)]);
    }

    #[test]
    fn a_voice_without_notes_has_no_summary() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        assert_eq!(tracker.live_voice_summaries(), [None, None]);
        tracker.record_hit(0.0, &timing, &score);
        assert!(matches!(tracker.live_voice_summaries(), [Some(_), None]));
    }

    #[test]
    fn a_late_hit_judged_after_its_delay_is_no_miss() {
        // Quarters at 120 BPM: the second note's window closes at 0.75 s.
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        // A hit on it (0.5 s) arrives at 0.8 s with 200 ms latency: misses
        // are judged 0.26 s behind, so not yet.
        let delay = 0.26;
        tracker.update_progress(0.8 - delay, &timing, &score);
        tracker.record_hit(0.8 - 0.3, &timing, &score);
        tracker.update_progress(1.2 - delay, &timing, &score);
        let summary = tracker.live_summary().unwrap();
        assert_eq!((summary.hits, summary.misses), (1, 1), "the first note was missed");
        assert!(matches!(tracker.mark_for_onset(0), Some(AccuracyMark::Miss)));
    }

    #[test]
    fn a_late_hit_after_the_loop_end_counts_for_the_next_loop() {
        // One bar of quarters at 120 BPM loops every 2 s.
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        // The delayed cursor is still in the first loop (all missed) when a
        // hit on the next loop's first note comes in.
        tracker.update_progress(1.9, &timing, &score);
        tracker.record_hit(2.02, &timing, &score);
        // Past that note's window: it was hit, not missed.
        tracker.update_progress(2.3, &timing, &score);
        let summary = tracker.live_summary().unwrap();
        assert_eq!((summary.hits, summary.misses), (1, 4));
        assert!(matches!(tracker.mark_for_onset(0), Some(AccuracyMark::Hit { .. })));
    }

    #[test]
    fn a_hit_after_the_cursor_wrapped_counts_for_the_loop_before() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        tracker.update_progress(1.9, &timing, &score);
        tracker.update_progress(2.1, &timing, &score);
        // Later than any delay: a hit on the first loop's last note (1.5 s).
        // It counts, but leaves the second loop's last note to be missed.
        tracker.record_hit(1.5, &timing, &score);
        tracker.update_progress(3.9, &timing, &score);
        let summary = tracker.live_summary().unwrap();
        assert_eq!((summary.hits, summary.misses), (1, 8));
        assert!(matches!(tracker.mark_for_onset(3 * 5040), Some(AccuracyMark::Miss)));
    }

    #[test]
    fn an_early_hit_on_the_next_loop_after_the_cursor_wrapped_counts_now() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        tracker.update_progress(1.9, &timing, &score);
        // The cursor has just wrapped when an early hit on the new loop's
        // first note (played at 1.98 s) comes in: it is that note's.
        tracker.update_progress(2.01, &timing, &score);
        tracker.record_hit(1.98, &timing, &score);
        tracker.update_progress(2.4, &timing, &score);
        let summary = tracker.live_summary().unwrap();
        assert_eq!((summary.hits, summary.misses), (1, 4));
        assert!(matches!(tracker.mark_for_onset(0), Some(AccuracyMark::Hit { .. })));
    }

    #[test]
    fn a_long_stall_still_wraps() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        tracker.update_progress(1.6, &timing, &score);
        // No frame for 1.9 s (a background tab): the cursor lands a little
        // before where it was, a loop later. That is a wrap, not a wait.
        tracker.update_progress(3.5, &timing, &score);
        assert_eq!(tracker.live_summary(), None);
        tracker.record_hit(3.5, &timing, &score);
        // First loop: 4 missed; second loop: 3 missed before the hit.
        assert_eq!(tracker.live_summary().unwrap().misses, 7);
    }

    #[test]
    fn a_longer_delay_waits_instead_of_wrapping() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        tracker.update_progress_behind(1.0, 0.06, &timing, &score);
        // A MIDI device with a 300 ms offset connects: the judged time
        // steps back, which is no wrap.
        tracker.update_progress_behind(1.05, 0.36, &timing, &score);
        tracker.update_progress_behind(1.3, 0.36, &timing, &score);
        tracker.record_hit(0.0, &timing, &score);
        let summary = tracker.live_summary().unwrap();
        // The first two notes were missed, no more: no wrap.
        assert_eq!(summary.misses, 2);
    }

    #[test]
    fn progress_waits_when_anchored_ahead_of_its_clock() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let tpb = DEFAULT_GRID.ticks_per_beat(&TimeSignature::FOUR_FOUR) as f64;
        let mut tracker = AccuracyTracker::new();
        // Anchored (e.g. after a tempo change) at the second beat, then
        // asked about a time a little before it: no wrap, no misses.
        tracker.on_playback_start_at(0.0, tpb);
        tracker.update_progress(timing.global_tick_to_seconds(0.8 * tpb), &timing, &score);
        tracker.update_progress(timing.global_tick_to_seconds(1.2 * tpb), &timing, &score);
        assert!(tracker.mark_for_onset(0).is_none());
        assert!(tracker.mark_for_onset(2 * tpb as u64).is_none());
    }

    #[test]
    fn record_hit_ignores_rest_beats() {
        // Measure with a rest in the middle. Use a 2/4 measure: beat 0 = note,
        // beat 1 = rest (default Rest in Measure::new). After set_beat(0, q),
        // beat 1 stays Rest because set_beat fills.
        let mut m = Measure::new(TimeSignature::TWO_FOUR);
        m.first_voice_mut().set_beat(0, Beat::note(q())).unwrap();
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
            m.first_voice_mut().set_beat(i, Beat::note(e())).unwrap();
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
        // Graded around their average, live as well.
        assert_eq!(live.tight, (2, 0));

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
    fn an_edit_drops_the_hits_before_it_from_the_chart() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        tracker.record_hit(0.010, &timing, &score);
        tracker.record_hit(0.510, &timing, &score);
        tracker.clear_for_edit();
        tracker.record_hit(1.0, &timing, &score);
        tracker.record_hit(1.5, &timing, &score);
        tracker.on_playback_stop();
        let drift = tracker.last_drift().expect("two hits after the edit");
        assert_eq!(drift.hits.iter().map(|h| h.delta_ms.round()).collect::<Vec<_>>(), [0.0, 0.0]);
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
            m.first_voice_mut().set_beat(i, Beat::note(s())).unwrap();
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

    #[test]
    fn early_downbeats_and_late_offbeats_show_apart() {
        let bar = "e e e e e e e e";
        let score =
            grooph_measure::notation::parse_score(&format!("4/4 {bar} | {bar} | {bar} | {bar}"))
                .unwrap();
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        // Eighths of 0.25 s: the beats 20 ms early, the "&"s 20 ms late,
        // from the second note on (the first early hit is before the start).
        for i in 1..17 {
            let off = if i % 2 == 0 { -0.02 } else { 0.02 };
            tracker.record_hit(i as f64 * 0.25 + off, &timing, &score);
        }
        tracker.on_playback_stop();
        assert!(tracker.last_summary().unwrap().mean_ms.abs() < 1e-6);
        let positions = tracker.last_positions().expect("two positions");
        let means: Vec<(u8, f64)> =
            positions.positions.iter().map(|p| (p.pos.parts, p.mean_ms.unwrap().round())).collect();
        assert_eq!(means, [(1, -20.0), (2, 20.0)]);
        assert!(positions.notable.is_some());
        tracker.dismiss_summary();
        assert!(tracker.last_positions().is_none());
    }
}
