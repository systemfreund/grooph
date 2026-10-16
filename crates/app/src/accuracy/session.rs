use std::collections::{HashMap, HashSet};

use grooph_measure::BeatKind;
use grooph_measure::Score;
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::tempo::ScoreTiming;

#[derive(Clone, Copy)]
pub(crate) enum AccuracyMark {
    Hit(f64),
    Miss,
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
    pub(super) fn push_hit(&mut self, delta_ms: f64) {
        let d = delta_ms.abs();
        if d <= PERFECT_WINDOW_MS {
            self.perfect += 1;
        } else if d <= GOOD_WINDOW_MS {
            self.good += 1;
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
}

/// Flatten every beat of every measure into a vector of score-global onset
/// ticks (Notes + Rests). The score iterates measures in order; within each
/// measure beats are in order.
pub(crate) fn compute_global_beat_onsets(
    score: &Score,
    timing: &ScoreTiming,
) -> Vec<GlobalBeatOnset> {
    let mut out = Vec::new();
    for (m_idx, measure) in score.measures.iter().enumerate() {
        let start = timing.measure_start_tick(m_idx);
        let local_onsets = DEFAULT_GRID.compute_onset_ticks(measure.beats());
        for (i, beat) in measure.beats().iter().enumerate() {
            if let Some(&local) = local_onsets.get(i) {
                out.push(GlobalBeatOnset {
                    onset_tick: start + local as u64,
                    performed_tick: timing.performed_global_tick(m_idx, local),
                    is_note: beat.kind == BeatKind::Note,
                });
            }
        }
    }
    out
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

    /// Shift the recording's clock anchor. No-op when `Idle`.
    pub(super) fn realign(&mut self, start_time: f64, last_tick: f64) {
        if let Self::Recording(d) = self {
            d.start_time = start_time;
            d.last_tick = Some(last_tick);
        }
    }
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
    fn counters_sort_hits_into_windows() {
        let mut c = RunCounters::default();
        for d in [0.0, -PERFECT_WINDOW_MS, PERFECT_WINDOW_MS + 1.0, -GOOD_WINDOW_MS, 100.0] {
            c.push_hit(d);
        }
        assert_eq!((c.perfect, c.good), (2, 2));
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
}
