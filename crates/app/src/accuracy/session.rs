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
    fn reset(&mut self) { *self = Self::default(); }

    pub(super) fn push(&mut self, delta_ms: f64) {
        self.count = self.count.saturating_add(1);
        self.sum_ms += delta_ms;
        self.sum_abs_ms += delta_ms.abs();
        self.sum_sq_ms += delta_ms * delta_ms;
        self.last_delta_ms = Some(delta_ms);
    }
}

/// A single beat's onset position in score-global ticks, along with whether it
/// is a Note (matchable for accuracy) or a Rest (only used as a structural
/// anchor when computing per-note hit windows). The list is monotonically
/// increasing in `onset_tick`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GlobalBeatOnset {
    pub onset_tick: u64,
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
}

impl RecordingData {
    fn new(start_time: f64, last_tick: Option<f64>) -> Self {
        Self {
            start_time,
            last_tick,
            marks_by_onset: HashMap::new(),
            hits_in_loop: HashSet::new(),
            hits_next_loop: HashSet::new(),
            stats: AccuracyStats::default(),
        }
    }
}

impl RecordingSession {
    pub(super) fn start(&mut self, start_time: f64, last_tick: Option<f64>) {
        *self = Self::Recording(Box::new(RecordingData::new(start_time, last_tick)));
    }

    pub(super) fn stop(&mut self) { *self = Self::Idle; }

    pub(super) fn is_recording(&self) -> bool { matches!(self, Self::Recording(_)) }

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
