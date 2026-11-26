use std::rc::Rc;

use grooph_measure::tempo::ScoreTiming;
use grooph_measure::{Instrument, Score, StaffKind};
use log::info;

use super::align::{Decision, Lane, LaneState, PendingHit};
use super::session::{
    AccuracyMark, BarEvent, ExtraMark, GlobalBeatOnset, GradedHit, HitWindows, Played,
    RecordingData, catch_window_ticks, global_beat_onsets, loop_offset, tallies_of,
};

/// Take in a hit at `timestamp` (seconds, on the shared clock). The hit is
/// mapped to a global tick via `timing` and kept with the open stretch of
/// its instrument's notes (see [`super::align`]); which note it counts for,
/// if any, is decided once that note is final ([`finalise`]). A hit no
/// note still open can take is extra at once: on a kit one with no note of
/// its instrument within the catch window
/// ([`KIT_CATCH_WINDOW_MS`](super::session::KIT_CATCH_WINDOW_MS)).
///
/// On a kit score the hit is on `instrument` and only aligned with its
/// heads; a hit without an instrument is ignored there. On one line
/// `instrument` plays no part.
///
/// Returns the hit's deviation in ms from the nearest note (of its
/// instrument; on a kit within the catch window), at once and whatever the
/// alignment makes of it: the calibration's measure.
pub(super) fn record_hit(
    data: &mut RecordingData,
    timestamp: f64,
    instrument: Option<Instrument>,
    timing: &ScoreTiming,
    score: &Score,
) -> Option<f64> {
    let instrument = match score.staff {
        StaffKind::Kit => Some(instrument?),
        StaffKind::Line => None,
    };
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
    let beats = global_beat_onsets(score, timing);
    remember_played(data, &beats, timing, score.staff);

    // The calibration's delta: to the nearest note, now.
    let delta_ms = best_match(hit_global, total, &beats, instrument).and_then(|(idx, diff)| {
        let measure_idx = beats[idx].measure_idx;
        let catch = catch_window_ticks(score.staff, timing, measure_idx);
        let tps = timing.ticks_per_sec_in_measure(measure_idx);
        (diff.abs() <= catch).then(|| if tps > 0.0 { diff / tps * 1000.0 } else { 0.0 })
    });

    // The hit's loop relative to the progress cursor's, which may lie
    // across the loop's end from it (judging with a delay).
    let loop_idx = data.loops
        + data.last_tick.map_or(0, |cursor| i64::from(loop_offset(cursor - hit_global, total)));
    let hit = PendingHit { loop_idx, tick: hit_global, timestamp };
    let played = instrument.unwrap_or(Instrument::Rhythm);
    let lane = Lane::new(&beats, played, total, timing, score.staff);
    if lane.is_empty() {
        // A kit's instrument with no note in the score.
        push_extra(data, &beats, total, hit, played, None);
        return delta_ms;
    }
    let x = lane.at(loop_idx, hit_global);
    // Without a cursor yet, the lanes start at the hit's note.
    data.origin.get_or_insert_with(|| {
        let n = lane.at_or_before(x);
        (lane.loop_of(n), lane.beats[lane.beat(n)].performed_tick as f64)
    });
    let mut state = data.lanes.remove(&played).unwrap_or_default();
    let open = frontier(&mut state, &lane, data.origin);
    let crosses = state.floor.is_some_and(|(l, t)| x <= lane.at(l, t));
    if crosses || !open.is_some_and(|n| lane.has_candidate(n, x)) {
        push_extra(data, &beats, total, hit, played, Some(&lane));
    } else {
        let at = state.hits.partition_point(|h| lane.at(h.loop_idx, h.tick) <= x);
        state.hits.insert(at, hit);
    }
    data.lanes.insert(played, state);
    delta_ms
}

/// The first open note of a lane, starting it at `origin` if it has not
/// started yet. `None` while there is no origin.
fn frontier(state: &mut LaneState, lane: &Lane, origin: Option<(i64, f64)>) -> Option<i64> {
    let n = match state.frontier {
        Some((loop_idx, tick)) => lane.first_from(loop_idx, tick),
        None => {
            let (loop_idx, tick) = origin?;
            lane.first_heard_from(lane.at(loop_idx, tick))
        }
    };
    state.frontier = Some(lane.key_of(n));
    Some(n)
}

/// Keep the score being played for [`finalise`] at the run's stop.
pub(super) fn remember_played(
    data: &mut RecordingData,
    beats: &Rc<[GlobalBeatOnset]>,
    timing: &ScoreTiming,
    staff: StaffKind,
) {
    if data.played.as_ref().is_some_and(|p| Rc::ptr_eq(&p.beats, beats) && p.staff == staff) {
        return;
    }
    data.played = Some(Played { beats: Rc::clone(beats), timing: timing.clone(), staff });
}

/// Make final what the progress cursor has passed: each lane's notes up to
/// where their lookahead ([`Lane::final_at`]) lies behind the cursor, with
/// the hits the alignment pairs them with, and the hits that no open note
/// can take any more as extra. With `stop`, the run ends: the whole open
/// stretch is aligned with the hits in, and a note whose window is still
/// open without a hit is no miss.
pub(super) fn finalise(data: &mut RecordingData, stop: bool) {
    let Some(played) = data.played.take() else {
        return;
    };
    let total = played.timing.total_loop_ticks() as f64;
    // Without a cursor (the marks were just cleared), only a stop aligns,
    // every window still open.
    let cursor = match data.last_tick {
        Some(tick) => Some(data.loops as f64 * total + tick),
        None => stop.then_some(f64::NEG_INFINITY),
    };
    if let Some(cursor) = cursor
        && total > 0.0
    {
        let mut instruments: Vec<Instrument> =
            played.beats.iter().filter(|b| b.is_note).map(|b| b.instrument).collect();
        instruments.sort();
        instruments.dedup();
        for instrument in instruments {
            let lane = Lane::new(&played.beats, instrument, total, &played.timing, played.staff);
            let mut state = data.lanes.remove(&instrument).unwrap_or_default();
            finalise_lane(data, &mut state, &lane, cursor, stop);
            data.lanes.insert(instrument, state);
        }
    }
    data.played = Some(played);
}

fn finalise_lane(
    data: &mut RecordingData,
    state: &mut LaneState,
    lane: &Lane,
    cursor: f64,
    stop: bool,
) {
    let instrument = lane.beats[lane.idx[0]].instrument;
    let Some(mut n) = frontier(state, lane, data.origin) else {
        return;
    };
    loop {
        let (_, after) = lane.window(n);
        let closed = lane.tick(n) + after < cursor;
        if stop {
            if state.hits.is_empty() && !closed {
                break;
            }
        } else if cursor < lane.final_at(n) {
            break;
        }
        let xs: Vec<f64> = state.hits.iter().map(|h| lane.at(h.loop_idx, h.tick)).collect();
        match lane.decide(n, &xs) {
            Decision::Hit(i) => {
                // The hits before it cannot go to a later note: extra.
                let before: Vec<PendingHit> = state.hits.drain(..i).collect();
                for h in before {
                    push_extra(data, lane.beats, lane.total, h, instrument, Some(lane));
                }
                let hit = state.hits.remove(0);
                push_hit(data, lane, n, hit, xs[i]);
                state.floor = Some((hit.loop_idx, hit.tick));
            }
            Decision::Miss if closed || !stop => push_miss(data, lane, n, cursor),
            Decision::Miss => {}
        }
        n += 1;
        state.frontier = Some(lane.key_of(n));
        // The hits no open note can take any more.
        let (keep, gone): (Vec<PendingHit>, Vec<PendingHit>) =
            state.hits.iter().partition(|h| lane.has_candidate(n, lane.at(h.loop_idx, h.tick)));
        state.hits = keep;
        for h in gone {
            push_extra(data, lane.beats, lane.total, h, instrument, Some(lane));
        }
    }
}

/// Record the final pair of occurrence `n` of `lane` and `hit`, at `x` on
/// the lane's timeline.
fn push_hit(data: &mut RecordingData, lane: &Lane, n: i64, hit: PendingHit, x: f64) {
    let beat = &lane.beats[lane.beat(n)];
    let diff_ticks = x - lane.tick(n);
    // Convert tick delta to ms using the onset measure's local rate. Since
    // |diff_ticks| stays within a gap, the choice of measure has negligible
    // impact for cross-measure hits.
    let tps = lane.tps(n);
    let delta_ms = if tps > 0.0 { (diff_ticks / tps) * 1000.0 } else { 0.0 };
    // When the note was due, for the bar it is in.
    let note_s = hit.timestamp - delta_ms / 1000.0;

    // The gap to the closest neighbouring onset, as heard, narrows the
    // grading windows for fast notes.
    let (before, after) = lane.gaps(n);
    let gap_ms = if tps > 0.0 { before.min(after) / tps * 1000.0 } else { f64::INFINITY };
    let windows = HitWindows::for_gap_ms(gap_ms);
    let grade = windows.grade(delta_ms);

    // Each run and voice grades its hits again around its new average.
    let graded = GradedHit { delta_ms, windows };
    data.stats.push(delta_ms);
    data.counters.push_hit(grade);
    data.tight.push(graded, data.stats.mean_ms().unwrap_or(0.0));
    for tally in tallies_of(&mut data.voice_tallies, beat.voices) {
        tally.push_hit(graded, grade);
    }
    data.bars.push(note_s, BarEvent::Hit(graded, grade));
    let bar_s =
        if tps > 0.0 { lane.timing.ticks_per_measure(beat.measure_idx) as f64 / tps } else { 0.0 };
    data.drift.push_hit(note_s, delta_ms, grade, bar_s);
    if let Some(pos) = beat.position {
        data.positions.push_hit(pos, delta_ms);
    }
    data.marks_by_onset.insert(beat.key(), AccuracyMark::Hit { diff_ticks, grade });
    info!(
        "Accuracy hit: note={:?} hit_global={:.2} delta_ms={:+.2} bpm={}",
        beat.key(),
        hit.tick,
        delta_ms,
        lane.timing.bpm()
    );
}

/// Record occurrence `n` of `lane` as missed, the progress cursor at
/// `cursor` on the lane's timeline.
fn push_miss(data: &mut RecordingData, lane: &Lane, n: i64, cursor: f64) {
    let beat = &lane.beats[lane.beat(n)];
    data.marks_by_onset.insert(beat.key(), AccuracyMark::Miss);
    data.counters.misses += 1;
    tallies_of(&mut data.voice_tallies, beat.voices).for_each(|t| t.counters.misses += 1);
    let tps = lane.tps(n);
    let note_s =
        if tps > 0.0 { data.cursor_s - (cursor - lane.tick(n)) / tps } else { data.cursor_s };
    data.bars.push(note_s, BarEvent::Miss);
    data.drift.push_miss(note_s);
    if let Some(pos) = beat.position {
        data.positions.push_miss(pos);
    }
}

/// Record `hit` on `instrument` ([`Instrument::Rhythm`] on one line) as
/// extra. It counts against the voices of the note of its `lane` nearest
/// to it, if that note's window holds it, and is drawn from the note of
/// any instrument nearest to it.
fn push_extra(
    data: &mut RecordingData,
    beats: &[GlobalBeatOnset],
    total: f64,
    hit: PendingHit,
    instrument: Instrument,
    lane: Option<&Lane>,
) {
    data.counters.extra += 1;
    data.bars.push(hit.timestamp, BarEvent::Extra);
    if let Some(lane) = lane {
        let x = lane.at(hit.loop_idx, hit.tick);
        let n = lane.at_or_before(x);
        let nearest = if x - lane.tick(n) <= lane.tick(n + 1) - x { n } else { n + 1 };
        if lane.holds(nearest, x) {
            let voices = beats[lane.beat(nearest)].voices;
            tallies_of(&mut data.voice_tallies, voices).for_each(|t| t.counters.extra += 1);
        }
    }
    if let Some((idx, diff_ticks)) = best_match(hit.tick, total, beats, None) {
        data.extras.push(ExtraMark {
            anchor_tick: beats[idx].onset_tick,
            measure_idx: beats[idx].measure_idx,
            diff_ticks,
            instrument,
            performed_tick: hit.tick,
            loop_idx: hit.loop_idx,
        });
    }
}

/// Find the note onset closest to `hit_global` on the score-loop circle,
/// comparing against where each note is heard (swing). With `instrument`,
/// only its heads.
///
/// Returns `(index into beats, signed_diff_on_loop)`.
fn best_match(
    hit_global: f64,
    total: f64,
    beats: &[GlobalBeatOnset],
    instrument: Option<Instrument>,
) -> Option<(usize, f64)> {
    let mut best: Option<(usize, f64)> = None;
    for (idx, beat) in beats.iter().enumerate() {
        if !beat.is_note || instrument.is_some_and(|i| i != beat.instrument) {
            continue;
        }
        let mut diff = hit_global - beat.performed_tick as f64;
        if diff > total * 0.5 {
            diff -= total;
        } else if diff < -total * 0.5 {
            diff += total;
        }
        if best.is_none_or(|(_, best_diff)| diff.abs() < best_diff.abs()) {
            best = Some((idx, diff));
        }
    }
    best
}
