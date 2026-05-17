//! Bidirectional tempo backbone for a [`Score`].
//!
//! Translates between global position (seconds, global ticks) and local
//! position (`measure_idx`, local ticks). The tick axis is **piecewise
//! linear**: each measure has its own `ticks_per_sec` because
//! `DEFAULT_GRID.ticks_per_beat` depends on `time_signature.beat_unit`.
//!
//! Forward direction (audio, schedule): given a measure index and local tick,
//! compute the global tick or seconds. Used to build the audio schedule and
//! advance the audio cursor through the entire score.
//!
//! Backward direction (midi, ui mapping): given a global tick (e.g. from the
//! audio thread), recover `(measure_idx, local_tick)` so a UI cursor or an
//! incoming MIDI event can be attributed to the correct measure.

use crate::Score;
use crate::grid::DEFAULT_GRID;

#[derive(Debug, Clone, PartialEq)]
pub struct ScoreTiming {
    bpm: u32,
    /// Cumulative global tick at which each measure starts. Length =
    /// `score.len() + 1`, with the last element equal to `total_loop_ticks`.
    measure_starts: Vec<u64>,
    /// Ticks per measure, per measure.
    measure_ticks: Vec<u32>,
    /// Ticks per beat, per measure (depends on TS beat_unit).
    ticks_per_beat: Vec<u32>,
    /// Cumulative global seconds at which each measure starts. Length =
    /// `score.len() + 1`, with the last element equal to `total_loop_seconds`.
    measure_seconds_starts: Vec<f64>,
    /// Seconds per tick, per measure.
    seconds_per_tick: Vec<f64>,
    total_loop_ticks: u64,
    total_loop_seconds: f64,
}

impl Default for ScoreTiming {
    /// Empty timing: zero measures, zero total ticks. Used as a placeholder
    /// before the first `from_score` call (e.g. while Audio is being
    /// constructed).
    fn default() -> Self {
        Self {
            bpm: 0,
            measure_starts: vec![0],
            measure_ticks: Vec::new(),
            ticks_per_beat: Vec::new(),
            measure_seconds_starts: vec![0.0],
            seconds_per_tick: Vec::new(),
            total_loop_ticks: 0,
            total_loop_seconds: 0.0,
        }
    }
}

impl ScoreTiming {
    pub fn from_score(score: &Score, bpm: u32) -> Self {
        let n = score.len();
        let mut measure_starts: Vec<u64> = Vec::with_capacity(n + 1);
        let mut measure_seconds_starts: Vec<f64> = Vec::with_capacity(n + 1);
        let mut measure_ticks: Vec<u32> = Vec::with_capacity(n);
        let mut ticks_per_beat: Vec<u32> = Vec::with_capacity(n);
        let mut seconds_per_tick: Vec<f64> = Vec::with_capacity(n);

        let mut tick_acc: u64 = 0;
        let mut sec_acc: f64 = 0.0;
        measure_starts.push(0);
        measure_seconds_starts.push(0.0);

        for m in &score.measures {
            let ts = m.time_signature();
            let tpb = DEFAULT_GRID.ticks_per_beat(&ts);
            let tpm = DEFAULT_GRID.ticks_per_measure(&ts);
            let tps = (bpm as f64 / 60.0) * tpb as f64;
            let spt = if tps > 0.0 { 1.0 / tps } else { 0.0 };

            measure_ticks.push(tpm);
            ticks_per_beat.push(tpb);
            seconds_per_tick.push(spt);

            tick_acc += tpm as u64;
            sec_acc += tpm as f64 * spt;
            measure_starts.push(tick_acc);
            measure_seconds_starts.push(sec_acc);
        }

        Self {
            bpm,
            measure_starts,
            measure_ticks,
            ticks_per_beat,
            measure_seconds_starts,
            seconds_per_tick,
            total_loop_ticks: tick_acc,
            total_loop_seconds: sec_acc,
        }
    }

    pub fn bpm(&self) -> u32 { self.bpm }
    pub fn measure_count(&self) -> usize { self.measure_ticks.len() }
    pub fn total_loop_ticks(&self) -> u64 { self.total_loop_ticks }
    pub fn total_loop_seconds(&self) -> f64 { self.total_loop_seconds }

    pub fn ticks_per_beat_in_measure(&self, idx: usize) -> u32 { self.ticks_per_beat[idx] }
    pub fn ticks_per_measure(&self, idx: usize) -> u32 { self.measure_ticks[idx] }
    pub fn ticks_per_sec_in_measure(&self, idx: usize) -> f64 {
        let spt = self.seconds_per_tick[idx];
        if spt > 0.0 { 1.0 / spt } else { 0.0 }
    }
    pub fn seconds_per_tick_in_measure(&self, idx: usize) -> f64 { self.seconds_per_tick[idx] }
    pub fn measure_start_tick(&self, idx: usize) -> u64 { self.measure_starts[idx] }
    pub fn measure_start_seconds(&self, idx: usize) -> f64 { self.measure_seconds_starts[idx] }

    /// Forward: `(measure_idx, local_tick) → global_tick`.
    pub fn to_global_tick(&self, measure_idx: usize, local_tick: u32) -> u64 {
        self.measure_starts[measure_idx] + local_tick as u64
    }

    /// Forward: real seconds → global tick. Wraps over `total_loop_seconds` if
    /// the input is outside `[0, total_loop_seconds)`.
    pub fn seconds_to_global_tick(&self, seconds: f64) -> f64 {
        if self.total_loop_seconds <= 0.0 {
            return 0.0;
        }
        let s = seconds.rem_euclid(self.total_loop_seconds);
        // Find the measure containing s. Linear scan is fine for realistic
        // score sizes; switch to binary search if profiling demands it.
        let mut idx = 0usize;
        for i in 0..self.measure_count() {
            if s < self.measure_seconds_starts[i + 1] {
                idx = i;
                break;
            }
            idx = i;
        }
        let local_seconds = s - self.measure_seconds_starts[idx];
        let local_tick = local_seconds / self.seconds_per_tick[idx];
        self.measure_starts[idx] as f64 + local_tick
    }

    /// Backward: global tick → `(measure_idx, local_tick)`. The input is
    /// wrapped over `total_loop_ticks`; the result's local_tick is in
    /// `[0, ticks_per_measure(measure_idx))`.
    pub fn to_local(&self, global_tick: f64) -> (usize, f64) {
        if self.total_loop_ticks == 0 {
            return (0, 0.0);
        }
        let g = global_tick.rem_euclid(self.total_loop_ticks as f64);
        let idx = self.measure_at_global_tick(g);
        let local = g - self.measure_starts[idx] as f64;
        (idx, local)
    }

    /// Backward: global tick → seconds.
    pub fn global_tick_to_seconds(&self, global_tick: f64) -> f64 {
        if self.total_loop_ticks == 0 {
            return 0.0;
        }
        let g = global_tick.rem_euclid(self.total_loop_ticks as f64);
        let idx = self.measure_at_global_tick(g);
        let local_tick = g - self.measure_starts[idx] as f64;
        self.measure_seconds_starts[idx] + local_tick * self.seconds_per_tick[idx]
    }

    /// Measure containing the given global tick. Assumes `global_tick` is in
    /// `[0, total_loop_ticks)`; for inputs outside this range, callers should
    /// `rem_euclid` first (the public `to_local` / `global_tick_to_seconds`
    /// methods do this internally).
    pub fn measure_at_global_tick(&self, global_tick: f64) -> usize {
        // Binary search over measure_starts (excluding the sentinel).
        // Find the largest i such that measure_starts[i] <= global_tick.
        let n = self.measure_count();
        if n == 0 {
            return 0;
        }
        let target = global_tick.max(0.0);
        let mut lo = 0usize;
        let mut hi = n; // upper bound on measure index
        while lo + 1 < hi {
            let mid = (lo + hi) / 2;
            if (self.measure_starts[mid] as f64) <= target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Measure, TimeSignature};

    fn score_of(ts: &[TimeSignature]) -> Score {
        Score { measures: ts.iter().map(|t| Measure::new(*t)).collect() }
    }

    #[test]
    fn single_measure_44_total_ticks_matches_grid() {
        let s = score_of(&[TimeSignature::FOUR_FOUR]);
        let t = ScoreTiming::from_score(&s, 120);
        let expected = DEFAULT_GRID.ticks_per_measure(&TimeSignature::FOUR_FOUR) as u64;
        assert_eq!(t.total_loop_ticks(), expected);
        assert_eq!(t.measure_count(), 1);
        assert_eq!(t.measure_start_tick(0), 0);
        assert_eq!(t.measure_start_tick(1), expected);
    }

    #[test]
    fn mixed_ts_total_ticks_is_sum() {
        let s = score_of(&[
            TimeSignature::FOUR_FOUR,
            TimeSignature::THREE_FOUR,
            TimeSignature::SIX_EIGHT,
        ]);
        let t = ScoreTiming::from_score(&s, 120);
        let sum: u64 = s
            .measures
            .iter()
            .map(|m| DEFAULT_GRID.ticks_per_measure(&m.time_signature()) as u64)
            .sum();
        assert_eq!(t.total_loop_ticks(), sum);
        // Cumulative starts are monotonic.
        for i in 1..t.measure_count() {
            assert!(t.measure_start_tick(i) > t.measure_start_tick(i - 1));
        }
    }

    #[test]
    fn to_global_then_to_local_roundtrip() {
        let s = score_of(&[
            TimeSignature::FOUR_FOUR,
            TimeSignature::THREE_FOUR,
            TimeSignature::SIX_EIGHT,
        ]);
        let t = ScoreTiming::from_score(&s, 120);
        for m_idx in 0..t.measure_count() {
            let tpm = t.ticks_per_measure(m_idx);
            for local in [0u32, 1, tpm / 2, tpm.saturating_sub(1)] {
                let g = t.to_global_tick(m_idx, local);
                let (back_idx, back_local) = t.to_local(g as f64);
                assert_eq!(back_idx, m_idx, "measure idx mismatch at g={g}");
                assert!(
                    (back_local - local as f64).abs() < 1e-9,
                    "local mismatch: expected {local}, got {back_local}"
                );
            }
        }
    }

    #[test]
    fn seconds_roundtrip() {
        let s = score_of(&[TimeSignature::FOUR_FOUR, TimeSignature::SIX_EIGHT]);
        let t = ScoreTiming::from_score(&s, 120);
        for &g_int in &[0u64, 1, 50, t.total_loop_ticks() / 2, t.total_loop_ticks() - 1] {
            let g = g_int as f64;
            let secs = t.global_tick_to_seconds(g);
            let back = t.seconds_to_global_tick(secs);
            assert!((back - g).abs() < 1e-6, "g={g} secs={secs} back={back}");
        }
    }

    #[test]
    fn mixed_ts_ticks_per_sec_differs_per_measure() {
        // 4/4 (beat_unit = quarter) vs. 4/8 (beat_unit = eighth):
        // at the same BPM, ticks_per_sec differs because ticks_per_beat differs.
        let s = score_of(&[TimeSignature::FOUR_FOUR, TimeSignature::FOUR_EIGHT]);
        let t = ScoreTiming::from_score(&s, 120);
        let tps0 = t.ticks_per_sec_in_measure(0);
        let tps1 = t.ticks_per_sec_in_measure(1);
        assert!(
            (tps0 - tps1).abs() > 1e-6,
            "expected different ticks_per_sec, got {tps0} and {tps1}"
        );
    }

    #[test]
    fn measure_at_global_tick_at_boundaries() {
        let s = score_of(&[
            TimeSignature::FOUR_FOUR,
            TimeSignature::THREE_FOUR,
            TimeSignature::FOUR_FOUR,
        ]);
        let t = ScoreTiming::from_score(&s, 120);
        // First tick of each measure.
        assert_eq!(t.measure_at_global_tick(0.0), 0);
        assert_eq!(t.measure_at_global_tick(t.measure_start_tick(1) as f64), 1);
        assert_eq!(t.measure_at_global_tick(t.measure_start_tick(2) as f64), 2);
        // Just before a boundary belongs to the previous measure.
        let just_before = (t.measure_start_tick(1) - 1) as f64;
        assert_eq!(t.measure_at_global_tick(just_before), 0);
    }

    #[test]
    fn to_local_at_total_loop_ticks_wraps_to_zero() {
        let s = score_of(&[TimeSignature::FOUR_FOUR, TimeSignature::THREE_FOUR]);
        let t = ScoreTiming::from_score(&s, 120);
        let total = t.total_loop_ticks() as f64;
        let (idx, local) = t.to_local(total);
        assert_eq!(idx, 0);
        assert!(local.abs() < 1e-9);
    }

    #[test]
    fn seconds_to_global_wraps_over_total_seconds() {
        let s = score_of(&[TimeSignature::FOUR_FOUR]);
        let t = ScoreTiming::from_score(&s, 120);
        let one_loop = t.total_loop_seconds();
        // 1.5 loops should map to 0.5 loops.
        let g_half = t.seconds_to_global_tick(0.5 * one_loop);
        let g_1_5 = t.seconds_to_global_tick(1.5 * one_loop);
        assert!((g_half - g_1_5).abs() < 1e-6);
    }
}
