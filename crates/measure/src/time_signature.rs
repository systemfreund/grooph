use crate::duration::NoteValue;
use crate::grouping;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::fmt::{Display, Formatter};
use std::ops::RangeInclusive;
use std::str::FromStr;

/// Represents a time signature (e.g., 4/4, 3/4, 6/8)
///
/// Invariant: at least one beat, and a beat unit with a note value (see
/// [`Self::beat_note_value`]). Deserializing checks it (see
/// [`InvalidTimeSignature`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawTimeSignature")]
pub struct TimeSignature {
    /// Number of beats per measure
    pub beats: u8,
    /// Note value that represents one beat (4 = quarter note, 8 = eighth note)
    pub beat_unit: u8,
    /// Index into [`Self::groupings`]: how the beats are grouped (7/8 as
    /// 2+3+2, 2+2+3 or 3+2+2). 0 is the usual grouping.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub grouping: u8,
}

fn is_zero(v: &u8) -> bool {
    *v == 0
}

/// A [`TimeSignature`] as saved, before its invariant is checked.
#[derive(Deserialize)]
struct RawTimeSignature {
    beats: u8,
    beat_unit: u8,
    #[serde(default)]
    grouping: u8,
}

/// A saved time signature without beats or with a beat unit that is no note
/// value, e.g. `0/4` or `4/0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTimeSignature {
    pub beats: u8,
    pub beat_unit: u8,
}

impl Display for InvalidTimeSignature {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{} is not a time signature", self.beats, self.beat_unit)
    }
}

impl std::error::Error for InvalidTimeSignature {}

/// A grouping out of range is kept: [`TimeSignature::groups`] falls back to
/// the usual one.
impl TryFrom<RawTimeSignature> for TimeSignature {
    type Error = InvalidTimeSignature;

    fn try_from(raw: RawTimeSignature) -> Result<Self, Self::Error> {
        let RawTimeSignature { beats, beat_unit, grouping } = raw;
        let ts = Self { beats, beat_unit, grouping };
        if beats == 0 || ts.beat_note_value().is_none() {
            return Err(InvalidTimeSignature { beats, beat_unit });
        }
        Ok(ts)
    }
}

impl TimeSignature {
    /// Most beats per measure the app offers (over any beat unit).
    pub const MAX_BEATS: u8 = 19;
    /// Beat units the app offers.
    pub const BEAT_UNITS: [u8; 3] = [4, 8, 16];

    pub const ONE_FOUR: Self = Self::new(1, 4);
    pub const TWO_FOUR: Self = Self::new(2, 4);
    pub const THREE_FOUR: Self = Self::new(3, 4);
    pub const ONE_SIXTEENTH: Self = Self::new(1, 16);
    pub const TWO_SIXTEENTH: Self = Self::new(2, 16);
    pub const FOUR_SIXTEENTH: Self = Self::new(4, 16);
    pub const FOUR_FOUR: Self = Self::new(4, 4);
    pub const TWO_EIGHT: Self = Self::new(2, 8);
    pub const FOUR_EIGHT: Self = Self::new(4, 8);
    pub const FIVE_EIGHT: Self = Self::new(5, 8);
    pub const SIX_EIGHT: Self = Self::new(6, 8);
    pub const SEVEN_EIGHT: Self = Self::new(7, 8);
    pub const NINE_EIGHT: Self = Self::new(9, 8);
    pub const TWELVE_EIGHT: Self = Self::new(12, 8);

    /// `beats/beat_unit` in its usual grouping.
    pub const fn new(beats: u8, beat_unit: u8) -> Self {
        Self { beats, beat_unit, grouping: 0 }
    }

    /// The same meter in grouping `grouping` (see [`Self::groupings`]).
    pub const fn with_grouping(self, grouping: u8) -> Self {
        Self { grouping, ..self }
    }

    /// The groupings offered for this meter, the usual one first; e.g. 7/8 as
    /// 2+3+2, 2+2+3 and 3+2+2. Only the odd x/8 and x/16 meters (5, 7, 8, 10
    /// and 11 beats) offer more than one.
    pub fn groupings(&self) -> Vec<Vec<u8>> {
        grouping::groupings(self.beats, self.beat_unit)
    }

    /// The beat groups of this meter in beat units, e.g. `[2, 2, 3]`.
    pub fn groups(&self) -> Cow<'static, [u8]> {
        grouping::default_groups_for(self)
    }

    /// Whether the meter groups its beats (6/8 as 3+3, 7/8 as 2+3+2), so a
    /// description names the grouping; x/4 meters count single beats.
    pub fn shows_grouping(&self) -> bool {
        let groups = self.groups();
        groups.len() > 1 && groups.iter().any(|&g| g > 1)
    }

    /// The beat groups joined with `+`, e.g. `2+2+3`.
    pub fn groups_label(&self) -> String {
        self.groups().iter().map(|g| g.to_string()).collect::<Vec<_>>().join("+")
    }

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
            && (self.grouping as usize) < self.groupings().len()
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
    /// Beat groups (`2+2+3/8`) that are not among the meter's groupings.
    UnsupportedGrouping(String),
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
            Self::UnsupportedGrouping(text) => write!(f, "grouping {text} not supported"),
        }
    }
}

impl std::error::Error for ParseTimeSignatureError {}

/// Parses `beats/unit`, e.g. `4/4` or `7/8`, or beat groups over the unit,
/// e.g. `2+2+3/8`, limited to what the app offers.
impl FromStr for TimeSignature {
    type Err = ParseTimeSignatureError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (top, unit) = s.split_once('/').ok_or(ParseTimeSignatureError::Malformed)?;
        let groups = top
            .split('+')
            .map(|g| g.parse::<u8>().map_err(|_| ParseTimeSignatureError::Malformed))
            .collect::<Result<Vec<_>, _>>()?;
        let beats = groups.iter().try_fold(0u8, |sum, &g| sum.checked_add(g));
        let beats = beats.ok_or(ParseTimeSignatureError::Malformed)?;
        let beat_unit = unit.parse::<u8>().map_err(|_| ParseTimeSignatureError::Malformed)?;
        let range = Self::beat_range(beat_unit)
            .ok_or(ParseTimeSignatureError::UnsupportedUnit(beat_unit))?;
        if !range.contains(&beats) {
            return Err(ParseTimeSignatureError::UnsupportedBeats { beats, unit: beat_unit });
        }
        let ts = Self::new(beats, beat_unit);
        if groups.len() == 1 {
            return Ok(ts);
        }
        let grouping = ts
            .groupings()
            .iter()
            .position(|g| *g == groups)
            .ok_or_else(|| ParseTimeSignatureError::UnsupportedGrouping(s.to_string()))?;
        Ok(ts.with_grouping(grouping as u8))
    }
}

/// Formats as `beats/unit`, e.g. `7/8`, or as beat groups over the unit
/// when the meter is not in its usual grouping, e.g. `2+2+3/8`; the inverse
/// of [`FromStr`].
impl Display for TimeSignature {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.grouping == 0 {
            return write!(f, "{}/{}", self.beats, self.beat_unit);
        }
        write!(f, "{}/{}", self.groups_label(), self.beat_unit)
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
    fn parses_and_formats_groupings() {
        let ts: TimeSignature = "2+2+3/8".parse().unwrap();
        assert_eq!(ts, TimeSignature::SEVEN_EIGHT.with_grouping(1));
        assert_eq!(*ts.groups(), [2, 2, 3]);
        assert_eq!(ts.to_string(), "2+2+3/8");
        // The usual grouping is written as a plain meter.
        assert_eq!("2+3+2/8".parse(), Ok(TimeSignature::SEVEN_EIGHT));
        assert_eq!(TimeSignature::SEVEN_EIGHT.to_string(), "7/8");
        for text in ["2+3/16", "3+2+2/8", "2+3+3/8", "2+2+3+2+2/8", "2+3+2+3/16"] {
            let ts: TimeSignature = text.parse().unwrap();
            assert!(ts.is_supported(), "{text}");
            assert_eq!(ts.to_string(), text);
        }
        assert_eq!(
            "2+2+2+1/8".parse::<TimeSignature>(),
            Err(ParseTimeSignatureError::UnsupportedGrouping("2+2+2+1/8".into()))
        );
        assert_eq!(
            "2+2/4".parse::<TimeSignature>().unwrap_err().to_string(),
            "grouping 2+2/4 not supported"
        );
        assert_eq!("2++3/8".parse::<TimeSignature>(), Err(ParseTimeSignatureError::Malformed));
        assert!(!TimeSignature::SIX_EIGHT.with_grouping(1).is_supported());
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

    #[test]
    fn deserializing_checks_beats_and_beat_unit() {
        let read = |text: &str| ron::from_str::<TimeSignature>(text);
        assert_eq!(
            read("(beats: 7, beat_unit: 8, grouping: 1)"),
            Ok(TimeSignature::SEVEN_EIGHT.with_grouping(1))
        );
        // Meters the app builds internally, beyond what it offers to pick.
        assert_eq!(read("(beats: 3, beat_unit: 32)"), Ok(TimeSignature::new(3, 32)));
        for text in [
            "(beats: 0, beat_unit: 4)",
            "(beats: 4, beat_unit: 0)",
            "(beats: 4, beat_unit: 6)",
            "(beats: 4, beat_unit: 64)",
        ] {
            let err = read(text).unwrap_err().to_string();
            assert!(err.contains("is not a time signature"), "{text}: {err}");
        }
        for ts in [TimeSignature::FOUR_FOUR, TimeSignature::SEVEN_EIGHT.with_grouping(2)] {
            assert_eq!(read(&ron::to_string(&ts).unwrap()), Ok(ts));
        }
    }
}
