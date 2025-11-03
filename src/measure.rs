/// Represents a time signature (e.g., 4/4, 3/4, 6/8)
pub struct TimeSignature {
    /// Number of beats per measure
    pub beats: u8,
    /// Note value that represents one beat (4 = quarter note, 8 = eighth note)
    pub beat_unit: u8,
}

impl TimeSignature {
    pub const ONE_FOUR: Self = Self {
        beats: 1,
        beat_unit: 4,
    };

    pub const FOUR_FOUR: Self = Self {
        beats: 4,
        beat_unit: 4,
    };

    /// Returns the total duration of this measure as a fraction of a whole note
    pub fn measure_duration(&self) -> f64 {
        let beat_value = 1.0 / (self.beat_unit as f64);
        (self.beats as f64) * beat_value
    }
}

pub enum Sticking {
    R,
    L,
}

pub enum Duration {
    Quarter,
    Eighth,
    TripletEighth,
    Sixteenth,
    QuintupletSixteenth,
    SextupletSixteenth,
    SeptupletSixteenth,
    ThirtySecond,
    NonupletThirtySecond,
}

impl Duration {
    /// Returns the duration as a fraction of a whole note
    pub fn value(&self) -> f64 {
        match self {
            Duration::Quarter => 0.25,
            Duration::Eighth => 0.125,
            Duration::TripletEighth => 1.0 / 12.0, // 1/3 of a quarter
            Duration::Sixteenth => 0.0625,
            Duration::QuintupletSixteenth => 0.05, // 1/5 of a quarter
            Duration::SextupletSixteenth => 1.0 / 24.0, // 1/6 of a quarter
            Duration::SeptupletSixteenth => 1.0 / 28.0, // 1/7 of a quarter
            Duration::ThirtySecond => 0.03125,
            Duration::NonupletThirtySecond => 1.0 / 36.0, // 1/9 of a quarter
        }
    }
}

pub enum BeatKind {
    Note(Option<Sticking>),
    Rest,
}

pub struct Beat {
    pub duration: Duration,
    pub kind: BeatKind,
}

impl Beat {
    /// Creates a new note with the given duration and sticking
    pub fn note_with_sticking(duration: Duration, sticking: Sticking) -> Self {
        Self {
            duration,
            kind: BeatKind::Note(Some(sticking)),
        }
    }

    /// Creates a new note with the given duration and no sticking
    pub fn note(duration: Duration) -> Self {
        Self {
            duration,
            kind: BeatKind::Note(None),
        }
    }

    /// Creates a new rest with the given duration
    pub fn rest(duration: Duration) -> Self {
        Self {
            duration,
            kind: BeatKind::Rest,
        }
    }

    /// Returns the duration of this beat as a fraction of a whole note
    pub fn duration(&self) -> f64 {
        self.duration.value()
    }
}

/// Errors that can occur when adding beats to a measure
#[derive(Debug, PartialEq)]
pub enum MeasureError {
    /// The beat would cause the measure to exceed its time signature
    Overflow {
        /// Duration that was attempted to add
        attempted: f64,
        /// Space available in the measure
        available: f64,
    },
}

/// Represents a musical measure containing a sequence of beats
pub struct Measure {
    beats: Vec<Beat>,
    time_signature: TimeSignature,
}

impl Measure {
    /// Creates a new empty measure with the given time signature
    pub fn new(time_signature: TimeSignature) -> Self {
        Self {
            beats: Vec::new(),
            time_signature,
        }
    }

    /// Returns the current total duration of all beats in this measure
    fn current_duration(&self) -> f64 {
        self.beats.iter().map(|beat| beat.duration()).sum()
    }

    /// Adds a beat to this measure if it doesn't exceed the time signature
    ///
    /// # Returns
    /// - `Ok(())` if the beat was successfully added
    /// - `Err(MeasureError::Overflow)` if adding the beat would exceed the measure's capacity
    pub fn add_beat(&mut self, beat: Beat) -> Result<(), MeasureError> {
        let current = self.current_duration();
        let max_duration = self.time_signature.measure_duration();
        let beat_duration = beat.duration();
        let new_total = current + beat_duration;

        if new_total > max_duration {
            let available = max_duration - current;
            Err(MeasureError::Overflow {
                attempted: beat_duration,
                available,
            })
        } else {
            self.beats.push(beat);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use Duration::Quarter;
    use crate::measure::Duration::{Eighth, Sixteenth, TripletEighth};
    use super::*;

    #[test]
    fn test_add_quarter_note_to_one_four_measure() {
        let mut measure = Measure::new(TimeSignature::ONE_FOUR);

        let result = measure.add_beat(Beat::note(Quarter));

        assert!(result.is_ok());
    }

    #[test]
    fn test_triplet() {
        let mut measure = Measure::new(TimeSignature::ONE_FOUR);

        assert!(measure.add_beat(Beat::note(TripletEighth)).is_ok());
        assert!(measure.add_beat(Beat::rest(TripletEighth)).is_ok());
        assert!(measure.add_beat(Beat::note(Quarter)).is_err());
        assert!(measure.add_beat(Beat::note(Eighth)).is_err());
        assert!(measure.add_beat(Beat::note(Sixteenth)).is_err());
    }
}
