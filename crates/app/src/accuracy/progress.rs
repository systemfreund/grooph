use std::collections::{HashMap, HashSet};

use grooph_measure::Score;
use grooph_measure::tempo::ScoreTiming;

use super::session::{AccuracyMark, GlobalBeatOnset, RecordingData, compute_global_beat_onsets};

/// Advance the progress cursor and mark any note onsets whose hit window has
/// elapsed as `Miss`. Wraps detected when the global tick decreases across a
/// frame (= score loop just completed).
pub(super) fn update_progress(
    data: &mut RecordingData,
    now_seconds: f64,
    timing: &ScoreTiming,
    score: &Score,
) {
    let total_ticks = timing.total_loop_ticks();
    if total_ticks == 0 || timing.total_loop_seconds() <= 0.0 {
        return;
    }
    let elapsed = now_seconds - data.start_time;
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
    let Some(last_tick) = data.last_tick else {
        data.last_tick = Some(current_tick);
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
            &mut data.marks_by_onset,
            &mut data.hits_in_loop,
        );
    } else {
        // Score-loop wrapped.
        process_segment(
            &global_beats,
            last_tick,
            total,
            0.0,
            total,
            &mut data.marks_by_onset,
            &mut data.hits_in_loop,
        );
        data.hits_in_loop = std::mem::take(&mut data.hits_next_loop);
        process_segment(
            &global_beats,
            0.0,
            current_tick,
            total,
            total,
            &mut data.marks_by_onset,
            &mut data.hits_in_loop,
        );
    }

    data.last_tick = Some(current_tick);
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
