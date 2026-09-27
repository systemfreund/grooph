//! Swing feel: delays every second note of a pair while the notation stays
//! straight.
//!
//! Swing is a playback property. The score keeps its written (straight)
//! ticks; [`Swing::performed_tick`] maps a written onset to the tick at which
//! it is heard, and [`Swing::written_tick`] maps an audio position back onto
//! the written axis so the playback cursor reaches a swung note exactly when
//! it sounds.
//!
//! The measure is cut into pairs of the swing unit (a quarter for swung
//! eighths, an eighth for swung sixteenths), starting at the downbeat. Inside a
//! pair, time is warped piecewise linearly: the first half is stretched to
//! `ratio` of the pair, the second half compressed into the rest. Onsets on the
//! binary grid of the pair (multiples of a quarter pair) follow the warp;
//! tuplet onsets stay where they are, since triplets are played as written in
//! swing. A trailing incomplete pair (e.g. the last eighth of 7/8) is straight,
//! so measure lengths never change.

use crate::duration::{e, q};
use crate::grid::DEFAULT_GRID;
use serde::{Deserialize, Serialize};

/// Straight feel.
pub const MIN_SWING_PERCENT: u8 = 50;
/// Dotted feel (3:1), the heaviest swing on offer.
pub const MAX_SWING_PERCENT: u8 = 75;

/// Note value that is swung.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SwingUnit {
    #[default]
    Eighths,
    Sixteenths,
}

impl SwingUnit {
    pub const ALL: [SwingUnit; 2] = [SwingUnit::Eighths, SwingUnit::Sixteenths];

    pub fn label(self) -> &'static str {
        match self {
            SwingUnit::Eighths => "8ths",
            SwingUnit::Sixteenths => "16ths",
        }
    }

    /// Ticks of one pair (two swing units).
    fn pair_ticks(self) -> u32 {
        let pair = match self {
            SwingUnit::Eighths => q(),
            SwingUnit::Sixteenths => e(),
        };
        DEFAULT_GRID.ticks_of(&pair).expect("pair duration on the default grid")
    }
}

/// Swing setting. `percent` is the share of a pair taken by its first note:
/// 50 is straight, 66 a triplet feel, 75 dotted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Swing {
    pub unit: SwingUnit,
    pub percent: u8,
}

impl Default for Swing {
    fn default() -> Self { Self { unit: SwingUnit::Eighths, percent: MIN_SWING_PERCENT } }
}

impl Swing {
    pub fn is_straight(&self) -> bool { self.percent <= MIN_SWING_PERCENT }

    fn ratio(&self) -> f64 {
        self.percent.clamp(MIN_SWING_PERCENT, MAX_SWING_PERCENT) as f64 / 100.0
    }

    /// Pair containing `local_tick`: `(pair_start, pair_ticks)`, or `None` if
    /// swing is off or the tick lies in a trailing incomplete pair.
    fn pair_of(&self, local_tick: f64, measure_ticks: u32) -> Option<(f64, f64)> {
        if self.is_straight() || local_tick < 0.0 {
            return None;
        }
        let pair = self.unit.pair_ticks() as f64;
        let start = (local_tick / pair).floor() * pair;
        (start + pair <= measure_ticks as f64).then_some((start, pair))
    }

    /// Tick at which a note written at `local_tick` is heard.
    pub fn performed_tick(&self, local_tick: u32, measure_ticks: u32) -> u32 {
        let Some((start, pair)) = self.pair_of(local_tick as f64, measure_ticks) else {
            return local_tick;
        };
        let pos = local_tick as f64 - start;
        let quarter_pair = pair / 4.0;
        if pos % quarter_pair != 0.0 {
            // Tuplet onsets keep their written position.
            return local_tick;
        }
        (start + self.warp(pos, pair)).round() as u32
    }

    /// Inverse of the warp: written position of the audio position
    /// `local_tick`. Continuous, for smooth cursor movement.
    pub fn written_tick(&self, local_tick: f64, measure_ticks: u32) -> f64 {
        let Some((start, pair)) = self.pair_of(local_tick, measure_ticks) else {
            return local_tick;
        };
        let pos = local_tick - start;
        let r = self.ratio();
        let split = r * pair;
        let written = if pos < split {
            pos / (2.0 * r)
        } else {
            pair / 2.0 + (pos - split) / (2.0 * (1.0 - r))
        };
        start + written
    }

    fn warp(&self, pos: f64, pair: f64) -> f64 {
        let r = self.ratio();
        let half = pair / 2.0;
        if pos < half { pos * 2.0 * r } else { r * pair + (pos - half) * 2.0 * (1.0 - r) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TimeSignature;
    use crate::duration::{s, t8};

    fn ticks(d: crate::duration::Duration) -> u32 { DEFAULT_GRID.ticks_of(&d).unwrap() }

    fn four_four() -> u32 { DEFAULT_GRID.ticks_per_measure(&TimeSignature::FOUR_FOUR) }

    fn swing(unit: SwingUnit, percent: u8) -> Swing { Swing { unit, percent } }

    #[test]
    fn straight_swing_changes_nothing() {
        let sw = Swing::default();
        for t in [0, ticks(s()), ticks(e()), ticks(t8()), ticks(q()) + ticks(e())] {
            assert_eq!(sw.performed_tick(t, four_four()), t);
            assert_eq!(sw.written_tick(t as f64, four_four()), t as f64);
        }
    }

    #[test]
    fn eighth_swing_delays_offbeat_eighths_only() {
        let sw = swing(SwingUnit::Eighths, 75);
        let (q, e) = (ticks(q()), ticks(e()));
        // Downbeats of every pair stay put.
        for beat in 0..4 {
            assert_eq!(sw.performed_tick(beat * q, four_four()), beat * q);
        }
        // "&" lands on the dotted-eighth position.
        assert_eq!(sw.performed_tick(e, four_four()), ticks(s()) * 3);
        assert_eq!(sw.performed_tick(q + e, four_four()), q + ticks(s()) * 3);
    }

    #[test]
    fn triplet_feel_puts_the_and_on_the_third_triplet() {
        let sw = swing(SwingUnit::Eighths, 66);
        let third_triplet = 2 * ticks(t8());
        let performed = sw.performed_tick(ticks(e()), four_four()) as i64;
        // 66% is within a tick of the exact 2/3.
        assert!((performed - third_triplet as i64).abs() <= ticks(q()) as i64 / 100);
    }

    #[test]
    fn sixteenths_under_eighth_swing_follow_the_warp() {
        let sw = swing(SwingUnit::Eighths, 75);
        let s = ticks(s()) as f64;
        // First half of the pair is stretched by 1.5, second compressed by 0.5.
        assert_eq!(sw.performed_tick(s as u32, four_four()), (1.5 * s).round() as u32);
        assert_eq!(sw.performed_tick(3 * s as u32, four_four()), (3.5 * s).round() as u32);
    }

    #[test]
    fn sixteenth_swing_pairs_are_eighths() {
        let sw = swing(SwingUnit::Sixteenths, 75);
        let s = ticks(s());
        assert_eq!(sw.performed_tick(ticks(e()), four_four()), ticks(e()));
        assert_eq!(sw.performed_tick(s, four_four()), (1.5 * s as f64).round() as u32);
        assert_eq!(sw.performed_tick(3 * s, four_four()), 3 * s + (0.5 * s as f64).round() as u32);
    }

    #[test]
    fn triplets_are_not_swung() {
        let sw = swing(SwingUnit::Eighths, 70);
        let t = ticks(t8());
        assert_eq!(sw.performed_tick(t, four_four()), t);
        assert_eq!(sw.performed_tick(2 * t, four_four()), 2 * t);
    }

    #[test]
    fn trailing_incomplete_pair_stays_straight() {
        let ts = TimeSignature { beats: 7, beat_unit: 8 };
        let tpm = DEFAULT_GRID.ticks_per_measure(&ts);
        let sw = swing(SwingUnit::Eighths, 75);
        // Offbeat of the third full pair is swung ...
        let e = ticks(e());
        assert_ne!(sw.performed_tick(5 * e, tpm), 5 * e);
        // ... the lone seventh eighth is not moved.
        assert_eq!(sw.performed_tick(6 * e, tpm), 6 * e);
        assert_eq!(sw.written_tick((6 * e) as f64 + 1.0, tpm), (6 * e) as f64 + 1.0);
    }

    #[test]
    fn written_tick_inverts_the_warp() {
        let sw = swing(SwingUnit::Eighths, 70);
        let tpm = four_four();
        for t in (0..tpm).step_by(ticks(s()) as usize) {
            let performed = sw.performed_tick(t, tpm) as f64;
            assert!((sw.written_tick(performed, tpm) - t as f64).abs() <= 1.0, "t={t}");
        }
    }

    #[test]
    fn written_tick_is_monotonic() {
        let sw = swing(SwingUnit::Sixteenths, 75);
        let tpm = four_four();
        let mut last = -1.0;
        for t in 0..tpm {
            let w = sw.written_tick(t as f64, tpm);
            assert!(w > last, "t={t}");
            last = w;
        }
    }
}
