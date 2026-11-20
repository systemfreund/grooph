use grooph_measure::tempo::ScoreTiming;
use grooph_measure::{Instrument, Score, StaffKind};
use log::info;

use super::session::{
    AccuracyMark, BarEvent, ExtraMark, GlobalBeatOnset, GradedHit, HitWindows, KIT_CATCH_WINDOW_MS,
    RecordingData, global_beat_onsets, loop_offset, neighbour_gap_ticks, neighbour_gaps_ticks,
    tallies_of,
};

/// Record an incoming hit at `timestamp` (seconds, on the shared clock). The
/// hit is mapped to a global tick via `timing`, then matched against the
/// nearest note onset across the whole score (shortest signed distance on
/// the loop), taking swing into account.
/// The delta is converted to milliseconds using the onset measure's tempo.
///
/// On a kit score the hit is on `instrument` and only matched against its
/// heads, within the catch window ([`KIT_CATCH_WINDOW_MS`], at most half the
/// gap to the instrument's neighbouring note); a hit without one there is
/// extra. A hit without an instrument is ignored there. On one line
/// `instrument` plays no part.
///
/// Returns the delta in ms when the hit was matched to a note; a second hit
/// on an already hit note counts as extra and returns `None`.
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

    // One pass over the score's onsets serves the match and the gap to the
    // neighbouring notes.
    let beats = global_beat_onsets(score, timing);
    let matched = best_match(hit_global, total, &beats, instrument).filter(|&(idx, diff, _)| {
        instrument.is_none() || in_catch_window(&beats, idx, diff, total, timing)
    });
    let Some((beat_idx, diff_ticks, raw_diff)) = matched else {
        // A kit's hit with no note of its instrument near: extra, drawn
        // from the note of any instrument nearest to it. (On one line only
        // a score without notes has no match.)
        if let Some(instrument) = instrument {
            data.counters.extra += 1;
            data.bars.push(timestamp, BarEvent::Extra);
            if let Some(anchor) = best_match(hit_global, total, &beats, None) {
                push_extra(data, &beats, anchor, instrument, hit_global, total);
            }
        }
        return None;
    };
    let global_onset = beats[beat_idx].key();
    let onset_m_idx = beats[beat_idx].measure_idx;
    let voices = beats[beat_idx].voices;
    let position = beats[beat_idx].position;

    // The note's loop relative to the progress cursor's, whose sets track
    // the hits: the hit may lie across the loop's end from the note, and
    // the cursor (judging misses with a delay) across it from the hit.
    let hit_loop = data.last_tick.map_or(0, |cursor| loop_offset(cursor - hit_global, total));
    let note_loop = hit_loop + loop_offset(raw_diff, total);
    let wrap_next = note_loop > 0;
    let wrap_prev = note_loop < 0;
    let loop_hits = if wrap_next {
        &mut data.hits_next_loop
    } else if wrap_prev {
        &mut data.hits_prev_loop
    } else {
        &mut data.hits_in_loop
    };
    let already_hit = !loop_hits.insert(global_onset);
    // Convert tick delta to ms using the onset measure's local rate. Since
    // |diff_ticks| stays close to a single beat width, the choice of measure
    // has negligible impact for cross-measure hits.
    let tps = timing.ticks_per_sec_in_measure(onset_m_idx);
    let delta_ms = if tps > 0.0 { (diff_ticks / tps) * 1000.0 } else { 0.0 };
    // When the note was due, for the bar it is in.
    let note_s = timestamp - delta_ms / 1000.0;

    if already_hit {
        if let Some(instrument) = instrument {
            push_extra(
                data,
                &beats,
                (beat_idx, diff_ticks, raw_diff),
                instrument,
                hit_global,
                total,
            );
        }
        data.counters.extra += 1;
        tallies_of(&mut data.voice_tallies, voices).for_each(|t| t.counters.extra += 1);
        data.bars.push(note_s, BarEvent::Extra);
        return None;
    }

    // The gap to the closest neighbouring onset, as heard, narrows the
    // grading windows for fast notes.
    let gap_ticks = neighbour_gap_ticks(&beats, beat_idx, total);
    let gap_ms = if tps > 0.0 { gap_ticks / tps * 1000.0 } else { f64::INFINITY };
    let windows = HitWindows::for_gap_ms(gap_ms);
    let grade = windows.grade(delta_ms);

    // Each run and voice grades its hits again around its new average.
    let graded = GradedHit { delta_ms, windows };
    data.stats.push(delta_ms);
    data.counters.push_hit(grade);
    data.tight.push(graded, data.stats.mean_ms().unwrap_or(0.0));
    for tally in tallies_of(&mut data.voice_tallies, voices) {
        tally.push_hit(graded, grade);
    }
    data.bars.push(note_s, BarEvent::Hit(graded, grade));
    let bar_s = if tps > 0.0 { timing.ticks_per_measure(onset_m_idx) as f64 / tps } else { 0.0 };
    data.drift.push_hit(note_s, delta_ms, grade, bar_s);
    if let Some(pos) = position {
        data.positions.push_hit(pos, delta_ms);
    }
    data.marks_by_onset.insert(global_onset, AccuracyMark::Hit { diff_ticks, grade });
    info!(
        "Accuracy hit: global_onset={:?} hit_global={:.2} delta_ms={:+.2} bpm={}",
        global_onset,
        hit_global,
        delta_ms,
        timing.bpm()
    );
    Some(delta_ms)
}

/// Keep a kit's extra hit on `instrument` at `hit_global` for its mark,
/// drawn from the note `anchor` (as [`best_match`] gives it).
fn push_extra(
    data: &mut RecordingData,
    beats: &[GlobalBeatOnset],
    (idx, diff_ticks, raw_diff): (usize, f64, f64),
    instrument: Instrument,
    hit_global: f64,
    total: f64,
) {
    // The note's loop, as for a hit: relative to the progress cursor.
    let hit_loop = data.last_tick.map_or(0, |cursor| loop_offset(cursor - hit_global, total));
    let note_loop = hit_loop + loop_offset(raw_diff, total);
    data.extras.push(ExtraMark {
        anchor_tick: beats[idx].onset_tick,
        measure_idx: beats[idx].measure_idx,
        diff_ticks,
        instrument,
        performed_tick: hit_global,
        loop_idx: data.loops + i64::from(note_loop),
    });
}

/// Whether a hit `diff_ticks` from the kit's note at `idx` is in its catch
/// window: at most [`KIT_CATCH_WINDOW_MS`] away, and at most half the gap
/// to the same instrument's neighbouring note on its side.
fn in_catch_window(
    beats: &[GlobalBeatOnset],
    idx: usize,
    diff_ticks: f64,
    total: f64,
    timing: &ScoreTiming,
) -> bool {
    let tps = timing.ticks_per_sec_in_measure(beats[idx].measure_idx);
    if tps <= 0.0 {
        return false;
    }
    let (before, after) = neighbour_gaps_ticks(beats, idx, total);
    let gap = if diff_ticks < 0.0 { before } else { after };
    let limit = (KIT_CATCH_WINDOW_MS / 1000.0 * tps).min(gap * 0.5);
    diff_ticks.abs() <= limit
}

/// Find the note onset closest to `hit_global` on the score-loop circle,
/// comparing against where each note is heard (swing). With `instrument`,
/// only its heads.
///
/// Returns `(index into beats, signed_diff_on_loop, raw_diff)`.
fn best_match(
    hit_global: f64,
    total: f64,
    beats: &[GlobalBeatOnset],
    instrument: Option<Instrument>,
) -> Option<(usize, f64, f64)> {
    let mut best: Option<(usize, f64, f64)> = None;
    for (idx, beat) in beats.iter().enumerate() {
        if !beat.is_note || instrument.is_some_and(|i| i != beat.instrument) {
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
