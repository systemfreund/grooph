use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use grooph_measure::MeasureIdx;
use grooph_measure::Score;
use grooph_measure::counting::{CountPos, position_in_beat};
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::tempo::ScoreTiming;

use super::drift::DriftLog;
use super::positions::PositionLog;

#[derive(Clone, Copy)]
pub(crate) enum AccuracyMark {
    /// Deviation in ticks (negative = early) and how well it landed.
    Hit {
        diff_ticks: f64,
        grade: HitGrade,
    },
    Miss,
}

/// Which timing window a hit landed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HitGrade {
    Perfect,
    Good,
    Off,
}

#[derive(Clone, Copy, Default)]
pub(super) struct AccuracyStats {
    pub count: u64,
    pub sum_ms: f64,
    pub sum_abs_ms: f64,
    pub sum_sq_ms: f64,
    pub last_delta_ms: Option<f64>,
}

impl AccuracyStats {
    fn reset(&mut self) {
        *self = Self::default();
    }

    pub(super) fn mean_ms(&self) -> Option<f64> {
        (self.count > 0).then(|| self.sum_ms / self.count as f64)
    }

    /// Standard deviation of the deltas around their mean.
    pub(super) fn spread_ms(&self) -> Option<f64> {
        let mean = self.mean_ms()?;
        let n = self.count as f64;
        Some((self.sum_sq_ms / n - mean * mean).max(0.0).sqrt())
    }

    /// Add the hits of `other`.
    pub(super) fn merge(&mut self, other: &Self) {
        self.count = self.count.saturating_add(other.count);
        self.sum_ms += other.sum_ms;
        self.sum_abs_ms += other.sum_abs_ms;
        self.sum_sq_ms += other.sum_sq_ms;
        self.last_delta_ms = other.last_delta_ms.or(self.last_delta_ms);
    }

    pub(super) fn push(&mut self, delta_ms: f64) {
        self.count = self.count.saturating_add(1);
        self.sum_ms += delta_ms;
        self.sum_abs_ms += delta_ms.abs();
        self.sum_sq_ms += delta_ms * delta_ms;
        self.last_delta_ms = Some(delta_ms);
    }
}

/// Hits within this distance of the note count as "perfect".
pub(crate) const PERFECT_WINDOW_MS: f64 = 25.0;
/// Hits within this distance (but outside the perfect window) count as "good".
pub(crate) const GOOD_WINDOW_MS: f64 = 60.0;
/// Fast notes narrow the windows to these shares of the gap to the closest
/// neighbouring onset, so "good" stays well inside the note's own hit window.
const PERFECT_GAP_SHARE: f64 = 0.2;
const GOOD_GAP_SHARE: f64 = 0.4;

/// Perfect/good windows of one note.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct HitWindows {
    pub perfect_ms: f64,
    pub good_ms: f64,
}

impl HitWindows {
    /// Windows for a note whose closest neighbouring onset is `gap_ms` away.
    pub(super) fn for_gap_ms(gap_ms: f64) -> Self {
        Self {
            perfect_ms: PERFECT_WINDOW_MS.min(gap_ms * PERFECT_GAP_SHARE),
            good_ms: GOOD_WINDOW_MS.min(gap_ms * GOOD_GAP_SHARE),
        }
    }

    pub(super) fn grade(&self, delta_ms: f64) -> HitGrade {
        let d = delta_ms.abs();
        if d <= self.perfect_ms {
            HitGrade::Perfect
        } else if d <= self.good_ms {
            HitGrade::Good
        } else {
            HitGrade::Off
        }
    }
}

/// A hit's deviation and the windows of its note, kept to grade it again
/// against the run's own average (see [`RunSummary::tight_percent`]).
#[derive(Clone, Copy)]
pub(super) struct GradedHit {
    pub delta_ms: f64,
    pub windows: HitWindows,
}

/// Perfect and good hits among `hits` when graded around `mean_ms` instead
/// of the click.
fn tight_counts(hits: &[GradedHit], mean_ms: f64) -> (u64, u64) {
    hits.iter().fold((0, 0), |(perfect, good), h| match h.windows.grade(h.delta_ms - mean_ms) {
        HitGrade::Perfect => (perfect + 1, good),
        HitGrade::Good => (perfect, good + 1),
        HitGrade::Off => (perfect, good),
    })
}

/// The hits of a run kept for its tight counts, and those counts. The
/// average moves with every hit and can turn an earlier hit's grade, so
/// each new hit grades all of them again: once per hit, not per frame.
/// That is O(n) per hit and keeps every hit of the run, a few microseconds
/// and some kilobytes even for an hour of endless mode. Should that ever
/// matter, sorted deltas per window size give the counts in O(log n).
#[derive(Clone, Default)]
pub(super) struct TightTally {
    hits: Vec<GradedHit>,
    counts: (u64, u64),
}

impl TightTally {
    /// Add `hit` and grade all hits again around `mean_ms`, the average
    /// including it.
    pub(super) fn push(&mut self, hit: GradedHit, mean_ms: f64) {
        self.hits.push(hit);
        self.counts = tight_counts(&self.hits, mean_ms);
    }
}

/// Hit/miss tallies of one recording session, beyond the raw delta stats.
#[derive(Clone, Copy, Default)]
pub(super) struct RunCounters {
    pub misses: u64,
    /// Hits on a note that was already hit in this loop.
    pub extra: u64,
    pub perfect: u64,
    pub good: u64,
}

impl RunCounters {
    pub(super) fn push_hit(&mut self, grade: HitGrade) {
        match grade {
            HitGrade::Perfect => self.perfect += 1,
            HitGrade::Good => self.good += 1,
            HitGrade::Off => {}
        }
    }
}

/// Evaluation of one play-along run, shown after playback stops.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RunSummary {
    pub hits: u64,
    pub misses: u64,
    pub extra: u64,
    pub perfect: u64,
    pub good: u64,
    /// Average signed deviation; negative = early.
    pub mean_ms: f64,
    /// Standard deviation of the deviations.
    pub spread_ms: f64,
    /// Perfect and good hits when graded around `mean_ms` instead of the
    /// click: how tight the playing is, whatever its offset.
    pub tight: (u64, u64),
}

impl RunSummary {
    /// Hits outside the good window.
    pub(crate) fn off(&self) -> u64 {
        self.hits.saturating_sub(self.perfect + self.good)
    }

    /// 0–100: perfect hits count fully, good ones half; misses and extra
    /// hits count against the score.
    pub(crate) fn score_percent(&self) -> f64 {
        self.percent_of(self.perfect, self.good)
    }

    /// [`Self::score_percent`] with every hit graded around the run's
    /// average deviation: playing steadily early or late scores as high as
    /// on the click.
    pub(crate) fn tight_percent(&self) -> f64 {
        self.percent_of(self.tight.0, self.tight.1)
    }

    fn percent_of(&self, perfect: u64, good: u64) -> f64 {
        let total = self.hits + self.misses + self.extra;
        if total == 0 {
            return 0.0;
        }
        (perfect as f64 + 0.5 * good as f64) / total as f64 * 100.0
    }
}

/// A single beat's onset position in score-global ticks, along with whether it
/// is a Note (matchable for accuracy) or a Rest (only used as a structural
/// anchor when computing per-note hit windows). The list is monotonically
/// increasing in `onset_tick`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GlobalBeatOnset {
    /// Written onset; keys the accuracy marks.
    pub onset_tick: u64,
    /// Onset as heard, i.e. with swing applied. Hit windows use this.
    pub performed_tick: u64,
    pub is_note: bool,
    pub measure_idx: MeasureIdx,
    /// The voices with a note here, one bit each from the first (see
    /// [`TRACKED_VOICES`]).
    pub voices: u8,
    /// Where in its beat the onset lies, as written; `None` off every count.
    pub position: Option<CountPos>,
}

/// Voices tallied on their own as well: the metronome's click bar and its
/// counter rhythm.
pub(crate) const TRACKED_VOICES: usize = 2;

/// Flatten the onsets of every measure into a vector of score-global onset
/// ticks (Notes + Rests): each tick where a beat of any voice starts, once,
/// a note if any voice plays one there. The score iterates measures in
/// order; within each measure onsets are in order.
pub(crate) fn compute_global_beat_onsets(
    score: &Score,
    timing: &ScoreTiming,
) -> Vec<GlobalBeatOnset> {
    let mut out = Vec::new();
    for (m_idx, measure) in score.measures().iter().enumerate() {
        let start = timing.measure_start_tick(m_idx);
        let ts = measure.time_signature();
        // Note ticks of each tracked voice.
        let note_ticks: Vec<Vec<u32>> = measure
            .voices()
            .iter()
            .take(TRACKED_VOICES)
            .map(|v| {
                let ticks = DEFAULT_GRID.compute_onset_ticks(v.beats());
                v.beats().iter().zip(ticks).filter(|(b, _)| b.is_note()).map(|(_, t)| t).collect()
            })
            .collect();
        for onset in measure.onsets() {
            let voices = note_ticks
                .iter()
                .enumerate()
                .filter(|(_, ticks)| ticks.binary_search(&onset.tick).is_ok())
                .fold(0, |bits, (v, _)| bits | 1 << v);
            out.push(GlobalBeatOnset {
                onset_tick: start + onset.tick as u64,
                performed_tick: timing.performed_global_tick(m_idx, onset.tick),
                is_note: onset.is_note,
                measure_idx: m_idx,
                voices,
                position: position_in_beat(&ts, onset.tick),
            });
        }
    }
    out
}

/// [`compute_global_beat_onsets`] of the last few scores asked for. The
/// progress cursor and the staff's marks ask every frame, the recorder on
/// every hit, mostly for the one score being played, which only changes on
/// an edit, a tempo change or an endless rotation. Telling a score from the
/// kept ones compares its measures, which is cheaper than finding its
/// onsets again and allocates nothing.
pub(crate) fn global_beat_onsets(score: &Score, timing: &ScoreTiming) -> Rc<[GlobalBeatOnset]> {
    /// The scores kept: the metronome's, Practice's and the score tab's,
    /// and one more for a document that was just swapped.
    const KEPT: usize = 4;
    type Kept = VecDeque<(Score, ScoreTiming, Rc<[GlobalBeatOnset]>)>;
    thread_local! {
        static KEPT_ONSETS: RefCell<Kept> = const { RefCell::new(VecDeque::new()) };
    }
    KEPT_ONSETS.with_borrow_mut(|kept| {
        if let Some((_, _, onsets)) =
            kept.iter().find(|(s, t, _)| t == timing && s.same_beats(score))
        {
            return Rc::clone(onsets);
        }
        let onsets: Rc<[GlobalBeatOnset]> = compute_global_beat_onsets(score, timing).into();
        kept.push_front((score.clone(), timing.clone(), Rc::clone(&onsets)));
        kept.truncate(KEPT);
        onsets
    })
}

/// How many loops ahead (+1) or back (-1) a point `diff` ticks from
/// another lies, taking the closer way round a loop of `total` ticks.
pub(super) fn loop_offset(diff: f64, total: f64) -> i32 {
    if diff > total * 0.5 {
        1
    } else if diff < -total * 0.5 {
        -1
    } else {
        0
    }
}

/// Distance in performed ticks from the beat at `idx` to the closer of its
/// neighbouring onsets (notes or rests) on the score loop.
pub(super) fn neighbour_gap_ticks(beats: &[GlobalBeatOnset], idx: usize, total: f64) -> f64 {
    if beats.len() < 2 || idx >= beats.len() {
        return total;
    }
    let cur = beats[idx].performed_tick as f64;
    let prev = beats[(idx + beats.len() - 1) % beats.len()].performed_tick as f64;
    let next = beats[(idx + 1) % beats.len()].performed_tick as f64;
    let before = (cur - prev).rem_euclid(total);
    let after = (next - cur).rem_euclid(total);
    before.min(after)
}

/// State of a hit-recording session.
///
/// The tracker is either `Idle` (no playback) or `Recording`, in which case it
/// owns all data accumulated since the session started. Replacing the previous
/// flat fields-with-`Option`-everywhere model removes "is this field valid?"
/// questions: if you have a `&RecordingData`, the session is live.
#[derive(Default)]
pub(super) enum RecordingSession {
    #[default]
    Idle,
    Recording(Box<RecordingData>),
}

pub(super) struct RecordingData {
    pub(super) start_time: f64,
    /// Last observed global tick within the score loop. `None` means
    /// `update_progress` hasn't run yet for this session (or was reset).
    pub(super) last_tick: Option<f64>,
    /// `last_tick` was set by anchoring the clock (start, realign), ahead of
    /// the delayed time misses are judged at, which has not caught up yet.
    pub(super) anchored_ahead: bool,
    /// How far behind the clock the last progress judged misses (s).
    pub(super) miss_delay_s: f64,
    /// Accuracy marks keyed by *global* onset tick across the score loop.
    pub(super) marks_by_onset: HashMap<u64, AccuracyMark>,
    /// Note onsets (global) that already received a hit in the current
    /// score-loop iteration. Prevents double-counting.
    pub(super) hits_in_loop: HashSet<u64>,
    /// Hits whose `raw_diff` crossed the score-loop midpoint and therefore
    /// belong to the *next* score-loop iteration. Promoted to `hits_in_loop`
    /// when the score wraps.
    pub(super) hits_next_loop: HashSet<u64>,
    pub(super) stats: AccuracyStats,
    pub(super) counters: RunCounters,
    pub(super) tight: TightTally,
    /// The same tallies per tracked voice. A note several voices play at
    /// once counts for each of them.
    pub(super) voice_tallies: [VoiceTally; TRACKED_VOICES],
    /// The same tallies for the bars with the click and the silent ones.
    pub(super) bars: BarTimeline,
    /// The run over time, for its chart.
    pub(super) drift: DriftLog,
    /// The run by position in the beat.
    pub(super) positions: PositionLog,
}

/// A judged note, kept until the state of its bar is known.
#[derive(Clone, Copy)]
pub(super) enum BarEvent {
    Hit(GradedHit, HitGrade),
    Miss,
    Extra,
}

/// The notes of the bars with the click and of the silent bars, once the
/// gap click is on. A frame learns a bar's state from the audio engine only
/// well inside the bar, where the two clocks cannot disagree on which bar
/// it is; the notes judged until then wait.
#[derive(Default)]
pub(super) struct BarTimeline {
    /// The bars seen, in seconds on the session's clock: start, end and
    /// whether they were silent. A bar no frame saw is not in here.
    seen: Vec<(f64, f64, bool)>,
    /// Judged notes, by their time, whose bar is not known yet.
    pending: Vec<(f64, BarEvent)>,
    /// The end of the last bar seen.
    known_until: f64,
    /// Bars with the click (0) and silent bars (1).
    pub(super) tallies: [VoiceTally; 2],
}

/// Notes at a bar's start come from ticks and offsets, a hair off it.
const BAR_EDGE_S: f64 = 1e-3;

impl BarTimeline {
    pub(super) fn push(&mut self, note_s: f64, event: BarEvent) {
        self.pending.push((note_s, event));
    }

    /// The bar from `start_s` to `end_s` is silent or not; `None` while the
    /// gap click is off. Tallies the waiting notes up to the bar's end.
    pub(super) fn observe(&mut self, bar: Option<(f64, f64, bool)>) {
        let Some((start_s, end_s, muted)) = bar else {
            self.pending.clear();
            return;
        };
        if self.seen.last().is_none_or(|&(last, _, _)| start_s - last > BAR_EDGE_S) {
            self.seen.push((start_s, end_s, muted));
        }
        self.known_until = end_s;
        self.settle();
    }

    /// Tally the waiting notes in the bars seen so far. Also at the end of
    /// a run, whose last bar is not seen again.
    pub(super) fn settle(&mut self) {
        let mut pending = std::mem::take(&mut self.pending);
        pending.retain(|&(note_s, event)| {
            if note_s + BAR_EDGE_S >= self.known_until {
                return true;
            }
            // A bar no frame saw is unknown: left out.
            if let Some(muted) = self.muted_at(note_s) {
                self.tallies[usize::from(muted)].push(event);
            }
            false
        });
        self.pending = pending;
    }

    /// The silent bars seen: start and end.
    pub(super) fn silent_spans(&self) -> impl Iterator<Item = (f64, f64)> + '_ {
        self.seen.iter().filter(|&&(_, _, muted)| muted).map(|&(start, end, _)| (start, end))
    }

    /// Whether the note at `note_s` was in a silent bar; `None` in a bar no
    /// frame saw. A note on a bar's start belongs to that bar.
    fn muted_at(&self, note_s: f64) -> Option<bool> {
        let note_s = note_s + BAR_EDGE_S;
        let i = self.seen.partition_point(|&(start, _, _)| start <= note_s).checked_sub(1)?;
        let (_, end, muted) = self.seen[i];
        (note_s < end).then_some(muted)
    }
}

/// Hit stats and counters of a part of a run: one voice, or the bars with
/// or without the click.
#[derive(Clone, Default)]
pub(super) struct VoiceTally {
    pub stats: AccuracyStats,
    pub counters: RunCounters,
    pub tight: TightTally,
}

impl VoiceTally {
    pub(super) fn push_hit(&mut self, hit: GradedHit, grade: HitGrade) {
        self.stats.push(hit.delta_ms);
        self.counters.push_hit(grade);
        self.tight.push(hit, self.stats.mean_ms().unwrap_or(0.0));
    }

    fn push(&mut self, event: BarEvent) {
        match event {
            BarEvent::Hit(hit, grade) => self.push_hit(hit, grade),
            BarEvent::Miss => self.counters.misses += 1,
            BarEvent::Extra => self.counters.extra += 1,
        }
    }

    /// `None` while the voice had no note to hit yet.
    fn summary(&self) -> Option<RunSummary> {
        if self.stats.count + self.counters.misses == 0 {
            return None;
        }
        Some(run_summary(&self.stats, &self.counters, &self.tight))
    }
}

/// The summary of `stats`, `counters` and the `tight` counts of the same hits.
fn run_summary(stats: &AccuracyStats, counters: &RunCounters, tight: &TightTally) -> RunSummary {
    let mean_ms = stats.mean_ms().unwrap_or(0.0);
    RunSummary {
        hits: stats.count,
        misses: counters.misses,
        extra: counters.extra,
        perfect: counters.perfect,
        good: counters.good,
        mean_ms,
        spread_ms: stats.spread_ms().unwrap_or(0.0),
        tight: tight.counts,
    }
}

/// The tallies of the voices in `voices` (see [`GlobalBeatOnset::voices`]).
pub(super) fn tallies_of(
    tallies: &mut [VoiceTally; TRACKED_VOICES],
    voices: u8,
) -> impl Iterator<Item = &mut VoiceTally> {
    tallies.iter_mut().enumerate().filter(move |(v, _)| voices & 1 << v != 0).map(|(_, t)| t)
}

impl RecordingData {
    /// Per tracked voice, as [`Self::summary`]: `None` for a voice without
    /// notes, or before the run's first hit.
    pub(super) fn voice_summaries(&self) -> [Option<RunSummary>; TRACKED_VOICES] {
        let any_hit = self.stats.count > 0;
        self.voice_tallies.each_ref().map(|t| t.summary().filter(|_| any_hit))
    }

    /// The notes in bars with the click and in silent bars, as
    /// [`Self::voice_summaries`]: `None` for both unless the gap click
    /// silenced a bar.
    pub(super) fn bar_summaries(&self) -> [Option<RunSummary>; 2] {
        let tallies = &self.bars.tallies;
        let silent = tallies[1].stats.count + tallies[1].counters.misses > 0;
        let any_hit = self.stats.count > 0;
        tallies.each_ref().map(|t| t.summary().filter(|_| silent && any_hit))
    }

    /// `None` until the first hit: a run without hits has nothing to show.
    pub(super) fn summary(&self) -> Option<RunSummary> {
        self.stats.mean_ms()?;
        Some(run_summary(&self.stats, &self.counters, &self.tight))
    }

    fn new(start_time: f64, last_tick: Option<f64>) -> Self {
        Self {
            start_time,
            last_tick,
            anchored_ahead: true,
            miss_delay_s: 0.0,
            marks_by_onset: HashMap::new(),
            hits_in_loop: HashSet::new(),
            hits_next_loop: HashSet::new(),
            stats: AccuracyStats::default(),
            counters: RunCounters::default(),
            tight: TightTally::default(),
            voice_tallies: Default::default(),
            bars: BarTimeline::default(),
            drift: DriftLog::default(),
            positions: PositionLog::default(),
        }
    }
}

impl RecordingSession {
    pub(super) fn start(&mut self, start_time: f64, last_tick: Option<f64>) {
        *self = Self::Recording(Box::new(RecordingData::new(start_time, last_tick)));
    }

    pub(super) fn stop(&mut self) {
        *self = Self::Idle;
    }

    pub(super) fn is_recording(&self) -> bool {
        matches!(self, Self::Recording(_))
    }

    pub(super) fn data(&self) -> Option<&RecordingData> {
        match self {
            Self::Recording(d) => Some(d),
            Self::Idle => None,
        }
    }

    pub(super) fn data_mut(&mut self) -> Option<&mut RecordingData> {
        match self {
            Self::Recording(d) => Some(d),
            Self::Idle => None,
        }
    }

    /// Reset stats, marks and progress cursor while staying in the same
    /// recording state. No-op when `Idle`.
    pub(super) fn clear_marks(&mut self) {
        if let Self::Recording(d) = self {
            d.marks_by_onset.clear();
            d.hits_in_loop.clear();
            d.hits_next_loop.clear();
            d.stats.reset();
            d.counters = RunCounters::default();
            d.tight = TightTally::default();
            d.voice_tallies = Default::default();
            d.bars = BarTimeline::default();
            d.drift = DriftLog::default();
            d.positions = PositionLog::default();
            d.last_tick = None;
        }
    }

    /// The score lost `ticks` from its front: drop the marks and loop hit
    /// bookkeeping of the onsets there and move the others back by as much,
    /// keeping stats and counters. No-op when `Idle`.
    pub(super) fn drop_leading_ticks(&mut self, ticks: u64) {
        if let Self::Recording(d) = self {
            d.marks_by_onset = shift_marks(std::mem::take(&mut d.marks_by_onset), ticks);
            let shift = |set: &mut HashSet<u64>| {
                *set = set.iter().filter_map(|t| t.checked_sub(ticks)).collect();
            };
            shift(&mut d.hits_in_loop);
            shift(&mut d.hits_next_loop);
        }
    }

    /// Shift the recording's clock anchor. No-op when `Idle`.
    pub(super) fn realign(&mut self, start_time: f64, last_tick: f64) {
        if let Self::Recording(d) = self {
            d.start_time = start_time;
            d.last_tick = Some(last_tick);
            d.anchored_ahead = true;
        }
    }
}

/// Marks keyed by onset tick, moved back by `ticks`; the ones before it go.
pub(super) fn shift_marks(
    marks: HashMap<u64, AccuracyMark>,
    ticks: u64,
) -> HashMap<u64, AccuracyMark> {
    marks.into_iter().filter_map(|(t, m)| Some((t.checked_sub(ticks)?, m))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(hits: u64, perfect: u64, good: u64, misses: u64, extra: u64) -> RunSummary {
        RunSummary {
            hits,
            misses,
            extra,
            perfect,
            good,
            mean_ms: 0.0,
            spread_ms: 0.0,
            tight: (perfect, good),
        }
    }

    #[test]
    fn score_weights_perfect_full_and_good_half() {
        // 4 perfect + 2 good + 2 off hits, 1 miss, 1 extra = 10 attempts.
        let s = summary(8, 4, 2, 1, 1);
        assert_eq!(s.off(), 2);
        assert!((s.score_percent() - 50.0).abs() < 1e-9);
        assert_eq!(summary(0, 0, 0, 0, 0).score_percent(), 0.0);
    }

    #[test]
    fn steady_early_hits_are_tight_but_off_the_click() {
        let windows = HitWindows::for_gap_ms(500.0);
        // Always about 40 ms early: "good" on the click, perfect around the
        // average.
        let graded: Vec<GradedHit> =
            [-38.0, -42.0, -40.0, -41.0].map(|delta_ms| GradedHit { delta_ms, windows }).to_vec();
        let mut stats = AccuracyStats::default();
        let mut counters = RunCounters::default();
        let mut tight = TightTally::default();
        for &h in &graded {
            stats.push(h.delta_ms);
            counters.push_hit(windows.grade(h.delta_ms));
            tight.push(h, stats.mean_ms().unwrap());
        }
        let s = run_summary(&stats, &counters, &tight);
        assert!((s.score_percent() - 50.0).abs() < 1e-9);
        assert_eq!(s.tight_percent(), 100.0);
    }

    /// The summary of hits with these deviations and windows, plus misses
    /// and extra hits.
    fn summary_of(hits: &[(f64, HitWindows)], misses: u64, extra: u64) -> RunSummary {
        let graded: Vec<GradedHit> =
            hits.iter().map(|&(delta_ms, windows)| GradedHit { delta_ms, windows }).collect();
        let mut stats = AccuracyStats::default();
        let mut counters = RunCounters { misses, extra, ..Default::default() };
        let mut tight = TightTally::default();
        for &h in &graded {
            stats.push(h.delta_ms);
            counters.push_hit(h.windows.grade(h.delta_ms));
            tight.push(h, stats.mean_ms().unwrap());
        }
        run_summary(&stats, &counters, &tight)
    }

    #[test]
    fn tight_equals_the_score_around_a_zero_average() {
        let w = HitWindows::for_gap_ms(500.0);
        let s = summary_of(&[(-30.0, w), (30.0, w), (-10.0, w), (10.0, w)], 1, 1);
        assert_eq!(s.mean_ms, 0.0);
        assert_eq!(s.tight_percent(), s.score_percent());
    }

    #[test]
    fn tight_counts_misses_and_extra_hits_against_it() {
        let w = HitWindows::for_gap_ms(500.0);
        // Two hits perfect around their average, one miss, one extra: 2 of 4.
        let s = summary_of(&[(-40.0, w), (-40.0, w)], 1, 1);
        assert_eq!(s.tight_percent(), 50.0);
    }

    #[test]
    fn tight_keeps_the_narrow_windows_of_fast_notes() {
        // Sixteenths at 160 BPM: perfect only within 18.75 ms.
        let fast = HitWindows::for_gap_ms(93.75);
        let slow = HitWindows::for_gap_ms(500.0);
        // Average -10 ms: the fast note is 20 ms off it, only good; on a
        // slow note it would be perfect.
        let s = summary_of(&[(-30.0, fast), (0.0, slow), (0.0, slow)], 0, 0);
        assert_eq!(s.mean_ms, -10.0);
        assert_eq!(s.tight, (2, 1));
        let s = summary_of(&[(-30.0, slow), (0.0, slow), (0.0, slow)], 0, 0);
        assert_eq!(s.tight, (3, 0));
    }

    #[test]
    fn notes_in_a_bar_no_frame_saw_are_left_out() {
        let hit = |delta_ms| {
            let windows = HitWindows::for_gap_ms(500.0);
            BarEvent::Hit(GradedHit { delta_ms, windows }, windows.grade(delta_ms))
        };
        let mut bars = BarTimeline::default();
        // Bars of 2 s: silent, then one with the click no frame saw (a
        // stall), then silent again.
        bars.push(1.0, hit(10.0));
        bars.observe(Some((0.0, 2.0, true)));
        bars.push(2.0, hit(20.0));
        bars.push(3.0, BarEvent::Miss);
        bars.push(4.0, hit(30.0));
        bars.observe(Some((4.0, 6.0, true)));
        bars.push(5.9995, BarEvent::Extra);
        bars.settle();
        let [audible, silent] = &bars.tallies;
        assert_eq!(audible.stats.count + audible.counters.misses, 0);
        assert_eq!((silent.stats.count, silent.counters.misses), (2, 0));
        // Within the edge of the next bar: not seen either.
        assert_eq!(silent.counters.extra, 0);
    }

    #[test]
    fn windows_sort_hits_into_grades() {
        let w = HitWindows::for_gap_ms(500.0);
        let grades: Vec<HitGrade> =
            [0.0, -PERFECT_WINDOW_MS, PERFECT_WINDOW_MS + 1.0, -GOOD_WINDOW_MS, 100.0]
                .into_iter()
                .map(|d| w.grade(d))
                .collect();
        use HitGrade::*;
        assert_eq!(grades, vec![Perfect, Perfect, Good, Good, Off]);

        let mut c = RunCounters::default();
        for g in grades {
            c.push_hit(g);
        }
        assert_eq!((c.perfect, c.good), (2, 2));
    }

    #[test]
    fn windows_narrow_for_fast_notes() {
        // Slow notes keep the fixed windows.
        assert_eq!(
            HitWindows::for_gap_ms(250.0),
            HitWindows { perfect_ms: PERFECT_WINDOW_MS, good_ms: GOOD_WINDOW_MS }
        );
        // 16ths at 160 BPM are ~94 ms apart: good must stay well inside the
        // ~47 ms half-gap that separates this note from its neighbour.
        let w = HitWindows::for_gap_ms(93.75);
        assert!((w.perfect_ms - 18.75).abs() < 1e-9);
        assert!((w.good_ms - 37.5).abs() < 1e-9);
        assert_eq!(w.grade(30.0), HitGrade::Good);
        assert_eq!(w.grade(40.0), HitGrade::Off);
    }

    #[test]
    fn neighbour_gap_wraps_around_the_loop() {
        let beat = |t: u64| GlobalBeatOnset {
            onset_tick: t,
            performed_tick: t,
            is_note: true,
            measure_idx: 0,
            voices: 1,
            position: None,
        };
        let beats = [beat(0), beat(10), beat(40)];
        // First beat: previous neighbour is the last beat, 60 ticks back across the wrap.
        assert_eq!(neighbour_gap_ticks(&beats, 0, 100.0), 10.0);
        assert_eq!(neighbour_gap_ticks(&beats, 2, 100.0), 30.0);
        assert_eq!(neighbour_gap_ticks(&beats[..1], 0, 100.0), 100.0);
    }

    #[test]
    fn stats_spread_is_standard_deviation() {
        let mut stats = AccuracyStats::default();
        for d in [-10.0, 10.0, -10.0, 10.0] {
            stats.push(d);
        }
        assert_eq!(stats.mean_ms(), Some(0.0));
        assert!((stats.spread_ms().unwrap() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn a_kit_scores_onsets_merge_its_voices() {
        let score =
            grooph_measure::notation::parse_score("2/4 H:q H:q ; r:e K:e r:q | 2/4 q q").unwrap();
        let timing = ScoreTiming::from_score(&score, 120);
        let onsets = compute_global_beat_onsets(&score, &timing);
        let q = grooph_measure::grid::DEFAULT_GRID
            .ticks_per_beat(&grooph_measure::TimeSignature::TWO_FOUR) as u64;
        let ticks: Vec<(u64, bool)> = onsets.iter().map(|o| (o.onset_tick, o.is_note)).collect();
        // The kick between the hi-hats is a note to hit; a tick is listed once.
        assert_eq!(ticks, [(0, true), (q / 2, true), (q, true), (2 * q, true), (3 * q, true)]);
        assert!(onsets.iter().skip(3).all(|o| o.measure_idx == 1));
    }

    #[test]
    fn onsets_are_found_again_only_for_another_score_or_tempo() {
        use grooph_measure::notation::parse_score;
        let score = parse_score("4/4 q q q q").unwrap();
        let timing = ScoreTiming::from_score(&score, 120);
        let first = global_beat_onsets(&score, &timing);
        assert!(Rc::ptr_eq(&first, &global_beat_onsets(&score.clone(), &timing)));

        let faster = ScoreTiming::from_score(&score, 140);
        assert!(!Rc::ptr_eq(&first, &global_beat_onsets(&score, &faster)));
        let edited = parse_score("4/4 q q h").unwrap();
        let onsets = global_beat_onsets(&edited, &timing);
        assert_eq!(onsets.len(), compute_global_beat_onsets(&edited, &timing).len());
        assert_eq!(onsets.len(), 3);
        // The first is still kept.
        assert!(Rc::ptr_eq(&first, &global_beat_onsets(&score, &timing)));
    }
}
