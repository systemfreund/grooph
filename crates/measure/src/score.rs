use crate::grid::DEFAULT_GRID;
use crate::{BeatIdx, Measure, VoiceBar, VoiceIdx};
use serde::{Deserialize, Serialize};

pub type MeasureIdx = usize;

/// A collection of one or more measures, on its staff.
///
/// Invariant: `measures` is never empty. The field is private so that only
/// the constructors and mutators here, which uphold it, change the list;
/// deserializing checks it (see [`EmptyScore`]).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "RawScore")]
pub struct Score {
    /// The staff the score is written on.
    #[serde(default, skip_serializing_if = "StaffKind::is_line")]
    pub staff: StaffKind,
    measures: Vec<Measure>,
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

    /// The score on `staff` instead.
    pub fn with_staff(self, staff: StaffKind) -> Self {
        Self { staff, ..self }
    }

    /// The measures, at least one.
    pub fn measures(&self) -> &[Measure] {
        &self.measures
    }

    /// The measures, to edit in place; adding or removing one goes through
    /// [`Score::push`], [`Score::insert`], [`Score::remove`] and
    /// [`Score::truncate`], which keep at least one.
    pub fn measures_mut(&mut self) -> &mut [Measure] {
        &mut self.measures
    }

    /// Strike each drum of the kit by one voice at a time in every measure
    /// (see [`Measure::undouble_instruments`]). Reports whether one was
    /// doubled.
    pub fn undouble_instruments(&mut self) -> bool {
        self.measures.iter_mut().fold(false, |any, m| m.undouble_instruments() | any)
    }

    /// The measures, taken out of the score: at least one.
    pub fn into_measures(self) -> Vec<Measure> {
        self.measures
    }

    /// Add `measure` at the end.
    pub fn push(&mut self, measure: Measure) {
        self.measures.push(measure);
    }

    /// Add `measure` at `idx`, shifting the ones after it. Panics if `idx`
    /// is past the end, like [`Vec::insert`].
    pub fn insert(&mut self, idx: MeasureIdx, measure: Measure) {
        self.measures.insert(idx, measure);
    }

    /// Take out measure `idx`; `None`, and no change, if it is the only one
    /// or there is no such measure.
    pub fn remove(&mut self, idx: MeasureIdx) -> Option<Measure> {
        (self.measures.len() > 1 && idx < self.measures.len()).then(|| self.measures.remove(idx))
    }

    /// Keep the first `len` measures, and always the first one.
    pub fn truncate(&mut self, len: usize) {
        self.measures.truncate(len.max(1));
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

    /// `cursor` moved to `voice` of its measure, onto the beat sounding when
    /// the cursor's beat starts; the last voice where the measure has fewer
    /// (see [`Score::clamp_cursor`]).
    pub fn cursor_in_voice(&self, cursor: Cursor, voice: VoiceIdx) -> Cursor {
        let from = self.clamp_cursor(cursor);
        let to = self.clamp_cursor(Cursor { voice, ..from });
        let onsets = |c: Cursor| DEFAULT_GRID.compute_onset_ticks(self.voice_at(c).beats());
        let tick = onsets(from).get(from.beat_idx).copied().unwrap_or(0);
        let beat_idx = onsets(to).iter().take_while(|&&t| t <= tick).count().saturating_sub(1);
        Cursor { beat_idx, ..to }
    }

    /// Whether a note of any voice plays an instrument of the kit
    /// ([`Beat::is_kit`](crate::Beat::is_kit)).
    pub fn has_kit_heads(&self) -> bool {
        self.measures.iter().flat_map(|m| m.voices()).flat_map(|v| v.beats()).any(|b| b.is_kit())
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

    /// Beat `beat_idx` of voice `voice` in measure `measure_idx`.
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
    fn a_drum_is_struck_by_one_voice_at_a_time() {
        use crate::Instrument;
        use crate::notation::parse_score;
        let doubled = parse_score("4/4 S-H:q H:q H:h ; S:q K:q K:h").unwrap();
        assert_eq!(doubled.measures()[0].doubled_instruments(), vec![(0, Instrument::Snare)]);
        // The same drum at another time, or another drum at the same time.
        let fine = parse_score("4/4 S-H:e H:e H:q H:h ; r:e S:e K:q K:h").unwrap();
        assert!(fine.measures()[0].doubled_instruments().is_empty());
        // The metronome's click and counter rhythm sound together.
        let rhythms = parse_score("4/4 q q q q ; q q q q").unwrap();
        assert!(rhythms.measures()[0].doubled_instruments().is_empty());
    }

    #[test]
    fn a_doubled_drum_stays_in_its_voice() {
        use crate::notation::{format_score, parse_score};
        let undoubled = |text: &str| {
            let mut score = parse_score(text).unwrap();
            let changed = score.undouble_instruments();
            assert!(score.measures()[0].doubled_instruments().is_empty());
            (changed, format_score(&score).unwrap())
        };
        // The snare stays above, its accent going along; the kick below.
        assert_eq!(
            undoubled("4/4 S-H:q H:q H:h ; >S:q K:q K:h"),
            (true, "4/4 >S-H:q H:q H:h ; r:q K:q K:h".to_owned())
        );
        assert_eq!(
            undoubled("4/4 K-H:q H:q H:h ; K:q K:q K:h"),
            (true, "4/4 H:q H:q H:h ; K:q K:q K:h".to_owned())
        );
        let fine = "4/4 S-H:e H:e H:q H:h ; r:e S:e K:q K:h";
        assert_eq!(undoubled(fine), (false, format_score(&parse_score(fine).unwrap()).unwrap()));
    }

    #[test]
    fn a_cursor_moves_to_the_beat_sounding_in_another_voice() {
        let mut score = crate::notation::parse_score("4/4 e e e e e e e e ; q q h").unwrap();
        let on = |s: &Score, beat, voice| s.cursor_in_voice(Cursor::in_voice(0, 0, beat), voice);
        assert_eq!(on(&score, 0, 1), Cursor::in_voice(0, 1, 0));
        assert_eq!(on(&score, 3, 1), Cursor::in_voice(0, 1, 1));
        assert_eq!(on(&score, 7, 1), Cursor::in_voice(0, 1, 2));
        assert_eq!(score.cursor_in_voice(Cursor::in_voice(0, 1, 2), 0), Cursor::in_voice(0, 0, 4));
        // A measure of one voice stays in it, until it has the other.
        let mut one = crate::notation::parse_score("4/4 e e e e e e e e").unwrap();
        assert_eq!(on(&one, 5, 1), Cursor::in_voice(0, 0, 5));
        one.measures_mut()[0].fill_voices(2);
        assert_eq!(one.measures()[0].voices().len(), 2);
        assert_eq!(on(&one, 5, 1), Cursor::in_voice(0, 1, 2));
        score.measures_mut()[0].fill_voices(2);
        assert_eq!(score.measures()[0].voices().len(), 2);
    }

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
        assert_eq!(other_meter.measures[1].first_voice().beats(), four.first_voice().beats());
        assert!(!score.same_beats(&other_meter));

        let mut other_beat = score.clone();
        other_beat.measures[1].first_voice_mut().toggle_beat_kind(0, crate::Instrument::Rhythm);
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
    fn remove_and_truncate_keep_a_measure() {
        use crate::TimeSignature;
        let one = || Measure::new(TimeSignature::FOUR_FOUR);
        let mut score = Score::single(one());
        assert!(score.remove(0).is_none());
        assert_eq!(score.len(), 1);
        score.push(Measure::new(TimeSignature::THREE_FOUR));
        assert!(score.remove(5).is_none());
        assert_eq!(score.remove(0).map(|m| m.time_signature()), Some(TimeSignature::FOUR_FOUR));
        assert_eq!(score.len(), 1);
        score.push(one());
        score.push(one());
        score.truncate(0);
        assert_eq!(score.len(), 1);
        assert_eq!(score.measures()[0].time_signature(), TimeSignature::THREE_FOUR);
    }

    #[test]
    fn kit_heads_are_found_in_any_voice() {
        use crate::notation::parse_score;
        assert!(!parse_score("4/4 q q q q").unwrap().has_kit_heads());
        assert!(!parse_score("kit 4/4 q q q q").unwrap().has_kit_heads());
        assert!(parse_score("4/4 q q q q ; K:h K:h").unwrap().has_kit_heads());
        let line = parse_score("4/4 q q q q").unwrap();
        assert!(!line.measures[0].first_voice().beats()[0].is_kit());
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
