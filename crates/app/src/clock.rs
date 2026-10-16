//! Monotonic clock shared by all play-along hit sources, so MIDI hits, taps
//! and the accuracy session anchor live on one timeline.

/// Seconds since an arbitrary, fixed origin. On the web this is
/// `performance.now()`, the same timeline as DOM `Event.timeStamp`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn now_seconds() -> f64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN.get_or_init(Instant::now).elapsed().as_secs_f64()
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn now_seconds() -> f64 {
    web_sys::window().and_then(|w| w.performance()).map(|p| p.now() / 1000.0).unwrap_or(0.0)
}
