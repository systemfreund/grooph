pub(crate) mod grouping;

use crate::beaming::BeamPlan;
use crate::duration::NoteValue::{Eighth, Sixteenth, ThirtySecond};
use crate::duration::{Duration, NoteValue, default_duration_set};
use crate::fill::best_fill_for_gap;
use crate::measure::BeatKind::Rest;
use BeatKind::Note;
use std::fmt::{Display, Formatter};
use std::vec;

/// Represents a time signature (e.g., 4/4, 3/4, 6/8)
#[derive(Debug, Clone, Copy)]
pub struct TimeSignature {
    /// Number of beats per measure
    pub beats: u8,
    /// Note value that represents one beat (4 = quarter note, 8 = eighth note)
    pub beat_unit: u8,
}

impl TimeSignature {
    pub const ONE_FOUR: Self = Self { beats: 1, beat_unit: 4 };
    pub const TWO_FOUR: Self = Self { beats: 2, beat_unit: 4 };
    pub const ONE_SIXTEENTH: Self = Self { beats: 1, beat_unit: 16 };
    pub const TWO_SIXTEENTH: Self = Self { beats: 2, beat_unit: 16 };
    pub const FOUR_SIXTEENTH: Self = Self { beats: 4, beat_unit: 16 };
    pub const FOUR_FOUR: Self = Self { beats: 4, beat_unit: 4 };
    pub const TWO_EIGHT: Self = Self { beats: 2, beat_unit: 8 };
    pub const FOUR_EIGHT: Self = Self { beats: 4, beat_unit: 8 };
    pub const FIVE_EIGHT: Self = Self { beats: 5, beat_unit: 8 };
    pub const SIX_EIGHT: Self = Self { beats: 6, beat_unit: 8 };
    pub const SEVEN_EIGHT: Self = Self { beats: 7, beat_unit: 8 };
    pub const NINE_EIGHT: Self = Self { beats: 9, beat_unit: 8 };
    pub const TWELVE_EIGHT: Self = Self { beats: 12, beat_unit: 8 };

    /// Returns the total duration in integer ticks
    pub const fn measure_duration_ticks(&self) -> u32 {
        // Use the unified duration set to derive the grid.
        let set = default_duration_set();
        ((self.beats as u32) * set.grid.ticks_per_whole) / (self.beat_unit as u32)
    }

    pub const fn beat_note_value(&self) -> Option<NoteValue> {
        match self.beat_unit {
            1 => Some(NoteValue::Whole),
            2 => Some(NoteValue::Half),
            4 => Some(NoteValue::Quarter),
            8 => Some(NoteValue::Eighth),
            16 => Some(NoteValue::Sixteenth),
            32 => Some(NoteValue::ThirtySecond),
            _ => None,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BeatKind {
    Note,
    Rest,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Beat {
    pub duration: Duration,
    pub kind: BeatKind,
    pub tremolo: Option<Tremolo>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Tremolo {
    /// Number of slashes (1..=3 typical)
    pub slashes: u8,
    /// If true, indicates measured tremolo; otherwise unmeasured (for future use)
    pub measured: bool,
}

impl Beat {
    /// Creates a new note with the given duration
    pub fn note(duration: Duration) -> Self { Self { duration, kind: Note, tremolo: None } }

    /// Creates a new rest with the given duration
    pub fn rest(duration: Duration) -> Self { Self { duration, kind: Rest, tremolo: None } }
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
#[derive(Debug, Clone)]
pub struct Measure {
    beats: Vec<Beat>,
    time_signature: TimeSignature,
    // Internal insertion pointer for add_beat progression (not a UI cursor)
    next_insert: usize,
}

impl Measure {
    /// Creates a new empty measure with the given time signature
    pub fn new(time_signature: TimeSignature) -> Self { Self::new_init(time_signature, Rest) }

    pub fn new_init(time_signature: TimeSignature, init: BeatKind) -> Self {
        let mut s = Self { beats: Vec::new(), time_signature, next_insert: 0 };
        s.fill_measure(init, &[Duration::Simple(time_signature.beat_note_value().unwrap())]);
        s
    }

    /// Expose a read-only view of beats
    pub fn beats(&self) -> &Vec<Beat> { &self.beats }

    /// Replace the beat at index `idx` with `beat` if it fits and the remainder stays fillable.
    pub fn set_beat_at(&mut self, idx: usize, beat: Beat) -> Result<(), MeasureError> {
        if idx >= self.beats.len() {
            return Err(MeasureError::Unfillable { attempted: 0.0, remaining: 0.0 });
        }
        let set = default_duration_set();
        let current_ticks = self.current_ticks();
        let dur_old = self.beats[idx].duration; // duration of the beat to be replaced
        let max_ticks = set.grid.ticks_per_measure(&self.time_signature);
        let new_ticks = set
            .grid
            .ticks_of(&beat.duration)
            .ok_or_else(|| MeasureError::Unfillable { attempted: 0.0, remaining: 0.0 })?;

        // Reject grid-incompatible replacement into a tuplet slot
        if let Duration::Tuplet { n: n_old, m: m_old, .. } = dur_old {
            match beat.duration {
                Duration::Tuplet { n: n_new, m: m_new, .. } if n_new == n_old && m_new == m_old => {
                    // ok: same tuplet grid
                }
                _ => {
                    // inserting a non-tuplet or different tuplet grid into a tuplet slot is invalid
                    let set = default_duration_set();
                    let attempted = set
                        .grid
                        .ticks_to_whole_notes(set.grid.ticks_of(&beat.duration).unwrap_or(0));
                    let remaining = 0.0; // not strictly applicable here
                    return Err(MeasureError::Unfillable { attempted, remaining });
                }
            }
        }

        let old_ticks = set.grid.ticks_of(&dur_old).unwrap();
        let new_total_ticks = current_ticks - old_ticks + new_ticks;
        if new_total_ticks > max_ticks {
            // Attempt to expand into subsequent contiguous rests to accommodate growth
            let need = new_ticks - old_ticks; // extra ticks required
            let mut k = idx + 1;
            let mut absorb_ticks = 0u32;
            while need > 0 && k < self.beats.len() {
                let b = self.beats[k];
                let t = set.grid.ticks_of(&b.duration).unwrap();
                absorb_ticks += t;
                if absorb_ticks >= need {
                    break;
                }
                k += 1;
            }
            return if absorb_ticks >= need {
                // We can grow by consuming rests from idx+1..=k
                // First set the new beat at idx
                self.beats[idx] = beat;
                // Now consume 'need' ticks from the following rests
                let p = idx + 1;
                let mut remaining_to_consume = need;
                while remaining_to_consume > 0 {
                    let b = self.beats[p];
                    let t = set.grid.ticks_of(&b.duration).unwrap();
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
                Ok(())
            } else {
                let available_ticks = (max_ticks - (current_ticks - old_ticks)).max(0);
                let available = (available_ticks as f64) / (set.grid.ticks_per_whole as f64);
                let attempted = (new_ticks as f64) / (set.grid.ticks_per_whole as f64);
                Err(MeasureError::Overflow { attempted, available })
            };
        }

        // Attempt-then-fill: do not modify the measure until we know we can spell the leftover
        // exactly using a context-aware set of durations.
        if new_ticks < old_ticks {
            // Special handling for tuplets when starting a new tuplet group (old slot is non-tuplet):
            // form a full tuplet group spanning m * base. If we're already inside a tuplet group,
            // fall back to the generic leftover fill to allow intra-group refinements (e.g., t8 -> t16).
            if let Duration::Tuplet { n, m, base } = beat.duration {
                if matches!(dur_old, Duration::Tuplet { .. }) {
                    // Do not span a new group when replacing within an existing tuplet grid.
                    // Proceed to generic leftover handling below.
                } else {
                    // Compute the total span this tuplet group should occupy
                    let base_ticks = set.grid.ticks_of(&Duration::Simple(base)).unwrap();
                    let group_span = (m as u32) * base_ticks;

                    // Collect ticks from idx forward until we cover group_span
                    let mut consumed = 0u32;
                    let mut k = idx; // exclusive end index for removal [idx, k)
                    while consumed < group_span {
                        if k >= self.beats.len() {
                            // Not enough space in this measure to span the tuplet group
                            let attempted = set.grid.ticks_to_whole_notes(new_ticks);
                            let available_ticks = (set
                                .grid
                                .ticks_per_measure(&self.time_signature)
                                .saturating_sub(current_ticks - old_ticks))
                            .max(0);
                            let available = set.grid.ticks_to_whole_notes(available_ticks);
                            return Err(MeasureError::Overflow { attempted, available });
                        }
                        let b = self.beats[k];
                        // If we encounter a tuplet of a different grid (different n/m), refuse
                        // (don't break existing groups). Base may differ (e.g., t8 vs t16) but
                        // n/m define the grid equivalence here.
                        if let Duration::Tuplet { n: n2, m: m2, .. } = b.duration {
                            if !(n2 == n && m2 == m) {
                                let attempted = set.grid.ticks_to_whole_notes(new_ticks);
                                let remaining = set
                                    .grid
                                    .ticks_to_whole_notes(group_span.saturating_sub(consumed));
                                return Err(MeasureError::Unfillable { attempted, remaining });
                            }
                        }
                        let t = set.grid.ticks_of(&b.duration).unwrap();
                        consumed += t;
                        k += 1;
                    }

                    // We will replace [idx..k) with the tuplet group (n items). If we consumed
                    // more ticks than the group span, we owe back a remainder after the group.
                    let overrun = consumed - group_span;

                    // Remove the covered region first
                    self.beats.drain(idx..k);

                    // Insert the tuplet items: first the requested beat, then n-1 rests of same tuplet duration
                    self.beats.insert(idx, beat);
                    let mut insert_at = idx + 1;
                    for _ in 1..n {
                        self.beats.insert(insert_at, Beat::rest(beat.duration));
                        insert_at += 1;
                    }

                    // If there is an overrun (we consumed into the next original beat), reinsert its remainder as rests
                    if overrun > 0 {
                        if let Some(fill) = best_fill_for_gap(overrun, &[]) {
                            for d in fill {
                                self.beats.insert(insert_at, Beat::rest(d));
                                insert_at += 1;
                            }
                        } else {
                            // Should not happen with our common durations, but guard anyway
                            let attempted = set.grid.ticks_to_whole_notes(new_ticks);
                            let remaining = set.grid.ticks_to_whole_notes(overrun);
                            return Err(MeasureError::Unfillable { attempted, remaining });
                        }
                    }

                    // Sanity: total ticks must remain the same
                    assert_eq!(self.current_ticks(), max_ticks);
                    return Ok(());
                }
            }

            // Non-tuplet: try to fill leftover locally
            let leftover = old_ticks - new_ticks;
            let allowed: Vec<Duration> = Vec::new();

            // Require an exact contextual spelling for the leftover
            if let Some(fill) = best_fill_for_gap(leftover, &allowed) {
                // Commit: perform replacement at idx and insert the remainder as rests
                self.beats[idx] = beat;
                let mut insert_at = idx + 1;
                for d in fill {
                    self.beats.insert(insert_at, Beat::rest(d));
                    insert_at += 1;
                }
                Ok(())
            } else {
                let attempted = set.grid.ticks_to_whole_notes(new_ticks);
                let remaining = set.grid.ticks_to_whole_notes(leftover);
                Err(MeasureError::Unfillable { attempted, remaining })
            }
        } else {
            // No leftover (equal or growth accommodated earlier). Just replace.
            self.beats[idx] = beat;
            assert_eq!(self.current_ticks(), max_ticks);
            Ok(())
        }
    }

    /// Expose the time signature (clone)
    pub fn time_signature(&self) -> TimeSignature { self.time_signature.clone() }

    /// Return a vector with the absolute position (1-based) of each beat as floats.
    /// Examples:
    /// - In 4/4 with four quarters: [1.0, 2.0, 3.0, 4.0]
    /// - If the first three notes are 8th-note triplets in 4/4: [1.0, 1.3333..., 1.6666...]
    /// Positions are computed from onset ticks relative to the measure's beat size (beat_unit).
    pub fn beat_positions(&self) -> Vec<f32> {
        let set = default_duration_set();
        let onsets = set.compute_onset_ticks(&self.beats);
        let ticks_per_beat = set.grid.ticks_per_beat(&self.time_signature);
        onsets.into_iter().map(|t| 1.0f32 + (t as f32) / (ticks_per_beat as f32)).collect()
    }

    /// Returns the current total duration in ticks (exact)
    fn current_ticks(&self) -> u32 {
        let set = default_duration_set();
        self.beats.iter().map(|beat| set.grid.ticks_of(&beat.duration).unwrap()).sum()
    }

    /// Returns the remaining number of ticks available in this measure
    /// (never negative; 0 when the measure is full)
    pub fn remaining_ticks(&self) -> u32 {
        let set = default_duration_set();
        let max_ticks = set.grid.ticks_per_measure(&self.time_signature);
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

    /// Intended to be used by tests
    pub(crate) fn add_beat(&mut self, beat: Beat) -> Result<(), MeasureError> {
        match self.set_beat_at(self.next_insert, beat) {
            Ok(()) => {
                self.next_insert += 1;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    pub fn remove(&mut self, idx: usize) {
        if idx >= self.beats.len() {
            return;
        }
        self.beats[idx].kind = Rest;
    }

    pub fn fill_measure(&mut self, kind: BeatKind, allowed: &[Duration]) {
        let remaining_ticks = self.remaining_ticks();
        if remaining_ticks <= 0 {
            return; // nothing to commit
        }
        if let Some(fill) = best_fill_for_gap(remaining_ticks, allowed) {
            let take = fill.len();
            for duration in fill.into_iter().take(take) {
                let beat = Beat { duration, kind, tremolo: None };
                self.beats.push(beat);
            }
        }
    }

    /// Toggle the beat kind at `idx` between Note and Rest while preserving duration.
    /// No-op if `idx` is out of bounds.
    pub fn toggle_beat_kind(&mut self, idx: usize) {
        if let Some(b) = self.beats.get_mut(idx) {
            // Clear tremolo in both cases to avoid invalid state on rests
            b.tremolo = None;
            b.kind = match b.kind {
                Rest => Note,
                Note => Rest,
            };
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

impl Display for Measure {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.beats.iter().fold(Ok(()), |result, beat| {
            result.and_then(|_| {
                let duration = beat.duration.base_note().fraction();
                write!(f, "{}", duration)
                    .and_then(|_| match beat.duration {
                        Duration::Simple(_) => Ok(()),
                        Duration::Dotted { base: _base, dots } => {
                            write!(f, "{}", ".".repeat(dots as usize))
                        }
                        Duration::Tuplet { .. } => write!(f, "ᵀ"),
                    })
                    .and_then(|_| write!(f, " "))
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duration::NoteValue::{Eighth, Sixteenth, ThirtySecond};
    use crate::duration::{Duration, e, q, qt16, s, t8, t16, t32, th};

    #[test]
    fn test_basic_measure_features() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        assert_eq!(m.beats().len(), 4);
        // add_beat should fill from the first rest slot; measure stays length 4
        m.add_beat(Beat::note(q())).unwrap();
        assert_eq!(m.beats().len(), 4);
    }

    #[test]
    fn test_triplet_in_one_four() {
        let mut measure = Measure::new(TimeSignature::ONE_FOUR);

        assert!(measure.add_beat(Beat::note(t8())).is_ok());
        let Beat { duration, kind, .. } = measure.beats()[1];
        assert_eq!(duration, t8());
        assert_eq!(kind, Rest);
        let Beat { duration, kind, .. } = measure.beats()[2];
        assert_eq!(duration, t8());
        assert_eq!(kind, Rest);

        assert!(
            measure.add_beat(Beat::note(q())).is_err(),
            "simple 1/4 note must not fit in triplet 1/8 group"
        );
        assert!(
            measure.add_beat(Beat::note(e())).is_err(),
            "simple 1/8 note must not fit in triplet 1/8 group"
        );
        assert!(
            measure.add_beat(Beat::note(s())).is_err(),
            "simple 1/16 note must not fit in triplet 1/8 group"
        );
        assert!(
            measure.add_beat(Beat::note(th())).is_err(),
            "simple 1/32 note must not fit in triplet 1/8 group"
        );

        assert!(measure.add_beat(Beat::note(t8())).is_ok());
        assert!(measure.add_beat(Beat::rest(t8())).is_ok());
    }

    #[test]
    fn test_invalid_tuplet_insertion_0() {
        let mut measure = Measure::new(TimeSignature::TWO_FOUR);

        assert!(measure.add_beat(Beat::note(t8())).is_ok());
        assert!(measure.add_beat(Beat::note(t8())).is_ok());
        assert!(measure.add_beat(Beat::note(t8())).is_ok());

        /* IGNORE intra-group refinements for now
        assert!(measure.add_beat(Beat::note(t16())).is_ok());
        assert!(measure.add_beat(Beat::note(t16())).is_ok());
        // The next triplet 1/8 overfills this tuplet group, which has only space for one triplet
        // 1/6 note left (or two triplet 1/32 subdivisions).
        // assert!(measure.add_beat(Beat::note(t8())).is_err(), "triplet 1/8 must not fit tuplet group");
        assert!(measure.add_beat(Beat::note(t32())).is_ok());
        assert!(measure.add_beat(Beat::note(t32())).is_ok());
        // The next triplet 1/16 overfills this tuplet group, which has only space for one triplet
        // 1/32 note left.
        // assert!(measure.add_beat(Beat::note(t16())).is_err());
        assert!(measure.add_beat(Beat::note(t32())).is_ok());

        // The next beat starts a new tuplet group, so this is valid.
        assert!(measure.add_beat(Beat::note(t8())).is_ok());
        */
    }

    #[test]
    fn test_triplet_insertions_1() {
        let mut measure = Measure::new(TimeSignature::ONE_FOUR);

        assert!(measure.add_beat(Beat::note(t16())).is_ok());
        assert!(measure.add_beat(Beat::note(t8())).is_ok());
        assert!(
            measure.add_beat(Beat::note(t8())).is_err(),
            "can't start a new triplet 1/8 group measure has only 1/8 space left"
        );
        assert!(measure.add_beat(Beat::note(e())).is_ok());
        assert_eq!(measure.remaining_ticks(), 0);
    }

    #[test]
    fn test_triplet_insertions_2() {
        let mut measure = Measure::new(TimeSignature::TWO_FOUR);

        // First triplet group
        assert!(measure.add_beat(Beat::note(t8())).is_ok());
        assert!(measure.add_beat(Beat::note(t16())).is_ok());
        assert!(measure.add_beat(Beat::note(t16())).is_ok());
        assert!(measure.add_beat(Beat::note(t8())).is_ok());

        // Second triplet group
        assert!(measure.add_beat(Beat::note(t16())).is_ok());
        assert!(measure.add_beat(Beat::note(t32())).is_ok());
        assert!(measure.add_beat(Beat::note(t32())).is_ok());
        assert!(measure.add_beat(Beat::note(t32())).is_ok());

        assert_eq!(measure.remaining_ticks(), 0);
    }

    #[test]
    fn test_invalid_tuplet_insertion_2() {
        let mut measure = Measure::new(TimeSignature::TWO_EIGHT);

        assert!(measure.add_beat(Beat::note(t8())).is_ok());
        assert!(measure.add_beat(Beat::note(t16())).is_ok());
        assert!(measure.add_beat(Beat::note(t8())).is_ok());
        // The next triplet 1/8 overfills this tuplet group, which has only space for one triplet
        // 1/6 note left (or two triplet 1/32 subdivisions).
        assert!(measure.add_beat(Beat::note(t8())).is_err());
        assert!(measure.add_beat(Beat::note(t32())).is_ok());
        // Doesn't fit.
        assert!(measure.add_beat(Beat::note(t16())).is_err());
        assert!(measure.add_beat(Beat::note(t32())).is_ok());

        // The next beat starts a new tuplet group, but we don't have enough space in our measure.
        assert!(measure.add_beat(Beat::note(t32())).is_err());
    }

    #[test]
    fn test_invalid_tuplet_insertion_3() {
        let mut measure = Measure::new(TimeSignature::ONE_FOUR);

        assert!(measure.add_beat(Beat::note(qt16())).is_ok());
        println!("{}", measure);
    }

    #[test]
    fn test_triplet_insertion_in_seven_eight() {
        let mut measure = Measure::new(TimeSignature::SEVEN_EIGHT);
        assert!(measure.add_beat(Beat::note(t8())).is_ok());
        assert!(measure.add_beat(Beat::note(t8())).is_ok());
        assert!(measure.add_beat(Beat::note(t16())).is_ok());
    }

    #[test]
    fn test_triplet_split_in_one_four() {
        let mut measure = Measure::new(TimeSignature::ONE_FOUR);
        assert!(measure.add_beat(Beat::note(t8())).is_ok());
        assert!(measure.add_beat(Beat::note(t8())).is_ok());
        assert!(measure.add_beat(Beat::note(t8())).is_ok());

        println!("{}", measure);
        assert!(measure.set_beat_at(2, Beat::note(t16())).is_ok());
        println!("{}", measure);
    }

    #[test]
    fn test_add_eighth_triplet_to_seven_eight_measure() {
        let mut measure = Measure::new(TimeSignature::SEVEN_EIGHT);
        measure.add_beat(Beat::note(Duration::Dotted { base: Eighth, dots: 1 })).unwrap();
        measure.add_beat(Beat::note(Duration::Dotted { base: Sixteenth, dots: 1 })).unwrap();
        measure.add_beat(Beat::note(Duration::Simple(ThirtySecond))).unwrap();
        measure.add_beat(Beat::note(Duration::Simple(Sixteenth))).unwrap();
        measure.add_beat(Beat::note(Duration::Simple(Eighth))).unwrap();
        measure.add_beat(Beat::note(Duration::Simple(Sixteenth))).unwrap();
        measure.add_beat(Beat::rest(Duration::Dotted { base: Eighth, dots: 1 })).unwrap();
    }

    #[test]
    fn test_beat_positions_quarters() {
        // Default 4/4 measure is filled with quarter rests by fill_measure
        let m = Measure::new(TimeSignature::FOUR_FOUR);
        let pos = m.beat_positions();
        assert_eq!(pos, vec![1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn test_beat_positions_triplet_eighths_start() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Replace the first three beats with eighth-note triplets
        m.set_beat_at(0, Beat::note(t8())).unwrap();
        m.set_beat_at(1, Beat::note(t8())).unwrap();
        m.set_beat_at(2, Beat::note(t8())).unwrap();
        let pos = m.beat_positions();
        // Expect positions: 1.0, 1 + 1/3, 1 + 2/3
        let expect = [1.0f32, 1.0 + 1.0 / 3.0, 1.0 + 2.0 / 3.0];
        let eps = 1e-4f32;
        assert!((pos[0] - expect[0]).abs() < eps);
        assert!((pos[1] - expect[1]).abs() < eps);
        assert!((pos[2] - expect[2]).abs() < eps);
    }

    #[test]
    fn test_quintuplets_0() {
        let mut measure = Measure::new(TimeSignature::FOUR_SIXTEENTH);
        println!("{}", measure);
        measure.add_beat(Beat::note(qt16())).unwrap(); // TODO fails but it mustn't
    }

    #[test]
    fn append_autofill_to_primary_boundary_simple() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Insert one eighth at start; autofill up to the next quarter boundary
        assert!(m.add_beat(Beat::note(e())).is_ok());
        // Expect: e() followed by an e() rest to reach the quarter boundary
        assert_eq!(m.beats()[1], Beat::rest(e()));
    }

    #[test]
    fn append_autofill_to_primary_boundary_triplet() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        assert!(m.add_beat(Beat::note(t8())).is_ok());
        // Expect two triplet-eighth rests to complete the triplet group
        assert_eq!(m.beats()[1], Beat::rest(t8()));
        assert_eq!(m.beats()[2], Beat::rest(t8()));
    }
}
