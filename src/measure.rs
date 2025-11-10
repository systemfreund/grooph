use crate::beaming::{BeamPlan, compute_beam_plan};
use crate::duration::{Duration, NoteValue, default_duration_set};
use crate::fill::best_fill_for_gap;
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
    pub fn note(duration: Duration) -> Self {
        Self { duration, kind: BeatKind::Note, tremolo: None }
    }

    /// Creates a new rest with the given duration
    pub fn rest(duration: Duration) -> Self {
        Self { duration, kind: BeatKind::Rest, tremolo: None }
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
    beam_plan: Option<BeamPlan>,
}

impl Measure {
    /// Creates a new empty measure with the given time signature
    pub fn new(time_signature: TimeSignature) -> Self {
        Self { beats: Vec::new(), time_signature, beam_plan: Some(BeamPlan { groups: vec![] }) }
    }

    /// Expose a read-only view of beats
    pub fn beats(&self) -> &Vec<Beat> { &self.beats }

    /// Split the beat at `idx` into two equal halves (by time), replacing it with two smaller beats.
    /// Only supported for simple durations down to Sixteenth; returns false if not possible.
    pub fn split_beat_by_two(&mut self, idx: usize) -> bool {
        if idx >= self.beats.len() { return false; }
        let base = self.beats[idx];
        let half = match base.duration.halve_simple() { Some(h) => h, None => return false };
        // Replace current with first half
        self.beats[idx].duration = half;
        self.beats[idx].tremolo = None;
        // Insert second half with same kind just after
        let second = Beat { duration: half, kind: base.kind, tremolo: None };
        self.beats.insert(idx + 1, second);
        self.recompute_beams();
        true
    }

    /// Unsplit (merge) the beat at `idx` with the immediately following beat if both are equal simple durations.
    /// This is the inverse of `split_beat_by_two`, e.g., two eighths -> one quarter. Returns true if merged.
    pub fn unsplit_beat_by_two(&mut self, idx: usize) -> bool {
        if idx + 1 >= self.beats.len() { return false; }
        let left = self.beats[idx];
        let right = self.beats[idx + 1];
        // Must be same kind and same duration to merge cleanly
        if left.kind != right.kind { return false; }
        if left.duration != right.duration { return false; }
        let doubled = match left.duration.double_simple() { Some(d) => d, None => return false };
        // Perform merge: set doubled duration at idx, clear tremolo, remove idx+1
        self.beats[idx].duration = doubled;
        self.beats[idx].tremolo = None;
        self.beats.remove(idx + 1);
        self.recompute_beams();
        true
    }

    /// Set (replace) the beat at index `idx` with `beat` if it fits and the remainder stays fillable.
    /// Caller should ensure the position exists (e.g., via `ensure_committed_position`).
    pub fn set_beat_at(&mut self, idx: usize, beat: Beat) -> Result<(), MeasureError> {
        if idx >= self.beats.len() {
            // Out of bounds after caller's ensure: treat as unfillable generically
            return Err(MeasureError::Unfillable { attempted: 0.0, remaining: 0.0 });
        }
        let set = default_duration_set();
        let current_ticks = self.current_ticks();
        let max_ticks = self.time_signature.measure_duration_ticks();
        let new_ticks = set.grid.ticks_of(&beat.duration).ok_or_else(|| {
            MeasureError::Unfillable { attempted: 0.0, remaining: 0.0 }
        })?;
        let old_ticks = set.grid.ticks_of(&self.beats[idx].duration).unwrap_or(0);
        let new_total_ticks = current_ticks - old_ticks + new_ticks;
        if new_total_ticks > max_ticks {
            let available_ticks = (max_ticks - (current_ticks - old_ticks)).max(0);
            let available = (available_ticks as f64) / (set.grid.ticks_per_whole as f64);
            let attempted = (new_ticks as f64) / (set.grid.ticks_per_whole as f64);
            return Err(MeasureError::Overflow { attempted, available });
        }
        let remaining_ticks = max_ticks - new_total_ticks;
        if remaining_ticks != 0 && !Self::is_remainder_fillable(remaining_ticks) {
            let remaining = (remaining_ticks as f64) / (set.grid.ticks_per_whole as f64);
            let attempted = (new_ticks as f64) / (set.grid.ticks_per_whole as f64);
            return Err(MeasureError::Unfillable { attempted, remaining });
        }
        self.beats[idx] = beat;
        self.recompute_beams();
        Ok(())
    }

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

    /// Recompute the beam plan explicitly
    pub fn recompute_beams(&mut self) { self.beam_plan = Some(compute_beam_plan(self)); }

    /// Remove the beat at `idx`. If there is a following beat (i.e., not deleting the last one),
    /// insert a sequence of rests whose total duration equals the removed beat so that the
    /// absolute positions of subsequent beats remain unchanged. No-op if `idx` is out of bounds.
    pub fn remove(&mut self, idx: usize) {
        if idx >= self.beats.len() {
            return;
        }
        let set = default_duration_set();
        let had_following = idx + 1 < self.beats.len();
        let removed_ticks =
            self.beats.get(idx).and_then(|b| set.grid.ticks_of(&b.duration)).unwrap_or(0);
        // Remove the beat at idx
        self.beats.remove(idx);

        // If there was a following beat, fill the removed span with rests to preserve positions
        if had_following && removed_ticks > 0 {
            if let Some(fill) = best_fill_for_gap(removed_ticks, &[]) {
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

    /// Delete the beat at `idx` and shift all subsequent committed beats left (like the Delete key
    /// in a text editor). No-op if `idx` is out of bounds.
    pub fn delete_shift_left(&mut self, idx: usize) {
        if idx >= self.beats.len() {
            return;
        }
        self.beats.remove(idx);
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
        self.fill_measure(Some(pos.saturating_add(1).saturating_sub(beats_len)));
    }

    pub fn fill_measure(&mut self, need: Option<usize>) {
        let remaining_ticks = self.remaining_ticks();
        if remaining_ticks <= 0 {
            return; // nothing to commit
        }
        if let Some(fill) = best_fill_for_gap(remaining_ticks, &[]) {
            let take = need.map_or(fill.len(), |n| n.min(fill.len()));
            for d in fill.into_iter().take(take) {
                self.beats.push(Beat::rest(d));
            }
            self.recompute_beams();
        }
    }

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

    /// Toggle dotted (one dot) for the beat at `idx`.
    /// - Simple(base) -> Dotted { base, dots: 1 }
    /// - Dotted { base, dots: 1 } -> Simple(base)
    /// No-op for other cases (tuplets, multi-dot) or if replacement doesn't fit.
    /// Returns true if the duration changed.
    pub fn toggle_dotted_at(&mut self, idx: usize) -> bool {
        if idx >= self.beats.len() { return false; }
        let current = self.beats[idx];
        let new_dur = match current.duration {
            Duration::Simple(base) => Some(Duration::Dotted { base, dots: 1 }),
            Duration::Dotted { base, dots } if dots == 1 => Some(Duration::Simple(base)),
            _ => None,
        };
        if let Some(dur) = new_dur {
            let new_beat = Beat { duration: dur, kind: current.kind, tremolo: None };
            if self.set_beat_at(idx, new_beat).is_ok() {
                return true;
            }
        }
        false
    }
}

enum DisplayItem {
    Beat(Beat),
    Cursor,
}

impl Display for Measure {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let remainder: Vec<_> = best_fill_for_gap(self.remaining_ticks(), &[])
            .unwrap_or_default()
            .iter()
            .map(|d| DisplayItem::Beat(Beat::rest(*d)))
            .collect();

        let mut beats: Vec<_> = self.beats.iter().map(|b| DisplayItem::Beat(*b)).collect();
        beats.append(&mut vec![DisplayItem::Cursor]);
        beats.extend(remainder);

        beats.iter().fold(Ok(()), |result, b| {
            result.and_then(|_| match b {
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
                DisplayItem::Cursor => {
                    write!(f, "|")
                }
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duration::NoteValue::{Eighth, Sixteenth, ThirtySecond};
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
        let t8 = Duration::Tuplet { n: 3, m: 2, base: Eighth };
        let t16 = Duration::Tuplet { n: 6, m: 4, base: Sixteenth };
        measure.add_beat(Beat::note(Duration::Dotted { base: Eighth, dots: 1 })).unwrap();
        measure.add_beat(Beat::note((Duration::Dotted { base: Sixteenth, dots: 1 }))).unwrap();
        measure.add_beat(Beat::note(Duration::Simple(ThirtySecond))).unwrap();
        measure.add_beat(Beat::note(Duration::Simple(Sixteenth))).unwrap();
        measure.add_beat(Beat::note(Duration::Simple(Eighth))).unwrap();
        measure.add_beat(Beat::note(Duration::Simple(Sixteenth))).unwrap();
        measure.add_beat(Beat::note((Duration::Dotted { base: ThirtySecond, dots: 1 }))).unwrap();
        measure.add_beat(Beat::rest(Duration::Dotted { base: Eighth, dots: 1 })).unwrap();
        println!("{}", measure);
    }

    #[test]
    fn test_delete_shift_left_middle() {
        // 4/4: q, e, e -> delete index 1 yields q, e
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        let q = Duration::Simple(NoteValue::Quarter);
        let e = Duration::Simple(NoteValue::Eighth);
        m.add_beat(Beat::note(q)).unwrap();
        m.add_beat(Beat::note(e)).unwrap();
        m.add_beat(Beat::rest(e)).unwrap();
        assert_eq!(m.beats().len(), 3);
        m.delete_shift_left(1);
        assert_eq!(m.beats().len(), 2);
        assert_eq!(m.beats()[0].duration, q);
        assert_eq!(m.beats()[1].duration, e);
    }

    #[test]
    fn test_delete_at_ghost_commits_then_removes_committed_rest() {
        // Start with one quarter note in 4/4, remainder is [q, q, q]
        // Commit up to index 2, then delete at 2: result should be [q, r(q)]
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        let q = Duration::Simple(NoteValue::Quarter);
        m.add_beat(Beat::note(q)).unwrap();
        m.ensure_committed_position(2);
        assert_eq!(m.beats().len(), 3);
        // The committed prefix should be [note(q), rest(q), rest(q)] now
        assert_eq!(m.beats()[0].kind, BeatKind::Note);
        assert_eq!(m.beats()[1].kind, BeatKind::Rest);
        assert_eq!(m.beats()[2].kind, BeatKind::Rest);
        m.delete_shift_left(2);
        // After deletion, length is 2 and second is still a rest
        assert_eq!(m.beats().len(), 2);
        assert_eq!(m.beats()[0].kind, BeatKind::Note);
        assert_eq!(m.beats()[1].kind, BeatKind::Rest);
        // Duration of the rest depends on remainder spelling; do not assert exact value.
    }
}
