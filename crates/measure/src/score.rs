use crate::{BeatIdx, Measure};
use serde::{Deserialize, Serialize};

pub type MeasureIdx = usize;

/// A collection of one or more measures.
///
/// Invariant: `measures` is never empty. Constructors and mutators must uphold this.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Score {
    pub measures: Vec<Measure>,
}

impl Score {
    pub fn single(measure: Measure) -> Self { Self { measures: vec![measure] } }

    pub fn current(&self, idx: MeasureIdx) -> &Measure { &self.measures[idx] }

    pub fn current_mut(&mut self, idx: MeasureIdx) -> &mut Measure { &mut self.measures[idx] }

    pub fn len(&self) -> usize { self.measures.len() }

    pub fn is_empty(&self) -> bool { self.measures.is_empty() }
}

/// Position within a score: which measure, and which beat within that measure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub measure_idx: MeasureIdx,
    pub beat_idx: BeatIdx,
}

impl Cursor {
    pub fn start() -> Self { Self { measure_idx: 0, beat_idx: 0 } }

    pub fn at(measure_idx: MeasureIdx, beat_idx: BeatIdx) -> Self { Self { measure_idx, beat_idx } }
}

impl Default for Cursor {
    fn default() -> Self { Self::start() }
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
