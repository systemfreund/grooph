use grooph_measure::Score;
use grooph_measure::tempo::ScoreTiming;
use log::info;

use super::session::{
    AccuracyMark, GlobalBeatOnset, HitWindows, RecordingData, compute_global_beat_onsets,
    neighbour_gap_ticks,
};

/// Record an incoming MIDI hit at `timestamp` (seconds, MIDI clock). The hit
/// is mapped to a global tick via `timing`, then matched against the nearest
/// note onset across the whole score (shortest signed distance on the loop),
/// taking swing into account.
/// The delta is converted to milliseconds using the onset measure's tempo.
///
/// Returns the delta in ms when the hit was matched to a note; a second hit
/// on an already hit note counts as extra and returns `None`.
pub(super) fn record_hit(
    data: &mut RecordingData,
    timestamp: f64,
    timing: &ScoreTiming,
    score: &Score,
) -> Option<f64> {
    let total_ticks = timing.total_loop_ticks();
    if total_ticks == 0 || timing.total_loop_seconds() <= 0.0 {
        return None;
    }
    let elapsed = timestamp - data.start_time;
    if elapsed < 0.0 {
        return None;
    }
    let total = total_ticks as f64;
    let hit_global = timing.seconds_to_global_tick(elapsed);

    // One pass over the score's onsets serves the match and the gap to the
    // neighbouring notes.
    let beats = compute_global_beat_onsets(score, timing);
    let (beat_idx, diff_ticks, raw_diff) = best_match(hit_global, total, &beats)?;
    let global_onset = beats[beat_idx].onset_tick;
    let onset_m_idx = beats[beat_idx].measure_idx;

    let wrap_next = raw_diff > total * 0.5;
    let wrap_prev = raw_diff < -total * 0.5;
    let already_hit = if wrap_next {
        data.hits_next_loop.contains(&global_onset)
    } else {
        !wrap_prev && data.hits_in_loop.contains(&global_onset)
    };
    if already_hit {
        data.counters.extra += 1;
        return None;
    }

    // Convert tick delta to ms using the onset measure's local rate. Since
    // |diff_ticks| stays close to a single beat width, the choice of measure
    // has negligible impact for cross-measure hits.
    let tps = timing.ticks_per_sec_in_measure(onset_m_idx);
    let delta_ms = if tps > 0.0 { (diff_ticks / tps) * 1000.0 } else { 0.0 };

    // The gap to the closest neighbouring onset, as heard, narrows the
    // grading windows for fast notes.
    let gap_ticks = neighbour_gap_ticks(&beats, beat_idx, total);
    let gap_ms = if tps > 0.0 { gap_ticks / tps * 1000.0 } else { f64::INFINITY };
    let windows = HitWindows::for_gap_ms(gap_ms);
    let grade = windows.grade(delta_ms);

    data.stats.push(delta_ms);
    data.counters.push_hit(grade);
    data.marks_by_onset.insert(global_onset, AccuracyMark::Hit { diff_ticks, grade });
    if wrap_next {
        data.hits_next_loop.insert(global_onset);
    } else if !wrap_prev {
        data.hits_in_loop.insert(global_onset);
    }
    info!(
        "Accuracy hit: global_onset={} hit_global={:.2} delta_ms={:+.2} bpm={}",
        global_onset,
        hit_global,
        delta_ms,
        timing.bpm()
    );
    Some(delta_ms)
}

/// Find the note onset closest to `hit_global` on the score-loop circle,
/// comparing against where each note is heard (swing).
///
/// Returns `(index into beats, signed_diff_on_loop, raw_diff)`.
fn best_match(hit_global: f64, total: f64, beats: &[GlobalBeatOnset]) -> Option<(usize, f64, f64)> {
    let mut best: Option<(usize, f64, f64)> = None;
    for (idx, beat) in beats.iter().enumerate() {
        if !beat.is_note {
            continue;
        }
        let raw_diff = hit_global - beat.performed_tick as f64;
        let mut diff = raw_diff;
        if diff > total * 0.5 {
            diff -= total;
        } else if diff < -total * 0.5 {
            diff += total;
        }
        if best.is_none_or(|(_, best_diff, _)| diff.abs() < best_diff.abs()) {
            best = Some((idx, diff, raw_diff));
        }
    }
    best
}
