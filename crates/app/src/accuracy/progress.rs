use grooph_measure::Score;
use grooph_measure::tempo::ScoreTiming;

use super::session::{
    AccuracyMark, BarEvent, GlobalBeatOnset, RecordingData, compute_global_beat_onsets,
    loop_offset, tallies_of,
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
    };
    if !wrapped {
        process_segment(data, &global_beats, last_tick, current_tick, 0.0, total, &cursor);
    } else {
        // Score-loop wrapped.
        process_segment(data, &global_beats, last_tick, total, 0.0, total, &cursor);
        data.hits_in_loop = std::mem::take(&mut data.hits_next_loop);
        process_segment(data, &global_beats, 0.0, current_tick, total, total, &cursor);
    }

    data.last_tick = Some(current_tick);
}

/// The progress cursor this frame: its time and its tick.
struct Cursor<'a> {
    seconds: f64,
    tick: f64,
    timing: &'a ScoreTiming,
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
    for (i, gb) in global_beats.iter().enumerate() {
        if !gb.is_note {
            continue;
        }
        let cur = gb.performed_tick as f64;
        let next = if i + 1 < global_beats.len() {
            global_beats[i + 1].performed_tick as f64
        } else {
            global_beats[0].performed_tick as f64 + total_loop_ticks
        };
        let mut window_end = cur + (next - cur) * 0.5;
        if window_end < segment_offset {
            window_end += total_loop_ticks;
        }
        let in_range = window_end > seg_start && window_end <= seg_end;
        if in_range {
            if !data.hits_in_loop.contains(&gb.onset_tick) {
                data.marks_by_onset.insert(gb.onset_tick, AccuracyMark::Miss);
                data.counters.misses += 1;
                tallies_of(&mut data.voice_tallies, gb.voices).for_each(|t| t.counters.misses += 1);
                let note_tick = window_end - (next - cur) * 0.5;
                data.bars.push(cursor.seconds_at(note_tick, gb.measure_idx), BarEvent::Miss);
            }
            data.hits_in_loop.remove(&gb.onset_tick);
        }
    }
}
