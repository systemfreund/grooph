//! MIDI abstraction with platform-specific backends.
//!
//! This crate provides a unified interface for MIDI input and output across native
//! (using midir) and web (using Web MIDI API) platforms.

mod error;
mod input;

pub use error::{Error, Result};
pub use input::{MidiClockState, MidiInputEvent};

#[cfg(not(target_arch = "wasm32"))]
pub use input::NativeMidiInput;

#[cfg(target_arch = "wasm32")]
pub use input::WebMidiInput;

/// MIDI note number type (0-127)
pub type MidiNote = u8;

/// MIDI velocity type (0-127)
pub type MidiVelocity = u8;
