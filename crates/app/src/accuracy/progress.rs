use grooph_measure::Score;
use grooph_measure::tempo::ScoreTiming;

use super::recorder::{Until, finalise, remember_played};
use super::session::{RecordingData, global_beat_onsets, loop_offset};

/// Advance the progress cursor to `delay_s` before `now_seconds` and make
/// final what it has passed (see [`finalise`]): the notes whose lookahead
/// lies behind it, as hit or missed, and their hits. Wraps detected when
/// the global tick decreases across a frame (= score loop just completed).
pub(super) fn update_progress(
    data: &mut RecordingData,
    now_seconds: f64,
    delay_s: f64,
    timing: &ScoreTiming,
    score: &Score,
) {
    // A longer delay than before moves the cursor's time back, as anchoring
    // the clock does.
    if delay_s > data.miss_delay_s {
        data.anchored_ahead = true;
    }
    data.miss_delay_s = delay_s;
    let now_seconds = now_seconds - delay_s;
    let total_ticks = timing.total_loop_ticks();
    if total_ticks == 0 || timing.total_loop_seconds() <= 0.0 {
        return;
    }
    let elapsed = now_seconds - data.start_time;
    if elapsed < 0.0 {
        return;
    }
    let total = total_ticks as f64;
    let global_beats = global_beat_onsets(score, timing);
    if global_beats.is_empty() {
        return;
    }
    remember_played(data, &global_beats, timing, score.staff);

    let mut current_tick = timing.seconds_to_global_tick(elapsed);
    // seconds_to_global_tick already rem_euclids, but be defensive.
    if current_tick >= total {
        current_tick %= total;
    }
    let Some(last_tick) = data.last_tick else {
        data.last_tick = Some(current_tick);
        data.cursor_s = now_seconds;
        data.origin.get_or_insert((data.loops, current_tick));
        return;
    };
    // Right after anchoring the clock, a step back by less than half the
    // loop is no wrap: the anchor is ahead of the delayed time misses are
    // judged at. Wait until that gets there. Holds while the delay stays
    // below half the loop (`miss_delay_s` in play_along.rs keeps it to a
    // quarter). Otherwise a step back is a wrap, also after a long stall.
    if data.anchored_ahead {
        if current_tick < last_tick && loop_offset(current_tick - last_tick, total) == 0 {
            return;
        }
        data.anchored_ahead = false;
    }
    let epsilon = total.max(1.0) * 1e-9;
    if current_tick + epsilon >= last_tick && current_tick < last_tick {
        current_tick = last_tick;
    }

    if current_tick < last_tick {
        // Score-loop wrapped.
        retain_extras(data, last_tick, total, total);
        data.loops += 1;
        retain_extras(data, 0.0, current_tick, total);
    } else {
        retain_extras(data, last_tick, current_tick, total);
    }
    data.last_tick = Some(current_tick);
    data.cursor_s = now_seconds;
    finalise(data, Until::Cursor);
}

/// The extra hits of an earlier loop go once the cursor passes their place
/// again (from `start` to `end` on the loop), as a note's mark changes when
/// it is judged again.
fn retain_extras(data: &mut RecordingData, start: f64, end: f64, total: f64) {
    let loops = data.loops;
    data.extras.retain(|e| {
        let at = e.performed_tick.rem_euclid(total);
        e.loop_idx >= loops || !(at > start && at <= end)
    });
}
