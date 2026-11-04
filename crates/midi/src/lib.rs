//! MIDI abstraction with platform-specific backends.
//!
//! This crate provides a unified interface for MIDI input and output across native
//! (using midir) and web (using Web MIDI API) platforms.

mod error;
mod input;
#[cfg(not(feature = "device"))]
mod no_device;
mod source;

pub use error::{Error, Result};
pub use input::{ClockSync, ClockSyncConfig, MidiClockState, MidiInput, MidiInputEvent};
pub use source::MidiMessageSource;

/// MIDI note number type (0-127)
pub type MidiNote = u8;

/// MIDI velocity type (0-127)
pub type MidiVelocity = u8;
