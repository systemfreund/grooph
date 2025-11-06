use crate::duration::Duration;

/// Represents a time signature (e.g., 4/4, 3/4, 6/8)
#[derive(Debug, Clone)]
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

    pub const ONE_SIXTEENTH: Self = Self {
        beats: 1,
        beat_unit: 16,
    };

    pub const FOUR_FOUR: Self = Self {
        beats: 4,
        beat_unit: 4,
    };

    pub const FOUR_EIGHT: Self = Self {
        beats: 4,
        beat_unit: 8,
    };

    pub const SEVEN_EIGHT: Self = Self {
        beats: 7,
        beat_unit: 8,
    };

    /// Returns the total duration in integer ticks
    pub fn measure_duration_ticks(&self) -> i32 {
        // number of whole-note fractions: beats / beat_unit of a whole note
        // Convert to ticks: (beats * TICKS_PER_WHOLE) / beat_unit
        ((self.beats as i32) * Duration::TICKS_PER_WHOLE) / (self.beat_unit as i32)
    }
}

#[derive(Debug)]
pub enum BeatKind {
    Note,
    Rest,
}

#[derive(Debug)]
pub struct Beat {
    pub duration: Duration,
    pub kind: BeatKind,
}

impl Beat {
    /// Creates a new note with the given duration
    pub fn note(duration: Duration) -> Self {
        Self {
            duration,
            kind: BeatKind::Note,
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
#[derive(Debug)]
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

    /// Expose a read-only view of beats (primarily for tests/inspection)
    pub fn beats(&self) -> &Vec<Beat> { &self.beats }

    /// Returns the current total duration in ticks (exact)
    fn current_ticks(&self) -> i32 {
        self.beats.iter().map(|beat| beat.duration.ticks()).sum()
    }

    /// Returns true if the remaining ticks can be exactly filled using the available durations
    fn is_remainder_fillable(remaining_ticks: i32) -> bool {
        if remaining_ticks == 0 { return true; }
        if remaining_ticks < 0 { return false; }
        // Build the available coin sizes (ticks) from the supported durations. Larger first helps pruning.
        let mut coins: Vec<i32> = Duration::DURATIONS
            .iter()
            .map(|&dur| Duration::TICKS_PER_WHOLE / Duration::denominator_of(dur))
            .collect();
        coins.sort_unstable_by(|a, b| b.cmp(a));

        // Simple DP (unbounded knapsack reachability)
        let target = remaining_ticks as usize;
        let mut dp = vec![false; target + 1];
        dp[0] = true;
        for i in 1..=target {
            let mut reachable = false;
            for &c in coins.iter() {
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
    use super::*;
    use crate::duration::Duration::{Eighth, Quarter, Sixteenth, ThirtySecond, TripletEighth};

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
        assert!(measure.add_beat(Beat::note(ThirtySecond)).is_err());
    }
}
