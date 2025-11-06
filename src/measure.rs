use std::fmt::{Display, Formatter};
use crate::duration::{Duration, NoteValue, Grid, COMMON_DURATIONS, default_grid};

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

    pub const TWO_SIXTEENTH: Self = Self {
        beats: 2,
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

    pub const TWO_EIGHT: Self = Self {
        beats: 2,
        beat_unit: 8,
    };

    pub const SEVEN_EIGHT: Self = Self {
        beats: 7,
        beat_unit: 8,
    };

    /// Returns the total duration in integer ticks
    pub fn measure_duration_ticks(&self) -> i32 {
        // Dynamic grid built from common durations, can be swapped to a fixed constant later
        let grid = default_grid();
        ((self.beats as i32) * grid.ticks_per_whole) / (self.beat_unit as i32)
    }
}

#[derive(Debug, PartialEq)]
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

    const fn to_glyph(beat: &Beat) -> (&'static str, &'static str) {
        // Glyphs are determined by base note value only; tuplets/rests share the same shapes
        match beat.duration.base_note() {
            NoteValue::Quarter => ("𝅘𝅥", "𝄽"),
            NoteValue::Eighth => ("𝅘𝅥𝅮", "𝄾"),
            NoteValue::Sixteenth => ("𝅘𝅥𝅯", "𝄿"),
            NoteValue::ThirtySecond => ("𝅘𝅥𝅰", "𝅀"),
            NoteValue::Half | NoteValue::Whole => ("𝅝", "𝄻"), // fallback; not used yet
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
        let grid = default_grid();
        self.beats.iter().map(|beat| grid.ticks_of(&beat.duration).unwrap()).sum()
    }

    /// Returns true if the remaining ticks can be exactly filled using the available durations
    fn is_remainder_fillable(remaining_ticks: i32) -> bool {
        if remaining_ticks == 0 { return true; }
        if remaining_ticks < 0 { return false; }
        // Build the available coin sizes (ticks) from the supported durations. Larger first helps pruning.
        let grid = default_grid();
        let mut coins: Vec<i32> = COMMON_DURATIONS
            .iter()
            .map(|dur| grid.ticks_of(dur).unwrap())
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
        let grid = default_grid();
        let current_ticks = self.current_ticks();
        let max_ticks = self.time_signature.measure_duration_ticks();
        let beat_ticks = grid.ticks_of(&beat.duration).ok_or_else(|| {
            // If beat cannot be represented on our default grid, treat as unfillable
            MeasureError::Unfillable { attempted: 0.0, remaining: 0.0 }
        })?;
        let new_total_ticks = current_ticks + beat_ticks;

        if new_total_ticks > max_ticks {
            let available_ticks = max_ticks - current_ticks;
            let available = (available_ticks as f64) / (grid.ticks_per_whole as f64);
            let attempted = (beat_ticks as f64) / (grid.ticks_per_whole as f64);
            return Err(MeasureError::Overflow { attempted, available });
        }

        let remaining_ticks = max_ticks - new_total_ticks;
        if remaining_ticks != 0 && !Self::is_remainder_fillable(remaining_ticks) {
            let remaining = (remaining_ticks as f64) / (grid.ticks_per_whole as f64);
            let attempted = (beat_ticks as f64) / (grid.ticks_per_whole as f64);
            return Err(MeasureError::Unfillable { attempted, remaining });
        }

        self.beats.push(beat);
        Ok(())
    }
}

impl Display for Measure {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.beats.iter().fold(Ok(()), |result, b| {
            result.and_then(|_| {
                let (note, rest) = Beat::to_glyph(b);
                let glyph = if b.kind == BeatKind::Note { note } else { rest };
                write!(f, "{}", glyph)
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duration::{Duration, NoteValue};
    fn q() -> Duration { Duration::Simple(NoteValue::Quarter) }
    fn e() -> Duration { Duration::Simple(NoteValue::Eighth) }
    fn t8() -> Duration { Duration::Tuplet { n: 3, m: 2, base: NoteValue::Eighth } }
    fn s16() -> Duration { Duration::Simple(NoteValue::Sixteenth) }
    fn t32() -> Duration { Duration::Simple(NoteValue::ThirtySecond) }

    #[test]
    fn test_add_quarter_note_to_one_four_measure() {
        let mut measure = Measure::new(TimeSignature::ONE_FOUR);

        let result = measure.add_beat(Beat::note(q()));

        assert!(result.is_ok());
    }

    #[test]
    fn test_triplet() {
        let mut measure = Measure::new(TimeSignature::ONE_FOUR);

        assert!(measure.add_beat(Beat::note(t8())).is_ok());
        assert!(measure.add_beat(Beat::rest(t8())).is_ok());
        assert!(measure.add_beat(Beat::note(q())).is_err());
        assert!(measure.add_beat(Beat::note(e())).is_err());
        assert!(measure.add_beat(Beat::note(s16())).is_err());
        assert!(measure.add_beat(Beat::note(t32())).is_err());
    }
}
