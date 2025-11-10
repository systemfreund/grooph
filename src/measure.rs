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
    pub fn measure_duration_ticks(&self) -> u32 {
        // Use the unified duration set to derive the grid.
        let set = default_duration_set();
        ((self.beats as u32) * set.grid.ticks_per_whole) / (self.beat_unit as u32)
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
    /// Current cursor position
    position: usize,
}

impl Measure {
    /// Creates a new empty measure with the given time signature
    pub fn new(time_signature: TimeSignature) -> Self {
        let mut s = Self {
            beats: Vec::new(),
            time_signature,
            beam_plan: Some(BeamPlan { groups: vec![] }),
            position: 0,
        };
        s.fill_measure();
        s
    }

    /// Expose a read-only view of beats
    pub fn beats(&self) -> &Vec<Beat> { &self.beats }

    /// Split the beat at `idx` into two equal halves (by time), replacing it with two smaller beats.
    /// Only supported for simple durations down to Sixteenth; returns false if not possible.
    pub fn split_beat_by_two(&mut self, idx: usize) -> bool {
        if idx >= self.beats.len() {
            return false;
        }
        let base = self.beats[idx];
        let half = match base.duration.halve_simple() {
            Some(h) => h,
            None => return false,
        };
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
    ///
    /// Greedy behavior for rests: if `left` is a rest and `right` is also a rest but not the same
    /// duration, attempt to greedily absorb subsequent contiguous rests into `right` until it matches
    /// `left`'s duration, then perform the merge. This allows merging two eighth rests into a quarter
    /// rest even if they are not already split symmetrically (e.g., 1/8 + 1/16 + 1/16 -> 1/4).
    pub fn unsplit_beat_by_two(&mut self, idx: usize) -> bool {
        if idx + 1 >= self.beats.len() {
            return false;
        }
        let left = self.beats[idx];
        let right = self.beats[idx + 1];

        // Fast path: must be same kind to ever merge
        if left.kind != right.kind {
            return false;
        }

        // Only simple doubling is supported
        let doubled = match left.duration.double_simple() {
            Some(d) => d,
            None => return false,
        };

        // If durations already equal, do the normal merge
        if left.duration == right.duration {
            self.beats[idx].duration = doubled;
            self.beats[idx].tremolo = None;
            self.beats.remove(idx + 1);
            self.recompute_beams();
            return true;
        }

        // Greedy rest merging: if left is a rest and right is a rest, try to grow right by
        // consuming subsequent rests until it equals left's duration, then merge.
        if left.kind == BeatKind::Rest {
            use crate::duration::default_duration_set;
            let set = default_duration_set();
            let left_ticks = match set.grid.ticks_of(&left.duration) {
                Some(t) => t,
                None => return false,
            };

            // Sum ticks of contiguous rests starting at idx+1
            let mut sum_ticks = 0u32;
            let mut k = idx + 1;
            while k < self.beats.len() {
                let b = self.beats[k];
                if b.kind != BeatKind::Rest {
                    break;
                }
                let t = match set.grid.ticks_of(&b.duration) {
                    Some(t) => t,
                    None => break,
                };
                sum_ticks += t;
                if sum_ticks >= left_ticks {
                    break;
                }
                k += 1;
            }
            if sum_ticks >= left_ticks {
                // Try to expand right into exactly left.duration by absorbing rests via set_beat_at
                if self
                    .set_beat_at(
                        idx + 1,
                        Beat { duration: left.duration, kind: BeatKind::Rest, tremolo: None },
                    )
                    .is_ok()
                {
                    // After successful expansion, durations should match: perform merge
                    self.beats[idx].duration = doubled;
                    self.beats[idx].tremolo = None;
                    self.beats.remove(idx + 1);
                    self.recompute_beams();
                    return true;
                }
            }
        }

        false
    }

    /// Replace the beat at index `idx` with `beat` if it fits and the remainder stays fillable.
    pub fn set_beat_at(&mut self, idx: usize, beat: Beat) -> Result<(), MeasureError> {
        if idx >= self.beats.len() {
            // Out of bounds after caller's ensure: treat as unfillable generically
            return Err(MeasureError::Unfillable { attempted: 0.0, remaining: 0.0 });
        }
        let set = default_duration_set();
        let current_ticks = self.current_ticks();
        let max_ticks = self.time_signature.measure_duration_ticks();
        let new_ticks = set
            .grid
            .ticks_of(&beat.duration)
            .ok_or_else(|| MeasureError::Unfillable { attempted: 0.0, remaining: 0.0 })?;
        let old_ticks = set.grid.ticks_of(&self.beats[idx].duration).unwrap_or(0);
        let new_total_ticks = current_ticks - old_ticks + new_ticks;
        if new_total_ticks > max_ticks {
            // Attempt to expand into subsequent contiguous rests to accommodate growth
            let mut need = new_ticks - old_ticks; // extra ticks required
            let mut k = idx + 1;
            let mut absorb_ticks = 0u32;
            while need > 0 && k < self.beats.len() {
                let b = self.beats[k];
                if b.kind != BeatKind::Rest {
                    break;
                }
                let t = set.grid.ticks_of(&b.duration).unwrap_or(0);
                absorb_ticks += t;
                if absorb_ticks >= need {
                    break;
                }
                k += 1;
            }
            if absorb_ticks >= need {
                // We can grow by consuming rests from idx+1..=k
                // First set the new beat at idx
                self.beats[idx] = beat;
                // Now consume 'need' ticks from the following rests
                let mut p = idx + 1;
                let mut remaining_to_consume = need;
                while remaining_to_consume > 0 {
                    let b = self.beats[p];
                    let t = set.grid.ticks_of(&b.duration).unwrap_or(0);
                    if t <= remaining_to_consume {
                        // Remove whole rest
                        self.beats.remove(p);
                        remaining_to_consume -= t;
                        // do not advance p because elements shift left
                    } else {
                        // Shorten this rest by consuming part of it
                        let new_ticks_rest = t - remaining_to_consume;
                        // Replace this single rest with a sequence that fills new_ticks_rest
                        self.beats.remove(p);
                        if let Some(fill) = best_fill_for_gap(new_ticks_rest, &[]) {
                            let mut insert_at = p;
                            for d in fill {
                                self.beats.insert(insert_at, Beat::rest(d));
                                insert_at += 1;
                            }
                        }
                        remaining_to_consume = 0;
                    }
                }
                self.recompute_beams();
                return Ok(());
            } else {
                let available_ticks = (max_ticks - (current_ticks - old_ticks)).max(0);
                let available = (available_ticks as f64) / (set.grid.ticks_per_whole as f64);
                let attempted = (new_ticks as f64) / (set.grid.ticks_per_whole as f64);
                return Err(MeasureError::Overflow { attempted, available });
            }
        }
        let remaining_ticks = max_ticks - new_total_ticks;
        if remaining_ticks != 0 && !Self::is_remainder_fillable(remaining_ticks) {
            let remaining = (remaining_ticks as f64) / (set.grid.ticks_per_whole as f64);
            let attempted = (new_ticks as f64) / (set.grid.ticks_per_whole as f64);
            return Err(MeasureError::Unfillable { attempted, remaining });
        }
        // Perform replacement at idx
        self.beats[idx] = beat;

        // If the new beat is shorter than the old one, split the leftover time
        // into concrete rest beats inserted immediately after idx so that subsequent
        // positions exist (elegant progression for add_beat at idx+1).
        if new_ticks < old_ticks {
            let leftover = old_ticks - new_ticks;
            if let Some(fill) = best_fill_for_gap(leftover, &[]) {
                let mut insert_at = idx + 1;
                for d in fill {
                    self.beats.insert(insert_at, Beat::rest(d));
                    insert_at += 1;
                }
            }
        }

        self.recompute_beams();
        Ok(())
    }

    /// Expose the time signature (clone)
    pub fn time_signature(&self) -> TimeSignature { self.time_signature.clone() }

    /// Expose the beaming plan for this measure
    pub fn beam_plan(&self) -> Option<&BeamPlan> { self.beam_plan.as_ref() }

    /// Get current cursor position inside the measure model
    pub fn position(&self) -> usize { self.position }

    /// Set current cursor position inside the measure model (no clamping)
    pub fn set_position(&mut self, pos: usize) { self.position = pos }

    /// Returns the current total duration in ticks (exact)
    fn current_ticks(&self) -> u32 {
        let set = default_duration_set();
        self.beats.iter().map(|beat| set.grid.ticks_of(&beat.duration).unwrap()).sum()
    }

    /// Returns the remaining number of ticks available in this measure
    /// (never negative; 0 when the measure is full)
    pub fn remaining_ticks(&self) -> u32 {
        let max_ticks = self.time_signature.measure_duration_ticks();
        let used = self.current_ticks();
        (max_ticks - used).max(0)
    }

    /// Returns true if the remaining ticks can be exactly filled using the available durations
    fn is_remainder_fillable(remaining_ticks: u32) -> bool {
        if remaining_ticks == 0 {
            return false;
        }
        // Build the available coin sizes (ticks) from the supported durations. Larger first helps pruning.
        let set = default_duration_set();
        let mut coins: Vec<u32> =
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

    /// Adds a beat to this measure at the current position if it doesn't exceed the time signature
    /// and remains fillable.
    ///
    /// # Returns
    /// - `Ok(())` if the beat was successfully added
    /// - `Err(MeasureError::Overflow)` if adding the beat would exceed the measure's capacity
    /// - `Err(MeasureError::Unfillable)` if the addition leaves an unfillable remainder
    pub fn add_beat(&mut self, beat: Beat) -> Result<(), MeasureError> {
        // Attempt to set the beat at the current position
        match self.set_beat_at(self.position, beat) {
            Ok(()) => {
                // Advance internal position to the next index for subsequent insertions
                let len = self.beats.len();
                // Advance to the next logical index; allow pointing one past the last element
                // so that the next add_beat() commits a new slot at the end.
                self.position = self.position.saturating_add(1).min(len);
                Ok(())
            }
            Err(e) => Err(e),
        }
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
        self.beats.remove(idx);
        self.fill_measure();
        self.minimize_remainder_rests_from(idx);
    }

    pub fn fill_measure(&mut self) {
        let remaining_ticks = self.remaining_ticks();
        if remaining_ticks <= 0 {
            return; // nothing to commit
        }
        if let Some(fill) = best_fill_for_gap(remaining_ticks, &[]) {
            let take = fill.len();
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
        if idx >= self.beats.len() {
            return false;
        }
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

impl Measure {
    /// Minimize the number of rest beats in the trailing remainder of the measure.
    ///
    /// This collapses the contiguous suffix of rests at the end of the measure into
    /// a minimal-count spelling that exactly matches the same total ticks. Musical
    /// content (notes or interior rests between notes) before that suffix is left
    /// untouched.
    /// Minimize the number of rest beats in the trailing remainder of the measure,
    /// starting from `start_idx`.
    ///
    /// Only the trailing suffix of rests that lies at or after `start_idx` is minimized.
    /// Any rests prior to `start_idx` are left untouched even if they are part of an
    /// earlier rest run. If there is no trailing rest suffix, this is a no-op.
    pub fn minimize_remainder_rests_from(&mut self, start_idx: usize) {
        if self.beats.is_empty() {
            return;
        }
        // Find the global start index of the trailing rest suffix (if any)
        let mut trailing_start = self.beats.len();
        for i in (0..self.beats.len()).rev() {
            if self.beats[i].kind == BeatKind::Rest {
                trailing_start = i;
            } else {
                break;
            }
        }
        if trailing_start >= self.beats.len() {
            // No trailing rests at all
            return;
        }
        // We only minimize starting at max(start_idx, trailing_start)
        let start = start_idx.max(trailing_start);
        if start >= self.beats.len() {
            return;
        }
        // Sum ticks from `start` to end
        let set = default_duration_set();
        let mut total_ticks: u32 = 0;
        for b in &self.beats[start..] {
            if let Some(t) = set.grid.ticks_of(&b.duration) {
                total_ticks += t;
            }
        }
        if total_ticks == 0 {
            return;
        }
        // Refill using the best (minimal-count) spelling
        if let Some(fill) = best_fill_for_gap(total_ticks, &[]) {
            // Remove old suffix starting at `start`
            self.beats.truncate(start);
            // Append new minimized rests
            for d in fill {
                self.beats.push(Beat::rest(d));
            }
            self.recompute_beams();
        }
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
    fn test_basic_measure_features() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        assert_eq!(m.beats().len(), 4);
        assert_eq!(m.position, 0);

        m.add_beat(Beat::note(q())).unwrap();
        assert_eq!(m.position, 1);
        assert_eq!(m.beats().len(), 4);
    }

    #[test]
    fn test_add_quarter_note_to_one_four_measure() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);

        m.add_beat(Beat::note(q())).unwrap();
        assert_eq!(m.beats().len(), 1);
        assert!(m.add_beat(Beat::note(q())).is_err());
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
    }

    #[test]
    fn test_remove_middle() {
        // 4/4: q, e, e -> delete index 1 yields q, e
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        let q = Duration::Simple(NoteValue::Quarter);
        let e = Duration::Simple(NoteValue::Eighth);
        m.add_beat(Beat::note(q)).unwrap();
        m.add_beat(Beat::note(e)).unwrap();
        m.add_beat(Beat::rest(e)).unwrap();
        m.remove(1);
        assert_eq!(m.beats()[0].duration, q);
        assert_eq!(m.beats()[1].duration, q); // because of minimization remaining rests
    }
}
