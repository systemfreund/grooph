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


    /// Returns the total duration in integer ticks
    pub fn measure_duration_ticks(&self) -> i32 {
        // number of whole-note fractions: beats / beat_unit of a whole note
        // Convert to ticks: (beats * TICKS_PER_WHOLE) / beat_unit
        ((self.beats as i32) * Duration::TICKS_PER_WHOLE) / (self.beat_unit as i32)
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
    /// Ticks per whole note. Choose LCM of denominators used by all durations.
    pub const TICKS_PER_WHOLE: i32 = 10080; // lcm(4,8,12,16,20,24,28,32,36)


    /// Returns the duration in integer ticks (exact)
    pub fn ticks(&self) -> i32 {
        match self {
            Duration::Quarter => Self::TICKS_PER_WHOLE / 4,           // 2520
            Duration::Eighth => Self::TICKS_PER_WHOLE / 8,            // 1260
            Duration::TripletEighth => Self::TICKS_PER_WHOLE / 12,    // 840
            Duration::Sixteenth => Self::TICKS_PER_WHOLE / 16,        // 630
            Duration::QuintupletSixteenth => Self::TICKS_PER_WHOLE / 20, // 504
            Duration::SextupletSixteenth => Self::TICKS_PER_WHOLE / 24,  // 420
            Duration::SeptupletSixteenth => Self::TICKS_PER_WHOLE / 28,  // 360
            Duration::ThirtySecond => Self::TICKS_PER_WHOLE / 32,     // 315
            Duration::NonupletThirtySecond => Self::TICKS_PER_WHOLE / 36, // 280
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

}

/// Errors that can occur when adding beats to a measure
#[derive(Debug, PartialEq)]
pub enum MeasureError {
    /// The beat would cause the measure to exceed its time signature
    Overflow {
        /// Duration that was attempted to add (fraction of a whole note)
        attempted: f64,
        /// Space available in the measure (fraction of a whole note)
        available: f64,
    },
    /// The beat would leave a remainder that cannot be exactly filled with available durations
    Unfillable {
        /// Duration that was attempted to add (fraction of a whole note)
        attempted: f64,
        /// Remaining space after the attempted add (fraction of a whole note)
        remaining: f64,
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


    /// Returns the current total duration in ticks (exact)
    fn current_ticks(&self) -> i32 {
        self.beats.iter().map(|beat| beat.duration.ticks()).sum()
    }

    /// Returns true if the remaining ticks can be exactly filled using the available durations
    fn is_remainder_fillable(remaining_ticks: i32) -> bool {
        if remaining_ticks == 0 { return true; }
        if remaining_ticks < 0 { return false; }
        // Available coin sizes (ticks) in non-increasing order for early pruning
        const COINS: [i32; 9] = [
            Duration::TICKS_PER_WHOLE / 4,   // Quarter = 2520
            Duration::TICKS_PER_WHOLE / 8,   // Eighth = 1260
            Duration::TICKS_PER_WHOLE / 12,  // TripletEighth = 840
            Duration::TICKS_PER_WHOLE / 16,  // Sixteenth = 630
            Duration::TICKS_PER_WHOLE / 20,  // Quintuplet 16th = 504
            Duration::TICKS_PER_WHOLE / 24,  // Sextuplet 16th = 420
            Duration::TICKS_PER_WHOLE / 28,  // Septuplet 16th = 360
            Duration::TICKS_PER_WHOLE / 32,  // ThirtySecond = 315
            Duration::TICKS_PER_WHOLE / 36,  // Nonuplet 32nd = 280
        ];
        // Simple DP (unbounded knapsack reachability)
        let target = remaining_ticks as usize;
        let mut dp = vec![false; target + 1];
        dp[0] = true;
        for i in 1..=target {
            let mut reachable = false;
            for &c in COINS.iter() {
                let cu = c as usize;
                if cu <= i && dp[i - cu] {
                    reachable = true;
                    break;
                }
            }
            dp[i] = reachable;
        }
        dp[target]
    }

    /// Adds a beat to this measure if it doesn't exceed the time signature and remains completable
    ///
    /// # Returns
    /// - `Ok(())` if the beat was successfully added
    /// - `Err(MeasureError::Overflow)` if adding the beat would exceed the measure's capacity
    /// - `Err(MeasureError::Unfillable)` if the addition leaves an unfillable remainder
    pub fn add_beat(&mut self, beat: Beat) -> Result<(), MeasureError> {
        let current_ticks = self.current_ticks();
        let max_ticks = self.time_signature.measure_duration_ticks();
        let beat_ticks = beat.duration.ticks();
        let new_total_ticks = current_ticks + beat_ticks;

        if new_total_ticks > max_ticks {
            let available_ticks = max_ticks - current_ticks;
            let available = (available_ticks as f64) / (Duration::TICKS_PER_WHOLE as f64);
            let attempted = (beat_ticks as f64) / (Duration::TICKS_PER_WHOLE as f64);
            return Err(MeasureError::Overflow { attempted, available });
        }

        let remaining_ticks = max_ticks - new_total_ticks;
        if remaining_ticks != 0 && !Self::is_remainder_fillable(remaining_ticks) {
            let remaining = (remaining_ticks as f64) / (Duration::TICKS_PER_WHOLE as f64);
            let attempted = (beat_ticks as f64) / (Duration::TICKS_PER_WHOLE as f64);
            return Err(MeasureError::Unfillable { attempted, remaining });
        }

        self.beats.push(beat);
        Ok(())
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
