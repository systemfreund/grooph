use crate::duration::NoteValue;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::str::FromStr;

/// Represents a time signature (e.g., 4/4, 3/4, 6/8)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeSignature {
    /// Number of beats per measure
    pub beats: u8,
    /// Note value that represents one beat (4 = quarter note, 8 = eighth note)
    pub beat_unit: u8,
}

impl TimeSignature {
    /// Most beats per measure the app offers.
    pub const MAX_BEATS: u8 = 17;
    /// Beat units the app offers.
    pub const BEAT_UNITS: [u8; 3] = [4, 8, 16];

    pub const ONE_FOUR: Self = Self { beats: 1, beat_unit: 4 };
    pub const TWO_FOUR: Self = Self { beats: 2, beat_unit: 4 };
    pub const THREE_FOUR: Self = Self { beats: 3, beat_unit: 4 };
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

/// Why a string is not a supported time signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseTimeSignatureError {
    /// Not of the form `beats/unit`, e.g. `7/8`.
    Malformed,
    /// Beats outside `1..=TimeSignature::MAX_BEATS`.
    UnsupportedBeats(u8),
    /// A beat unit not in [`TimeSignature::BEAT_UNITS`].
    UnsupportedUnit(u8),
}

impl Display for ParseTimeSignatureError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed => write!(f, "expected beats/unit, e.g. 4/4 or 7/8"),
            Self::UnsupportedBeats(beats) => {
                write!(f, "{beats} beats not supported (1-{})", TimeSignature::MAX_BEATS)
            }
            Self::UnsupportedUnit(unit) => write!(f, "beat unit {unit} not supported (4, 8 or 16)"),
        }
    }
}

impl std::error::Error for ParseTimeSignatureError {}

/// Parses `beats/unit`, e.g. `4/4` or `7/8`, limited to what the app offers.
impl FromStr for TimeSignature {
    type Err = ParseTimeSignatureError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (beats, unit) = s.split_once('/').ok_or(ParseTimeSignatureError::Malformed)?;
        let beats = beats.parse::<u8>().map_err(|_| ParseTimeSignatureError::Malformed)?;
        let beat_unit = unit.parse::<u8>().map_err(|_| ParseTimeSignatureError::Malformed)?;
        if !(1..=Self::MAX_BEATS).contains(&beats) {
            return Err(ParseTimeSignatureError::UnsupportedBeats(beats));
        }
        if !Self::BEAT_UNITS.contains(&beat_unit) {
            return Err(ParseTimeSignatureError::UnsupportedUnit(beat_unit));
        }
        Ok(Self { beats, beat_unit })
    }
}

/// Formats as `beats/unit`, e.g. `7/8`; the inverse of [`FromStr`].
impl Display for TimeSignature {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.beats, self.beat_unit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_supported_time_signatures() {
        assert_eq!("7/8".parse(), Ok(TimeSignature::SEVEN_EIGHT));
        assert_eq!("17/16".parse::<TimeSignature>().map(|ts| ts.to_string()), Ok("17/16".into()));
        assert_eq!(TimeSignature::FOUR_FOUR.to_string(), "4/4");
    }

    #[test]
    fn rejects_unsupported_time_signatures() {
        assert_eq!("4".parse::<TimeSignature>(), Err(ParseTimeSignatureError::Malformed));
        assert_eq!("x/4".parse::<TimeSignature>(), Err(ParseTimeSignatureError::Malformed));
        assert_eq!(
            "0/4".parse::<TimeSignature>(),
            Err(ParseTimeSignatureError::UnsupportedBeats(0))
        );
        assert_eq!(
            "18/8".parse::<TimeSignature>(),
            Err(ParseTimeSignatureError::UnsupportedBeats(18))
        );
        assert_eq!(
            "4/5".parse::<TimeSignature>(),
            Err(ParseTimeSignatureError::UnsupportedUnit(5))
        );
    }
}
