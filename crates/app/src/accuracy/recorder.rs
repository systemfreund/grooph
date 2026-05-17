use grooph_measure::Score;
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::tempo::ScoreTiming;
use grooph_measure::{BeatKind, MeasureIdx};
use log::info;

use super::session::{AccuracyMark, RecordingData};

/// Record an incoming MIDI hit at `timestamp` (seconds, MIDI clock). The hit
/// is mapped to a global tick via `timing`, then matched against the nearest
/// note onset across the whole score (shortest signed distance on the loop).
/// The delta is converted to milliseconds using the onset measure's tempo.
pub(super) fn record_hit(
    data: &mut RecordingData,
    timestamp: f64,
    timing: &ScoreTiming,
    score: &Score,
) {
    let total_ticks = timing.total_loop_ticks();
    if total_ticks == 0 || timing.total_loop_seconds() <= 0.0 {
        return;
    }
    let elapsed = timestamp - data.start_time;
    if elapsed < 0.0 {
        return;
    }
    let total = total_ticks as f64;
    let hit_global = timing.seconds_to_global_tick(elapsed);

    let Some((global_onset, onset_m_idx, diff_ticks, raw_diff)) =
        best_match(hit_global, total, score, timing)
    else {
        return;
    };

    let wrap_next = raw_diff > total * 0.5;
    let wrap_prev = raw_diff < -total * 0.5;
    if wrap_next {
        if data.hits_next_loop.contains(&global_onset) {
            return;
        }
    } else if !wrap_prev && data.hits_in_loop.contains(&global_onset) {
        return;
    }

    // Convert tick delta to ms using the onset measure's local rate. Since
    // |diff_ticks| stays close to a single beat width, the choice of measure
    // has negligible impact for cross-measure hits.
    let tps = timing.ticks_per_sec_in_measure(onset_m_idx);
    let delta_ms = if tps > 0.0 { (diff_ticks / tps) * 1000.0 } else { 0.0 };

    data.stats.push(delta_ms);
    data.marks_by_onset.insert(global_onset, AccuracyMark::Hit(diff_ticks));
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
}

/// Walk every note beat in every measure and find the closest onset to
/// `hit_global` on the score-loop circle.
///
/// Returns `(global_onset, measure_idx, signed_diff_on_loop, raw_diff)`.
fn best_match(
    hit_global: f64,
    total: f64,
    score: &Score,
    timing: &ScoreTiming,
) -> Option<(u64, MeasureIdx, f64, f64)> {
    let mut best: Option<(u64, MeasureIdx, f64, f64)> = None;
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
    best
}
