//! Tap tempo: the tempo of a few taps in time, e.g. on a "Tap" button.

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
    /// are two. A pause longer than [`TAP_RESET_S`] starts over.
    pub fn tap(&mut self, now: f64) -> Option<u32> {
        if self.taps.last().is_some_and(|&last| now - last > TAP_RESET_S) {
            self.taps.clear();
        }
        self.taps.push(now);
        if self.taps.len() > MAX_TAPS {
            self.taps.remove(0);
        }
        let (first, last) = (*self.taps.first()?, *self.taps.last()?);
        let interval = (last - first) / (self.taps.len() - 1).max(1) as f64;
        (interval > 0.0).then(|| ((60.0 / interval).round() as u32).clamp(MIN_BPM, MAX_BPM))
    }

    /// Forget the taps, so the next one starts a new measurement.
    pub fn reset(&mut self) {
        self.taps.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_tempo_needs_two_taps() {
        let mut taps = TapTempo::default();
        assert_eq!(taps.tap(10.0), None);
        assert_eq!(taps.tap(10.5), Some(120));
        assert_eq!(taps.tap(11.0), Some(120));
    }

    #[test]
    fn tap_tempo_averages_and_restarts_after_a_pause() {
        let mut taps = TapTempo::default();
        for t in [0.0, 0.6, 1.2, 1.8] {
            taps.tap(t);
        }
        assert_eq!(taps.taps.len(), 4);
        // 0.6 s per beat = 100 BPM.
        assert_eq!(taps.tap(2.4), Some(100));
        // A long pause starts a new measurement.
        assert_eq!(taps.tap(10.0), None);
        assert_eq!(taps.taps, vec![10.0]);
    }

    #[test]
    fn tap_tempo_is_clamped() {
        let mut taps = TapTempo::default();
        taps.tap(0.0);
        assert_eq!(taps.tap(0.01), Some(MAX_BPM));
    }

    #[test]
    fn reset_starts_over() {
        let mut taps = TapTempo::default();
        taps.tap(0.0);
        taps.reset();
        assert_eq!(taps.tap(0.5), None);
    }
}
