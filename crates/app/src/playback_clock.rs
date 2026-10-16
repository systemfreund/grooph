//! The playback cursor's clock: advances the cursor between frames, keeps it
//! in step with the audio engine and tracks the count-in. Pure functions of
//! the previous state, so they can be tested without a UI or audio device.

use crate::state::{COUNT_IN_WAIT_TIMEOUT_S, CountInState};
use grooph_measure::tempo::ScoreTiming;

/// Audio engine position: global tick (negative during the count-in) and the
/// loop length in ticks.
pub(crate) type AudioPos = (f64, u64);

/// Share of the distance to the audio position the cursor closes per frame.
const AUDIO_NUDGE: f64 = 0.1;

/// Cursor position (global ticks) `dt` seconds after `tick`: moves at the
/// playing measure's rate, then follows the audio position (shifted back by
/// `latency_s`), snapping when more than half a beat off and nudging
/// otherwise. Held at 0 while `counting_in`; wraps at the end of the score.
pub(crate) fn advance_tick(
    tick: f64,
    dt: f64,
    audio_pos: Option<AudioPos>,
    latency_s: f64,
    counting_in: bool,
    timing: &ScoreTiming,
) -> f64 {
    if counting_in {
        // Hold the cursor on the first downbeat until the count-in bar is over.
        return 0.0;
    }
    let total = timing.total_loop_ticks() as f64;
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
        let audio_tick = (raw_audio_tick - offset_ticks).rem_euclid(audio_total);

        // Shortest way round the loop.
        let mut diff = audio_tick - next;
        if diff > audio_total * 0.5 {
            diff -= audio_total;
        } else if diff < -audio_total * 0.5 {
            diff += audio_total;
        }

        let half_beat = timing.ticks_per_beat_in_measure(audio_m) as f64 * 0.5;
        if diff.abs() > half_beat {
            next = audio_tick;
        } else {
            next += diff * AUDIO_NUDGE;
        }
    }

    if total > 0.0 { next.rem_euclid(total) } else { next }
}

/// The count-in after this frame: `Waiting` until the audio engine reports a
/// negative position (or gives up after [`COUNT_IN_WAIT_TIMEOUT_S`]),
/// `Active` while the latency-adjusted position is before the first
/// downbeat, then `Off`.
pub(crate) fn next_count_in(
    state: CountInState,
    now: f64,
    audio_pos: Option<AudioPos>,
    latency_s: f64,
    timing: &ScoreTiming,
) -> CountInState {
    let tps = if timing.measure_count() > 0 { timing.ticks_per_sec_in_measure(0) } else { 0.0 };
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
        CountInState::Active => match audio_pos {
            Some((tick, _)) if tick - latency_s * tps >= 0.0 => CountInState::Off,
            // Busy engine (None) or still counting in: stay.
            _ => CountInState::Active,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grooph_measure::{Measure, Score, TimeSignature};

    fn timing() -> ScoreTiming {
        let score = Score {
            measures: vec![
                Measure::new(TimeSignature::FOUR_FOUR),
                Measure::new(TimeSignature::FOUR_FOUR),
            ],
        };
        ScoreTiming::from_score(&score, 120)
    }

    #[test]
    fn without_audio_the_cursor_runs_at_the_tempo() {
        let t = timing();
        let tps = t.ticks_per_sec_in_measure(0);
        assert_eq!(advance_tick(0.0, 0.5, None, 0.0, false, &t), tps * 0.5);
    }

    #[test]
    fn the_cursor_wraps_at_the_end_of_the_score() {
        let t = timing();
        let total = t.total_loop_ticks() as f64;
        let tps = t.ticks_per_sec_in_measure(1);
        let next = advance_tick(total - 1.0, 1.0 / tps * 3.0, None, 0.0, false, &t);
        assert!((next - 2.0).abs() < 1e-6, "{next}");
    }

    #[test]
    fn the_cursor_nudges_toward_nearby_audio_and_snaps_to_far_audio() {
        let t = timing();
        let total = t.total_loop_ticks();
        let tpb = t.ticks_per_beat_in_measure(0) as f64;
        // A tenth of a beat ahead: close a tenth of the gap.
        let near = advance_tick(0.0, 0.0, Some((tpb * 0.1, total)), 0.0, false, &t);
        assert!((near - tpb * 0.01).abs() < 1e-6, "{near}");
        // A beat ahead: jump there.
        assert_eq!(advance_tick(0.0, 0.0, Some((tpb, total)), 0.0, false, &t), tpb);
    }

    #[test]
    fn latency_shifts_the_audio_position_back() {
        let t = timing();
        let total = t.total_loop_ticks();
        let tps = t.ticks_per_sec_in_measure(0);
        let tpb = t.ticks_per_beat_in_measure(0) as f64;
        // Audio two beats in, half a second of latency (one beat at 120 BPM).
        let next = advance_tick(0.0, 0.0, Some((2.0 * tpb, total)), 0.5, false, &t);
        assert!((next - (2.0 * tpb - 0.5 * tps)).abs() < 1e-6, "{next}");
    }

    #[test]
    fn audio_just_past_the_loop_start_is_reached_the_short_way() {
        let t = timing();
        let total = t.total_loop_ticks();
        let tpb = t.ticks_per_beat_in_measure(0) as f64;
        let near_end = total as f64 - tpb * 0.1;
        let next = advance_tick(near_end, 0.0, Some((tpb * 0.1, total)), 0.0, false, &t);
        // Nudged forward across the wrap, not dragged back through the score.
        assert!(next > near_end, "{next}");
    }

    #[test]
    fn counting_in_holds_the_cursor() {
        let t = timing();
        assert_eq!(advance_tick(500.0, 0.1, Some((-10.0, 1)), 0.0, true, &t), 0.0);
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
        // With 0.5 s latency the downbeat is heard half a second of ticks later.
        assert_eq!(next_count_in(active, 0.0, Some((0.0, 1)), 0.5, &t), CountInState::Active);
        let heard = Some((0.5 * tps, 1));
        assert_eq!(next_count_in(active, 0.0, heard, 0.5, &t), CountInState::Off);
    }
}
