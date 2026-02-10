use std::collections::{HashMap, HashSet};

use grooph_measure::{Beat, BeatKind};
use log::info;

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

#[derive(Default)]
pub(crate) struct AccuracyTracker {
    start_time: Option<f64>,
    stats: AccuracyStats,
    marks_by_onset: HashMap<u32, AccuracyMark>,
    hits_in_loop: HashSet<u32>,
    hits_next_loop: HashSet<u32>,
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

    pub(crate) fn mark_for_onset(&self, onset_tick: u32) -> Option<AccuracyMark> {
        self.marks_by_onset.get(&onset_tick).copied()
    }

    pub(crate) fn record_hit(
        &mut self,
        timestamp: f64,
        ticks_per_sec: f64,
        ticks_per_measure: f64,
        beats: &[Beat],
        beat_onsets: &[u32],
        bpm: u32,
    ) {
        let Some(start_time) = self.start_time else {
            return;
        };
        if ticks_per_sec <= 0.0
            || ticks_per_measure <= 0.0
            || beats.is_empty()
            || beats.len() != beat_onsets.len()
        {
            return;
        }
        let elapsed = timestamp - start_time;
        if elapsed < 0.0 {
            return;
        }
        let hit_tick = (elapsed * ticks_per_sec).rem_euclid(ticks_per_measure);
        let mut best: Option<(usize, f64, f64)> = None;
        for (idx, &onset_tick_u32) in beat_onsets.iter().enumerate() {
            let onset_tick = onset_tick_u32 as f64;
            let raw_diff = hit_tick - onset_tick;
            let mut diff = raw_diff;
            if diff > ticks_per_measure * 0.5 {
                diff -= ticks_per_measure;
            } else if diff < -ticks_per_measure * 0.5 {
                diff += ticks_per_measure;
            }
            if best.map_or(true, |(_, best_diff, _)| diff.abs() < best_diff.abs()) {
                best = Some((idx, diff, raw_diff));
            }
        }
        let Some((best_idx, diff_ticks, raw_diff)) = best else {
            return;
        };
        if beats.get(best_idx).map_or(true, |b| b.kind != BeatKind::Note) {
            return;
        }
        let onset_tick = beat_onsets[best_idx];
        let wrap_next = raw_diff > ticks_per_measure * 0.5;
        let wrap_prev = raw_diff < -ticks_per_measure * 0.5;
        if wrap_next {
            if self.hits_next_loop.contains(&onset_tick) {
                return;
            }
        } else if !wrap_prev && self.hits_in_loop.contains(&onset_tick) {
            return;
        }

        let delta_ms = (diff_ticks / ticks_per_sec) * 1000.0;
        self.stats.push(delta_ms);
        self.marks_by_onset
            .insert(onset_tick, AccuracyMark::Hit(diff_ticks));
        if wrap_next {
            self.hits_next_loop.insert(onset_tick);
        } else if !wrap_prev {
            self.hits_in_loop.insert(onset_tick);
        }
        info!(
            "Accuracy hit: onset_tick={} hit_tick={:.2} delta_ms={:+.2} bpm={}",
            onset_tick,
            hit_tick,
            delta_ms,
            bpm
        );
    }

    pub(crate) fn update_progress(
        &mut self,
        now_seconds: f64,
        ticks_per_sec: f64,
        ticks_per_measure: f64,
        beats: &[Beat],
        beat_onsets: &[u32]
    ) {
        let Some(start_time) = self.start_time else {
            return;
        };
        if beats.len() != beat_onsets.len() || beats.is_empty() {
            return;
        }
        let elapsed = now_seconds - start_time;
        if elapsed < 0.0 {
            return;
        }
        let current_tick = (elapsed * ticks_per_sec).rem_euclid(ticks_per_measure);
        let Some(last_tick) = self.last_tick else {
            self.last_tick = Some(current_tick);
            return;
        };

        fn process_segment(
            beats: &[Beat],
            beat_onsets: &[u32],
            start: f64,
            end: f64,
            segment_offset: f64,
            ticks_per_measure: f64,
            marks_by_onset: &mut HashMap<u32, AccuracyMark>,
            hits_in_loop: &mut HashSet<u32>,
        ) {
            if beats.is_empty() || beat_onsets.is_empty() {
                return;
            }
            let seg_start = segment_offset + start;
            let seg_end = segment_offset + end;
            for (idx, beat) in beats.iter().enumerate() {
                if beat.kind != BeatKind::Note {
                    continue;
                }
                let onset_tick = *beat_onsets.get(idx).unwrap_or(&0);
                let cur = onset_tick as f64;
                let next = if idx + 1 < beat_onsets.len() {
                    beat_onsets[idx + 1] as f64
                } else {
                    beat_onsets[0] as f64 + ticks_per_measure
                };
                let mut window_end = cur + (next - cur) * 0.5;
                if window_end < segment_offset {
                    window_end += ticks_per_measure;
                }
                let in_range = window_end > seg_start && window_end <= seg_end;
                if in_range {
                    if !hits_in_loop.contains(&onset_tick) {
                        marks_by_onset.insert(onset_tick, AccuracyMark::Miss);
                    }
                    hits_in_loop.remove(&onset_tick);
                }
            }
        }

        if current_tick >= last_tick {
            process_segment(
                beats,
                beat_onsets,
                last_tick,
                current_tick,
                0.0,
                ticks_per_measure,
                &mut self.marks_by_onset,
                &mut self.hits_in_loop,
            );
        } else {
            // wrapped
            process_segment(
                beats,
                beat_onsets,
                last_tick,
                ticks_per_measure,
                0.0,
                ticks_per_measure,
                &mut self.marks_by_onset,
                &mut self.hits_in_loop,
            );
            self.hits_in_loop = std::mem::take(&mut self.hits_next_loop);
            process_segment(
                beats,
                beat_onsets,
                0.0,
                current_tick,
                ticks_per_measure,
                ticks_per_measure,
                &mut self.marks_by_onset,
                &mut self.hits_in_loop,
            );
        }

        self.last_tick = Some(current_tick);
    }

    pub(crate) fn clamp_diff_to_beat_window(
        diff_ticks: f64,
        beat_index: usize,
        beat_onsets: &[u32],
        total_ticks: f64,
    ) -> f64 {
        if total_ticks <= 0.0 {
            return diff_ticks;
        }
        if beat_onsets.is_empty() || beat_index >= beat_onsets.len() {
            return diff_ticks;
        }
        if beat_onsets.len() == 1 {
            let half = total_ticks * 0.5;
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
            cur_tick + total_ticks - prev_tick
        };
        let dist_next = if next_tick >= cur_tick {
            next_tick - cur_tick
        } else {
            next_tick + total_ticks - cur_tick
        };
        let left = -dist_prev * 0.5;
        let right = dist_next * 0.5;
        diff_ticks.clamp(left, right)
    }
}
