use crate::{BeatIdx, Measure, VoiceBar, VoiceIdx};
use serde::{Deserialize, Serialize};

pub type MeasureIdx = usize;

/// A collection of one or more measures, on its staff.
///
/// Invariant: `measures` is never empty. Constructors and mutators must uphold this;
/// deserializing checks it (see [`EmptyScore`]).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "RawScore")]
pub struct Score {
    /// The staff the score is written on.
    #[serde(default, skip_serializing_if = "StaffKind::is_line")]
    pub staff: StaffKind,
    pub measures: Vec<Measure>,
}

/// The staff a score is written on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StaffKind {
    /// One line, for a rhythm of one sound.
    #[default]
    Line,
    /// Five lines, for a drum kit: each instrument on its own line or space
    /// ([`crate::Instrument::staff_pos`]).
    Kit,
}

impl StaffKind {
    fn is_line(&self) -> bool {
        *self == StaffKind::Line
    }
}

/// A [`Score`] as saved, before its invariant is checked.
#[derive(Deserialize)]
struct RawScore {
    #[serde(default)]
    staff: StaffKind,
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
        Ok(Self { staff: raw.staff, measures: raw.measures })
    }
}

impl Score {
    /// `measures` on one line. Panics if there are none.
    pub fn new(measures: Vec<Measure>) -> Self {
        assert!(!measures.is_empty(), "a score needs at least one measure");
        Self { staff: StaffKind::Line, measures }
    }

    pub fn single(measure: Measure) -> Self {
        Self::new(vec![measure])
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

    /// The voice `cursor` is in. Panics if `cursor` is not in the score;
    /// [`Score::clamp_cursor`] puts it there.
    pub fn voice_at(&self, cursor: Cursor) -> &VoiceBar {
        self.measures[cursor.measure_idx].voice(cursor.voice).expect("the cursor's voice")
    }

    /// The voice `cursor` is in, to edit; see [`Score::voice_at`].
    pub fn voice_at_mut(&mut self, cursor: Cursor) -> &mut VoiceBar {
        self.measures[cursor.measure_idx].voice_mut(cursor.voice).expect("the cursor's voice")
    }

    /// `cursor` moved onto the score: the last measure, voice or beat where
    /// it is past it. An empty score (only possible by building one by hand,
    /// since [`Score::new`] and loading refuse it) gives [`Cursor::start`].
    pub fn clamp_cursor(&self, cursor: Cursor) -> Cursor {
        let Some(measure_idx) = self.len().checked_sub(1).map(|last| cursor.measure_idx.min(last))
        else {
            return Cursor::start();
        };
        let measure = &self.measures[measure_idx];
        let voice = cursor.voice.min(measure.voices().len().saturating_sub(1));
        let beats = measure.voice(voice).map_or(0, |v| v.beats().len());
        Cursor { measure_idx, voice, beat_idx: cursor.beat_idx.min(beats.saturating_sub(1)) }
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

/// Position within a score: which measure, which of its voices, and which
/// beat of that voice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub measure_idx: MeasureIdx,
    /// Cursors saved before voices are in the first.
    #[serde(default)]
    pub voice: VoiceIdx,
    pub beat_idx: BeatIdx,
}

impl Cursor {
    pub fn start() -> Self {
        Self::at(0, 0)
    }

    /// Beat `beat_idx` of the first voice of measure `measure_idx`.
    pub fn at(measure_idx: MeasureIdx, beat_idx: BeatIdx) -> Self {
        Self::in_voice(measure_idx, 0, beat_idx)
    }

    pub fn in_voice(measure_idx: MeasureIdx, voice: VoiceIdx, beat_idx: BeatIdx) -> Self {
        Self { measure_idx, voice, beat_idx }
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
        let score = Score::new(vec![four.clone(), four.clone()]);
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
    fn a_score_saved_before_staves_is_on_one_line() {
        let line = Score::single(Measure::new(TimeSignature::FOUR_FOUR));
        let saved = ron::to_string(&line).unwrap();
        assert!(!saved.contains("staff"), "a score on one line is saved as before: {saved}");
        let back: Score = ron::from_str(&saved).unwrap();
        assert_eq!(back.staff, StaffKind::Line);
        let kit = Score { staff: StaffKind::Kit, ..line };
        let back: Score = ron::from_str(&ron::to_string(&kit).unwrap()).unwrap();
        assert_eq!(back.staff, StaffKind::Kit);
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
        assert_eq!(c.voice, 0);
        assert_eq!(c.beat_idx, 5);
    }

    #[test]
    fn a_cursor_saved_before_voices_is_in_the_first() {
        let back: Cursor = ron::from_str("(measure_idx: 2, beat_idx: 5)").unwrap();
        assert_eq!(back, Cursor::at(2, 5));
        let lower = Cursor::in_voice(1, 1, 3);
        assert_eq!(ron::from_str::<Cursor>(&ron::to_string(&lower).unwrap()).unwrap(), lower);
    }

    #[test]
    fn clamp_cursor_on_a_hand_built_empty_score_is_the_start() {
        let empty = Score { staff: StaffKind::Line, measures: Vec::new() };
        assert_eq!(empty.clamp_cursor(Cursor::in_voice(3, 1, 2)), Cursor::start());
    }

    #[test]
    fn clamp_cursor_keeps_it_on_the_score() {
        let score = crate::notation::parse_score("4/4 q q q q ; h h | 2/4 q q").unwrap();
        assert_eq!(score.clamp_cursor(Cursor::in_voice(0, 1, 9)), Cursor::in_voice(0, 1, 1));
        // The second measure has one voice.
        assert_eq!(score.clamp_cursor(Cursor::in_voice(1, 1, 1)), Cursor::in_voice(1, 0, 1));
        assert_eq!(score.clamp_cursor(Cursor::in_voice(7, 0, 0)), Cursor::at(1, 0));
        assert_eq!(score.voice_at(Cursor::in_voice(0, 1, 0)).beats().len(), 2);
    }
}
