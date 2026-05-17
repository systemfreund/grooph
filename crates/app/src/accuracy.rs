use std::collections::{HashMap, HashSet};

use grooph_measure::Score;
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::tempo::ScoreTiming;
use grooph_measure::{BeatKind, MeasureIdx};
use log::info;

use crate::TransportState;

pub(crate) struct AccuracyState {
    pub(crate) tracker: AccuracyTracker,
    pub(crate) enabled: bool,
}

impl AccuracyState {
    pub(crate) fn new(enabled: bool) -> Self { Self { tracker: AccuracyTracker::new(), enabled } }

    pub(crate) fn set_enabled(&mut self, enabled: bool, transport: TransportState) {
        if self.enabled == enabled {
            return;
        }
        self.enabled = enabled;
        if enabled {
            if transport == TransportState::Playing {
                self.tracker.on_playback_stop();
            } else {
                self.tracker.clear_for_edit();
            }
            return;
        }
        self.tracker.on_playback_stop();
    }
}

#[derive(Clone, Copy)]
pub(crate) enum AccuracyMark {
    Hit(f64),
    Miss,
}

#[derive(Clone, Copy, Default)]
struct AccuracyStats {
    pub count: u64,
    pub sum_ms: f64,
    pub sum_abs_ms: f64,
    pub sum_sq_ms: f64,
    pub last_delta_ms: Option<f64>,
}

impl AccuracyStats {
    fn reset(&mut self) { *self = Self::default(); }

    fn push(&mut self, delta_ms: f64) {
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

#[derive(Default)]
pub(crate) struct AccuracyTracker {
    start_time: Option<f64>,
    stats: AccuracyStats,
    /// Accuracy marks keyed by *global* onset tick across the score loop.
    marks_by_onset: HashMap<u64, AccuracyMark>,
    /// Note onsets (global) that already received a hit in the current
    /// score-loop iteration. Prevents double-counting.
    hits_in_loop: HashSet<u64>,
    /// Hits whose `raw_diff` crossed the score-loop midpoint and therefore
    /// belong to the *next* score-loop iteration. Promoted to `hits_in_loop`
    /// when the score wraps.
    hits_next_loop: HashSet<u64>,
    /// Last observed global tick in the score loop, used by `update_progress`
    /// to detect wraps at the score boundary.
    last_tick: Option<f64>,
}

impl AccuracyTracker {
    pub(crate) fn new() -> Self { Self::default() }

    pub(crate) fn has_start_time(&self) -> bool { self.start_time.is_some() }

    pub(crate) fn update_state(
        &mut self,
        playing: bool,
        is_connected: bool,
        now_seconds: f64,
    ) -> bool {
        if playing && is_connected {
            if self.start_time.is_none() {
                self.clear_for_edit();
                self.start_time = Some(now_seconds);
                self.last_tick = Some(0.0);
            }
            true
        } else {
            if self.start_time.is_some() {
                self.clear_for_edit();
                self.start_time = None;
            }
            self.last_tick = None;
            false
        }
    }

    pub(crate) fn on_playback_start(&mut self, now_seconds: Option<f64>) {
        self.stats.reset();
        self.marks_by_onset.clear();
        self.hits_in_loop.clear();
        self.hits_next_loop.clear();
        self.last_tick = None;
        self.start_time = now_seconds;
        if self.start_time.is_some() {
            self.last_tick = Some(0.0);
        }
    }

    pub(crate) fn on_playback_start_at(&mut self, start_time: f64, last_tick: f64) {
        self.stats.reset();
        self.marks_by_onset.clear();
        self.hits_in_loop.clear();
        self.hits_next_loop.clear();
        self.start_time = Some(start_time);
        self.last_tick = Some(last_tick);
    }

    pub(crate) fn realign_start_time(&mut self, start_time: f64, last_tick: f64) {
        self.start_time = Some(start_time);
        self.last_tick = Some(last_tick);
    }

    pub(crate) fn on_playback_stop(&mut self) {
        self.start_time = None;
        self.marks_by_onset.clear();
        self.hits_in_loop.clear();
        self.hits_next_loop.clear();
        self.last_tick = None;
    }

    pub(crate) fn clear_for_edit(&mut self) {
        self.stats.reset();
        self.marks_by_onset.clear();
        self.hits_in_loop.clear();
        self.hits_next_loop.clear();
        self.last_tick = None;
    }

    /// Look up the accuracy mark for a score-global onset tick.
    pub(crate) fn mark_for_onset(&self, global_onset_tick: u64) -> Option<AccuracyMark> {
        self.marks_by_onset.get(&global_onset_tick).copied()
    }

    /// Record an incoming MIDI hit at `timestamp` (seconds, MIDI clock). The
    /// hit is mapped to a global tick via `timing`, then matched against the
    /// nearest note onset across the whole score (shortest signed distance on
    /// the loop). The delta is converted to milliseconds using the onset
    /// measure's tempo.
    pub(crate) fn record_hit(&mut self, timestamp: f64, timing: &ScoreTiming, score: &Score) {
        let Some(start_time) = self.start_time else {
            return;
        };
        let total_ticks = timing.total_loop_ticks();
        if total_ticks == 0 || timing.total_loop_seconds() <= 0.0 {
            return;
        }
        let elapsed = timestamp - start_time;
        if elapsed < 0.0 {
            return;
        }
        let total = total_ticks as f64;
        let hit_global = timing.seconds_to_global_tick(elapsed);

        // Best-match across all note beats in all measures.
        let mut best: Option<(u64, MeasureIdx, f64, f64)> = None; // (global_onset, m_idx, signed_diff, raw_diff)
        for (m_idx, measure) in score.measures.iter().enumerate() {
            if measure.beats().is_empty() {
                continue;
            }
            let measure_start = timing.measure_start_tick(m_idx);
            let local_onsets = DEFAULT_GRID.compute_onset_ticks(measure.beats());
            for (i, beat) in measure.beats().iter().enumerate() {
                if beat.kind != BeatKind::Note {
                    continue;
                }
                let Some(&local) = local_onsets.get(i) else {
                    continue;
                };
                let global_onset = measure_start + local as u64;
                let raw_diff = hit_global - global_onset as f64;
                let mut diff = raw_diff;
                if diff > total * 0.5 {
                    diff -= total;
                } else if diff < -total * 0.5 {
                    diff += total;
                }
                if best.is_none_or(|(_, _, best_diff, _)| diff.abs() < best_diff.abs()) {
                    best = Some((global_onset, m_idx, diff, raw_diff));
                }
            }
        }

        let Some((global_onset, onset_m_idx, diff_ticks, raw_diff)) = best else {
            return;
        };

        let wrap_next = raw_diff > total * 0.5;
        let wrap_prev = raw_diff < -total * 0.5;
        if wrap_next {
            if self.hits_next_loop.contains(&global_onset) {
                return;
            }
        } else if !wrap_prev && self.hits_in_loop.contains(&global_onset) {
            return;
        }

        // Convert tick delta to ms using the onset measure's local rate. Since
        // |diff_ticks| stays close to a single beat width, the choice of
        // measure has negligible impact for cross-measure hits.
        let tps = timing.ticks_per_sec_in_measure(onset_m_idx);
        let delta_ms = if tps > 0.0 { (diff_ticks / tps) * 1000.0 } else { 0.0 };

        self.stats.push(delta_ms);
        self.marks_by_onset.insert(global_onset, AccuracyMark::Hit(diff_ticks));
        if wrap_next {
            self.hits_next_loop.insert(global_onset);
        } else if !wrap_prev {
            self.hits_in_loop.insert(global_onset);
        }
        info!(
            "Accuracy hit: global_onset={} hit_global={:.2} delta_ms={:+.2} bpm={}",
            global_onset,
            hit_global,
            delta_ms,
            timing.bpm()
        );
    }

    /// Advance the progress cursor and mark any note onsets whose hit window
    /// has elapsed as `Miss`. Wraps detected when the global tick decreases
    /// across a frame (= score loop just completed).
    pub(crate) fn update_progress(
        &mut self,
        now_seconds: f64,
        timing: &ScoreTiming,
        score: &Score,
    ) {
        let Some(start_time) = self.start_time else {
            return;
        };
        let total_ticks = timing.total_loop_ticks();
        if total_ticks == 0 || timing.total_loop_seconds() <= 0.0 {
            return;
        }
        let elapsed = now_seconds - start_time;
        if elapsed < 0.0 {
            return;
        }
        let total = total_ticks as f64;
        let global_beats = compute_global_beat_onsets(score, timing);
        if global_beats.is_empty() {
            return;
        }

        let mut current_tick = timing.seconds_to_global_tick(elapsed);
        // seconds_to_global_tick already rem_euclids, but be defensive.
        if current_tick >= total {
            current_tick %= total;
        }
        let Some(last_tick) = self.last_tick else {
            self.last_tick = Some(current_tick);
            return;
        };
        let epsilon = total.max(1.0) * 1e-9;
        if current_tick + epsilon >= last_tick && current_tick < last_tick {
            current_tick = last_tick;
        }

        if current_tick >= last_tick {
            process_segment(
                &global_beats,
                last_tick,
                current_tick,
                0.0,
                total,
                &mut self.marks_by_onset,
                &mut self.hits_in_loop,
            );
        } else {
            // Score-loop wrapped.
            process_segment(
                &global_beats,
                last_tick,
                total,
                0.0,
                total,
                &mut self.marks_by_onset,
                &mut self.hits_in_loop,
            );
            self.hits_in_loop = std::mem::take(&mut self.hits_next_loop);
            process_segment(
                &global_beats,
                0.0,
                current_tick,
                total,
                total,
                &mut self.marks_by_onset,
                &mut self.hits_in_loop,
            );
        }

        self.last_tick = Some(current_tick);
    }

    /// Clamp a hit's `diff_ticks` to the half-distance to its neighbouring
    /// onsets, so the visual marker stays inside the beat's hit window.
    /// Operates on score-global beat onsets.
    pub(crate) fn clamp_diff_to_beat_window(
        diff_ticks: f64,
        beat_index: usize,
        beat_onsets: &[u64],
        total_loop_ticks: f64,
    ) -> f64 {
        if total_loop_ticks <= 0.0 {
            return diff_ticks;
        }
        if beat_onsets.is_empty() || beat_index >= beat_onsets.len() {
            return diff_ticks;
        }
        if beat_onsets.len() == 1 {
            let half = total_loop_ticks * 0.5;
            return diff_ticks.clamp(-half, half);
        }
        let cur_tick = beat_onsets[beat_index] as f64;
        let prev_idx = (beat_index + beat_onsets.len() - 1) % beat_onsets.len();
        let next_idx = (beat_index + 1) % beat_onsets.len();
        let prev_tick = beat_onsets[prev_idx] as f64;
        let next_tick = beat_onsets[next_idx] as f64;
        let dist_prev = if cur_tick >= prev_tick {
            cur_tick - prev_tick
        } else {
            cur_tick + total_loop_ticks - prev_tick
        };
        let dist_next = if next_tick >= cur_tick {
            next_tick - cur_tick
        } else {
            next_tick + total_loop_ticks - cur_tick
        };
        let left = -dist_prev * 0.5;
        let right = dist_next * 0.5;
        diff_ticks.clamp(left, right)
    }
}

#[allow(clippy::too_many_arguments)]
fn process_segment(
    global_beats: &[GlobalBeatOnset],
    start: f64,
    end: f64,
    segment_offset: f64,
    total_loop_ticks: f64,
    marks_by_onset: &mut HashMap<u64, AccuracyMark>,
    hits_in_loop: &mut HashSet<u64>,
) {
    if global_beats.is_empty() {
        return;
    }
    let seg_start = segment_offset + start;
    let seg_end = segment_offset + end;
    for (i, gb) in global_beats.iter().enumerate() {
        if !gb.is_note {
            continue;
        }
        let cur = gb.onset_tick as f64;
        let next = if i + 1 < global_beats.len() {
            global_beats[i + 1].onset_tick as f64
        } else {
            global_beats[0].onset_tick as f64 + total_loop_ticks
        };
        let mut window_end = cur + (next - cur) * 0.5;
        if window_end < segment_offset {
            window_end += total_loop_ticks;
        }
        let in_range = window_end > seg_start && window_end <= seg_end;
        if in_range {
            if !hits_in_loop.contains(&gb.onset_tick) {
                marks_by_onset.insert(gb.onset_tick, AccuracyMark::Miss);
            }
            hits_in_loop.remove(&gb.onset_tick);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grooph_measure::duration::q;
    use grooph_measure::{Beat, Measure, Score, TimeSignature};

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
