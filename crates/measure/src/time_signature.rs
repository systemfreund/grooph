use crate::duration::NoteValue;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::ops::RangeInclusive;
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
    /// Most beats per measure the app offers (over any beat unit).
    pub const MAX_BEATS: u8 = 19;
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

    /// Beats per measure the app offers over `beat_unit`: 1-7 quarters,
    /// 3-13 eighths, 5-19 sixteenths. `None` for other beat units.
    pub const fn beat_range(beat_unit: u8) -> Option<RangeInclusive<u8>> {
        match beat_unit {
            4 => Some(1..=7),
            8 => Some(3..=13),
            16 => Some(5..=Self::MAX_BEATS),
            _ => None,
        }
    }

    /// Whether the app offers this time signature (see [`Self::beat_range`]).
    pub fn is_supported(&self) -> bool {
        Self::beat_range(self.beat_unit).is_some_and(|r| r.contains(&self.beats))
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

/// Why a string is not a supported time signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseTimeSignatureError {
    /// Not of the form `beats/unit`, e.g. `7/8`.
    Malformed,
    /// Beats outside [`TimeSignature::beat_range`] for the unit.
    UnsupportedBeats { beats: u8, unit: u8 },
    /// A beat unit not in [`TimeSignature::BEAT_UNITS`].
    UnsupportedUnit(u8),
}

impl Display for ParseTimeSignatureError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed => write!(f, "expected beats/unit, e.g. 4/4 or 7/8"),
            Self::UnsupportedBeats { beats, unit } => {
                let range =
                    TimeSignature::beat_range(*unit).unwrap_or(1..=TimeSignature::MAX_BEATS);
                write!(
                    f,
                    "{beats}/{unit} not supported ({}-{} beats over {unit})",
                    range.start(),
                    range.end()
                )
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
        let range = Self::beat_range(beat_unit)
            .ok_or(ParseTimeSignatureError::UnsupportedUnit(beat_unit))?;
        if !range.contains(&beats) {
            return Err(ParseTimeSignatureError::UnsupportedBeats { beats, unit: beat_unit });
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
        assert_eq!("19/16".parse::<TimeSignature>().map(|ts| ts.to_string()), Ok("19/16".into()));
        for ok in ["1/4", "7/4", "3/8", "13/8", "5/16"] {
            assert!(ok.parse::<TimeSignature>().is_ok(), "{ok}");
        }
        assert_eq!(TimeSignature::FOUR_FOUR.to_string(), "4/4");
    }

    #[test]
    fn rejects_unsupported_time_signatures() {
        assert_eq!("4".parse::<TimeSignature>(), Err(ParseTimeSignatureError::Malformed));
        assert_eq!("x/4".parse::<TimeSignature>(), Err(ParseTimeSignatureError::Malformed));
        for (text, beats, unit) in
            [("0/4", 0, 4), ("8/4", 8, 4), ("2/8", 2, 8), ("14/8", 14, 8), ("4/16", 4, 16)]
        {
            assert_eq!(
                text.parse::<TimeSignature>(),
                Err(ParseTimeSignatureError::UnsupportedBeats { beats, unit }),
                "{text}"
            );
        }
        assert_eq!(
            "20/16".parse::<TimeSignature>().unwrap_err().to_string(),
            "20/16 not supported (5-19 beats over 16)"
        );
        assert_eq!(
            "4/5".parse::<TimeSignature>(),
            Err(ParseTimeSignatureError::UnsupportedUnit(5))
        );
    }
}
