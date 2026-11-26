use std::collections::HashMap;

use grooph_measure::tempo::ScoreTiming;
use grooph_measure::{Instrument, Score};

use super::drift::Drift;
use super::positions::PositionTallies;
use super::progress::update_progress;
use super::recorder::{finalise, record_hit};
use super::session::{
    AccuracyMark, ExtraMark, NoteKey, RecordingSession, RunSummary, TRACKED_VOICES, shift_extras,
    shift_marks,
};

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
    last_positions: Option<PositionTallies>,
    /// Marks of that run, kept on the staff after stopping until the next
    /// run starts or the score is edited.
    last_marks: HashMap<NoteKey, AccuracyMark>,
    /// That run's extra hits, kept as its marks are.
    last_extras: Vec<ExtraMark>,
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
        self.last_extras.clear();
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

    /// The running session's clock anchor and progress cursor (tick).
    #[cfg(test)]
    pub(crate) fn anchor(&self) -> Option<(f64, Option<f64>)> {
        self.session.data().map(|d| (d.start_time, d.last_tick))
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
            // The open stretch with the hits in, as far as it got.
            finalise(data, true);
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
            self.last_extras = data.extras.clone();
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
    pub(crate) fn last_positions(&self) -> Option<&PositionTallies> {
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
        self.last_extras.clear();
    }

    /// The score lost `ticks` from its front (endless mode dropped a played
    /// measure): the marks of the notes still there move back with them.
    pub(crate) fn drop_leading_ticks(&mut self, ticks: u64) {
        self.session.drop_leading_ticks(ticks);
        self.last_marks = shift_marks(std::mem::take(&mut self.last_marks), ticks);
        self.last_extras = shift_extras(std::mem::take(&mut self.last_extras), ticks);
    }

    /// Whether there are marks to draw: a running session, or the marks a
    /// finished run left behind.
    pub(crate) fn has_marks(&self) -> bool {
        self.session.is_recording() || !self.last_marks.is_empty()
    }

    /// Look up the accuracy mark for a score-global onset tick of a
    /// one-line score: the running session's, else the last finished run's.
    pub(crate) fn mark_for_onset(&self, global_onset_tick: u64) -> Option<AccuracyMark> {
        self.mark_for_note(NoteKey::onset(global_onset_tick))
    }

    /// Look up the accuracy mark of the head of `instrument` at a
    /// score-global onset tick of a kit score, as [`Self::mark_for_onset`].
    pub(crate) fn mark_for_head(
        &self,
        global_onset_tick: u64,
        instrument: Instrument,
    ) -> Option<AccuracyMark> {
        self.mark_for_note(NoteKey { tick: global_onset_tick, instrument })
    }

    /// The extra hits to draw: the running session's, else the last
    /// finished run's.
    pub(crate) fn extra_marks(&self) -> &[ExtraMark] {
        match self.session.data() {
            Some(data) => &data.extras,
            None => &self.last_extras,
        }
    }

    fn mark_for_note(&self, key: NoteKey) -> Option<AccuracyMark> {
        let marks = match self.session.data() {
            Some(data) => &data.marks_by_onset,
            None => &self.last_marks,
        };
        marks.get(&key).copied()
    }

    /// A hit on a one-line score (from any source). Which note it counts
    /// for is decided once that note is final (see `recorder::record_hit`).
    /// Returns the hit's deviation in ms from the nearest note, at once:
    /// the calibration's measure.
    pub(crate) fn record_hit(
        &mut self,
        timestamp: f64,
        timing: &ScoreTiming,
        score: &Score,
    ) -> Option<f64> {
        record_hit(self.session.data_mut()?, timestamp, None, timing, score)
    }

    /// A hit on `instrument`: on a kit score aligned with its heads only
    /// (see `recorder::record_hit`), on one line as [`Self::record_hit`].
    /// Returns the deviation from the instrument's nearest head within the
    /// catch window.
    pub(crate) fn record_instrument_hit(
        &mut self,
        timestamp: f64,
        instrument: Instrument,
        timing: &ScoreTiming,
        score: &Score,
    ) -> Option<f64> {
        record_hit(self.session.data_mut()?, timestamp, Some(instrument), timing, score)
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

    pub(super) fn score_of_quarters(ts_list: &[TimeSignature]) -> Score {
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
        // Final halfway between the next two notes.
        assert!(tracker.mark_for_onset(m1_start_tick).is_none());
        tracker.update_progress(m1_start_seconds + 0.76, &timing, &score);

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
        // The second quarter is final halfway between the next loop's two
        // first notes.
        tracker.update_progress(timing.global_tick_to_seconds(2.3 * q), &timing, &score);

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
        tracker.update_progress(0.8, &timing, &score);
        assert!(matches!(tracker.live_voice_summaries(), [Some(_), None]));
    }

    #[test]
    fn a_late_hit_judged_after_its_delay_is_no_miss() {
        // Quarters at 120 BPM: the second note's window closes at 0.75 s.
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        // A hit on it (0.5 s) arrives at 0.8 s with 200 ms latency: notes
        // are judged 0.26 s behind, so not yet. It is final at 1.25 s,
        // halfway between the next two notes.
        let delay = 0.26;
        tracker.update_progress(0.8 - delay, &timing, &score);
        tracker.record_hit(0.8 - 0.3, &timing, &score);
        tracker.update_progress(1.55 - delay, &timing, &score);
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
        // Past that note's lookahead: it was hit, not missed.
        tracker.update_progress(2.8, &timing, &score);
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
        // Later than any delay: a hit on the first loop's last note (1.5 s),
        // not final before 2.25 s. It counts for it.
        tracker.record_hit(1.5, &timing, &score);
        tracker.update_progress(3.9, &timing, &score);
        let summary = tracker.live_summary().unwrap();
        assert_eq!((summary.hits, summary.misses), (1, 6));
        assert!(matches!(tracker.mark_for_onset(3 * 5040), Some(AccuracyMark::Hit { .. })));
        // The second loop's last note is missed when not played.
        tracker.update_progress(4.3, &timing, &score);
        assert_eq!(tracker.live_summary().unwrap().misses, 7);
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
        tracker.update_progress(2.8, &timing, &score);
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
        tracker.update_progress(4.3, &timing, &score);
        // First loop: 4 missed; second loop: 3 missed before the hit.
        let summary = tracker.live_summary().unwrap();
        assert_eq!((summary.hits, summary.misses), (1, 7));
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
        tracker.record_hit(1.0, &timing, &score);
        tracker.update_progress_behind(2.2, 0.36, &timing, &score);
        let summary = tracker.live_summary().unwrap();
        // The first two notes were missed, no more: no wrap.
        assert_eq!((summary.hits, summary.misses), (1, 2));
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
        // Final once the next loop's note is half a second behind.
        tracker.update_progress(0.9, &timing, &score);
        tracker.update_progress(1.6, &timing, &score);

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
        tracker.update_progress(0.8, &timing, &score);
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

        // Advance time past the first note of measure 1's lookahead without
        // recording any hits: halfway between the next two notes.
        let m1_first_onset = timing.measure_start_tick(1);
        let tpb = DEFAULT_GRID.ticks_per_beat(&TimeSignature::FOUR_FOUR) as f64;
        let window_end_tick = m1_first_onset as f64 + 1.6 * tpb;
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
        tracker.update_progress(1.0, &timing, &score);

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
        // The calibration's delta, to the nearest note, for either.
        assert_eq!(tracker.record_hit(beat_s + 0.050, &timing, &score).map(f64::round), Some(50.0));
        tracker.update_progress(beat_s * 3.9, &timing, &score);
        // The last note is final halfway into the next loop's first beat.
        tracker.update_progress(beat_s * 4.6, &timing, &score);

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
        tracker.update_progress(beat_s * 2.6, &timing, &score);
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
        tracker.update_progress(2.8, &timing, &score);

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
        tracker.update_progress(0.4, &timing, &score);

        let grade = |tick: u64| match tracker.mark_for_onset(tick) {
            Some(AccuracyMark::Hit { grade, .. }) => grade,
            _ => panic!("expected a hit at {tick}"),
        };
        assert_eq!(grade(0), HitGrade::Off);
        assert_eq!(grade(sixteenth as u64), HitGrade::Good);
        let summary = tracker.live_summary().unwrap();
        assert_eq!((summary.perfect, summary.good, summary.off()), (0, 1, 1));
    }

    /// Moves the progress cursor from `from` to `to` (s) in frames of 60 fps,
    /// judging `delay_s` behind the clock.
    fn frames_behind(
        tracker: &mut AccuracyTracker,
        (from, to): (f64, f64),
        delay_s: f64,
        score: &Score,
        timing: &ScoreTiming,
    ) {
        let mut t = from;
        while t < to {
            t = (t + 1.0 / 60.0).min(to);
            tracker.update_progress_behind(t, delay_s, timing, score);
        }
    }

    #[test]
    fn a_late_hit_on_a_note_before_a_rest_is_no_miss_as_well() {
        use grooph_measure::notation::parse_score;
        // A on 1 (0 s), a rest on 2, B on 3 (1 s) at 120 BPM. A played 400 ms
        // late, nearer to A than to B, while the cursor judges 60 ms behind:
        // the rest does not end A's window, so A is hit, not missed and hit.
        let score = parse_score("4/4 q r:q q q").unwrap();
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        frames_behind(&mut tracker, (0.0, 0.4), 0.06, &score, &timing);
        let delta = tracker.record_hit(0.4, &timing, &score).expect("A's hit");
        assert!((delta - 400.0).abs() < 1e-6);
        // Final halfway between B and the note after it.
        frames_behind(&mut tracker, (0.4, 1.4), 0.06, &score, &timing);
        let s = tracker.live_summary().unwrap();
        assert_eq!((s.hits, s.misses, s.extra), (1, 0, 0));
        assert!(matches!(tracker.mark_for_onset(0), Some(AccuracyMark::Hit { .. })));
    }

    #[test]
    fn the_same_music_written_otherwise_is_judged_the_same() {
        use grooph_measure::notation::parse_score;
        // A quarter, or a sixteenth and rests: the same four hits to play.
        // At 160 BPM a sixteenth's gap would narrow the first note's windows
        // to 18.75/37.5 ms if the rest after it counted.
        let run = |text: &str| {
            let score = parse_score(text).unwrap();
            let timing = ScoreTiming::from_score(&score, 160);
            let mut tracker = AccuracyTracker::new();
            tracker.on_playback_start_at(0.0, 0.0);
            let beat = 60.0 / 160.0;
            for hit in [0.020, beat - 0.050, 2.0 * beat + 0.140] {
                tracker.record_hit(hit, &timing, &score);
            }
            frames_behind(&mut tracker, (0.0, 4.0 * beat - 0.05), 0.0, &score, &timing);
            let grade = match tracker.mark_for_onset(0) {
                Some(AccuracyMark::Hit { grade, .. }) => Some(grade),
                _ => None,
            };
            (tracker.live_summary(), grade)
        };
        let quarters = run("4/4 q q q q");
        assert_eq!(quarters, run("4/4 s r:s r:e q q q"));
        // 20 ms late on a quarter's windows: perfect (narrowed by the rest it
        // would only be good).
        assert_eq!(quarters.1, Some(super::super::session::HitGrade::Perfect));
    }

    #[test]
    fn a_bar_starting_with_rests_judges_its_last_note_past_the_loop_end() {
        use grooph_measure::notation::parse_score;
        // Notes at 1 s and 1.5 s of a 2 s loop: the last one's window ends
        // at the next loop's first note, and it is final halfway between
        // that one and the next, at 3.25 s.
        let score = parse_score("4/4 r:h q q").unwrap();
        let timing = ScoreTiming::from_score(&score, 120);
        let last = timing.seconds_to_global_tick(1.5).round() as u64;
        let mut unplayed = AccuracyTracker::new();
        unplayed.on_playback_start_at(0.0, 0.0);
        unplayed.record_hit(1.0, &timing, &score);
        frames_behind(&mut unplayed, (0.0, 3.2), 0.0, &score, &timing);
        assert!(unplayed.mark_for_onset(last).is_none());
        frames_behind(&mut unplayed, (3.2, 3.3), 0.0, &score, &timing);
        assert!(matches!(unplayed.mark_for_onset(last), Some(AccuracyMark::Miss)));
        // Played 600 ms late, after the loop's end: a hit, and no miss.
        let mut late = AccuracyTracker::new();
        late.on_playback_start_at(0.0, 0.0);
        late.record_hit(1.0, &timing, &score);
        frames_behind(&mut late, (0.0, 2.05), 0.0, &score, &timing);
        let delta = late.record_hit(2.1, &timing, &score).expect("the last note's hit");
        assert!((delta - 600.0).abs() < 1e-6);
        frames_behind(&mut late, (2.05, 3.3), 0.0, &score, &timing);
        let s = late.live_summary().unwrap();
        assert_eq!((s.hits, s.misses, s.extra), (2, 0, 0));
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
        let positions = tracker
            .last_positions()
            .and_then(|t| t.in_system(grooph_measure::counting::CountingSystem::Numbers))
            .expect("two positions");
        let means: Vec<(u8, f64)> =
            positions.positions.iter().map(|p| (p.pos.parts, p.mean_ms.unwrap().round())).collect();
        assert_eq!(means, [(1, -20.0), (2, 20.0)]);
        assert!(positions.notable.is_some());
        tracker.dismiss_summary();
        assert!(tracker.last_positions().is_none());
    }
}

/// Kit scores: each head is judged against the hits on its own instrument.
#[cfg(test)]
mod kit_tests {
    use super::tests::score_of_quarters;
    use super::*;
    use Instrument::{HiHat, HiHatFoot, Kick, Snare};
    use grooph_measure::notation::parse_score;
    use grooph_measure::{Instrument, StaffKind, TimeSignature};

    use super::super::session::{HitGrade, compute_global_beat_onsets};

    /// Hi-hat eighths, snare on 2 and 4, kick on 1 and 3, at 120 BPM: an
    /// eighth is 250 ms.
    fn groove() -> (Score, ScoreTiming) {
        let score =
            parse_score("4/4 H-K:e H:e H-S:e H:e H-K:e H:e H-S:e H:e").expect("a kit groove");
        assert_eq!(score.staff, StaffKind::Kit);
        let timing = ScoreTiming::from_score(&score, 120);
        (score, timing)
    }

    /// Global tick at `seconds` into the groove.
    fn tick_at(timing: &ScoreTiming, seconds: f64) -> u64 {
        timing.seconds_to_global_tick(seconds).round() as u64
    }

    fn recording() -> AccuracyTracker {
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        tracker
    }

    fn grade(tracker: &AccuracyTracker, tick: u64, instrument: Instrument) -> Option<HitGrade> {
        match tracker.mark_for_head(tick, instrument)? {
            AccuracyMark::Hit { grade, .. } => Some(grade),
            AccuracyMark::Miss => None,
        }
    }

    #[test]
    fn each_head_is_one_entry() {
        let (score, timing) = groove();
        let onsets = compute_global_beat_onsets(&score, &timing);
        // 8 hi-hats, 2 kicks, 2 snares.
        assert_eq!(onsets.len(), 12);
        let first: Vec<Instrument> =
            onsets.iter().filter(|o| o.onset_tick == 0).map(|o| o.instrument).collect();
        assert_eq!(first, [Kick, HiHat]);
        assert!(onsets.iter().all(|o| o.is_note));
    }

    #[test]
    fn a_chord_played_whole_is_two_hits_with_two_marks() {
        let (score, timing) = groove();
        let mut tracker = recording();
        assert_eq!(tracker.record_instrument_hit(0.0, Kick, &timing, &score), Some(0.0));
        assert_eq!(tracker.record_instrument_hit(0.0, HiHat, &timing, &score), Some(0.0));
        // The kick is final halfway to the next bar's kick, at 1.5 s.
        tracker.update_progress(1.6, &timing, &score);
        assert_eq!(grade(&tracker, 0, Kick), Some(HitGrade::Perfect));
        assert_eq!(grade(&tracker, 0, HiHat), Some(HitGrade::Perfect));
        let live = tracker.live_summary().unwrap();
        assert_eq!((live.hits, live.extra), (2, 0));
    }

    #[test]
    fn a_head_left_out_of_a_chord_is_missed() {
        let (score, timing) = groove();
        let mut tracker = recording();
        tracker.record_instrument_hit(0.0, HiHat, &timing, &score);
        tracker.record_instrument_hit(0.25, HiHat, &timing, &score);
        // The kick's window ends 150 ms after it; it is final at 1.5 s. A
        // stop before that judges what it has.
        tracker.update_progress(0.55, &timing, &score);
        assert!(tracker.mark_for_head(0, Kick).is_none());
        tracker.on_playback_stop();
        assert!(matches!(tracker.mark_for_head(0, Kick), Some(AccuracyMark::Miss)));
        assert_eq!(grade(&tracker, 0, HiHat), Some(HitGrade::Perfect));
        // The hi-hat on 2, its window still open, is no miss.
        let last = tracker.last_summary().unwrap();
        assert_eq!((last.hits, last.misses, last.extra), (2, 1, 0));
    }

    #[test]
    fn a_kick_where_no_kick_is_written_is_extra() {
        let (score, timing) = groove();
        let mut tracker = recording();
        tracker.record_instrument_hit(0.0, HiHat, &timing, &score);
        // Beat 2 has a snare and a hi-hat, no kick: 500 ms from either kick.
        assert_eq!(tracker.record_instrument_hit(0.5, Kick, &timing, &score), None);
        assert!(tracker.mark_for_head(0, Kick).is_none());
        assert!(tracker.mark_for_head(tick_at(&timing, 0.5), Snare).is_none());
        assert!(tracker.mark_for_head(tick_at(&timing, 1.0), Kick).is_none());
        // Extra at once; the hi-hat once final.
        tracker.update_progress(0.4, &timing, &score);
        let live = tracker.live_summary().unwrap();
        assert_eq!((live.hits, live.extra), (1, 1));
        // Both kicks are still open, and missed when not played.
        tracker.update_progress(1.9, &timing, &score);
        tracker.update_progress(2.6, &timing, &score);
        assert!(matches!(tracker.mark_for_head(0, Kick), Some(AccuracyMark::Miss)));
        assert!(matches!(
            tracker.mark_for_head(tick_at(&timing, 1.0), Kick),
            Some(AccuracyMark::Miss)
        ));
    }

    #[test]
    fn a_late_kick_counts_within_the_catch_window_only() {
        let (score, timing) = groove();
        let mut tracker = recording();
        let delta = tracker.record_instrument_hit(0.1, Kick, &timing, &score).expect("caught");
        assert!((delta - 100.0).abs() < 1e-6);
        tracker.update_progress(1.6, &timing, &score);
        assert_eq!(grade(&tracker, 0, Kick), Some(HitGrade::Off));

        let mut tracker = recording();
        tracker.record_instrument_hit(0.0, HiHat, &timing, &score);
        assert_eq!(tracker.record_instrument_hit(0.2, Kick, &timing, &score), None);
        tracker.update_progress(0.4, &timing, &score);
        assert!(tracker.mark_for_head(0, Kick).is_none());
        let live = tracker.live_summary().unwrap();
        assert_eq!((live.hits, live.extra), (1, 1));
    }

    #[test]
    fn the_calibration_delta_is_to_the_instruments_nearest_head() {
        // Hi-hat sixteenths at 120 BPM: 125 ms apart.
        let score = parse_score("4/4 H:s H:s H:s H:s H:q H:q H:q").unwrap();
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = recording();
        // 70 ms after the first: closer to the second, 55 ms early on it.
        let delta = tracker.record_instrument_hit(0.07, HiHat, &timing, &score).unwrap();
        assert!((delta + 55.0).abs() < 1e-6);
        // 140 ms after the quarter on 3: within the catch window.
        let mut tracker = recording();
        assert!(tracker.record_instrument_hit(1.0 + 0.14, HiHat, &timing, &score).is_some());
    }

    #[test]
    fn hits_without_an_instrument_are_ignored_on_a_kit() {
        let (score, timing) = groove();
        let mut tracker = recording();
        assert_eq!(tracker.record_hit(0.0, &timing, &score), None);
        tracker.record_instrument_hit(0.25, HiHat, &timing, &score);
        tracker.update_progress(0.7, &timing, &score);
        let live = tracker.live_summary().unwrap();
        assert_eq!((live.hits, live.extra), (1, 0));
    }

    #[test]
    fn the_hi_hat_foot_is_judged_where_written() {
        let score = parse_score("4/4 H:q H:q H:q H:q ; K:q HF:q K:q HF:q").unwrap();
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = recording();
        assert!(tracker.record_instrument_hit(0.5, HiHatFoot, &timing, &score).is_some());
        tracker.on_playback_stop();
        assert_eq!(grade(&tracker, tick_at(&timing, 0.5), HiHatFoot), Some(HitGrade::Perfect));
    }

    #[test]
    fn the_windows_narrow_by_the_instruments_own_neighbours() {
        // Kick quarters under hi-hat sixteenths at 160 BPM: the sixteenths
        // are 93.75 ms apart, which narrows their windows (perfect 18.75
        // ms); the kicks are 375 ms apart and keep 25 ms.
        let score = parse_score(
            "4/4 H-K:s H:s H:s H:s H-K:s H:s H:s H:s H-K:s H:s H:s H:s H-K:s H:s H:s H:s",
        )
        .unwrap();
        let timing = ScoreTiming::from_score(&score, 160);
        let mut tracker = recording();
        tracker.record_instrument_hit(0.020, Kick, &timing, &score);
        tracker.record_instrument_hit(0.020, HiHat, &timing, &score);
        tracker.on_playback_stop();
        assert_eq!(grade(&tracker, 0, Kick), Some(HitGrade::Perfect));
        assert_eq!(grade(&tracker, 0, HiHat), Some(HitGrade::Good));
    }

    #[test]
    fn heads_are_judged_alike_in_an_odd_meter() {
        // 7/8 (2+2+3) at 120 BPM: an eighth is 250 ms; kicks on 1, the
        // second group and the third, hi-hat eighths throughout.
        let score =
            parse_score("7/8 H-K:e H:e H-K:e H:e H-K:e H:e H:e").expect("a kit groove in 7/8");
        let timing = ScoreTiming::from_score(&score, 120);
        let eighth = 0.25;
        let mut tracker = recording();
        // The kick of the second group 100 ms late: caught, off. A kick on
        // the second eighth, 250 ms from both kicks around it: extra.
        assert!(tracker.record_instrument_hit(2.0 * eighth + 0.1, Kick, &timing, &score).is_some());
        assert_eq!(tracker.record_instrument_hit(eighth, Kick, &timing, &score), None);
        tracker.record_instrument_hit(eighth, HiHat, &timing, &score);
        tracker.on_playback_stop();
        assert_eq!(grade(&tracker, tick_at(&timing, 2.0 * eighth), Kick), Some(HitGrade::Off));
        assert_eq!(grade(&tracker, tick_at(&timing, eighth), HiHat), Some(HitGrade::Perfect));
        let live = tracker.last_summary().unwrap();
        assert_eq!((live.hits, live.extra), (2, 1));
    }

    /// Kicks on beat 2 (0.5 s) and on the bar's last sixteenth (1.875 s)
    /// under hi-hat eighths at 120 BPM: the loop is 2 s, and the last kick's
    /// window runs on past the loop's end, to its catch window's end
    /// (2.025 s). It is final 500 ms after the next loop's first kick, at
    /// 3 s (halfway to the one after would be 3.1875 s).
    fn kicks_across_the_loop_end() -> (Score, ScoreTiming) {
        let score = parse_score("4/4 H:e H:e H-K:e H:e H:e H:e H:e H:s K:s").unwrap();
        let timing = ScoreTiming::from_score(&score, 120);
        (score, timing)
    }

    /// Moves the progress cursor from `from` to `to` (s) in frames of 60 fps.
    fn frames(
        tracker: &mut AccuracyTracker,
        from: f64,
        to: f64,
        score: &Score,
        timing: &ScoreTiming,
    ) {
        let mut t = from;
        while t < to {
            t = (t + 1.0 / 60.0).min(to);
            tracker.update_progress(t, timing, score);
        }
    }

    #[test]
    fn a_window_past_the_loop_end_closes_frames_after_the_wrap() {
        let (score, timing) = kicks_across_the_loop_end();
        let last_kick = tick_at(&timing, 1.875);
        let mut tracker = recording();
        tracker.record_instrument_hit(0.0, HiHat, &timing, &score);
        // The first loop: no kick of a loop before it to judge.
        frames(&mut tracker, 0.0, 0.3, &score, &timing);
        assert!(tracker.mark_for_head(last_kick, Kick).is_none());
        // The last kick goes unplayed: missed once final, in the next loop.
        frames(&mut tracker, 0.3, 2.95, &score, &timing);
        assert!(tracker.mark_for_head(last_kick, Kick).is_none());
        frames(&mut tracker, 2.95, 3.01, &score, &timing);
        assert!(matches!(tracker.mark_for_head(last_kick, Kick), Some(AccuracyMark::Miss)));
        let misses = tracker.live_summary().unwrap().misses;
        // Judged once (the next note final is a hi-hat at 3.125 s).
        frames(&mut tracker, 3.01, 3.12, &score, &timing);
        assert_eq!(tracker.live_summary().unwrap().misses, misses);
    }

    #[test]
    fn a_late_kick_across_the_loop_end_is_no_miss_frame_by_frame() {
        let (score, timing) = kicks_across_the_loop_end();
        let last_kick = tick_at(&timing, 1.875);
        // The same run without the kick: one miss more.
        let mut unplayed = recording();
        unplayed.record_instrument_hit(0.0, HiHat, &timing, &score);
        frames(&mut unplayed, 0.0, 3.1, &score, &timing);
        let all_missed = unplayed.live_summary().unwrap().misses;
        assert!(matches!(unplayed.mark_for_head(last_kick, Kick), Some(AccuracyMark::Miss)));
        // The hit comes in before or after the cursor wrapped, before the
        // window closed (the miss delay keeps the cursor behind the hits).
        for cursor_at in [1.95, 2.02] {
            let mut tracker = recording();
            tracker.record_instrument_hit(0.0, HiHat, &timing, &score);
            frames(&mut tracker, 0.0, cursor_at, &score, &timing);
            tracker.record_instrument_hit(2.015, Kick, &timing, &score).expect("caught");
            frames(&mut tracker, cursor_at, 3.1, &score, &timing);
            assert_eq!(grade(&tracker, last_kick, Kick), Some(HitGrade::Off), "{cursor_at}");
            let live = tracker.live_summary().unwrap();
            assert_eq!((live.hits, live.extra), (2, 0), "{cursor_at}");
            assert_eq!(live.misses + 1, all_missed, "{cursor_at}");
        }
    }

    #[test]
    fn a_late_kick_across_the_loop_end_counts_for_the_loop_before() {
        let (score, timing) = kicks_across_the_loop_end();
        let last_kick = tick_at(&timing, 1.875);
        let mut tracker = recording();
        tracker.update_progress(1.95, &timing, &score);
        // 140 ms late, in the next loop: within 150 ms and short of halfway
        // to the next kick.
        let delta = tracker.record_instrument_hit(2.015, Kick, &timing, &score).expect("caught");
        assert!((delta - 140.0).abs() < 1e-6, "{delta}");
        tracker.update_progress(2.3, &timing, &score);
        tracker.update_progress(3.1, &timing, &score);
        assert_eq!(grade(&tracker, last_kick, Kick), Some(HitGrade::Off));
        let live = tracker.live_summary().unwrap();
        // The kick was hit, not missed as well; nothing extra.
        assert_eq!(live.extra, 0);
        assert!(!matches!(tracker.mark_for_head(last_kick, Kick), Some(AccuracyMark::Miss)));
        // Its next loop's kick is missed when not played.
        tracker.update_progress(3.95, &timing, &score);
        tracker.update_progress(4.3, &timing, &score);
        tracker.update_progress(5.1, &timing, &score);
        assert!(matches!(tracker.mark_for_head(last_kick, Kick), Some(AccuracyMark::Miss)));
    }

    #[test]
    fn an_early_kick_before_the_loop_end_counts_for_the_next_loop() {
        let (score, timing) = kicks_across_the_loop_end();
        let first_kick = tick_at(&timing, 0.5);
        let mut tracker = recording();
        tracker.update_progress(1.8, &timing, &score);
        tracker.record_instrument_hit(1.875, Kick, &timing, &score);
        tracker.update_progress(2.3, &timing, &score);
        // 140 ms early on the next loop's first kick (2.5 s).
        let delta = tracker.record_instrument_hit(2.36, Kick, &timing, &score).expect("caught");
        assert!((delta + 140.0).abs() < 1e-6, "{delta}");
        tracker.update_progress(3.0, &timing, &score);
        tracker.update_progress(4.3, &timing, &score);
        assert_eq!(grade(&tracker, first_kick, Kick), Some(HitGrade::Off));
        let live = tracker.live_summary().unwrap();
        assert_eq!(live.extra, 0);
        // Only the hi-hats went unplayed.
        assert!(live.misses > 0);
        assert!(!matches!(tracker.mark_for_head(first_kick, Kick), Some(AccuracyMark::Miss)));
    }

    #[test]
    fn swung_heads_are_caught_where_they_sound() {
        use grooph_measure::swing::{Swing, SwingUnit};
        // Hi-hat eighths, kicks on 1 and on the "&" of 1, swung 75 %: the
        // "&" sounds a sixteenth late, at 375 ms instead of 250 ms.
        let score = parse_score("4/4 H-K:e H-K:e H:e H:e H:e H:e H:e H:e").unwrap();
        let swing = Swing { unit: SwingUnit::Eighths, percent: 75 };
        let timing = ScoreTiming::from_score(&score, 120).with_swing(swing);
        let and = tick_at(&ScoreTiming::from_score(&score, 120), 0.25);
        let performed = timing.performed_global_tick(0, and as u32);
        assert_ne!(performed, and);
        let sounds = timing.global_tick_to_seconds(performed as f64);
        let mut tracker = recording();
        // Dead on where it sounds: perfect, marked on the written note.
        let delta = tracker.record_instrument_hit(sounds, Kick, &timing, &score).unwrap();
        assert!(delta.abs() < 1e-6, "{delta}");
        tracker.on_playback_stop();
        assert_eq!(grade(&tracker, and, Kick), Some(HitGrade::Perfect));
        // 100 ms late on the swung kick: caught, off; 200 ms: extra.
        let mut tracker = recording();
        assert!(tracker.record_instrument_hit(sounds + 0.1, Kick, &timing, &score).is_some());
        tracker.on_playback_stop();
        assert_eq!(grade(&tracker, and, Kick), Some(HitGrade::Off));
        let mut tracker = recording();
        tracker.record_instrument_hit(0.0, HiHat, &timing, &score);
        assert_eq!(tracker.record_instrument_hit(sounds + 0.2, Kick, &timing, &score), None);
        tracker.on_playback_stop();
        assert_eq!(tracker.last_summary().unwrap().extra, 1);
    }

    /// The tick distance of `seconds` at the groove's tempo.
    fn ticks_of(timing: &ScoreTiming, seconds: f64) -> f64 {
        timing.seconds_to_global_tick(seconds)
    }

    #[test]
    fn an_extra_kick_is_marked_where_it_was_played() {
        let (score, timing) = groove();
        let mut tracker = recording();
        // On beat 2, where only snare and hi-hat play.
        tracker.record_instrument_hit(0.5, Kick, &timing, &score);
        // 200 ms late on the kick of 1: past its catch window, 50 ms before
        // the hi-hat on the "&".
        tracker.record_instrument_hit(0.2, Kick, &timing, &score);
        let extras = tracker.extra_marks();
        assert_eq!(extras.len(), 2);
        assert_eq!((extras[0].instrument, extras[0].anchor_tick), (Kick, tick_at(&timing, 0.5)));
        assert!(extras[0].diff_ticks.abs() < 1e-6);
        assert_eq!(extras[0].measure_idx, 0);
        assert_eq!(extras[1].anchor_tick, tick_at(&timing, 0.25));
        assert!((extras[1].diff_ticks + ticks_of(&timing, 0.05)).abs() < 1e-6);
        // No mark on a note for them.
        assert!(tracker.mark_for_head(tick_at(&timing, 0.5), Snare).is_none());
    }

    #[test]
    fn a_second_hit_on_a_head_is_marked_as_extra() {
        let (score, timing) = groove();
        let mut tracker = recording();
        tracker.record_instrument_hit(0.0, Kick, &timing, &score);
        tracker.record_instrument_hit(0.03, Kick, &timing, &score);
        // Extra once the kick is final.
        assert!(tracker.extra_marks().is_empty());
        tracker.update_progress(1.6, &timing, &score);
        let extras = tracker.extra_marks();
        assert_eq!(extras.len(), 1);
        assert_eq!((extras[0].instrument, extras[0].anchor_tick), (Kick, 0));
        assert!((extras[0].diff_ticks - ticks_of(&timing, 0.03)).abs() < 1e-6);
        // The first hit keeps its mark.
        assert_eq!(grade(&tracker, 0, Kick), Some(HitGrade::Perfect));
    }

    #[test]
    fn an_extra_mark_stays_until_its_place_comes_round_again() {
        let (score, timing) = groove();
        let mut tracker = recording();
        tracker.record_instrument_hit(0.5, Kick, &timing, &score);
        frames(&mut tracker, 0.0, 2.4, &score, &timing);
        // Still there in the next loop, before its place.
        assert_eq!(tracker.extra_marks().len(), 1);
        frames(&mut tracker, 2.4, 2.6, &score, &timing);
        assert!(tracker.extra_marks().is_empty());
    }

    #[test]
    fn extra_marks_stay_after_stop_until_the_next_run_or_an_edit() {
        let (score, timing) = groove();
        let mut tracker = recording();
        tracker.record_instrument_hit(0.0, HiHat, &timing, &score);
        tracker.record_instrument_hit(0.5, Kick, &timing, &score);
        tracker.on_playback_stop();
        assert_eq!(tracker.extra_marks().len(), 1);
        tracker.clear_for_edit();
        assert!(tracker.extra_marks().is_empty());

        let mut tracker = recording();
        tracker.record_instrument_hit(0.0, HiHat, &timing, &score);
        tracker.record_instrument_hit(0.5, Kick, &timing, &score);
        tracker.on_playback_stop();
        tracker.on_playback_start(Some(10.0));
        assert!(tracker.extra_marks().is_empty());
    }

    #[test]
    fn a_second_hit_on_one_line_is_marked_as_extra_too() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = recording();
        tracker.record_hit(0.0, &timing, &score);
        tracker.record_hit(0.05, &timing, &score);
        // Extra once the second note is final, which it might have been for.
        tracker.update_progress(1.3, &timing, &score);
        assert_eq!(tracker.live_summary().unwrap().extra, 1);
        let extras = tracker.extra_marks();
        assert_eq!(extras.len(), 1);
        assert_eq!((extras[0].instrument, extras[0].anchor_tick), (Instrument::Rhythm, 0));
        assert!((extras[0].diff_ticks - ticks_of(&timing, 0.05)).abs() < 1e-6);
        // Judged as before: the first hit keeps its mark.
        assert!(matches!(tracker.mark_for_onset(0), Some(AccuracyMark::Hit { .. })));
    }

    #[test]
    fn a_kit_note_is_missed_without_a_hit_in_its_catch_window() {
        // A crash once a bar at 120 BPM: the next one is 2 s away, but it
        // can only be hit within 150 ms.
        let score = parse_score("4/4 H-CR:q H:q H:q H:q").unwrap();
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = recording();
        tracker.record_instrument_hit(0.0, HiHat, &timing, &score);
        // A crash 200 ms late is extra, at once.
        assert_eq!(tracker.record_instrument_hit(0.2, Instrument::Crash, &timing, &score), None);
        assert_eq!(tracker.extra_marks().len(), 1);
        // The crash is final 500 ms after the next one, and missed.
        frames(&mut tracker, 0.0, 2.45, &score, &timing);
        assert!(tracker.mark_for_head(0, Instrument::Crash).is_none());
        frames(&mut tracker, 2.45, 2.55, &score, &timing);
        assert!(matches!(tracker.mark_for_head(0, Instrument::Crash), Some(AccuracyMark::Miss)));
        let live = tracker.live_summary().unwrap();
        assert_eq!((live.hits, live.extra), (1, 1));
    }

    #[test]
    fn statistics_count_heads() {
        // One chord a bar; only its hi-hat is played, two loops.
        let score = parse_score("4/4 H-K:q r:q r:q r:q").unwrap();
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = recording();
        tracker.record_instrument_hit(0.010, HiHat, &timing, &score);
        tracker.update_progress(1.9, &timing, &score);
        tracker.record_instrument_hit(2.010, HiHat, &timing, &score);
        tracker.update_progress(3.8, &timing, &score);
        // The second loop's chord is final 500 ms into the third.
        tracker.update_progress(4.6, &timing, &score);
        let live = tracker.live_summary().unwrap();
        assert_eq!((live.hits, live.misses), (2, 2));
        let [voice, _] = tracker.live_voice_summaries();
        let voice = voice.expect("the first voice played");
        assert_eq!((voice.hits, voice.misses), (2, 2));
        tracker.on_playback_stop();
        let drift = tracker.last_drift().expect("two hits");
        // At the stop, the third loop's chord is missed as well: its
        // windows had closed.
        assert_eq!((drift.hits.len(), drift.misses.len()), (2, 4));
    }
}

/// Hits are paired with notes by an alignment over a short stretch, which
/// becomes final after a lookahead (see `align`).
#[cfg(test)]
mod alignment_tests {
    use super::*;
    use grooph_measure::notation::parse_score;

    /// Eighths at 120 BPM: a gap of 250 ms, a loop of 2 s.
    const GAP: f64 = 0.25;

    fn eighths() -> (Score, ScoreTiming) {
        let score = parse_score("4/4 e e e e e e e e").unwrap();
        let timing = ScoreTiming::from_score(&score, 120);
        (score, timing)
    }

    /// Plays `hits` (s) as they come in, frame by frame from 0 to `to`,
    /// judging 60 ms behind the clock as with a hit source's offset.
    fn play(score: &Score, timing: &ScoreTiming, hits: &[f64], to: f64) -> AccuracyTracker {
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        let mut hits = hits.iter().copied().peekable();
        let mut now = 0.0;
        while now < to {
            now = (now + 1.0 / 60.0).min(to);
            while let Some(hit) = hits.next_if(|&h| h <= now) {
                tracker.record_hit(hit, timing, score);
            }
            tracker.update_progress_behind(now, 0.06, timing, score);
        }
        tracker
    }

    /// The deviation of the mark on the note at `index` eighths, in gaps;
    /// `None` for a miss, and a panic for no mark.
    fn deviation(tracker: &AccuracyTracker, timing: &ScoreTiming, index: u64) -> Option<f64> {
        let gap_ticks = timing.total_loop_ticks() as f64 / 8.0;
        match tracker.mark_for_onset(index * gap_ticks as u64) {
            Some(AccuracyMark::Hit { diff_ticks, .. }) => Some(diff_ticks / gap_ticks),
            Some(AccuracyMark::Miss) => None,
            None => panic!("no mark on note {index}"),
        }
    }

    /// A case of the issue's table: A is the third eighth, B the fourth, C
    /// the fifth; the first two and the sixth are played on time as well.
    /// `played` in gaps from the bar's start. Returns the deviations of A,
    /// B and C (in gaps, `None` for a miss) and hits, misses and extras of
    /// the first six notes.
    fn case(played: &[f64]) -> ([Option<f64>; 3], (u64, u64, u64)) {
        let (score, timing) = eighths();
        let mut hits = vec![0.0, 1.0, 5.0];
        hits.extend_from_slice(played);
        hits.sort_by(f64::total_cmp);
        let hits: Vec<f64> = hits.iter().map(|g| g * GAP).collect();
        // The sixth note is final at 1.625 s, the seventh not before 1.875 s.
        let tracker = play(&score, &timing, &hits, 1.85 + 0.06);
        let s = tracker.live_summary().unwrap();
        let notes = [2, 3, 4].map(|i| deviation(&tracker, &timing, i));
        (notes.map(|d| d.map(|d| (d * 100.0).round() / 100.0)), (s.hits, s.misses, s.extra))
    }

    #[test]
    fn a_late_hit_counts_for_its_note_when_the_next_gets_its_own() {
        // A 0.6 late, B on time: A +0.6, B ±0.
        assert_eq!(case(&[2.6, 3.0, 4.0]), ([Some(0.6), Some(0.0), Some(0.0)], (6, 0, 0)));
    }

    #[test]
    fn a_hit_late_by_more_than_three_quarters_of_the_gap_is_extra() {
        // A 0.8 late, B on time: extra, A missed, B ±0.
        assert_eq!(case(&[2.8, 3.0, 4.0]), ([None, Some(0.0), Some(0.0)], (5, 1, 1)));
    }

    #[test]
    fn a_late_hit_without_one_on_the_next_note_goes_to_the_nearer() {
        // A 0.6 late, B missing: A missed, B −0.4.
        assert_eq!(case(&[2.6, 4.0]), ([None, Some(-0.4), Some(0.0)], (5, 1, 0)));
    }

    #[test]
    fn two_late_hits_before_an_on_time_one() {
        // A and B each 0.6 late, C on time: A missed, B −0.4, extra, C ±0.
        assert_eq!(case(&[2.6, 3.6, 4.0]), ([None, Some(-0.4), Some(0.0)], (5, 1, 1)));
    }

    #[test]
    fn a_double_hit_after_a_missed_note() {
        // A missing, B hit at −0.1 and +0.05: extra, A missed, B +0.05.
        assert_eq!(case(&[2.9, 3.05, 4.0]), ([None, Some(0.05), Some(0.0)], (5, 1, 1)));
    }

    #[test]
    fn a_flam_before_an_on_time_hit() {
        // A played 0 and +0.3, B on time: A ±0, extra, B ±0.
        assert_eq!(case(&[2.0, 2.3, 3.0, 4.0]), ([Some(0.0), Some(0.0), Some(0.0)], (6, 0, 1)));
    }

    #[test]
    fn a_flam_before_a_missed_note() {
        // A played 0 and +0.3, B missing: A ±0, B −0.7.
        assert_eq!(case(&[2.0, 2.3, 4.0]), ([Some(0.0), Some(-0.7), Some(0.0)], (6, 0, 0)));
    }

    #[test]
    fn a_single_hit_between_two_unplayed_notes_goes_to_the_nearer() {
        let (score, timing) = eighths();
        // Between A and B, nearer to A.
        let hits = [0.0, 1.0, 2.4, 4.0, 5.0].map(|g| g * GAP);
        let tracker = play(&score, &timing, &hits, 1.96);
        assert_eq!(deviation(&tracker, &timing, 2).map(|d| (d * 10.0).round()), Some(4.0));
        assert_eq!(deviation(&tracker, &timing, 3), None);
    }

    #[test]
    fn nothing_shows_before_its_note_is_final() {
        let (score, timing) = eighths();
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        let delay = 0.06;
        let a = timing.total_loop_ticks() / 4;
        // A (0.5 s) 0.6 of a gap late: the calibration gets its deviation
        // from the nearest note at once, B's.
        tracker.update_progress_behind(0.65, delay, &timing, &score);
        let delta = tracker.record_hit(0.65, &timing, &score).unwrap();
        assert!((delta + 100.0).abs() < 1e-6, "{delta}");
        tracker.update_progress_behind(0.75, delay, &timing, &score);
        tracker.record_hit(0.75, &timing, &score);
        assert!(tracker.mark_for_onset(a).is_none());
        assert_eq!(tracker.live_summary().map_or(0, |s| s.hits), 0);
        // Final halfway between B and C (0.875 s), the delay behind.
        tracker.update_progress_behind(0.875 + delay - 0.005, delay, &timing, &score);
        assert!(tracker.mark_for_onset(a).is_none());
        tracker.update_progress_behind(0.875 + delay + 0.005, delay, &timing, &score);
        assert_eq!(deviation(&tracker, &timing, 2).map(|d| (d * 10.0).round()), Some(6.0));
        assert_eq!(tracker.live_summary().map(|s| (s.hits, s.extra)), Some((1, 0)));
    }

    #[test]
    fn at_a_slow_tempo_a_note_is_final_half_a_second_after_the_next() {
        // Quarters at 40 BPM, 1.5 s apart: halfway would be 0.75 s.
        let score = parse_score("4/4 q q q q").unwrap();
        let timing = ScoreTiming::from_score(&score, 40);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        let a = timing.total_loop_ticks() / 4;
        tracker.update_progress(1.6, &timing, &score);
        tracker.record_hit(1.6, &timing, &score);
        tracker.update_progress(3.0 + 0.49, &timing, &score);
        assert!(tracker.mark_for_onset(a).is_none());
        tracker.update_progress(3.0 + 0.51, &timing, &score);
        assert!(matches!(tracker.mark_for_onset(a), Some(AccuracyMark::Hit { .. })));
    }

    #[test]
    fn across_the_loop_end_as_inside_the_loop() {
        let (score, timing) = eighths();
        // The loop's last note 0.6 late, the next loop's first two on time.
        let hits = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.6, 8.0, 9.0].map(|g| g * GAP);
        // The next loop's first note is final at 2.375 s.
        let tracker = play(&score, &timing, &hits, 2.45);
        assert_eq!(deviation(&tracker, &timing, 7).map(|d| (d * 10.0).round()), Some(6.0));
        assert_eq!(deviation(&tracker, &timing, 0), Some(0.0));
        let s = tracker.live_summary().unwrap();
        assert_eq!((s.hits, s.misses, s.extra), (9, 0, 0));
    }

    #[test]
    fn a_stop_aligns_the_open_stretch() {
        let score = parse_score("4/4 q q q q").unwrap();
        let timing = ScoreTiming::from_score(&score, 120);
        // Notes 0.5 s apart; nothing final before 0.75 s.
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        tracker.record_hit(0.0, &timing, &score);
        tracker.record_hit(0.52, &timing, &score);
        tracker.update_progress(0.6, &timing, &score);
        assert!(tracker.live_summary().is_none());
        tracker.on_playback_stop();
        let s = tracker.last_summary().unwrap();
        // The third note's window was still open: no miss.
        assert_eq!((s.hits, s.misses, s.extra), (2, 0, 0));
        assert!(tracker.mark_for_onset(2 * timing.total_loop_ticks() / 4).is_none());

        // The second note unplayed, its window closed at 1 s: missed.
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        tracker.record_hit(0.0, &timing, &score);
        tracker.update_progress(1.1, &timing, &score);
        tracker.on_playback_stop();
        let s = tracker.last_summary().unwrap();
        assert_eq!((s.hits, s.misses, s.extra), (1, 1, 0));
    }

    #[test]
    fn the_tallies_count_the_final_pairs() {
        // A on 2, B on its "&": A 0.6 of a gap late, B on time.
        let score = parse_score("2/4 r:q e e").unwrap();
        let timing = ScoreTiming::from_score(&score, 120);
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        tracker.update_progress(0.6, &timing, &score);
        tracker.record_hit(0.5 + 0.15, &timing, &score);
        tracker.record_hit(0.75, &timing, &score);
        tracker.update_progress(0.9, &timing, &score);
        tracker.on_playback_stop();
        let [Some(voice), None] = tracker.last_voice_summaries() else {
            panic!("the first voice played");
        };
        assert_eq!((voice.hits, voice.misses, voice.extra), (2, 0, 0));
        let drift = tracker.last_drift().expect("two hits");
        let deltas: Vec<f64> = drift.hits.iter().map(|h| h.delta_ms.round()).collect();
        assert_eq!(deltas, [150.0, 0.0]);
    }
}

/// The alignment on a kit runs per instrument.
#[cfg(test)]
mod kit_alignment_tests {
    use super::*;
    use Instrument::{HiHat, Kick};
    use grooph_measure::notation::parse_score;

    #[test]
    fn a_late_hi_hat_counts_for_its_note_and_a_kick_takes_no_part() {
        // Hi-hat eighths at 160 BPM, 187.5 ms apart; kicks on 1 and 3.
        let score = parse_score("4/4 H-K:e H:e H:e H:e H-K:e H:e H:e H:e").unwrap();
        let timing = ScoreTiming::from_score(&score, 160);
        let gap = 0.1875;
        let mut tracker = AccuracyTracker::new();
        tracker.on_playback_start_at(0.0, 0.0);
        let mut hits: Vec<(f64, Instrument)> =
            [0.0, 1.0, 2.6, 3.0, 4.0, 5.0].iter().map(|g| (g * gap, HiHat)).collect();
        // The kicks on time, and one more with A's late hit.
        hits.extend([(0.0, Kick), (2.6 * gap, Kick), (4.0 * gap, Kick)]);
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut hits = hits.into_iter().peekable();
        let mut now = 0.0;
        while now < 1.2 {
            now += 1.0 / 60.0;
            while let Some((t, i)) = hits.next_if(|&(t, _)| t <= now) {
                tracker.record_instrument_hit(t, i, &timing, &score);
            }
            tracker.update_progress(now, &timing, &score);
        }
        let eighth = timing.total_loop_ticks() / 8;
        let deviation = |index: u64| match tracker.mark_for_head(index * eighth, HiHat) {
            Some(AccuracyMark::Hit { diff_ticks, .. }) => {
                (diff_ticks / eighth as f64 * 10.0).round()
            }
            _ => panic!("hi-hat {index} not hit"),
        };
        assert_eq!((deviation(2), deviation(3)), (6.0, 0.0));
        let s = tracker.live_summary().unwrap();
        // Hi-hats 1 to 5 and the first kick; the kick on A's hit is extra.
        assert_eq!((s.hits, s.misses, s.extra), (6, 0, 1));
    }
}
