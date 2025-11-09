use crate::duration::{Duration, NoteValue, default_duration_set};
use crate::fill::best_fill_for_gap;
use crate::beaming::{BeamPlan, compute_beam_plan};
use std::fmt::{Display, Formatter};
use std::vec;

/// Represents a time signature (e.g., 4/4, 3/4, 6/8)
#[derive(Debug, Clone)]
pub struct TimeSignature {
    /// Number of beats per measure
    pub beats: u8,
    /// Note value that represents one beat (4 = quarter note, 8 = eighth note)
    pub beat_unit: u8,
}

impl TimeSignature {
    pub const ONE_FOUR: Self = Self { beats: 1, beat_unit: 4 };
    pub const ONE_SIXTEENTH: Self = Self { beats: 1, beat_unit: 16 };
    pub const TWO_SIXTEENTH: Self = Self { beats: 2, beat_unit: 16 };
    pub const FOUR_FOUR: Self = Self { beats: 4, beat_unit: 4 };
    pub const FOUR_EIGHT: Self = Self { beats: 4, beat_unit: 8 };
    pub const TWO_EIGHT: Self = Self { beats: 2, beat_unit: 8 };
    pub const SEVEN_EIGHT: Self = Self { beats: 7, beat_unit: 8 };

    /// Returns the total duration in integer ticks
    pub fn measure_duration_ticks(&self) -> i32 {
        // Use the unified duration set to derive the grid.
        let set = default_duration_set();
        ((self.beats as i32) * set.grid.ticks_per_whole) / (self.beat_unit as i32)
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum BeatKind {
    Note,
    Rest,
}

#[derive(Copy, Clone, Debug)]
pub struct Beat {
    pub duration: Duration,
    pub kind: BeatKind,
    pub tremolo: Option<Tremolo>,
}

#[derive(Copy, Clone, Debug)]
pub struct Tremolo {
    /// Number of slashes (1..=3 typical)
    pub slashes: u8,
    /// If true, indicates measured tremolo; otherwise unmeasured (for future use)
    pub measured: bool,
}

impl Beat {
    /// Creates a new note with the given duration
    pub fn note(duration: Duration) -> Self { Self { duration, kind: BeatKind::Note, tremolo: None } }

    /// Creates a new rest with the given duration
    pub fn rest(duration: Duration) -> Self { Self { duration, kind: BeatKind::Rest, tremolo: None } }
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
    beam_plan: Option<BeamPlan>,
}

impl Measure {
    /// Creates a new empty measure with the given time signature
    pub fn new(time_signature: TimeSignature) -> Self {
        Self { beats: Vec::new(), time_signature, beam_plan: Some(BeamPlan { groups: vec![] }) }
    }

    /// Expose a read-only view of beats (primarily for tests/inspection)
    pub fn beats(&self) -> &Vec<Beat> { &self.beats }

    /// Expose the time signature (clone)
    pub fn time_signature(&self) -> TimeSignature { self.time_signature.clone() }

    /// Expose the beaming plan for this measure
    pub fn beam_plan(&self) -> Option<&BeamPlan> { self.beam_plan.as_ref() }

    /// Returns the current total duration in ticks (exact)
    fn current_ticks(&self) -> i32 {
        let set = default_duration_set();
        self.beats.iter().map(|beat| set.grid.ticks_of(&beat.duration).unwrap()).sum()
    }

    /// Returns the remaining number of ticks available in this measure
    /// (never negative; 0 when the measure is full)
    pub fn remaining_ticks(&self) -> i32 {
        let max_ticks = self.time_signature.measure_duration_ticks();
        let used = self.current_ticks();
        (max_ticks - used).max(0)
    }

    /// Returns true if the remaining ticks can be exactly filled using the available durations
    fn is_remainder_fillable(remaining_ticks: i32) -> bool {
        if remaining_ticks == 0 {
            return true;
        }
        if remaining_ticks < 0 {
            return false;
        }
        // Build the available coin sizes (ticks) from the supported durations. Larger first helps pruning.
        let set = default_duration_set();
        let mut coins: Vec<i32> =
            set.durations.iter().map(|dur| set.grid.ticks_of(dur).unwrap()).collect();
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
        let set = default_duration_set();
        let current_ticks = self.current_ticks();
        let max_ticks = self.time_signature.measure_duration_ticks();
        let beat_ticks = set.grid.ticks_of(&beat.duration).ok_or_else(|| {
            // If beat cannot be represented on our default grid, treat as unfillable
            MeasureError::Unfillable { attempted: 0.0, remaining: 0.0 }
        })?;
        let new_total_ticks = current_ticks + beat_ticks;

        if new_total_ticks > max_ticks {
            let available_ticks = max_ticks - current_ticks;
            let available = (available_ticks as f64) / (set.grid.ticks_per_whole as f64);
            let attempted = (beat_ticks as f64) / (set.grid.ticks_per_whole as f64);
            return Err(MeasureError::Overflow { attempted, available });
        }

        let remaining_ticks = max_ticks - new_total_ticks;
        if remaining_ticks != 0 && !Self::is_remainder_fillable(remaining_ticks) {
            let remaining = (remaining_ticks as f64) / (set.grid.ticks_per_whole as f64);
            let attempted = (beat_ticks as f64) / (set.grid.ticks_per_whole as f64);
            return Err(MeasureError::Unfillable { attempted, remaining });
        }

        self.beats.push(beat);
        // Recompute beaming plan after mutation
        self.beam_plan = Some(compute_beam_plan(self));
        Ok(())
    }

    /// Replace the beat at `idx` with a rest of the same duration. No-op if out of bounds or already a rest.
    pub fn set_beat_to_rest(&mut self, idx: usize) {
        if let Some(b) = self.beats.get_mut(idx) {
            if b.kind != BeatKind::Rest {
                b.kind = BeatKind::Rest;
                b.tremolo = None; // rests have no tremolo
                // Recompute beams since note/rest membership affects grouping visuals
                self.recompute_beams();
            }
        }
    }

    /// Recompute the beam plan explicitly (optional helper)
    pub fn recompute_beams(&mut self) {
        self.beam_plan = Some(compute_beam_plan(self));
    }

    /// Remove the beat at `idx`. If there is a following beat (i.e., not deleting the last one),
    /// insert a sequence of rests whose total duration equals the removed beat so that the
    /// absolute positions of subsequent beats remain unchanged. No-op if `idx` is out of bounds.
    pub fn backspace_remove_and_fill(&mut self, idx: usize) {
        if idx >= self.beats.len() { return; }
        let set = default_duration_set();
        let had_following = idx + 1 < self.beats.len();
        let removed_ticks = self
            .beats
            .get(idx)
            .and_then(|b| set.grid.ticks_of(&b.duration))
            .unwrap_or(0);
        // Remove the beat at idx
        self.beats.remove(idx);

        // If there was a following beat, fill the removed span with rests to preserve positions
        if had_following && removed_ticks > 0 {
            if let Some(fill) = best_fill_for_gap(removed_ticks) {
                let mut insert_at = idx;
                for d in fill {
                    self.beats.insert(insert_at, Beat::rest(d));
                    insert_at += 1;
                }
            }
        }
        // Recompute beams after mutation
        self.recompute_beams();
    }

    /// Ensure that an absolute position `pos` (0-based) is committed as a real beat.
    /// If `pos` is already within committed beats, this is a no-op.
    /// If `pos` lies within the remainder preview, commit the minimal prefix
    /// of the remainder (as rests) so that `pos` becomes a valid index in `self.beats`.
    pub fn ensure_committed_position(&mut self, pos: usize) {
        let beats_len = self.beats.len();
        if pos < beats_len {
            return; // already committed
        }
        let remaining_ticks = self.remaining_ticks();
        if remaining_ticks <= 0 {
            return; // nothing to commit
        }
        let need = pos.saturating_add(1).saturating_sub(beats_len);
        if let Some(fill) = best_fill_for_gap(remaining_ticks) {
            let take = need.min(fill.len());
            for d in fill.into_iter().take(take) {
                self.beats.push(Beat::rest(d));
            }
            self.recompute_beams();
        }
    }
}

enum DisplayItem {
    Beat(Beat),
    Cursor,
}

impl Display for Measure {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let remainder: Vec<_> = best_fill_for_gap(self.remaining_ticks())
            .unwrap_or_default()
            .iter()
            .map(|d| DisplayItem::Beat(Beat::rest(*d)))
            .collect();

        let mut beats: Vec<_> = self.beats.iter()
            .map(|b| DisplayItem::Beat(*b))
            .collect();
        beats.append(&mut vec![DisplayItem::Cursor]);
        beats.extend(remainder);

        beats.iter().fold(Ok(()), |result, b| {
            result.and_then(|_| {
                match b {
                    DisplayItem::Beat(beat) => {
                        let (note, rest) = beat.duration.to_glyph();
                        let glyph = if beat.kind == BeatKind::Note { note } else { rest };
                        write!(f, "{}", glyph).and_then(|_| match beat.duration {
                            Duration::Simple(_) => Ok(()),
                            Duration::Dotted { base: _base, dots } => {
                                write!(f, "{}", "\u{1D16D}".repeat(dots as usize))
                            }
                            Duration::Tuplet { .. } => write!(f, "ᵀ"),
                        })
                    }
                    DisplayItem::Cursor => { write!(f, "|") }
                }
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

    #[test]
    fn test_add_eighth_triplet_to_seven_eight_measure() {
        let mut measure = Measure::new(TimeSignature::SEVEN_EIGHT);
        let t8 = Duration::Tuplet { n: 3, m: 2, base: NoteValue::Eighth };
        measure.add_beat(Beat::note(t8)).unwrap();
        measure.add_beat(Beat::note(t8)).unwrap();
        measure.add_beat(Beat::note(t8)).unwrap();

        // measure.current_ticks()

        // measure.add_beat(Beat::note(Duration::Simple(NoteValue::Eighth))).unwrap();

        println!("{}", measure.remaining_ticks());
        println!("{}", measure);
    }
}


impl Measure {
    /// Toggle the beat kind at `idx` between Note and Rest while preserving duration.
    /// No-op if `idx` is out of bounds.
    pub fn toggle_beat_kind(&mut self, idx: usize) {
        if let Some(b) = self.beats.get_mut(idx) {
            // Clear tremolo in both cases to avoid invalid state on rests
            b.tremolo = None;
            b.kind = match b.kind {
                BeatKind::Rest => BeatKind::Note,
                BeatKind::Note => BeatKind::Rest,
            };
            // Beaming may change when toggling between note/rest
            self.recompute_beams();
        }
    }
}
