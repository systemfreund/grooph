use std::collections::{HashMap, HashSet};

use grooph_measure::MeasureIdx;
use grooph_measure::Score;
use grooph_measure::tempo::ScoreTiming;

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
}

impl RunSummary {
    /// Hits outside the good window.
    pub(crate) fn off(&self) -> u64 {
        self.hits.saturating_sub(self.perfect + self.good)
    }

    /// 0–100: perfect hits count fully, good ones half; misses and extra
    /// hits count against the score.
    pub(crate) fn score_percent(&self) -> f64 {
        let total = self.hits + self.misses + self.extra;
        if total == 0 {
            return 0.0;
        }
        (self.perfect as f64 + 0.5 * self.good as f64) / total as f64 * 100.0
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
}

/// Flatten the onsets of every measure into a vector of score-global onset
/// ticks (Notes + Rests): each tick where a beat of any voice starts, once,
/// a note if any voice plays one there. The score iterates measures in
/// order; within each measure onsets are in order.
pub(crate) fn compute_global_beat_onsets(
    score: &Score,
    timing: &ScoreTiming,
) -> Vec<GlobalBeatOnset> {
    let mut out = Vec::new();
    for (m_idx, measure) in score.measures.iter().enumerate() {
        let start = timing.measure_start_tick(m_idx);
        for onset in measure.onsets() {
            out.push(GlobalBeatOnset {
                onset_tick: start + onset.tick as u64,
                performed_tick: timing.performed_global_tick(m_idx, onset.tick),
                is_note: onset.is_note,
                measure_idx: m_idx,
            });
        }
    }
    out
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
}

impl RecordingData {
    /// `None` until the first hit: a run without hits has nothing to show.
    pub(super) fn summary(&self) -> Option<RunSummary> {
        let mean_ms = self.stats.mean_ms()?;
        Some(RunSummary {
            hits: self.stats.count,
            misses: self.counters.misses,
            extra: self.counters.extra,
            perfect: self.counters.perfect,
            good: self.counters.good,
            mean_ms,
            spread_ms: self.stats.spread_ms().unwrap_or(0.0),
        })
    }

    fn new(start_time: f64, last_tick: Option<f64>) -> Self {
        Self {
            start_time,
            last_tick,
            marks_by_onset: HashMap::new(),
            hits_in_loop: HashSet::new(),
            hits_next_loop: HashSet::new(),
            stats: AccuracyStats::default(),
            counters: RunCounters::default(),
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
        RunSummary { hits, misses, extra, perfect, good, mean_ms: 0.0, spread_ms: 0.0 }
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
}
