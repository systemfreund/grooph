//! The playback cursor's clock: advances the cursor between frames, keeps it
//! in step with the audio engine and tracks the count-in. Pure functions of
//! the previous state, so they can be tested without a UI or audio device.

use crate::state::{COUNT_IN_WAIT_TIMEOUT_S, CountInState};
use grooph_measure::BarRange;
use grooph_measure::tempo::ScoreTiming;

/// Audio engine position: global tick (negative during the count-in) and the
/// loop length in ticks.
pub(crate) type AudioPos = (f64, u64);

/// Share of the distance to the audio position the cursor closes per frame.
const AUDIO_NUDGE: f64 = 0.1;

/// Cursor position (global ticks) `dt` seconds after `tick`: moves at the
/// playing measure's rate, then follows the audio position (shifted back by
/// `latency_s`), snapping when more than half a beat off and nudging
/// otherwise. Held on the loop's first downbeat while `counting_in`; at a
/// barline it goes on into the next bar of the loop, as the audio engine
/// does (see [`next_bar_tick`]), so it never leaves the loop range and
/// does not jump when the range changes.
/// `next_loop` is the range the next bar comes from at a barline (the
/// document's, which the engine takes over there; see
/// [`crate::loop_range`]), `timing`'s otherwise.
pub(crate) fn advance_tick(
    tick: f64,
    dt: f64,
    audio_pos: Option<AudioPos>,
    latency_s: f64,
    counting_in: bool,
    timing: &ScoreTiming,
    next_loop: BarRange,
) -> f64 {
    if counting_in {
        // Hold the cursor on the first downbeat until the count-in bar is over.
        return loop_start(timing);
    }
    let current_m = timing.measure_at_global_tick(tick);
    let mut next = tick + timing.ticks_per_sec_in_measure(current_m) * dt;

    if let Some((raw_audio_tick, audio_total)) = audio_pos
        && audio_total > 0
    {
        let audio_total = audio_total as f64;
        // The latency offset uses the playing measure's rate (a sub-tick
        // approximation across time signature changes).
        let audio_m = timing.measure_at_global_tick(raw_audio_tick);
        let offset_ticks = latency_s * timing.ticks_per_sec_in_measure(audio_m);
        let audio_tick =
            heard_tick(raw_audio_tick, offset_ticks, audio_m, current_m, timing, next_loop);

        // Shortest way round the loop.
        let mut diff = audio_tick - next;
        if diff > audio_total * 0.5 {
            diff -= audio_total;
        } else if diff < -audio_total * 0.5 {
            diff += audio_total;
        }

        let half_beat = timing.ticks_per_beat_in_measure(audio_m) as f64 * 0.5;
        if diff.abs() > half_beat {
            return audio_tick;
        }
        next += diff * AUDIO_NUDGE;
    }
    next_bar_tick(next, current_m, timing, next_loop)
}

/// Where the audio engine's position `delayed` (shifted back by the output
/// latency from inside measure `audio_m`) is heard, the cursor being in
/// `current_m`. Shifted back over the start of the engine's bar, it is
/// still in the bar heard before, which the cursor has not left yet: the
/// end of `current_m`. Once the cursor is in the engine's bar (or still
/// counting in), the bar before is the one before it round `next_loop`, the
/// engine's range (not `timing`'s, which may still hold the old one).
fn heard_tick(
    raw: f64,
    offset: f64,
    audio_m: usize,
    current_m: usize,
    timing: &ScoreTiming,
    next_loop: BarRange,
) -> f64 {
    let delayed = raw - offset;
    let bar_start = timing.measure_start_tick(audio_m) as f64;
    if delayed >= bar_start {
        return delayed;
    }
    // Counting in, the engine's position is negative: no bar was heard.
    if raw >= 0.0 && current_m != audio_m {
        let end = timing.measure_start_tick(current_m + 1) as f64;
        return end - (bar_start - delayed);
    }
    let start = timing.measure_start_tick(next_loop.first()) as f64;
    let len = timing.measure_start_tick(next_loop.last() + 1) as f64 - start;
    if len <= 0.0 { delayed } else { start + (delayed - start).rem_euclid(len) }
}

/// The loop's first tick (0 for an empty timing).
pub(crate) fn loop_start(timing: &ScoreTiming) -> f64 {
    if timing.measure_count() == 0 { 0.0 } else { timing.loop_start_tick() as f64 }
}

/// `tick`, reached from inside measure `from`, carried past `from`'s end
/// into the bar that plays next with `bars` looping (see
/// [`BarRange::next_bar`]).
pub(crate) fn next_bar_tick(tick: f64, from: usize, timing: &ScoreTiming, bars: BarRange) -> f64 {
    let n = timing.measure_count();
    if n == 0 {
        return tick;
    }
    let from = from.min(n - 1);
    let end = timing.measure_start_tick(from + 1) as f64;
    if tick < end {
        return tick;
    }
    let next = bars.next_bar(from, n);
    // Never past the next bar's end, also after a long frame.
    let into = (tick - end).min(timing.ticks_per_measure(next) as f64 - 1e-6).max(0.0);
    timing.measure_start_tick(next) as f64 + into
}

/// The count-in after this frame: `Waiting` until the audio engine reports a
/// negative position (or gives up after [`COUNT_IN_WAIT_TIMEOUT_S`]),
/// `Active` while it reports the count-in, `Heard` from when it left it
/// until its first downbeat is heard (the output latency later), then `Off`.
pub(crate) fn next_count_in(
    state: CountInState,
    now: f64,
    audio_pos: Option<AudioPos>,
    latency_s: f64,
    timing: &ScoreTiming,
) -> CountInState {
    // The count-in runs at the tempo of the loop's first bar.
    let first = timing.loop_bars().first();
    let tps =
        if timing.measure_count() > first { timing.ticks_per_sec_in_measure(first) } else { 0.0 };
    match state {
        CountInState::Off => CountInState::Off,
        CountInState::Waiting { since } => {
            let since = if since.is_nan() { now } else { since };
            match audio_pos {
                Some((tick, _)) if tick < 0.0 => CountInState::Active,
                _ if now - since > COUNT_IN_WAIT_TIMEOUT_S => CountInState::Off,
                _ => CountInState::Waiting { since },
            }
        }
        // Past the count-in the engine is at the loop's first downbeat, or a
        // little on from it: that is heard the latency after it played.
        CountInState::Active => match audio_pos {
            Some((tick, _)) if tick >= 0.0 => {
                let past = if tps > 0.0 { (tick - loop_start(timing)).max(0.0) / tps } else { 0.0 };
                heard_at(now - past + latency_s, now)
            }
            // Busy engine (None) or still counting in: stay.
            _ => CountInState::Active,
        },
        CountInState::Heard { at } => heard_at(at, now),
    }
}

/// `Heard` until `at`, then `Off`.
fn heard_at(at: f64, now: f64) -> CountInState {
    if now >= at { CountInState::Off } else { CountInState::Heard { at } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grooph_measure::{Measure, Score, TimeSignature};

    fn timing() -> ScoreTiming {
        let score = Score::new(vec![
            Measure::new(TimeSignature::FOUR_FOUR),
            Measure::new(TimeSignature::FOUR_FOUR),
        ]);
        ScoreTiming::from_score(&score, 120)
    }

    #[test]
    fn without_audio_the_cursor_runs_at_the_tempo() {
        let t = timing();
        let tps = t.ticks_per_sec_in_measure(0);
        assert_eq!(advance_tick(0.0, 0.5, None, 0.0, false, &t, t.loop_bars()), tps * 0.5);
    }

    #[test]
    fn the_cursor_wraps_at_the_end_of_the_score() {
        let t = timing();
        let total = t.total_loop_ticks() as f64;
        let tps = t.ticks_per_sec_in_measure(1);
        let next = advance_tick(total - 1.0, 1.0 / tps * 3.0, None, 0.0, false, &t, t.loop_bars());
        assert!((next - 2.0).abs() < 1e-6, "{next}");
    }

    #[test]
    fn the_cursor_nudges_toward_nearby_audio_and_snaps_to_far_audio() {
        let t = timing();
        let total = t.total_loop_ticks();
        let tpb = t.ticks_per_beat_in_measure(0) as f64;
        // A tenth of a beat ahead: close a tenth of the gap.
        let near = advance_tick(0.0, 0.0, Some((tpb * 0.1, total)), 0.0, false, &t, t.loop_bars());
        assert!((near - tpb * 0.01).abs() < 1e-6, "{near}");
        // A beat ahead: jump there.
        assert_eq!(advance_tick(0.0, 0.0, Some((tpb, total)), 0.0, false, &t, t.loop_bars()), tpb);
    }

    #[test]
    fn latency_shifts_the_audio_position_back() {
        let t = timing();
        let total = t.total_loop_ticks();
        let tps = t.ticks_per_sec_in_measure(0);
        let tpb = t.ticks_per_beat_in_measure(0) as f64;
        // Audio two beats in, half a second of latency (one beat at 120 BPM).
        let next = advance_tick(0.0, 0.0, Some((2.0 * tpb, total)), 0.5, false, &t, t.loop_bars());
        assert!((next - (2.0 * tpb - 0.5 * tps)).abs() < 1e-6, "{next}");
    }

    #[test]
    fn audio_just_past_the_loop_start_is_reached_the_short_way() {
        let t = timing();
        let total = t.total_loop_ticks();
        let tpb = t.ticks_per_beat_in_measure(0) as f64;
        let near_end = total as f64 - tpb * 0.1;
        let next =
            advance_tick(near_end, 0.0, Some((tpb * 0.1, total)), 0.0, false, &t, t.loop_bars());
        // Nudged forward across the wrap, not dragged back through the score.
        assert!(next > near_end, "{next}");
    }

    #[test]
    fn counting_in_holds_the_cursor() {
        let t = timing();
        assert_eq!(advance_tick(500.0, 0.1, Some((-10.0, 1)), 0.0, true, &t, t.loop_bars()), 0.0);
    }

    /// Four bars of 4/4 at 120 BPM with bars `a..=b` (from 1) looping.
    fn looping(a: usize, b: usize) -> ScoreTiming {
        looping_of(4, a, b)
    }

    /// `n` bars of 4/4 at 120 BPM with bars `a..=b` (from 1) looping.
    fn looping_of(n: usize, a: usize, b: usize) -> ScoreTiming {
        let score = Score::new(vec![Measure::new(TimeSignature::FOUR_FOUR); n]);
        ScoreTiming::from_score(&score, 120).with_loop(BarRange::new(a - 1, b - 1))
    }

    #[test]
    fn the_cursor_goes_round_the_loop_range() {
        let t = looping(2, 3);
        let tps = t.ticks_per_sec_in_measure(0);
        let start = t.measure_start_tick(1) as f64;
        assert_eq!(advance_tick(900.0, 0.1, None, 0.0, true, &t, t.loop_bars()), start);
        // From the end of bar 3 into bar 2, never bar 4.
        let end = t.measure_start_tick(3) as f64;
        let next = advance_tick(end - 1.0, 3.0 / tps, None, 0.0, false, &t, t.loop_bars());
        assert!((next - (start + 2.0)).abs() < 1e-6, "{next}");
        // Audio just past the loop's start, latency taking it back before it:
        // the end of the loop, not bar 1.
        let total = t.total_loop_ticks();
        let next = advance_tick(
            end - 10.0,
            0.0,
            Some((start + 1.0, total)),
            0.25,
            false,
            &t,
            t.loop_bars(),
        );
        assert!(next > t.measure_start_tick(2) as f64 && next < end, "{next}");
    }

    #[test]
    fn a_cursor_outside_a_new_range_plays_its_bar_to_the_end() {
        // Bar 1 plays when the range becomes 3..=4: on to its end, then 3.
        let t = looping(3, 4);
        let tps = t.ticks_per_sec_in_measure(0);
        let in_bar_1 = 100.0;
        assert_eq!(
            advance_tick(in_bar_1, 0.5, None, 0.0, false, &t, t.loop_bars()),
            in_bar_1 + 0.5 * tps
        );
        let bar_2 = t.measure_start_tick(1) as f64;
        let next = advance_tick(bar_2 - 1.0, 3.0 / tps, None, 0.0, false, &t, t.loop_bars());
        assert!((next - (t.measure_start_tick(2) as f64 + 2.0)).abs() < 1e-6, "{next}");
        // The audio playing there too keeps the cursor where it is.
        let total = t.total_loop_ticks();
        assert_eq!(
            advance_tick(in_bar_1, 0.0, Some((in_bar_1, total)), 0.0, false, &t, t.loop_bars()),
            in_bar_1
        );
    }

    #[test]
    fn count_in_waits_for_the_engine_then_runs_until_the_downbeat() {
        let t = timing();
        let tps = t.ticks_per_sec_in_measure(0);
        let waiting = next_count_in(CountInState::Waiting { since: f64::NAN }, 1.0, None, 0.0, &t);
        assert_eq!(waiting, CountInState::Waiting { since: 1.0 });
        assert_eq!(next_count_in(waiting, 1.1, Some((-100.0, 1)), 0.0, &t), CountInState::Active);
        assert_eq!(next_count_in(waiting, 2.0, None, 0.0, &t), CountInState::Off);
        let active = CountInState::Active;
        assert_eq!(next_count_in(active, 0.0, None, 0.0, &t), CountInState::Active);
        // With 0.5 s latency the downbeat is heard half a second later.
        let leaving = next_count_in(active, 10.0, Some((0.0, 1)), 0.5, &t);
        assert_eq!(leaving, CountInState::Heard { at: 10.5 });
        assert_eq!(next_count_in(leaving, 10.4, Some((0.4 * tps, 1)), 0.5, &t), leaving);
        assert_eq!(next_count_in(leaving, 10.5, None, 0.5, &t), CountInState::Off);
        // Seen late, the engine already a little on: that much earlier.
        let heard = Some((0.5 * tps, 1));
        assert_eq!(next_count_in(active, 10.0, heard, 0.5, &t), CountInState::Off);
    }

    #[test]
    fn a_count_in_into_a_range_ends_once_its_first_downbeat_is_heard() {
        let t = looping(3, 4);
        let tps = t.ticks_per_sec_in_measure(2);
        let start = t.loop_start_tick() as f64;
        let active = CountInState::Active;
        // The engine is at bar 3; with 0.5 s latency not yet heard.
        let at = |ticks: f64| Some((start + ticks, t.total_loop_ticks()));
        assert!(matches!(
            next_count_in(active, 0.0, at(10.0), 0.5, &t),
            CountInState::Heard { .. }
        ));
        assert_eq!(next_count_in(active, 0.0, at(0.5 * tps), 0.5, &t), CountInState::Off);
        // A loop shorter than the latency: the engine goes round it many
        // times before the downbeat is heard, which still ends the wait.
        let short = looping_of(2, 2, 2);
        let wrapped = Some((short.loop_start_tick() as f64 + 1.0, short.total_loop_ticks()));
        let leaving = next_count_in(active, 5.0, wrapped, 3.0, &short);
        assert!(matches!(
            next_count_in(leaving, 7.9, wrapped, 3.0, &short),
            CountInState::Heard { .. }
        ));
        assert_eq!(next_count_in(leaving, 8.0, wrapped, 3.0, &short), CountInState::Off);
    }

    #[test]
    fn latency_back_over_a_barline_stays_in_the_bar_still_heard() {
        // Bar 5 of 3..=6 plays when the range becomes 3..=5: the engine
        // goes on to bar 3, the bar heard is still bar 5, never bar 6.
        let old = looping_of(8, 3, 6);
        let new = BarRange::new(2, 4).unwrap();
        let tps = old.ticks_per_sec_in_measure(0);
        let near_end_of_5 = old.measure_start_tick(5) as f64 - 0.05 * tps;
        let engine = Some((old.measure_start_tick(2) as f64 + 0.05 * tps, old.total_loop_ticks()));
        let next = advance_tick(near_end_of_5, 0.0, engine, 0.1, false, &old, new);
        assert_eq!(old.measure_at_global_tick(next), 4, "{next}");
        // Once the cursor is in bar 3 too, the bar before it is bar 5, the
        // new range's last, not the old range's.
        let in_3 = old.measure_start_tick(2) as f64 + 1.0;
        let next = advance_tick(in_3, 0.0, engine, 0.2, false, &old, new);
        assert_eq!(old.measure_at_global_tick(next), 4, "{next}");
    }
}
