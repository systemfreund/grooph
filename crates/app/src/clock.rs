//! Monotonic clock shared by all play-along hit sources, so MIDI hits, taps
//! and the accuracy session anchor live on one timeline
//! ([`grooph_clock::now_seconds`]).

pub(crate) use grooph_clock::now_seconds;
