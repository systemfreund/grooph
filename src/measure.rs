mod beat;
pub(crate) mod duration;
mod fill;
pub(crate) mod grouping;
pub(crate) mod time_signature;

use crate::measure::BeatKind::Rest;
pub(crate) use crate::measure::beat::{Beat, BeatKind};
use crate::measure::duration::NoteValue::{Eighth, Sixteenth, ThirtySecond};
use crate::measure::duration::{
    Duration, DurationSet, default_duration_set, duration_to_debug_str, qt16,
};
use crate::measure::fill::best_fill_for_gap;
pub(crate) use crate::measure::time_signature::TimeSignature;
use BeatKind::Note;
use either::Either;
use std::collections::HashMap;
use std::fmt::{Debug, Display, Formatter};
use std::vec;

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
        attempted: f64
    },
}

/// Stable anchor describing a tuplet group span and semantics
#[derive(Clone, Debug)]
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
    // Internal insertion pointer for add_beat progression (not a UI cursor)
    next_insert: usize,
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
            next_insert: 0,
            tuplet_anchors: HashMap::new(),
            next_tuplet_id: 1,
        };
        s.fill_measure(init, &[Duration::Simple(time_signature.beat_note_value().unwrap())]);
        s
    }

    /// Expose a read-only view of beats
    pub fn beats(&self) -> &Vec<Beat> { &self.beats }

    /// Replace the beat at index `idx` with `beat` if it fits and the remainder stays fillable.
    pub fn set_beat_at(&mut self, idx: usize, beat: Beat) -> Result<(), MeasureError> {
        assert!(idx < self.beats.len());
        let set = default_duration_set();
        let old_accent = self.beats[idx].accented;
        let dur_old = self.beats[idx].duration; // duration of the beat to be replaced
        let max_ticks = self.max_ticks();
        let new_ticks = set
            .grid
            .ticks_of(&beat.duration)
            .ok_or(MeasureError::Unfillable { attempted: 0.0 })?;

        // Reject grid-incompatible replacement into a tuplet slot; also prepare id inheritance
        let mut new_beat = beat;
        if let Duration::Tuplet { n: n_old, m: m_old, .. } = dur_old {
            match beat.duration {
                Duration::Tuplet { n: n_new, m: m_new, .. } if n_new == n_old && m_new == m_old => {
                    // ok: same tuplet grid - inherit group id if present
                    new_beat.tuplet_group_id = self.beats[idx].tuplet_group_id;
                }
                _ => {
                    // inserting a non-tuplet or different tuplet grid into a tuplet slot is invalid
                    return self.unfillable_err(new_ticks);
                }
            }
        }

        let old_ticks = set.grid.ticks_of(&dur_old).unwrap();
        let new_total_ticks = max_ticks - old_ticks + new_ticks;

        // "Growing" branch, i.e., when a larger beat replaces a smaller one.
        if new_total_ticks > max_ticks {
            let need = new_ticks - old_ticks; // extra ticks required
            assert!(need > 0);

            // Compute how many ticks we can absorb from following beats.
            let absorb_ticks = self.compute_ticks_to_absorb(idx, &set, dur_old, need);
            return if absorb_ticks >= need {
                self.beats[idx] = new_beat;
                self.beats[idx].accented = old_accent;
                let p = idx + 1;
                let mut remaining_to_consume = need;
                while remaining_to_consume > 0 {
                    let b = self.beats[p];
                    let t = set.grid.ticks_of(&b.duration).unwrap();
                    if t <= remaining_to_consume {
                        self.beats.remove(p);
                        remaining_to_consume -= t;
                    } else {
                        let new_ticks_rest = t - remaining_to_consume;
                        self.beats.remove(p);
                        self.fill_at(p, new_ticks_rest, &[], Either::Left(self.beats[idx]))?;
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
            // Are we replacing a non-tuplet beat with a tuplet beat?
            if let Duration::Tuplet { n, m, base } = new_beat.duration
                && !matches!(dur_old, Duration::Tuplet { .. })
            {
                // Compute the total span this tuplet group should occupy
                let base_ticks = set.grid.ticks_of(&Duration::Simple(base)).unwrap();
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
                    if let Duration::Tuplet { n: n2, m: m2, .. } = b.duration
                        && !(n2 == n && m2 == m)
                    {
                        return self.unfillable_err(new_ticks);
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
                self.beats[idx].accented = old_accent;
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

            let leftover = old_ticks - new_ticks;

            // If we are inside a tuplet slot, constrain the filler to durations that belong to the
            // same tuplet grid (same n,m).
            let allowed: Vec<Duration> = match dur_old {
                Duration::Tuplet { n: n_old, m: m_old, .. } => default_duration_set()
                    .durations
                    .iter()
                    .cloned()
                    .filter(|d| matches!(d, Duration::Tuplet { n, m, .. } if *n == n_old && *m == m_old))
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
        self.beats[idx].accented = old_accent;
        Ok(())
    }

    fn compute_ticks_to_absorb(
        &self,
        idx: usize,
        set: &DurationSet,
        dur_old: Duration,
        need: u32,
    ) -> u32 {
        let mut absorb_ticks = 0u32;
        let mut k = idx + 1;
        while k < self.beats.len() {
            let t = set.grid.ticks_of(&self.beats[k].duration).unwrap();
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

    /// Return a vector with the absolute position (1-based) of each beat as floats.
    /// Examples:
    /// - In 4/4 with four quarters: [1.0, 2.0, 3.0, 4.0]
    /// - If the first three notes are 8th-note triplets in 4/4: [1.0, 1.3333..., 1.6666...]
    ///
    /// Positions are computed from onset ticks relative to the measure's beat size (beat_unit).
    pub fn beat_positions(&self) -> Vec<f32> {
        let set = default_duration_set();
        let onsets = set.compute_onset_ticks(&self.beats);
        let ticks_per_beat = set.grid.ticks_per_beat(&self.time_signature);
        onsets.into_iter().map(|t| 1.0f32 + (t as f32) / (ticks_per_beat as f32)).collect()
    }

    fn max_ticks(&self) -> u32 {
        let set = default_duration_set();
        set.grid.ticks_per_measure(&self.time_signature)
    }

    fn remaining_ticks(&self, idx: usize) -> u32 {
        if idx >= self.beats.len() {
            return 0;
        }
        let set = default_duration_set();
        let mut sum = 0u32;
        for b in &self.beats[idx..] {
            sum += set.grid.ticks_of(&b.duration).unwrap();
        }
        sum
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

    fn fill_measure(&mut self, kind: BeatKind, allowed: &[Duration]) {
        if !self.beats.is_empty() {
            // currently we only support filling empty/uninitalized measures
            return;
        }
        if let Some(fill) = best_fill_for_gap(self.max_ticks(), allowed) {
            let take = fill.len();
            for duration in fill.into_iter().take(take) {
                let beat =
                    Beat { duration, kind, tremolo: None, accented: false, tuplet_group_id: None };
                self.beats.push(beat);
            }
        }
    }

    fn fill_at(
        &mut self,
        idx: usize,
        ticks: u32,
        allowed: &[Duration],
        init: Either<Beat, BeatKind>,
    ) -> Result<(), MeasureError> {
        if let Some(fill) = best_fill_for_gap(ticks, allowed) {
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
    ///
    /// No-op for other cases (tuplets, multi-dot) or if replacement doesn't fit.
    /// Returns true if the duration changed.
    pub fn toggle_dotted_at(&mut self, idx: usize) -> bool {
        if idx >= self.beats.len() {
            return false;
        }
        let current = self.beats[idx];
        let new_dur = match current.duration {
            Duration::Simple(base) => Some(Duration::Dotted { base, dots: 1 }),
            Duration::Dotted { base, dots: 1 } => Some(Duration::Simple(base)),
            _ => None,
        };
        if let Some(dur) = new_dur {
            let new_beat = Beat {
                duration: dur,
                kind: current.kind,
                tremolo: None,
                accented: current.accented,
                tuplet_group_id: current.tuplet_group_id,
            };
            if self.set_beat_at(idx, new_beat).is_ok() {
                return true;
            }
        }
        false
    }

    /// Set the user accent flag at index `idx`.
    pub fn set_accent_at(&mut self, idx: usize, accented: bool) {
        if let Some(b) = self.beats.get_mut(idx) {
            b.accented = accented;
        }
    }

    /// Toggle the user accent flag at index `idx`.
    pub fn toggle_accent_at(&mut self, idx: usize) {
        if let Some(b) = self.beats.get_mut(idx) {
            b.accented = !b.accented;
        }
    }

    /// Query the user accent flag at index `idx`.
    pub fn is_accented_at(&self, idx: usize) -> bool {
        self.beats.get(idx).map(|b| b.accented).unwrap_or(false)
    }

    fn unfillable_err(&self, attempted: u32) -> Result<(), MeasureError> {
        let set = default_duration_set();
        Err(MeasureError::Unfillable {
            attempted: set.grid.ticks_to_whole_notes(attempted)
        })
    }

    fn overflow_err(&self, attempted: u32, remaining: u32) -> Result<(), MeasureError> {
        let set = default_duration_set();
        Err(MeasureError::Overflow {
            attempted: set.grid.ticks_to_whole_notes(attempted),
            available: set.grid.ticks_to_whole_notes(remaining),
        })
    }

    /// Löst die Tuplet‑Gruppe auf, in der sich `idx` befindet.
    ///
    /// Ersetzt die gesamte Spanne der Gruppe durch eine einfache (nicht‑Tuplet) Auffüllung
    /// mit Ruhezeichen, entfernt die `tuplet_group_id` und den verknüpften Anchor.
    /// Rückgabe: `true` bei erfolgreicher Auflösung, sonst `false` (z. B. wenn kein Tuplet an `idx`).
    pub fn dissolve_tuplet_group_at(&mut self, idx: usize) -> bool {
        if idx >= self.beats.len() {
            return false;
        }
        let Some(group_id) = self.beats[idx].tuplet_group_id else {
            return false;
        };

        // Finde zusammenhängende Spanne gleicher group_id
        let mut start = idx;
        while start > 0 && self.beats[start - 1].tuplet_group_id == Some(group_id) {
            start -= 1;
        }
        let mut end = idx + 1;
        while end < self.beats.len() && self.beats[end].tuplet_group_id == Some(group_id) {
            end += 1;
        }
        if start >= end {
            return false;
        }

        let set = default_duration_set();

        self.beats.drain(start..end);

        let allowed: Vec<Duration> =
            set.durations.iter().copied().filter(|d| matches!(d, Duration::Simple(_))).collect();

        let span_ticks = self.tuplet_anchors.get(&group_id).unwrap().target_ticks;
        self.fill_at(start, span_ticks, &allowed, Either::Right(Rest)).unwrap();
        true
    }
}

impl Debug for Measure {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.beats.iter().enumerate().try_fold((), |_, (idx, beat)| {
            if beat.kind == Rest {
                write!(f, "(")?
            }
            write!(f, "{}", duration_to_debug_str(&beat.duration))?;
            if beat.kind == Rest {
                write!(f, ")")?
            }
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
    use crate::measure::duration::{Duration, e, q, qt16, s, t8, t16, t32, th};
    // no layout imports here to avoid using private modules from this scope

    fn durations_of(measure: &Measure) -> Vec<Duration> {
        measure.beats().iter().map(|b| b.duration).collect()
    }

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
        let mut m = Measure::new(TimeSignature::ONE_FOUR);

        assert!(m.add_beat(Beat::note(t8())).is_ok());
        let Beat { duration, kind, .. } = m.beats()[1];
        assert_eq!(duration, t8());
        assert_eq!(kind, Rest);
        let Beat { duration, kind, .. } = m.beats()[2];
        assert_eq!(duration, t8());
        assert_eq!(kind, Rest);

        assert!(
            m.add_beat(Beat::note(q())).is_err(),
            "simple 1/4 note must not fit in triplet 1/8 group"
        );
        assert!(
            m.add_beat(Beat::note(e())).is_err(),
            "simple 1/8 note must not fit in triplet 1/8 group"
        );
        assert!(
            m.add_beat(Beat::note(s())).is_err(),
            "simple 1/16 note must not fit in triplet 1/8 group"
        );
        assert!(
            m.add_beat(Beat::note(th())).is_err(),
            "simple 1/32 note must not fit in triplet 1/8 group"
        );

        assert!(m.add_beat(Beat::note(t8())).is_ok());
        assert!(m.add_beat(Beat::rest(t8())).is_ok());
    }

    #[test]
    fn test_triplet_insertions_0() {
        let mut m = Measure::new(TimeSignature::TWO_FOUR);

        // First triplet group
        assert!(m.add_beat(Beat::note(t16())).is_ok());
        assert!(m.add_beat(Beat::note(t16())).is_ok());
        // The next triplet 1/8 overfills this tuplet group, which has only space for one triplet
        // 1/6 note left (or two triplet 1/32 subdivisions).
        assert!(m.add_beat(Beat::note(t8())).is_err(), "triplet 1/8 must not fit tuplet group");
    }

    #[test]
    fn test_triplet_insertions_1() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);

        assert!(m.add_beat(Beat::note(t16())).is_ok());
        assert!(m.add_beat(Beat::note(t8())).is_ok());
        assert!(
            m.add_beat(Beat::note(t8())).is_err(),
            "can't start a new triplet 1/8 group measure has only 1/8 space left"
        );
        assert!(m.add_beat(Beat::note(e())).is_ok());
        assert_eq!(m.remaining_ticks(m.next_insert), 0);
    }

    #[test]
    fn test_triplet_insertions_2() {
        let mut m = Measure::new(TimeSignature::TWO_FOUR);

        // First triplet group
        assert!(m.add_beat(Beat::note(t8())).is_ok());
        assert!(m.add_beat(Beat::note(t16())).is_ok());
        assert!(m.add_beat(Beat::note(t16())).is_ok());
        assert!(m.add_beat(Beat::note(t8())).is_ok());

        // Second triplet group
        assert!(m.add_beat(Beat::note(t16())).is_ok());
        assert!(m.add_beat(Beat::note(t32())).is_ok());
        assert!(m.add_beat(Beat::note(t32())).is_ok());
        assert!(m.add_beat(Beat::note(t32())).is_ok());
        assert!(m.add_beat(Beat::note(t32())).is_ok());

        assert!(
            m.add_beat(Beat::note(t8())).is_err(),
            "can't start a new triplet 1/8 group measure has only 1/8 space left"
        );
        assert!(m.add_beat(Beat::note(e())).is_ok());
        assert_eq!(m.remaining_ticks(m.next_insert), 0);
    }

    #[test]
    fn test_triplet_insertions_3() {
        let mut m = Measure::new(TimeSignature::TWO_EIGHT);

        assert!(m.add_beat(Beat::note(t8())).is_ok());
        assert!(m.add_beat(Beat::note(t16())).is_ok());
        assert!(m.add_beat(Beat::note(t8())).is_ok());
        // The next triplet 1/8 overfills this tuplet group, which has only space for one triplet
        // 1/6 note left (or two triplet 1/32 subdivisions).
        assert!(m.add_beat(Beat::note(t8())).is_err());
        assert!(m.add_beat(Beat::note(t32())).is_ok());
        // Doesn't fit.
        assert!(m.add_beat(Beat::note(t16())).is_err());
        assert!(m.add_beat(Beat::note(t32())).is_ok());

        // The next beat starts a new tuplet group, but we don't have enough space in our measure.
        assert_eq!(m.remaining_ticks(m.next_insert), 0);
    }

    #[test]
    fn test_triplet_insertion_in_seven_eight() {
        let mut m = Measure::new(TimeSignature::SEVEN_EIGHT);
        assert!(m.add_beat(Beat::note(t8())).is_ok());
        assert!(m.add_beat(Beat::note(t8())).is_ok());
        assert!(m.add_beat(Beat::note(t16())).is_ok());
    }

    #[test]
    fn test_triplet_split_1() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        assert!(m.add_beat(Beat::note(t8())).is_ok());
        assert!(m.add_beat(Beat::note(t8())).is_ok());
        assert!(m.add_beat(Beat::note(t8())).is_ok());

        assert!(m.set_beat_at(1, Beat::note(t16())).is_ok());
        assert_eq!(&durations_of(&m), &[t8(), t16(), t16(), t8()]);

        assert!(m.set_beat_at(0, Beat::note(t16())).is_ok());
        assert_eq!(&durations_of(&m), &[t16(), t16(), t16(), t16(), t8()]);
    }

    #[test]
    fn test_triplet_split_2() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        assert!(m.add_beat(Beat::note(t8())).is_ok());
        assert!(m.add_beat(Beat::note(t8())).is_ok());
        assert!(m.add_beat(Beat::note(t8())).is_ok());

        assert!(m.set_beat_at(2, Beat::note(t32())).is_ok());
        assert_eq!(&durations_of(&m), &[t8(), t8(), t32(), t32(), t16()]);
    }

    #[test]
    fn test_triplet_split_3() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        assert!(m.add_beat(Beat::note(t32())).is_ok());
        assert!(m.add_beat(Beat::note(t32())).is_ok());
        assert!(m.add_beat(Beat::note(t32())).is_ok());

        assert!(m.set_beat_at(0, Beat::note(t8())).is_err());
        assert!(m.set_beat_at(0, Beat::note(t16())).is_ok());
        assert_eq!(&durations_of(&m), &[t16(), t32(), s(), e()]);

        assert!(m.set_beat_at(2, Beat::note(t16())).is_ok());
        assert_eq!(&durations_of(&m), &[t16(), t32(), t16(), t16(), t16(), s()]);
    }

    #[test]
    fn test_triplet_split_4() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        assert!(m.add_beat(Beat::note(t16())).is_ok());
        assert!(m.add_beat(Beat::note(t16())).is_ok());
        assert!(m.add_beat(Beat::note(t16())).is_ok());

        // Cannot merge last note in the group (not enough space).
        assert!(m.set_beat_at(2, Beat::note(t8())).is_err());

        // Merge t16 note in the middle.
        assert!(m.set_beat_at(1, Beat::note(t8())).is_ok());
        assert_eq!(&durations_of(&m), &[t16(), t8(), e()]);

        // Subdivide t8 note in the middle.
        assert!(m.set_beat_at(1, Beat::rest(t16())).is_ok());
        assert_eq!(&durations_of(&m), &[t16(), t16(), t16(), e()]);

        // Subdivide third t16 note.
        assert!(m.set_beat_at(2, Beat::rest(t32())).is_ok());
        assert_eq!(&durations_of(&m), &[t16(), t16(), t32(), t32(), e()]);

        // Merge second note to t8. Must work because the remainder of the tuplet has enough space.
        assert!(m.set_beat_at(1, Beat::rest(t8())).is_ok());
        assert_eq!(&durations_of(&m), &[t16(), t8(), e()]);
    }

    #[test]
    fn test_add_eighth_triplet_to_seven_eight_measure() {
        let mut m = Measure::new(TimeSignature::SEVEN_EIGHT);
        m.add_beat(Beat::note(Duration::Dotted { base: Eighth, dots: 1 })).unwrap();
        m.add_beat(Beat::note(Duration::Dotted { base: Sixteenth, dots: 1 })).unwrap();
        m.add_beat(Beat::note(Duration::Simple(ThirtySecond))).unwrap();
        m.add_beat(Beat::note(Duration::Simple(Sixteenth))).unwrap();
        m.add_beat(Beat::note(Duration::Simple(Eighth))).unwrap();
        m.add_beat(Beat::note(Duration::Simple(Sixteenth))).unwrap();
        m.add_beat(Beat::rest(Duration::Dotted { base: Eighth, dots: 1 })).unwrap();
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
        measure.add_beat(Beat::note(qt16())).unwrap();
        // TODO extend
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
        m.set_beat_at(0, Beat::rest(t8())).unwrap();
        // Subdivide 2nd t8 + 3rd t8 beat into two t16s each:
        m.set_beat_at(1, Beat::rest(t16())).unwrap();
        // Merge tuplet at the 'odd' position. This yields the measure: t8 t16 t8 t16
        m.set_beat_at(2, Beat::rest(t8())).unwrap();

        assert_eq!(&durations_of(&m), &[t8(), t16(), t8(), t16()]);
        assert_eq!(
            m.beats().iter().enumerate().find_map(|(idx, beat)| {
                if !matches!(beat.tuplet_group_id, Some(1)) { Some(idx) } else { None }
            }),
            None
        );
    }
}
