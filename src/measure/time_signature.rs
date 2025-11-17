use crate::measure::duration::{default_duration_set, NoteValue};

/// Represents a time signature (e.g., 4/4, 3/4, 6/8)
#[derive(Debug, Clone, Copy)]
pub struct TimeSignature {
    /// Number of beats per measure
    pub beats: u8,
    /// Note value that represents one beat (4 = quarter note, 8 = eighth note)
    pub beat_unit: u8,
}

impl TimeSignature {
    pub const ONE_FOUR: Self = Self { beats: 1, beat_unit: 4 };
    pub const TWO_FOUR: Self = Self { beats: 2, beat_unit: 4 };
    pub const ONE_SIXTEENTH: Self = Self { beats: 1, beat_unit: 16 };
    pub const TWO_SIXTEENTH: Self = Self { beats: 2, beat_unit: 16 };
    pub const FOUR_SIXTEENTH: Self = Self { beats: 4, beat_unit: 16 };
    pub const FOUR_FOUR: Self = Self { beats: 4, beat_unit: 4 };
    pub const TWO_EIGHT: Self = Self { beats: 2, beat_unit: 8 };
    pub const FOUR_EIGHT: Self = Self { beats: 4, beat_unit: 8 };
    pub const FIVE_EIGHT: Self = Self { beats: 5, beat_unit: 8 };
    pub const SIX_EIGHT: Self = Self { beats: 6, beat_unit: 8 };
    pub const SEVEN_EIGHT: Self = Self { beats: 7, beat_unit: 8 };
    pub const NINE_EIGHT: Self = Self { beats: 9, beat_unit: 8 };
    pub const TWELVE_EIGHT: Self = Self { beats: 12, beat_unit: 8 };

    /// Returns the total duration in integer ticks
    pub const fn measure_duration_ticks(&self) -> u32 {
        // Use the unified duration set to derive the grid.
        let set = default_duration_set();
        ((self.beats as u32) * set.grid.ticks_per_whole) / (self.beat_unit as u32)
    }

    pub const fn beat_note_value(&self) -> Option<NoteValue> {
        match self.beat_unit {
            1 => Some(NoteValue::Whole),
            2 => Some(NoteValue::Half),
            4 => Some(NoteValue::Quarter),
            8 => Some(NoteValue::Eighth),
            16 => Some(NoteValue::Sixteenth),
            32 => Some(NoteValue::ThirtySecond),
            _ => None,
        }
    }
}
