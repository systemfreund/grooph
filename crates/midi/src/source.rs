//! Abstract MIDI event source for testability.
//!
//! The concrete [`MidiInput`](crate::MidiInput) implements this trait; tests
//! and headless scenarios can supply a mock implementation without dragging
//! in `midir` or a real device. See `tests` at the bottom of `input.rs` for a
//! minimal mock example.

use crate::{MidiClockState, MidiInputEvent};

/// Read-only view of an active MIDI event stream.
///
/// Lifecycle operations (port enumeration, connect/disconnect) intentionally
/// live on the concrete type — they are platform-specific and not part of
/// what an app's event-processing loop needs to be tested against.
pub trait MidiMessageSource {
    /// Drain queued input events. Subsequent calls return only events that
    /// arrived after the previous call.
    fn drain_events(&self) -> Vec<MidiInputEvent>;

    /// Current time in seconds relative to this source's epoch. Used as the
    /// timebase for the `timestamp` field on incoming events.
    fn now_seconds(&self) -> f64;

    /// Whether the source is currently connected to a device.
    fn is_connected(&self) -> bool;

    /// Shared MIDI clock state (BPM, running flag) updated as clock messages
    /// arrive.
    fn clock_state(&self) -> &MidiClockState;
}
