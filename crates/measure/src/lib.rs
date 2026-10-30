mod beat;
pub mod click;
pub mod counter;
pub mod counting;
pub mod duration;
pub mod editing;
mod fill;
pub mod generator;
pub mod ghost;
pub mod grid;
pub(crate) mod grouping;
pub mod head;
mod math;
pub mod notation;
pub mod onsets;
#[cfg(test)]
mod random_tests;
pub mod score;
pub mod swing;
pub mod tap_tempo;
pub mod tempo;
pub mod time_signature;
pub mod tuplet;
mod voice;

use BeatKind::Note;
use BeatKind::Rest;
pub use beat::{Beat, BeatKind};
use duration::{Duration, TupletSpec};
use editing::{GroupSpan, Modification};
use either::Either;
use grid::DEFAULT_GRID;
pub use head::{Head, Heads, Instrument, Technique};
pub use score::{Cursor, EmptyScore, MeasureIdx, Score, StaffKind};
use std::fmt::{Debug, Formatter};
pub use time_signature::{InvalidTimeSignature, ParseTimeSignatureError, TimeSignature};
pub use tuplet::{TupletAnchor, TupletGroupId, TupletRegistry};
pub use voice::{DifferentMeter, Measure, VoiceBar, VoiceIdx};

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

/// Classification of how `set_beat` will mutate the measure. Produced by
/// [`Measure::classify_replacement`] and consumed by [`Measure::set_beat`].
#[derive(Debug, PartialEq, Eq)]
enum ReplacementKind {
    /// New duration equals old: just overwrite the slot.
    InPlace,
    /// New duration is larger than old: absorb following beats.
    Grow,
    /// New duration is smaller than old: refill the trailing ticks.
    Shrink { leftover: u32 },
    /// Non-tuplet slot becomes a tuplet: construct the whole group.
    NewTupletGroup(TupletSpec),
}

/// Why a saved measure does not hold (see [`Measure`]); spans in grid ticks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidMeasure {
    /// The measure has no voice.
    NoVoice,
    /// The beat at `idx` has a duration the measure does not support.
    UnsupportedDuration { idx: BeatIdx },
    /// The beats do not fill the measure exactly.
    WrongLength { ticks: u64, expected: u32 },
    /// The beat at `idx` is in a tuplet group without an anchor.
    UnknownTupletGroup { idx: BeatIdx, id: TupletGroupId },
    /// The beats of a tuplet group do not fill its anchor's span.
    TupletGroupLength { id: TupletGroupId, ticks: u64, expected: u32 },
}

impl std::fmt::Display for InvalidMeasure {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoVoice => write!(f, "the measure has no voice"),
            Self::UnsupportedDuration { idx } => {
                write!(f, "beat {idx} has an unsupported duration")
            }
            Self::WrongLength { ticks, expected } => {
                write!(f, "beats span {ticks} ticks, the measure {expected}")
            }
            Self::UnknownTupletGroup { idx, id } => {
                write!(f, "beat {idx} is in tuplet group {} without an anchor", id.as_u32())
            }
            Self::TupletGroupLength { id, ticks, expected } => {
                write!(f, "tuplet group {} spans {ticks} ticks, its anchor {expected}", id.as_u32())
            }
        }
    }
}

impl std::error::Error for InvalidMeasure {}

impl VoiceBar {
    /// Read-only access to the tuplet anchor registry.
    pub fn tuplets(&self) -> &TupletRegistry {
        &self.tuplets
    }

    /// Expose a read-only view of beats
    pub fn beats(&self) -> &Vec<Beat> {
        &self.beats
    }

    /// Replace the beat at index `idx` with `beat` if it fits and the remainder stays fillable.
    ///
    /// Internally classifies the edit via [`Measure::classify_replacement`] and
    /// dispatches to the matching handler:
    /// - [`ReplacementKind::InPlace`] — same tick count, write the slot in place.
    /// - [`ReplacementKind::Grow`] — larger duration, absorb following beats.
    /// - [`ReplacementKind::Shrink`] — smaller duration, refill the leftover ticks.
    /// - [`ReplacementKind::NewTupletGroup`] — non-tuplet slot becomes a tuplet,
    ///   construct the whole group.
    pub fn set_beat(&mut self, idx: BeatIdx, beat: Beat) -> Result<(), MeasureError> {
        assert!(idx < self.beats.len());
        let (kind, new_beat, dur_old) = self.classify_replacement(idx, beat)?;
        match kind {
            ReplacementKind::NewTupletGroup(spec) => {
                self.insert_tuplet_group_at(idx, new_beat, spec)
            }
            ReplacementKind::Grow => self.grow_beat(idx, new_beat, dur_old),
            ReplacementKind::Shrink { leftover } => {
                self.shrink_remainder(idx, dur_old, leftover)?;
                self.write_in_place(idx, new_beat);
                Ok(())
            }
            ReplacementKind::InPlace => {
                self.write_in_place(idx, new_beat);
                Ok(())
            }
        }
    }

    /// Validate a `set_beat` request and classify the resulting operation.
    ///
    /// Returns the classification, the adjusted new beat (with tuplet group id
    /// inherited from the slot when the grid stays the same), and the slot's
    /// previous duration (handy for downstream handlers that gate on the old
    /// tuplet grid). Errors mirror the public `set_beat` contract.
    fn classify_replacement(
        &self,
        idx: BeatIdx,
        beat: Beat,
    ) -> Result<(ReplacementKind, Beat, Duration), MeasureError> {
        let dur_old = self.beats[idx].duration;
        let new_ticks = DEFAULT_GRID
            .ticks_of(&beat.duration)
            .ok_or(MeasureError::Unfillable { attempted: 0.0 })?;

        // The grid alone may accept more rational durations than the configured
        // DurationSet officially supports - reject the latter.
        if !DEFAULT_GRID.supports(&beat.duration) {
            return Err(MeasureError::Unfillable {
                attempted: DEFAULT_GRID.ticks_to_whole_notes(new_ticks),
            });
        }

        // Inside a tuplet slot only a same-grid tuplet replacement is allowed,
        // and it must inherit the existing group id.
        let mut new_beat = beat;
        if let Some(spec_old) = dur_old.as_tuplet_spec() {
            match beat.duration.as_tuplet_spec() {
                Some(spec_new) if spec_new.same_grid(&spec_old) => {
                    new_beat.tuplet_group_id = self.beats[idx].tuplet_group_id;
                }
                _ => {
                    return Err(MeasureError::Unfillable {
                        attempted: DEFAULT_GRID.ticks_to_whole_notes(new_ticks),
                    });
                }
            }
        }

        // Non-tuplet → tuplet always builds a whole new group.
        if let Some(spec) = new_beat.duration.as_tuplet_spec()
            && !dur_old.is_tuplet()
        {
            return Ok((ReplacementKind::NewTupletGroup(spec), new_beat, dur_old));
        }

        let old_ticks = DEFAULT_GRID.ticks_of(&dur_old).unwrap();
        let kind = match new_ticks.cmp(&old_ticks) {
            std::cmp::Ordering::Greater => ReplacementKind::Grow,
            std::cmp::Ordering::Less => ReplacementKind::Shrink { leftover: old_ticks - new_ticks },
            std::cmp::Ordering::Equal => ReplacementKind::InPlace,
        };
        Ok((kind, new_beat, dur_old))
    }

    /// Overwrite the slot at `idx` with `new_beat`, carrying the slot's prior
    /// accent forward (only on notes - rests can never be accented).
    fn write_in_place(&mut self, idx: BeatIdx, new_beat: Beat) {
        let old_accent = self.beats[idx].accented();
        self.beats[idx] = new_beat;
        self.beats[idx].set_accented(old_accent);
    }

    /// Replace `[idx..]` with a freshly created tuplet group described by `spec`.
    ///
    /// Consumes following beats until the group span (`m * base`) is covered, allocates
    /// a stable group id + anchor, inserts `new_beat` followed by `n-1` matching rests,
    /// and reinstates any overrun as rests after the group.
    fn insert_tuplet_group_at(
        &mut self,
        idx: BeatIdx,
        new_beat: Beat,
        spec: TupletSpec,
    ) -> Result<(), MeasureError> {
        let TupletSpec { n, m, base } = spec;
        let old_accent = self.beats[idx].accented();
        let old_ticks = DEFAULT_GRID.ticks_of(&self.beats[idx].duration).unwrap();
        let new_ticks = DEFAULT_GRID.ticks_of(&new_beat.duration).unwrap();
        let max_ticks = self.max_ticks();

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
            // Never grow across an existing tuplet group boundary with different id
            if self.beats[k].tuplet_group_id.is_some() {
                return self.unfillable_err(group_span);
            }
            // If we encounter a tuplet of a different grid (different n/m), refuse
            // (don't break existing groups). Base may differ (e.g., t8 vs t16) but
            // n/m define the grid equivalence here.
            if let Some(other) = self.beats[k].duration.as_tuplet_spec()
                && !other.same_grid(&spec)
            {
                return self.unfillable_err(new_ticks);
            }
            consumed += DEFAULT_GRID.ticks_of(&self.beats[k].duration).unwrap();
            k += 1;
        }

        // We will replace [idx..k) with the tuplet group (n items). If we consumed
        // more ticks than the group span, we owe back a remainder after the group.
        let overrun = consumed - group_span;

        self.beats.drain(idx..k);

        // Allocate a stable id and register an anchor for this group
        let id = self.tuplets.register(n, m, group_span, base);

        // Insert the tuplet items: first the requested beat, then n-1 rests of same tuplet duration
        let mut first = new_beat;
        first.tuplet_group_id = Some(id);
        self.beats.insert(idx, first);
        // Preserve the prior accent on the first inserted beat
        self.beats[idx].set_accented(old_accent);
        let mut insert_at = idx + 1;
        for _ in 1..n {
            let mut r = Beat::rest(new_beat.duration);
            r.tuplet_group_id = Some(id);
            self.beats.insert(insert_at, r);
            insert_at += 1;
        }

        if overrun > 0 {
            self.fill_at(insert_at, overrun, &[], Either::Right(Rest))?
        }

        Ok(())
    }

    /// Grow the beat at `idx` to `new_beat`, absorbing ticks from following beats.
    ///
    /// Fails with `Overflow` if not enough absorbable space exists ahead. If absorption
    /// partially consumes a beat, its remainder is refilled (constrained to the same
    /// tuplet grid when growing inside a tuplet slot).
    fn grow_beat(
        &mut self,
        idx: BeatIdx,
        new_beat: Beat,
        dur_old: Duration,
    ) -> Result<(), MeasureError> {
        let old_ticks = DEFAULT_GRID.ticks_of(&dur_old).unwrap();
        let new_ticks = DEFAULT_GRID.ticks_of(&new_beat.duration).unwrap();
        let max_ticks = self.max_ticks();
        let old_accent = self.beats[idx].accented();

        let need = new_ticks - old_ticks;
        debug_assert!(need > 0);

        // A tuplet note may not fill its whole group alone (e.g. one 2:3
        // quarter in a 2:3 eighth group): a group needs two notes.
        if let Some(anchor) = self.beats[idx].tuplet_group_id.and_then(|id| self.tuplets.get(id))
            && new_ticks >= anchor.target_ticks
        {
            return self.overflow_err(new_ticks, max_ticks - old_ticks);
        }

        let absorb_ticks = self.compute_ticks_to_absorb(idx, dur_old, need);
        if absorb_ticks < need {
            return self.overflow_err(new_ticks, max_ticks - old_ticks);
        }

        self.beats[idx] = new_beat;
        self.beats[idx].set_accented(old_accent);
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
                let allowed: Vec<Duration> = dur_old
                    .as_tuplet_spec()
                    .map(|s| DEFAULT_GRID.tuplet_grid_durations(s.n, s.m))
                    .unwrap_or_default();
                // When growing, the consumed remainder after the enlarged beat
                // should preserve the original beat's kind (Note vs Rest) and metadata
                // (e.g. tuplet ids), but adapt to the new duration.
                self.fill_at(p, new_ticks_rest, &allowed, Either::Left(b))?;
                remaining_to_consume = 0;
            }
        }
        Ok(())
    }

    /// Fill the `leftover` ticks behind `idx` after shrinking the beat there.
    ///
    /// When shrinking inside a tuplet slot, the filler is restricted to the same
    /// tuplet grid (same `n, m`).
    fn shrink_remainder(
        &mut self,
        idx: BeatIdx,
        dur_old: Duration,
        leftover: u32,
    ) -> Result<(), MeasureError> {
        let allowed: Vec<Duration> = dur_old
            .as_tuplet_spec()
            .map(|s| DEFAULT_GRID.tuplet_grid_durations(s.n, s.m))
            .unwrap_or_default();
        self.fill_at(idx + 1, leftover, &allowed, Either::Left(self.beats[idx].with_kind(Rest)))
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

    /// Inclusive index range of the tuplet group that contains the beat at `idx`,
    /// together with the group's stable id.
    ///
    /// Returns `None` if the beat is not part of a tuplet group or if the group
    /// has only a single beat (a degenerate state that callers treat as "no group").
    pub fn tuplet_group_at(&self, idx: BeatIdx) -> Option<GroupSpan> {
        let id = self.beats.get(idx)?.tuplet_group_id?;
        let start_idx = self.tuplet_group_start(idx, id);
        let end_exclusive = self.tuplet_group_end(idx, id);
        if end_exclusive - start_idx < 2 {
            return None;
        }
        Some(GroupSpan { start_idx, end_idx: end_exclusive - 1, id })
    }

    /// All tuplet groups in beat order. Singleton groups are skipped (see
    /// [`Measure::tuplet_group_at`]).
    pub fn tuplet_groups(&self) -> Vec<GroupSpan> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.beats.len() {
            let Some(id) = self.beats[i].tuplet_group_id else {
                i += 1;
                continue;
            };
            // Only emit on the first beat of a group
            if i > 0 && self.beats[i - 1].tuplet_group_id == Some(id) {
                i += 1;
                continue;
            }
            let end_exclusive = self.tuplet_group_end(i, id);
            if end_exclusive - i >= 2 {
                out.push(GroupSpan { start_idx: i, end_idx: end_exclusive - 1, id });
            }
            i = end_exclusive;
        }
        out
    }

    fn tuplet_group_start(&self, idx: BeatIdx, id: TupletGroupId) -> BeatIdx {
        let mut start = idx;
        while start > 0 && self.beats[start - 1].tuplet_group_id == Some(id) {
            start -= 1;
        }
        start
    }

    fn tuplet_group_end(&self, idx: BeatIdx, id: TupletGroupId) -> BeatIdx {
        let mut end = idx + 1;
        while end < self.beats.len() && self.beats[end].tuplet_group_id == Some(id) {
            end += 1;
        }
        end
    }

    /// Return a vector with the absolute position (1-based) of each beat as floats.
    /// Examples:
    /// - In 4/4 with four quarters: [1.0, 2.0, 3.0, 4.0]
    /// - If the first three notes are 8th-note triplets in 4/4: [1.0, 1.3333..., 1.6666...]
    ///
    /// Positions are computed from onset ticks relative to the measure's beat size (beat_unit).
    pub fn beat_positions(&self) -> Vec<f32> {
        let onsets = DEFAULT_GRID.compute_onset_ticks(&self.beats);
        let ticks_per_beat = DEFAULT_GRID.ticks_per_beat(&self.time_signature());
        onsets.into_iter().map(|t| 1.0f32 + (t as f32) / (ticks_per_beat as f32)).collect()
    }

    fn max_ticks(&self) -> u32 {
        DEFAULT_GRID.ticks_per_measure(&self.time_signature())
    }

    /// Play `heads` at `idx`: a rest if there are none. The duration and
    /// tuplet group stay, so the measure stays valid. No-op if `idx` is out
    /// of bounds.
    pub fn set_heads(&mut self, idx: BeatIdx, heads: Heads) {
        if let Some(beat) = self.beats.get_mut(idx) {
            beat.set_heads(heads);
        }
    }

    /// Turn the beat at `idx` into a rest; it loses its accent and soft flag.
    pub fn remove(&mut self, idx: BeatIdx) {
        if idx >= self.beats.len() {
            return;
        }
        self.beats[idx] = self.beats[idx].with_kind(Rest);
    }

    fn fill_measure(&mut self, kind: BeatKind, allowed: &[Duration]) {
        if !self.beats.is_empty() {
            // currently we only support filling empty/uninitalized measures
            return;
        }
        if let Some(fill) = self.best_fill_for_gap(self.max_ticks(), allowed) {
            let take = fill.len();
            for duration in fill.into_iter().take(take) {
                self.beats.push(Beat::new(duration, kind));
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
            for (insert_at, d) in (idx..).zip(fill) {
                let beat = init.either(
                    |mut beat| {
                        beat.duration = d;
                        beat
                    },
                    |beat_kind| Beat::new(d, beat_kind),
                );
                self.beats.insert(insert_at, beat);
            }
            Ok(())
        } else {
            self.unfillable_err(ticks)
        }
    }

    /// Toggle the beat kind at `idx` between Note and Rest while preserving duration.
    /// A new rest loses its accent and soft flag. No-op if `idx` is out of bounds.
    pub fn toggle_beat_kind(&mut self, idx: BeatIdx) -> Option<Modification> {
        if let Some(b) = self.beats.get_mut(idx) {
            let new_kind = match b.kind() {
                Rest => Note,
                Note => Rest,
            };
            *b = b.with_kind(new_kind);
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

impl Debug for VoiceBar {
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
    use crate::duration::NoteValue::{Eighth, Quarter, Sixteenth, ThirtySecond};
    use crate::duration::{Duration, e, q, s, st8, st16, t8, t16, t32, th};

    fn durations_of(measure: &Measure) -> Vec<Duration> {
        measure.beats().iter().map(|b| b.duration).collect()
    }

    impl VoiceBar {
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
    }

    #[test]
    fn test_triplet_in_one_four() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);

        assert!(m.set_beat(0, Beat::note(t8())).is_ok());
        let (duration, kind) = (m.beats()[1].duration, m.beats()[1].kind());
        assert_eq!(duration, t8());
        assert_eq!(kind, Rest);
        let (duration, kind) = (m.beats()[2].duration, m.beats()[2].kind());
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
    fn test_triplet_insertions_5() {
        let mut m = Measure::new(TimeSignature::TWO_FOUR);

        assert!(m.set_beat(0, Beat::note(t16())).is_ok());

        assert_eq!(&durations_of(&m), &[t16(), t16(), t16(), e(), q()]);
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
        let (d1, k1) = (m.beats()[1].duration, m.beats()[1].kind());
        let (d2, k2) = (m.beats()[2].duration, m.beats()[2].kind());
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
                if !matches!(beat.tuplet_group_id, Some(TupletGroupId(1))) {
                    Some(idx)
                } else {
                    None
                }
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
    fn change_note_to_dotted_does_not_turn_overwritten_beat_to_rest() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);

        m.set_beat(0, Beat::note(e())).unwrap();
        m.set_beat(1, Beat::rest(e())).unwrap();

        let dotted_e = Beat::note(Duration::Dotted { base: Eighth, dots: 1 });
        assert!(m.set_beat(0, dotted_e).is_ok());
        assert_eq!(m.beats.to_vec(), vec![dotted_e, Beat::rest(s())]);

        // Now try again where the second beat is not a rest
        m.set_beat(0, Beat::note(e())).unwrap();
        m.set_beat(1, Beat::note(e())).unwrap();

        assert!(m.set_beat(0, dotted_e).is_ok());
        assert_eq!(m.beats.to_vec(), vec![dotted_e, Beat::note(s())]);
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

    #[test]
    fn rests_keep_no_accent_or_soft_flag() {
        let mut m = Measure::new_init(TimeSignature::FOUR_FOUR, Note);
        m.toggle_accent(0);
        m.toggle_soft(1);
        m.remove(0);
        m.toggle_beat_kind(1);
        assert_eq!(m.beats()[..2], [Beat::rest(q()), Beat::rest(q())]);
        // Back to notes, they come back plain.
        m.toggle_beat_kind(0);
        m.toggle_beat_kind(1);
        assert_eq!(m.beats()[..2], [Beat::note(q()), Beat::note(q())]);
    }

    #[test]
    fn tuplet_note_cannot_fill_its_group_alone() {
        let duplet = |base| Duration::Tuplet(TupletSpec { n: 2, m: 3, base });
        let mut m = Measure::new(TimeSignature::new(3, 8));
        m.set_beat(0, Beat::note(duplet(Eighth))).unwrap();
        assert_eq!(m.beats().len(), 2);
        // One 2:3 quarter would take the whole group of 2:3 eighths.
        assert!(m.set_beat(0, Beat::note(duplet(Quarter))).is_err());
        assert_eq!(m.beats().len(), 2);
        assert!(m.tuplet_group_at(0).is_some());
    }

    fn read(text: &str) -> Result<Measure, String> {
        ron::from_str::<Measure>(text).map_err(|err| err.to_string())
    }

    fn round_trips(m: &Measure) -> bool {
        let back = read(&ron::to_string(m).unwrap()).unwrap_or_else(|err| panic!("{m:?}: {err}"));
        // `Beat`'s `PartialEq` ignores the tuplet group: compare it and its anchor too.
        let groups = |v: &VoiceBar| -> Vec<_> {
            v.beats()
                .iter()
                .map(|b| b.tuplet_group_id.map(|id| (id, v.tuplets().get(id).cloned())))
                .collect()
        };
        back.time_signature() == m.time_signature()
            && back.voices().len() == m.voices().len()
            && back.voices().iter().zip(m.voices()).all(|(b, v)| {
                b.time_signature() == v.time_signature()
                    && b.beats() == v.beats()
                    && groups(b) == groups(v)
                    && b.tuplets().len() == v.tuplets().len()
            })
    }

    fn parse(notation: &str) -> Measure {
        crate::notation::parse_score(notation).unwrap().measures.remove(0)
    }

    #[test]
    fn a_measure_saved_before_voices_reads_as_one_voice() {
        // As saved before voices: the one voice's beats and tuplets inline.
        let old = r#"{"beats":[(duration:Tuplet((n:3,m:2,base:Eighth)),heads:[(instrument:Rhythm)],tuplet_group_id:Some(1)),(duration:Tuplet((n:3,m:2,base:Eighth)),heads:[],tuplet_group_id:Some(1)),(duration:Tuplet((n:3,m:2,base:Eighth)),heads:[],tuplet_group_id:Some(1))],"time_signature":(beats:1,beat_unit:4),"tuplet_anchors":{1:(id:1,n:3,m:2,target_ticks:0,base_hint:Eighth)},"next_tuplet_id":2}"#;
        let m: Measure = ron::from_str(old).unwrap();
        assert_eq!(m.voices().len(), 1);
        assert_eq!(format!("{m:?}"), format!("{:?}", parse("1/4 t8 r:t8 r:t8")));
        assert!(m.tuplet_group_at(0).is_some());
        // Saved again, it has its voices.
        let again: Measure = ron::from_str(&ron::to_string(&m).unwrap()).unwrap();
        assert!(crate::click::same_bar(&again, &m));
    }

    #[test]
    fn voices_share_the_meter_and_round_trip() {
        let mut m = parse("2/4 q q");
        m.push_voices(&parse("2/4 t8 t8 t8 r:q")).unwrap();
        assert_eq!(m.push_voices(&parse("3/4 q q q")), Err(DifferentMeter));
        assert_eq!(m.voices().len(), 2);
        assert_eq!(format!("{m:?}"), "1/4 1/4 ; 1/8[3:2] 1/8[3:2] 1/8[3:2] (1/4)");
        // Editing the measure edits its first voice.
        m.toggle_beat_kind(0);
        assert!(m.voice(0).unwrap().beats()[0].is_rest());
        assert!(m.voice(1).unwrap().beats()[0].is_note());
        m.voice_mut(1).unwrap().set_beat(3, Beat::note(q())).unwrap();

        // Every voice comes back with its beats, tuplet groups and anchors.
        assert!(round_trips(&m));
        let first_only = parse("2/4 r:q q");
        assert!(!crate::click::same_bar(&first_only, &m), "the second voice counts");
        assert!(
            ron::from_str::<Measure>(r#"{"time_signature":(beats:2,beat_unit:4),"voices":[]}"#)
                .unwrap_err()
                .to_string()
                .contains("no voice")
        );
    }

    #[test]
    fn a_new_time_signature_refits_every_voice() {
        let mut m = parse("4/4 q q q q");
        m.push_voices(&parse("4/4 h h")).unwrap();
        m.set_time_signature(TimeSignature::THREE_FOUR).unwrap();
        assert_eq!(format!("{m:?}"), "1/4 1/4 1/4 ; 1/2 (1/4)");
        assert!(m.voices().iter().all(|v| v.time_signature() == TimeSignature::THREE_FOUR));
    }

    #[test]
    fn deserializing_keeps_the_measures_the_app_writes() {
        use crate::click::{default_click_bar, meter_click_bar};
        use crate::counter::{counter_bar, own_ratios};
        use crate::generator::{
            GeneratorSettings, MAX_COMPLEXITY, Rng, Subdivision, generate_measure,
        };

        let meters = TimeSignature::BEAT_UNITS.into_iter().flat_map(|unit| {
            TimeSignature::beat_range(unit).unwrap().flat_map(move |beats| {
                let ts = TimeSignature::new(beats, unit);
                (0..ts.groupings().len() as u8).map(move |g| ts.with_grouping(g))
            })
        });
        // The meters of the tool buttons, beyond the picker.
        for (beats, unit) in [(1, 8), (2, 8), (1, 16), (2, 16), (3, 16), (4, 16), (3, 32)] {
            assert!(round_trips(&Measure::new(TimeSignature::new(beats, unit))));
        }
        let mut rng = Rng::new(178);
        for ts in meters {
            assert!(round_trips(&Measure::new(ts)));
            assert!(round_trips(&default_click_bar(ts)));
            assert!(round_trips(&meter_click_bar(ts)));
            for ratio in own_ratios(ts) {
                assert!(round_trips(&counter_bar(ts, ratio).unwrap()));
            }
            for subdivision in [
                Subdivision::Eighths,
                Subdivision::Sixteenths,
                Subdivision::Triplets,
                Subdivision::Mixed,
            ] {
                for complexity in 1..=MAX_COMPLEXITY {
                    let settings = GeneratorSettings {
                        subdivision,
                        complexity,
                        time_signature: ts,
                        ..GeneratorSettings::default()
                    };
                    if let Ok(m) = generate_measure(&settings, &mut rng) {
                        assert!(round_trips(&m));
                    }
                }
            }
        }
    }

    #[test]
    fn deserializing_keeps_edited_measures() {
        use crate::duration::COMMON_DURATIONS;
        use crate::generator::Rng;

        let mut rng = Rng::new(17);
        for ts in [TimeSignature::FOUR_FOUR, TimeSignature::SEVEN_EIGHT, TimeSignature::new(5, 16)]
        {
            let mut m = Measure::new(ts);
            for _ in 0..2000 {
                let idx = rng.below(m.beats().len());
                match rng.below(4) {
                    0 => {
                        m.dissolve_tuplet_group(idx);
                    }
                    1 => {
                        m.toggle_beat_kind(idx);
                    }
                    _ => {
                        let d = COMMON_DURATIONS[rng.below(COMMON_DURATIONS.len())];
                        let _ = m.set_beat(idx, Beat::note(d));
                    }
                }
                assert!(round_trips(&m));
            }
        }
    }

    #[test]
    fn deserializing_rejects_broken_measures() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        m.set_beat(1, Beat::note(t8())).unwrap();
        let saved = ron::to_string(&m).unwrap();
        assert!(read(&saved).is_ok());
        let broken = |from: &str, to: &str| {
            assert!(saved.contains(from), "{from} not in {saved}");
            read(&saved.replacen(from, to, 1)).unwrap_err()
        };

        // A time signature that is none.
        assert!(broken("beat_unit:4", "beat_unit:0").contains("4/0 is not a time signature"));
        assert!(broken("beats:4", "beats:0").contains("0/4 is not a time signature"));
        // Beats that overfill or underfill the measure.
        assert!(broken("beats:4", "beats:3").contains("beats span"));
        assert!(broken("beats:4", "beats:5").contains("beats span"));
        // A duration off the grid, and one that overflows the tick arithmetic.
        let quarter = "duration:Simple(Quarter)";
        assert!(
            broken(quarter, "duration:Dotted(base:Quarter,dots:40)")
                .contains("unsupported duration")
        );
        assert!(
            broken(quarter, "duration:Tuplet((n:0,m:0,base:Quarter))")
                .contains("unsupported duration")
        );
        // A tuplet group without an anchor, and one its beats do not fill.
        assert!(
            broken("tuplet_group_id:Some(1)", "tuplet_group_id:Some(9)")
                .contains("without an anchor")
        );
        assert!(broken("m:2,target_ticks", "m:3,target_ticks").contains("tuplet group 1 spans"));
    }

    #[test]
    fn deserializing_never_hands_out_a_saved_tuplet_id_again() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        m.set_beat(0, Beat::note(t8())).unwrap();
        let saved = ron::to_string(&m).unwrap();
        assert!(saved.contains("\"next_tuplet_id\":2"), "{saved}");
        let saved = saved.replace("\"next_tuplet_id\":2", "\"next_tuplet_id\":1");
        let mut back = read(&saved).unwrap();
        back.set_beat(3, Beat::note(t8())).unwrap();
        let first = back.beats()[0].tuplet_group_id.unwrap();
        let second = back.beats()[3].tuplet_group_id.unwrap();
        assert_ne!(first, second);
        assert!(back.tuplets().get(first).is_some() && back.tuplets().get(second).is_some());
    }
}
