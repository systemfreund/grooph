use grooph_measure::tempo::ScoreTiming;
use grooph_measure::{Score, StaffKind};

use super::session::{
    AccuracyMark, BarEvent, GlobalBeatOnset, RecordingData, catch_window_ticks, global_beat_onsets,
    loop_offset, note_window, tallies_of,
};

/// Advance the progress cursor to `delay_s` before `now_seconds` and mark
/// any note onsets whose hit window has elapsed as `Miss`. Wraps detected
/// when the global tick decreases across a frame (= score loop just
/// completed).
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

    let mut current_tick = timing.seconds_to_global_tick(elapsed);
    // seconds_to_global_tick already rem_euclids, but be defensive.
    if current_tick >= total {
        current_tick %= total;
    }
    let Some(last_tick) = data.last_tick else {
        data.last_tick = Some(current_tick);
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

    // Where the cursor is, on the ticks of the segments: past the loop's
    // end after a wrap. Misses are dated back from it.
    let wrapped = current_tick < last_tick;
    let cursor = Cursor {
        seconds: now_seconds,
        tick: current_tick + if wrapped { total } else { 0.0 },
        timing,
        staff: score.staff,
    };
    if !wrapped {
        process_segment(data, &global_beats, last_tick, current_tick, 0.0, total, &cursor);
    } else {
        // Score-loop wrapped.
        process_segment(data, &global_beats, last_tick, total, 0.0, total, &cursor);
        // The loop just ended keeps its hits for the windows that run on
        // past its end (on a kit, an instrument's last note before its
        // first one in the bar).
        let next = std::mem::take(&mut data.hits_next_loop);
        data.hits_prev_loop = std::mem::replace(&mut data.hits_in_loop, next);
        data.loops += 1;
        process_segment(data, &global_beats, 0.0, current_tick, total, total, &cursor);
    }

    data.last_tick = Some(current_tick);
}

/// The progress cursor this frame: its time and its tick, on the score
/// played.
struct Cursor<'a> {
    seconds: f64,
    tick: f64,
    timing: &'a ScoreTiming,
    /// The score's staff, which sets its notes' catch window.
    staff: StaffKind,
}

impl Cursor<'_> {
    /// When the note at `tick` (on the cursor's ticks) was due.
    fn seconds_at(&self, tick: f64, measure_idx: usize) -> f64 {
        let tps = self.timing.ticks_per_sec_in_measure(measure_idx);
        if tps > 0.0 { self.seconds - (self.tick - tick) / tps } else { self.seconds }
    }
}

/// Mark the notes whose hit window closed between `start` and `end` (plus
/// `segment_offset`) and that got no hit as missed.
fn process_segment(
    data: &mut RecordingData,
    global_beats: &[GlobalBeatOnset],
    start: f64,
    end: f64,
    segment_offset: f64,
    total_loop_ticks: f64,
    cursor: &Cursor,
) {
    if global_beats.is_empty() {
        return;
    }
    let seg_start = segment_offset + start;
    let seg_end = segment_offset + end;
    // The extra hits of an earlier loop go once the cursor passes their
    // place again, as a note's mark changes when it is judged again.
    let loops = data.loops;
    data.extras.retain(|e| {
        let at = e.performed_tick.rem_euclid(total_loop_ticks) + segment_offset;
        e.loop_idx >= loops || !(at > seg_start && at <= seg_end)
    });
    for (i, gb) in global_beats.iter().enumerate() {
        if !gb.is_note {
            continue;
        }
        // The note's window (see `note_window`): on one line it ends halfway
        // to the next onset; on a kit with its catch window, at most halfway
        // to the instrument's next head.
        let cur = gb.performed_tick as f64;
        let catch = catch_window_ticks(cursor.staff, cursor.timing, gb.measure_idx);
        let (_, after) = note_window(global_beats, i, total_loop_ticks, catch);
        // A window ending past the loop's end (on a kit, an instrument's
        // last note before its first one in the bar; on one line none, as
        // every bar has an onset at its start) closes in the loop after its
        // note's: it is judged there, against the hits of the loop before,
        // and not in the first loop, which has none before it.
        let of_prev_loop = cur + after > total_loop_ticks;
        if of_prev_loop && data.loops == 0 {
            continue;
        }
        let window_end =
            cur + after + segment_offset - if of_prev_loop { total_loop_ticks } else { 0.0 };
        let in_range = window_end > seg_start && window_end <= seg_end;
        if in_range {
            let hit = if of_prev_loop {
                data.hits_prev_loop.remove(&gb.key())
            } else {
                data.hits_in_loop.remove(&gb.key())
            };
            if !hit {
                data.marks_by_onset.insert(gb.key(), AccuracyMark::Miss);
                data.counters.misses += 1;
                tallies_of(&mut data.voice_tallies, gb.voices).for_each(|t| t.counters.misses += 1);
                let note_tick = window_end - after;
                let note_s = cursor.seconds_at(note_tick, gb.measure_idx);
                data.bars.push(note_s, BarEvent::Miss);
                data.drift.push_miss(note_s);
                if let Some(pos) = gb.position {
                    data.positions.push_miss(pos);
                }
            }
        }
    }
}
