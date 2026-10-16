//! Tap tempo: the tempo of a few taps in time, e.g. on a "Tap" button.
//!
//! BPM counts quarter notes, but people tap the pulse they feel: dotted
//! quarters in 6/8, eighths in 7/8. [`tap_pulse`] says which pulse a meter
//! is tapped in, and [`TapTempo::tap`] converts taps in that pulse to BPM.

use crate::TimeSignature;
use crate::tempo::{MAX_BPM, MIN_BPM};

/// Taps further apart than this (seconds) start a new measurement.
pub const TAP_RESET_S: f64 = 2.0;
/// Only the most recent taps count, so the tempo follows a change quickly.
const MAX_TAPS: usize = 8;

/// Recent taps, as times in seconds on any steady clock.
#[derive(Clone, Debug, Default)]
pub struct TapTempo {
    taps: Vec<f64>,
}

impl TapTempo {
    /// Add a tap at `now` and return the tempo of the recent taps once there
    /// are two, in quarter-note BPM for taps on `pulse` (see [`tap_pulse`]).
    /// A pause longer than [`TAP_RESET_S`] starts over.
    pub fn tap(&mut self, now: f64, pulse: TapPulse) -> Option<u32> {
        if self.taps.last().is_some_and(|&last| now - last > TAP_RESET_S) {
            self.taps.clear();
        }
        self.taps.push(now);
        if self.taps.len() > MAX_TAPS {
            self.taps.remove(0);
        }
        let (first, last) = (*self.taps.first()?, *self.taps.last()?);
        let interval = (last - first) / (self.taps.len() - 1).max(1) as f64;
        let bpm = 60.0 / interval * pulse.quarters();
        (interval > 0.0).then(|| (bpm.round() as u32).clamp(MIN_BPM, MAX_BPM))
    }

    /// Forget the taps, so the next one starts a new measurement.
    pub fn reset(&mut self) {
        self.taps.clear();
    }
}

/// The note value a meter is tapped in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapPulse {
    Quarter,
    DottedQuarter,
    Eighth,
    DottedEighth,
    Sixteenth,
}

impl TapPulse {
    /// Length in quarter notes.
    pub const fn quarters(self) -> f64 {
        match self {
            Self::Quarter => 1.0,
            Self::DottedQuarter => 1.5,
            Self::Eighth => 0.5,
            Self::DottedEighth => 0.75,
            Self::Sixteenth => 0.25,
        }
    }

    /// Plural name for hints, e.g. "dotted quarters".
    pub const fn name(self) -> &'static str {
        match self {
            Self::Quarter => "quarters",
            Self::DottedQuarter => "dotted quarters",
            Self::Eighth => "eighths",
            Self::DottedEighth => "dotted eighths",
            Self::Sixteenth => "sixteenths",
        }
    }
}

/// The pulse `ts` is tapped in: quarters in x/4; in x/8 and x/16 one tap per
/// beat group when all groups are equal (dotted quarters in 6/8, quarters in
/// 4/8), else one tap per beat unit (eighths in 7/8).
pub fn tap_pulse(ts: TimeSignature) -> TapPulse {
    let groups = ts.groups();
    let even = groups.first().filter(|&&g| groups.iter().all(|&x| x == g));
    match (ts.beat_unit, even) {
        (8, Some(2)) => TapPulse::Quarter,
        (8, Some(3)) => TapPulse::DottedQuarter,
        (8, _) => TapPulse::Eighth,
        (16, Some(2)) => TapPulse::Eighth,
        (16, Some(3)) => TapPulse::DottedEighth,
        (16, _) => TapPulse::Sixteenth,
        _ => TapPulse::Quarter,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_pulse_follows_the_beat_groups() {
        assert_eq!(tap_pulse(TimeSignature::FOUR_FOUR), TapPulse::Quarter);
        assert_eq!(tap_pulse(TimeSignature::SIX_EIGHT), TapPulse::DottedQuarter);
        assert_eq!(tap_pulse(TimeSignature::new(12, 8)), TapPulse::DottedQuarter);
        assert_eq!(tap_pulse(TimeSignature::new(4, 8)), TapPulse::Quarter);
        assert_eq!(tap_pulse(TimeSignature::SEVEN_EIGHT), TapPulse::Eighth);
        assert_eq!(tap_pulse(TimeSignature::new(6, 16)), TapPulse::DottedEighth);
        assert_eq!(tap_pulse(TimeSignature::new(7, 16)), TapPulse::Sixteenth);
    }

    #[test]
    fn taps_convert_to_quarter_bpm() {
        // Dotted quarters 0.5 s apart in 6/8: 120 taps a minute = 180 BPM.
        let mut taps = TapTempo::default();
        taps.tap(0.0, TapPulse::DottedQuarter);
        assert_eq!(taps.tap(0.5, TapPulse::DottedQuarter), Some(180));
        // Eighths 0.25 s apart in 7/8 = 120 BPM.
        let mut taps = TapTempo::default();
        taps.tap(0.0, TapPulse::Eighth);
        assert_eq!(taps.tap(0.25, TapPulse::Eighth), Some(120));
    }

    #[test]
    fn tap_tempo_needs_two_taps() {
        let mut taps = TapTempo::default();
        assert_eq!(taps.tap(10.0, TapPulse::Quarter), None);
        assert_eq!(taps.tap(10.5, TapPulse::Quarter), Some(120));
        assert_eq!(taps.tap(11.0, TapPulse::Quarter), Some(120));
    }

    #[test]
    fn tap_tempo_averages_and_restarts_after_a_pause() {
        let mut taps = TapTempo::default();
        for t in [0.0, 0.6, 1.2, 1.8] {
            taps.tap(t, TapPulse::Quarter);
        }
        assert_eq!(taps.taps.len(), 4);
        // 0.6 s per beat = 100 BPM.
        assert_eq!(taps.tap(2.4, TapPulse::Quarter), Some(100));
        // A long pause starts a new measurement.
        assert_eq!(taps.tap(10.0, TapPulse::Quarter), None);
        assert_eq!(taps.taps, vec![10.0]);
    }

    #[test]
    fn tap_tempo_is_clamped() {
        let mut taps = TapTempo::default();
        taps.tap(0.0, TapPulse::Quarter);
        assert_eq!(taps.tap(0.01, TapPulse::Quarter), Some(MAX_BPM));
    }

    #[test]
    fn reset_starts_over() {
        let mut taps = TapTempo::default();
        taps.tap(0.0, TapPulse::Quarter);
        taps.reset();
        assert_eq!(taps.tap(0.5, TapPulse::Quarter), None);
    }
}
