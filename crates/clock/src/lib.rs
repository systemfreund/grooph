//! The app's monotonic clock, shared by everything that timestamps a hit:
//! the accuracy session, taps, MIDI and the microphone. All of them read
//! [`now_seconds`], so their times can be compared directly.
//!
//! Devices stamp their events on their own clocks (midir's microseconds, an
//! audio context's `currentTime`). [`OffsetTracker`] maps such stamps onto
//! [`now_seconds`].

/// Seconds since an arbitrary, fixed origin; never goes backwards. On the
/// web this is `performance.now()`, the timeline of DOM `Event.timeStamp`
/// (not `Date.now()`, which follows the wall clock and jumps when it is
/// set).
#[cfg(not(target_arch = "wasm32"))]
pub fn now_seconds() -> f64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// Seconds since an arbitrary, fixed origin; never goes backwards. On the
/// web this is `performance.now()`, the timeline of DOM `Event.timeStamp`
/// (not `Date.now()`, which follows the wall clock and jumps when it is
/// set).
#[cfg(target_arch = "wasm32")]
pub fn now_seconds() -> f64 {
    web_sys::window().and_then(|w| w.performance()).map(|p| p.now() / 1000.0).unwrap_or(0.0)
}

/// The offset from a device's clock to [`now_seconds`], estimated from
/// readings `now - stamp` taken as events arrive.
///
/// Delivery latency (a late callback, an audio block that is read a render
/// quantum later) only ever makes a reading larger than the true offset, so
/// the smallest recent reading is the best estimate. The kept offset may
/// creep up by [`OffsetTracker::DRIFT`] per reading, which follows the slow
/// drift between two crystal clocks (tens of ppm) without letting one late
/// event pull it up; a reading more than [`OffsetTracker::JUMP`] above it
/// means the device's clock jumped (a restart, a glitch or stall), and the
/// estimate starts over from that reading.
#[derive(Clone, Copy, Debug, Default)]
pub struct OffsetTracker {
    offset: Option<f64>,
}

impl OffsetTracker {
    /// How far the kept offset may rise per reading (s): 10 µs, enough for
    /// a drift of 100 ppm at one reading every 0.1 s.
    pub const DRIFT: f64 = 1e-5;
    /// A reading this far above the kept offset (s) starts the estimate
    /// over: more than any delivery latency, less than a noticeable timing
    /// error.
    pub const JUMP: f64 = 0.05;

    /// Take the reading `now - stamp` (both in seconds) and return the
    /// offset to add to a device stamp to get [`now_seconds`] time.
    pub fn observe(&mut self, reading: f64) -> f64 {
        let offset = match self.offset {
            Some(kept) if reading - kept < Self::JUMP => reading.min(kept + Self::DRIFT),
            _ => reading,
        };
        self.offset = Some(offset);
        offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_smallest_reading_wins() {
        let mut t = OffsetTracker::default();
        assert_eq!(t.observe(10.003), 10.003);
        // Less latency: the estimate drops at once.
        assert_eq!(t.observe(10.001), 10.001);
        // More latency: the estimate creeps up by DRIFT only.
        let crept = t.observe(10.02);
        assert!((crept - (10.001 + OffsetTracker::DRIFT)).abs() < 1e-12, "{crept}");
    }

    #[test]
    fn a_jump_starts_over() {
        let mut t = OffsetTracker::default();
        t.observe(1.0);
        assert_eq!(t.observe(1.0 + OffsetTracker::JUMP), 1.0 + OffsetTracker::JUMP);
    }

    #[test]
    fn the_clock_does_not_go_backwards() {
        let a = now_seconds();
        let b = now_seconds();
        assert!(b >= a);
    }
}
