mod beat;
pub(crate) mod duration;
pub(crate) mod editing;
mod fill;
pub(crate) mod grid;
pub(super) mod grouping;
mod math;
pub(crate) mod time_signature;

pub(crate) use crate::measure::beat::{Beat, BeatKind};
use crate::measure::duration::NoteValue::{Eighth, Sixteenth, ThirtySecond};
use crate::measure::duration::{Duration, TupletSpec};
use crate::measure::editing::{GroupSpan, Modification};
use crate::measure::grid::DEFAULT_GRID;
pub(crate) use crate::measure::time_signature::TimeSignature;
use crate::measure::BeatKind::Rest;
use either::Either;
use std::collections::HashMap;
use std::fmt::{Debug, Display, Formatter};
use std::vec;
use BeatKind::Note;

/// Logical Beat-Index within a measure (0-based)
pub type BeatIdx = usize;

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
    },
}

/// Stable anchor describing a tuplet group span and semantics
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TupletAnchor {
    pub id: u32,
    pub n: u8,
    pub m: u8,
    /// Frozen logical span = ticks(Simple(base_hint)) * m
    pub target_ticks: u32,
    /// Intended base for UI/export; not authoritative for grid validity
    pub base_hint: duration::NoteValue,
}

/// Represents a musical measure containing a sequence of beats
#[derive(Clone)]
pub struct Measure {
    beats: Vec<Beat>,
    time_signature: TimeSignature,
    /// Tuplet anchor table keyed by stable id
    pub tuplet_anchors: HashMap<u32, TupletAnchor>,
    /// Next id to hand out when inserting a tuplet group
    pub next_tuplet_id: u32,
}

impl Measure {
    /// Creates a new empty measure with the given time signature
    pub fn new(time_signature: TimeSignature) -> Self { Self::new_init(time_signature, Rest) }

    pub fn new_init(time_signature: TimeSignature, init: BeatKind) -> Self {
        let mut s = Self {
            beats: Vec::new(),
            time_signature,
            tuplet_anchors: HashMap::new(),
            next_tuplet_id: 1,
        };
        s.fill_measure(init, &[Duration::Simple(time_signature.beat_note_value().unwrap())]);
        s
    }

    /// Expose a read-only view of beats
    pub fn beats(&self) -> &Vec<Beat> { &self.beats }

    pub fn delete_beat(&mut self, idx: BeatIdx) {
        self.beats.remove(idx);
    }
    
    /// Replace the beat at index `idx` with `beat` if it fits and the remainder stays fillable.
    pub fn set_beat(&mut self, idx: BeatIdx, beat: Beat) -> Result<(), MeasureError> {
        assert!(idx < self.beats.len());
        let old_accent = self.beats[idx].accented;
        let dur_old = self.beats[idx].duration; // duration of the beat to be replaced
        let max_ticks = self.max_ticks();
        let new_ticks = DEFAULT_GRID
            .ticks_of(&beat.duration)
            .ok_or(MeasureError::Unfillable { attempted: 0.0 })?;

        // Additionally ensure the new duration is part of the configured DurationSet.
        // The grid alone may accept more rational durations than we officially support.
        if !DEFAULT_GRID.durations.contains(&beat.duration) {
            return self.unfillable_err(new_ticks);
        }

        // Reject grid-incompatible replacement into a tuplet slot; also prepare id inheritance
        let mut new_beat = beat;
        if let Duration::Tuplet(TupletSpec { n: n_old, m: m_old, .. }) = dur_old {
            match beat.duration {
                Duration::Tuplet(TupletSpec { n: n_new, m: m_new, .. })
                    if n_new == n_old && m_new == m_old =>
                {
                    // ok: same tuplet grid - inherit group id if present
                    new_beat.tuplet_group_id = self.beats[idx].tuplet_group_id;
                }
                _ => {
                    // inserting a non-tuplet or different tuplet grid into a tuplet slot is invalid
                    return self.unfillable_err(new_ticks);
                }
            }
        }

        let old_ticks = DEFAULT_GRID.ticks_of(&dur_old).unwrap();

        // Special case: turning a non-tuplet slot into a tuplet beat should always construct
        // the entire tuplet group (n items spanning m·base), regardless of whether this is a
        // net growth or shrink relative to the original slot. Handling it here ensures we
        // don't create partial groups in the generic grow/shrink logic.
        if let Duration::Tuplet(TupletSpec { n, m, base }) = new_beat.duration
            && !matches!(dur_old, Duration::Tuplet { .. })
        {
            // Compute the total span this tuplet group should occupy
            let base_ticks = DEFAULT_GRID.ticks_of(&Duration::Simple(base)).unwrap();
            let group_span = (m as u32) * base_ticks;

            // Collect ticks from idx forward until we cover group_span
            let mut consumed = 0u32;
            let mut k = idx; // exclusive end index for removal [idx, k)
            while consumed < group_span {
                if k >= self.beats.len() {
                    // Not enough space in this measure to span the tuplet group
                    return self.overflow_err(new_ticks, max_ticks - old_ticks);
                }
                let b = self.beats[k];
                // Never grow across an existing tuplet group boundary with different id
                if self.beats[k].tuplet_group_id.is_some() {
                    return self.unfillable_err(group_span);
                }
                // If we encounter a tuplet of a different grid (different n/m), refuse
                // (don't break existing groups). Base may differ (e.g., t8 vs t16) but
                // n/m define the grid equivalence here.
                if let Duration::Tuplet(TupletSpec { n: n2, m: m2, .. }) = b.duration
                    && !(n2 == n && m2 == m)
                {
                    return self.unfillable_err(new_ticks);
                }
                let t = DEFAULT_GRID.ticks_of(&b.duration).unwrap();
                consumed += t;
                k += 1;
            }

            // We will replace [idx..k) with the tuplet group (n items). If we consumed
            // more ticks than the group span, we owe back a remainder after the group.
            let overrun = consumed - group_span;

            // Remove the covered region first
            self.beats.drain(idx..k);

            // Allocate a stable id and register an anchor for this group
            let id = self.next_tuplet_id;
            self.next_tuplet_id = self.next_tuplet_id.saturating_add(1);
            let anchor = TupletAnchor { id, n, m, target_ticks: group_span, base_hint: base };
            self.tuplet_anchors.insert(id, anchor);

            // Insert the tuplet items: first the requested beat, then n-1 rests of same tuplet duration
            let mut first = beat;
            // ensure first inherits id
            first.tuplet_group_id = Some(id);
            self.beats.insert(idx, first);
            // Preserve the prior accent on the first inserted beat
            self.beats[idx].accented = old_accent && beat.kind == Note;
            let mut insert_at = idx + 1;
            for _ in 1..n {
                let mut r = Beat::rest(beat.duration);
                r.tuplet_group_id = Some(id);
                self.beats.insert(insert_at, r);
                insert_at += 1;
            }

            // If there is an overrun (we consumed into the next original beat), reinsert its remainder as rests
            if overrun > 0 {
                self.fill_at(insert_at, overrun, &[], Either::Right(Rest))?
            }

            return Ok(());
        }

        let new_total_ticks = max_ticks - old_ticks + new_ticks;

        // "Growing" branch, i.e., when a larger beat replaces a smaller one.
        if new_total_ticks > max_ticks {
            let need = new_ticks - old_ticks; // extra ticks required
            assert!(need > 0);

            // Compute how many ticks we can absorb from following beats.
            let absorb_ticks = self.compute_ticks_to_absorb(idx, dur_old, need);
            return if absorb_ticks >= need {
                self.beats[idx] = new_beat;
                self.beats[idx].accented = old_accent && beat.kind == Note;
                let p = idx + 1;
                let mut remaining_to_consume = need;
                while remaining_to_consume > 0 {
                    let b = self.beats[p];
                    let t = DEFAULT_GRID.ticks_of(&b.duration).unwrap();
                    if t <= remaining_to_consume {
                        self.beats.remove(p);
                        remaining_to_consume -= t;
                    } else {
                        let new_ticks_rest = t - remaining_to_consume;
                        self.beats.remove(p);
                        // When growing inside a tuplet slot, constrain the remainder to the same tuplet grid
                        let allowed: Vec<Duration> = match dur_old {
                            Duration::Tuplet(TupletSpec { n: n_old, m: m_old, .. }) => DEFAULT_GRID
                                .durations
                                .iter()
                                .cloned()
                                .filter(|d| matches!(d, Duration::Tuplet(TupletSpec { n, m, .. }) if *n == n_old && *m == m_old))
                                .collect(),
                            _ => Vec::new(),
                        };
                        // When growing, the consumed remainder after the enlarged beat
                        // should be filled with rests (not notes). Use the current beat
                        // as a template to preserve metadata (e.g., tuplet ids), but
                        // switch kind to Rest.
                        self.fill_at(
                            p,
                            new_ticks_rest,
                            &allowed,
                            Either::Left(self.beats[idx].with_kind(Rest)),
                        )?;
                        remaining_to_consume = 0;
                    }
                }
                Ok(())
            } else {
                self.overflow_err(new_ticks, max_ticks - old_ticks)
            };
        }

        // "Shrinking" branch, i.e., when a smaller beat replaces a larger one.
        if new_ticks < old_ticks {
            let leftover = old_ticks - new_ticks;

            // If we are inside a tuplet slot, constrain the filler to durations that belong to the
            // same tuplet grid (same n,m).
            let allowed: Vec<Duration> = match dur_old {
                Duration::Tuplet(TupletSpec { n: n_old, m: m_old, .. }) => DEFAULT_GRID
                    .durations
                    .iter()
                    .cloned()
                    .filter(|d| matches!(d, Duration::Tuplet(TupletSpec { n, m, .. }) if *n == n_old && *m == m_old))
                    .collect(),
                _ => Vec::new(),
            };

            // Require an exact contextual spelling for the leftover using the allowed set
            self.fill_at(
                idx + 1,
                leftover,
                &allowed,
                Either::Left(self.beats[idx].with_kind(Rest)),
            )?;
        }

        self.beats[idx] = new_beat;
        self.beats[idx].accented = old_accent && beat.kind == Note;
        Ok(())
    }

    fn compute_ticks_to_absorb(&self, idx: BeatIdx, dur_old: Duration, need: u32) -> u32 {
        let mut absorb_ticks = 0u32;
        let mut k = idx + 1;
        while k < self.beats.len() {
            let t = DEFAULT_GRID.ticks_of(&self.beats[k].duration).unwrap();
            // Respect tuplet id/group boundaries when growing
            match dur_old {
                // If we are inside a tuplet slot, limit absorption strictly to the bounds and grid of
                // the current tuplet group.
                Duration::Tuplet { .. } => {
                    // If current slot is part of an id-annotated group, never cross to a different id
                    if let Some(cur_id) = self.beats[idx].tuplet_group_id
                        && self.beats[k].tuplet_group_id == Some(cur_id)
                    {
                        absorb_ticks += t;
                    } else {
                        break;
                    }
                }
                // Absorb from following beats freely if we are not inside a tuplet slot.
                _ => {
                    // But do not cross into any tuplet group (protected region)
                    if self.beats[k].tuplet_group_id.is_some() {
                        break;
                    }
                    absorb_ticks += t;
                }
            }
            if absorb_ticks >= need {
                break;
            }
            k += 1;
        }
        absorb_ticks
    }

    pub fn time_signature(&self) -> TimeSignature { self.time_signature }

    /// Change the measure's time signature using the default policy:
    /// - If the new measure is shorter: remove whole beats from the end; if a tuplet is touched,
    ///   remove the entire tuplet group (no splitting).
    /// - If the new measure is longer (or space remains after removals): pad with rests using the
    ///   new signature's primary beat unit.
    ///
    /// Returns a Modification describing the TS change. Undo/redo is expected to be snapshot-based
    /// in the UI, but we still report a consolidated modification.
    pub fn set_time_signature(
        &mut self,
        new_ts: TimeSignature,
    ) -> Result<Modification, MeasureError> {
        use crate::measure::editing::Modification;

        let old_ts = self.time_signature;
        if old_ts == new_ts {
            return Ok(Modification::ChangeTimeSignature(old_ts, new_ts));
        }

        // Compute current used ticks and target capacity in ticks
        let mut used: u32 =
            self.beats.iter().map(|b| DEFAULT_GRID.ticks_of(&b.duration).unwrap_or(0)).sum();
        let new_max = DEFAULT_GRID.ticks_per_measure(&new_ts);

        // If we need to shrink: remove from the tail, whole beats or entire tuplet groups
        if used > new_max {
            let mut i: isize = self.beats.len() as isize - 1;
            while i >= 0 && used > new_max {
                let idx = i as usize;
                let beat = self.beats[idx];
                if let Some(GroupSpan { start_idx, end_idx, id: group_id }) =
                    self.find_group_span(idx)
                {
                    // Sum ticks for the span and remove it
                    let span_ticks: u32 = (start_idx..=end_idx)
                        .map(|k| DEFAULT_GRID.ticks_of(&self.beats[k].duration).unwrap_or(0))
                        .sum();
                    // Remove anchor entry if present
                    self.tuplet_anchors.remove(&group_id);
                    self.beats.drain(start_idx..=end_idx);
                    used = used.saturating_sub(span_ticks);
                    i = start_idx as isize - 1;
                } else {
                    let removed = DEFAULT_GRID.ticks_of(&beat.duration).unwrap_or(0);
                    self.beats.remove(idx);
                    used = used.saturating_sub(removed);
                    i -= 1;
                }
            }
        }

        // Apply the new time signature
        self.time_signature = new_ts;

        // If we have remaining space, pad with rests
        if used < new_max {
            let remaining = new_max - used;
            let insert_at = self.beats.len();
            self.fill_at(insert_at, remaining, &[], Either::Right(Rest))?;
        }

        Ok(Modification::ChangeTimeSignature(old_ts, new_ts))
    }

    /// Return a vector with the absolute position (1-based) of each beat as floats.
    /// Examples:
    /// - In 4/4 with four quarters: [1.0, 2.0, 3.0, 4.0]
    /// - If the first three notes are 8th-note triplets in 4/4: [1.0, 1.3333..., 1.6666...]
    ///
    /// Positions are computed from onset ticks relative to the measure's beat size (beat_unit).
    pub fn beat_positions(&self) -> Vec<f32> {
        let onsets = DEFAULT_GRID.compute_onset_ticks(&self.beats);
        let ticks_per_beat = DEFAULT_GRID.ticks_per_beat(&self.time_signature);
        onsets.into_iter().map(|t| 1.0f32 + (t as f32) / (ticks_per_beat as f32)).collect()
    }

    fn max_ticks(&self) -> u32 { DEFAULT_GRID.ticks_per_measure(&self.time_signature) }

    fn remaining_ticks(&self, idx: BeatIdx) -> u32 {
        if idx >= self.beats.len() {
            return 0;
        }
        let mut sum = 0u32;
        for b in &self.beats[idx..] {
            sum += DEFAULT_GRID.ticks_of(&b.duration).unwrap();
        }
        sum
    }

    pub fn remove(&mut self, idx: BeatIdx) {
        if idx >= self.beats.len() {
            return;
        }
        self.beats[idx].kind = Rest;
    }

    fn fill_measure(&mut self, kind: BeatKind, allowed: &[Duration]) {
        if !self.beats.is_empty() {
            // currently we only support filling empty/uninitalized measures
            return;
        }
        if let Some(fill) = self.best_fill_for_gap(self.max_ticks(), allowed) {
            let take = fill.len();
            for duration in fill.into_iter().take(take) {
                let beat = Beat { duration, kind, accented: false, tuplet_group_id: None };
                self.beats.push(beat);
            }
        }
    }

    fn fill_at(
        &mut self,
        idx: BeatIdx,
        ticks: u32,
        allowed: &[Duration],
        init: Either<Beat, BeatKind>,
    ) -> Result<(), MeasureError> {
        if let Some(fill) = self.best_fill_for_gap(ticks, allowed) {
            let mut insert_at = idx;
            for d in fill {
                let beat = init.either(
                    |mut beat| {
                        beat.duration = d;
                        beat
                    },
                    |beat_kind| Beat::new(d, beat_kind),
                );
                self.beats.insert(insert_at, beat);
                insert_at += 1;
            }
            Ok(())
        } else {
            self.unfillable_err(ticks)
        }
    }

    /// Toggle the beat kind at `idx` between Note and Rest while preserving duration.
    /// No-op if `idx` is out of bounds.
    pub fn toggle_beat_kind(&mut self, idx: BeatIdx) -> Option<Modification> {
        if let Some(b) = self.beats.get_mut(idx) {
            let new_kind = match b.kind {
                Rest => Note,
                Note => Rest,
            };
            b.kind = new_kind;
            b.accented = b.accented && b.kind == Note;
            Some(Modification::ToggleKind(idx, new_kind))
        } else {
            None
        }
    }

    fn unfillable_err(&self, attempted: u32) -> Result<(), MeasureError> {
        Err(MeasureError::Unfillable { attempted: DEFAULT_GRID.ticks_to_whole_notes(attempted) })
    }

    fn overflow_err(&self, attempted: u32, remaining: u32) -> Result<(), MeasureError> {
        Err(MeasureError::Overflow {
            attempted: DEFAULT_GRID.ticks_to_whole_notes(attempted),
            available: DEFAULT_GRID.ticks_to_whole_notes(remaining),
        })
    }
}

impl Debug for Measure {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.beats.iter().enumerate().try_fold((), |_, (idx, beat)| {
            beat.fmt(f)?;
            if idx < self.beats.len() - 1 {
                write!(f, " ")?
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure::duration::NoteValue::{Eighth, Sixteenth, ThirtySecond};
    use crate::measure::duration::{e, q, s, st16, st8, t16, t32, t8, th, Duration};

    fn durations_of(measure: &Measure) -> Vec<Duration> {
        measure.beats().iter().map(|b| b.duration).collect()
    }

    #[test]
    fn test_triplet_in_one_four() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);

        assert!(m.set_beat(0, Beat::note(t8())).is_ok());
        let Beat { duration, kind, .. } = m.beats()[1];
        assert_eq!(duration, t8());
        assert_eq!(kind, Rest);
        let Beat { duration, kind, .. } = m.beats()[2];
        assert_eq!(duration, t8());
        assert_eq!(kind, Rest);

        assert!(m.set_beat(1, Beat::note(q())).is_err());
        assert!(m.set_beat(1, Beat::note(e())).is_err());
        assert!(m.set_beat(1, Beat::note(s())).is_err());
        assert!(m.set_beat(1, Beat::note(th())).is_err());

        assert!(m.set_beat(1, Beat::note(t8())).is_ok());
        assert!(m.set_beat(2, Beat::rest(t8())).is_ok());
    }

    #[test]
    fn test_triplet_insertions_0() {
        let mut m = Measure::new(TimeSignature::TWO_FOUR);

        // First triplet group
        assert!(m.set_beat(0, Beat::note(t16())).is_ok());
        assert!(m.set_beat(1, Beat::note(t16())).is_ok());
        // The next triplet 1/8 overfills this tuplet group, which has only space for one triplet
        // 1/6 note left (or two triplet 1/32 subdivisions).
        assert!(m.set_beat(2, Beat::note(t8())).is_err());
    }

    #[test]
    fn test_triplet_insertions_1() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);

        assert!(m.set_beat(0, Beat::note(t16())).is_ok());
        assert!(m.set_beat(1, Beat::note(t8())).is_ok());
        assert!(
            m.set_beat(2, Beat::note(t8())).is_err(),
            "can't start a new triplet 1/8 group measure has only 1/8 space left"
        );
        assert!(m.set_beat(2, Beat::note(e())).is_ok());
        assert_eq!(m.remaining_ticks(3), 0);
    }

    #[test]
    fn test_triplet_insertions_2() {
        let mut m = Measure::new(TimeSignature::TWO_FOUR);

        // First triplet group
        assert!(m.set_beat(0, Beat::note(t8())).is_ok());
        assert!(m.set_beat(1, Beat::note(t16())).is_ok());
        assert!(m.set_beat(2, Beat::note(t16())).is_ok());
        assert!(m.set_beat(3, Beat::note(t8())).is_ok());

        // Second triplet group
        assert!(m.set_beat(4, Beat::note(t16())).is_ok());
        assert!(m.set_beat(5, Beat::note(t32())).is_ok());
        assert!(m.set_beat(6, Beat::note(t32())).is_ok());
        assert!(m.set_beat(7, Beat::note(t32())).is_ok());
        assert!(m.set_beat(8, Beat::note(t32())).is_ok());

        assert!(m.set_beat(9, Beat::note(t8())).is_err());
        assert!(m.set_beat(9, Beat::note(e())).is_ok());
        assert_eq!(m.remaining_ticks(10), 0);
    }

    #[test]
    fn test_triplet_insertions_3() {
        let mut m = Measure::new(TimeSignature::TWO_EIGHT);

        assert!(m.set_beat(0, Beat::note(t8())).is_ok());
        assert!(m.set_beat(1, Beat::note(t16())).is_ok());
        assert!(m.set_beat(2, Beat::note(t8())).is_ok());
        // The next triplet 1/8 overfills this tuplet group, which has only space for one triplet
        // 1/6 note left (or two triplet 1/32 subdivisions).
        assert!(m.set_beat(3, Beat::note(t8())).is_err());
        assert!(m.set_beat(3, Beat::note(t32())).is_ok());
        // Doesn't fit.
        assert!(m.set_beat(4, Beat::note(t16())).is_err());
        assert!(m.set_beat(4, Beat::note(t32())).is_ok());

        // The next beat starts a new tuplet group, but we don't have enough space in our measure.
        assert_eq!(m.remaining_ticks(5), 0);
    }

    #[test]
    fn test_triplet_insertions_4() {
        let mut m = Measure::new(TimeSignature::SEVEN_EIGHT);

        assert!(m.set_beat(1, Beat::rest(s())).is_ok());
        assert!(m.set_beat(1, Beat::rest(t8())).is_ok());
        assert_eq!(&durations_of(&m), &[e(), t8(), t8(), t8(), e(), e(), e(), e()]);
    }

    #[test]
    fn test_triplet_insertion_in_seven_eight() {
        let mut m = Measure::new(TimeSignature::SEVEN_EIGHT);
        assert!(m.set_beat(0, Beat::note(t8())).is_ok());
        assert!(m.set_beat(1, Beat::note(t8())).is_ok());
        assert!(m.set_beat(2, Beat::note(t16())).is_ok());
    }

    #[test]
    fn test_triplet_split_1() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        assert!(m.set_beat(0, Beat::note(t8())).is_ok());
        assert!(m.set_beat(1, Beat::note(t8())).is_ok());
        assert!(m.set_beat(2, Beat::note(t8())).is_ok());

        assert!(m.set_beat(1, Beat::note(t16())).is_ok());
        assert_eq!(&durations_of(&m), &[t8(), t16(), t16(), t8()]);

        assert!(m.set_beat(0, Beat::note(t16())).is_ok());
        assert_eq!(&durations_of(&m), &[t16(), t16(), t16(), t16(), t8()]);
    }

    #[test]
    fn test_triplet_split_2() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        assert!(m.set_beat(0, Beat::note(t8())).is_ok());
        assert!(m.set_beat(1, Beat::note(t8())).is_ok());
        assert!(m.set_beat(2, Beat::note(t8())).is_ok());

        assert!(m.set_beat(2, Beat::note(t32())).is_ok());
        assert_eq!(&durations_of(&m), &[t8(), t8(), t32(), t32(), t16()]);
    }

    #[test]
    fn test_triplet_split_3() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        assert!(m.set_beat(0, Beat::note(t32())).is_ok());
        assert!(m.set_beat(1, Beat::note(t32())).is_ok());
        assert!(m.set_beat(2, Beat::note(t32())).is_ok());

        assert!(m.set_beat(0, Beat::note(t8())).is_err());
        assert!(m.set_beat(0, Beat::note(t16())).is_ok());
        assert_eq!(&durations_of(&m), &[t16(), t32(), s(), e()]);

        assert!(m.set_beat(2, Beat::note(t16())).is_ok());
        assert_eq!(&durations_of(&m), &[t16(), t32(), t16(), t16(), t16(), s()]);
    }

    #[test]
    fn test_triplet_split_4() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        assert!(m.set_beat(0, Beat::note(t16())).is_ok());
        assert!(m.set_beat(1, Beat::note(t16())).is_ok());
        assert!(m.set_beat(2, Beat::note(t16())).is_ok());

        // Cannot merge last note in the group (not enough space).
        assert!(m.set_beat(2, Beat::note(t8())).is_err());

        // Merge t16 note in the middle.
        assert!(m.set_beat(1, Beat::note(t8())).is_ok());
        assert_eq!(&durations_of(&m), &[t16(), t8(), e()]);

        // Subdivide t8 note in the middle.
        assert!(m.set_beat(1, Beat::rest(t16())).is_ok());
        assert_eq!(&durations_of(&m), &[t16(), t16(), t16(), e()]);

        // Subdivide third t16 note.
        assert!(m.set_beat(2, Beat::rest(t32())).is_ok());
        assert_eq!(&durations_of(&m), &[t16(), t16(), t32(), t32(), e()]);

        // Merge second note to t8. Must work because the remainder of the tuplet has enough space.
        assert!(m.set_beat(1, Beat::rest(t8())).is_ok());
        assert_eq!(&durations_of(&m), &[t16(), t8(), e()]);
    }

    #[test]
    fn test_tuplet_split_5() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        m.set_beat(0, Beat::rest(st16())).unwrap();

        // Merge 2nd+3rd st16 beats -> st8
        m.set_beat(1, Beat::rest(st8())).unwrap();
        assert_eq!(&durations_of(&m), &[st16(), st8(), st16(), st16(), st16()]);

        // Merge 1st st16 beat -> st8
        m.set_beat(0, Beat::rest(st8())).unwrap();
        assert_eq!(&durations_of(&m), &[st8(), st16(), st16(), st16(), st16()]);
    }

    #[test]
    fn test_add_eighth_triplet_to_seven_eight_measure() {
        let mut m = Measure::new(TimeSignature::SEVEN_EIGHT);
        m.set_beat(0, Beat::note(Duration::Dotted { base: Eighth, dots: 1 })).unwrap();
        m.set_beat(1, Beat::note(Duration::Dotted { base: Sixteenth, dots: 1 })).unwrap();
        m.set_beat(2, Beat::note(Duration::Simple(ThirtySecond))).unwrap();
        m.set_beat(3, Beat::note(Duration::Simple(Sixteenth))).unwrap();
        m.set_beat(4, Beat::note(Duration::Simple(Eighth))).unwrap();
        m.set_beat(5, Beat::note(Duration::Simple(Sixteenth))).unwrap();
        m.set_beat(6, Beat::rest(Duration::Dotted { base: Eighth, dots: 1 })).unwrap();
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
        m.set_beat(0, Beat::note(t8())).unwrap();
        m.set_beat(1, Beat::note(t8())).unwrap();
        m.set_beat(2, Beat::note(t8())).unwrap();
        let pos = m.beat_positions();
        // Expect positions: 1.0, 1 + 1/3, 1 + 2/3
        let expect = [1.0f32, 1.0 + 1.0 / 3.0, 1.0 + 2.0 / 3.0];
        let eps = 1e-4f32;
        assert!((pos[0] - expect[0]).abs() < eps);
        assert!((pos[1] - expect[1]).abs() < eps);
        assert!((pos[2] - expect[2]).abs() < eps);
    }

    #[test]
    fn append_autofill_to_primary_boundary_simple() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Insert one eighth at start; autofill up to the next quarter boundary
        assert!(m.set_beat(0, Beat::note(e())).is_ok());
        // Expect: e() followed by an e() rest to reach the quarter boundary
        assert_eq!(m.beats()[1], Beat::rest(e()));
    }

    #[test]
    fn append_autofill_to_primary_boundary_triplet() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        assert!(m.set_beat(0, Beat::note(t8())).is_ok());
        // Expect two triplet-eighth rests to complete the triplet group
        let Beat { duration: d1, kind: k1, .. } = m.beats()[1];
        let Beat { duration: d2, kind: k2, .. } = m.beats()[2];
        assert_eq!(d1, t8());
        assert_eq!(k1, Rest);
        assert_eq!(d2, t8());
        assert_eq!(k2, Rest);
    }

    #[test]
    fn tuplet_group_0() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        m.set_beat(0, Beat::rest(t8())).unwrap();
        // Subdivide 2nd t8 + 3rd t8 beat into two t16s each:
        m.set_beat(1, Beat::rest(t16())).unwrap();
        // Merge tuplet at the 'odd' position. This yields the measure: t8 t16 t8 t16
        m.set_beat(2, Beat::rest(t8())).unwrap();

        assert_eq!(&durations_of(&m), &[t8(), t16(), t8(), t16()]);
        assert_eq!(
            m.beats().iter().enumerate().find_map(|(idx, beat)| {
                if !matches!(beat.tuplet_group_id, Some(1)) { Some(idx) } else { None }
            }),
            None
        );
    }

    #[test]
    fn set_bigger_beat_on_smaller_rest() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);

        // Arrange: create rests 1/16 at idx 0 and 1/8 at idx 1
        m.set_beat(0, Beat::rest(s())).unwrap();
        m.set_beat(1, Beat::rest(e())).unwrap();

        // Act: set a bigger note (1/8) at position 0
        assert!(m.set_beat(0, Beat::note(e())).is_ok());

        // Assert: index 0 becomes 1/8 note, index 1 becomes 1/16 rest
        assert_eq!(m.beats()[0], Beat::note(e()));
        assert_eq!(m.beats()[1], Beat::rest(s()));
    }

    #[test]
    fn change_ts_removes_entire_final_triplet_group_on_shrink() {
        // Arrange: 4/4 where the last quarter is a triplet of eighths
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Replace the last quarter with an 1/8 triplet group (occupies exactly one quarter)
        m.set_beat(3, Beat::note(t8())).unwrap();

        // Act: shrink 4/4 -> 3/4
        let modif = m.set_time_signature(TimeSignature::THREE_FOUR).unwrap();
        match modif {
            Modification::ChangeTimeSignature(old, new) => {
                assert_eq!(old, TimeSignature::FOUR_FOUR);
                assert_eq!(new, TimeSignature::THREE_FOUR);
            }
            _ => panic!("unexpected modification variant"),
        }

        // Should consist of three quarters (rests by default)
        assert_eq!(m.beats.to_vec(), vec![Beat::rest(q()), Beat::rest(q()), Beat::rest(q())]);
    }

    #[test]
    fn change_ts_keep_and_pad_on_extend() {
        // Arrange: 3/4 with two eighths at the beginning
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        assert!(m.set_beat(0, Beat::note(e())).is_ok());
        assert!(m.set_beat(1, Beat::note(e())).is_ok());

        // Act: extend to 2/4
        let _ = m.set_time_signature(TimeSignature::TWO_FOUR).unwrap();

        // Assert: contents preserved; the measure is padded with one quarter rest at the end
        assert_eq!(m.time_signature(), TimeSignature::TWO_FOUR);
        assert_eq!(m.beats().to_vec(), vec![Beat::note(e()), Beat::note(e()), Beat::rest(q())])
    }

    #[test]
    fn change_ts_shorten_non_tuplet_whole_beats() {
        // Arrange: default 4/4 is four quarter rests
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        m.set_beat(1, Beat::note(q())).unwrap();

        // Act: shrink to 2/4
        let _ = m.set_time_signature(TimeSignature::TWO_FOUR).unwrap();

        // Assert: last two quarters removed; no splitting; exactly two quarter beats remain
        assert_eq!(m.time_signature(), TimeSignature::TWO_FOUR);
        assert_eq!(m.beats().to_vec(), vec![Beat::rest(q()), Beat::note(q())]);
    }

    #[test]
    fn change_ts_removes_entire_offset_triplet_group_on_shrink() {
        // Arrange: 4/4 where the last quarter is a triplet of eighths
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        m.set_beat(0, Beat::note(e())).unwrap(); // offset triplet by an 1/8
        m.set_beat(1, Beat::note(t8())).unwrap();

        // Act: shrink 4/4 -> 1/4
        let modif = m.set_time_signature(TimeSignature::ONE_FOUR).unwrap();
        match modif {
            Modification::ChangeTimeSignature(old, new) => {
                assert_eq!(old, TimeSignature::FOUR_FOUR);
                assert_eq!(new, TimeSignature::ONE_FOUR);
            }
            _ => panic!("unexpected modification variant"),
        }

        assert_eq!(m.beats().to_vec(), vec![Beat::note(e()), Beat::rest(e())]);
    }
}
