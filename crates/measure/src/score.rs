use crate::{BeatIdx, Measure};
use serde::{Deserialize, Serialize};

pub type MeasureIdx = usize;

/// A collection of one or more measures.
///
/// Invariant: `measures` is never empty. Constructors and mutators must uphold this;
/// deserializing checks it (see [`EmptyScore`]).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "RawScore")]
pub struct Score {
    pub measures: Vec<Measure>,
}

/// A [`Score`] as saved, before its invariant is checked.
#[derive(Deserialize)]
struct RawScore {
    measures: Vec<Measure>,
}

/// A saved score without measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyScore;

impl std::fmt::Display for EmptyScore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a score needs at least one measure")
    }
}

impl std::error::Error for EmptyScore {}

impl TryFrom<RawScore> for Score {
    type Error = EmptyScore;

    fn try_from(raw: RawScore) -> Result<Self, Self::Error> {
        if raw.measures.is_empty() {
            return Err(EmptyScore);
        }
        Ok(Self { measures: raw.measures })
    }
}

impl Score {
    pub fn single(measure: Measure) -> Self {
        Self { measures: vec![measure] }
    }

    pub fn current(&self, idx: MeasureIdx) -> &Measure {
        &self.measures[idx]
    }

    pub fn current_mut(&mut self, idx: MeasureIdx) -> &mut Measure {
        &mut self.measures[idx]
    }

    pub fn len(&self) -> usize {
        self.measures.len()
    }

    pub fn is_empty(&self) -> bool {
        self.measures.is_empty()
    }

    /// Whether `other` has the same meters and beats, measure by measure:
    /// everything that decides how the score sounds.
    pub fn same_beats(&self, other: &Score) -> bool {
        self.len() == other.len()
            && self.measures.iter().zip(&other.measures).all(|(a, b)| crate::click::same_bar(a, b))
    }
}

/// Position within a score: which measure, and which beat within that measure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub measure_idx: MeasureIdx,
    pub beat_idx: BeatIdx,
}

impl Cursor {
    pub fn start() -> Self {
        Self { measure_idx: 0, beat_idx: 0 }
    }

    pub fn at(measure_idx: MeasureIdx, beat_idx: BeatIdx) -> Self {
        Self { measure_idx, beat_idx }
    }
}

impl Default for Cursor {
    fn default() -> Self {
        Self::start()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TimeSignature;

    #[test]
    fn score_single_constructor_yields_one_measure() {
        let m = Measure::new(TimeSignature::FOUR_FOUR);
        let s = Score::single(m);
        assert_eq!(s.len(), 1);
        assert!(!s.is_empty());
    }

    #[test]
    fn same_beats_compares_meters_and_beats() {
        let four = Measure::new(TimeSignature::FOUR_FOUR);
        let score = Score { measures: vec![four.clone(), four.clone()] };
        assert!(score.same_beats(&score.clone()));

        // The same beats under another meter of the same length.
        let mut other_meter = score.clone();
        other_meter.measures[1].set_time_signature(TimeSignature::new(2, 2)).unwrap();
        assert_eq!(other_meter.measures[1].beats(), four.beats());
        assert!(!score.same_beats(&other_meter));

        let mut other_beat = score.clone();
        other_beat.measures[1].toggle_beat_kind(0);
        assert!(!score.same_beats(&other_beat));

        assert!(!score.same_beats(&Score::single(four)));
    }

    #[test]
    fn deserializing_rejects_an_empty_score() {
        let score = Score::single(Measure::new(TimeSignature::SEVEN_EIGHT));
        let back: Score = ron::from_str(&ron::to_string(&score).unwrap()).unwrap();
        assert!(score.same_beats(&back));
        let err = ron::from_str::<Score>("(measures: [])").unwrap_err().to_string();
        assert!(err.contains("at least one measure"), "{err}");
    }

    #[test]
    fn cursor_start_is_zero_zero() {
        let c = Cursor::start();
        assert_eq!(c.measure_idx, 0);
        assert_eq!(c.beat_idx, 0);
    }

    #[test]
    fn cursor_at_sets_fields() {
        let c = Cursor::at(2, 5);
        assert_eq!(c.measure_idx, 2);
        assert_eq!(c.beat_idx, 5);
    }
}
