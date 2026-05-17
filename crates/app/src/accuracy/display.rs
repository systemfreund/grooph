/// Clamp a hit's `diff_ticks` to the half-distance to its neighbouring onsets,
/// so the visual marker stays inside the beat's hit window. Operates on
/// score-global beat onsets.
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
