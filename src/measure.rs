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
    pub const TWO_FOUR: Self = Self { beats: 2, beat_unit: 4 };
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
    // Internal insertion pointer for add_beat progression (not a UI cursor)
    next_insert: usize,
}

impl Measure {
    /// Creates a new empty measure with the given time signature
    pub fn new(time_signature: TimeSignature) -> Self {
        let mut s = Self {
            beats: Vec::new(),
            time_signature,
            beam_plan: Some(BeamPlan { groups: vec![] }),
            next_insert: 0,
        };
        s.fill_measure(BeatKind::Note);
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
        let max_ticks = set.grid.ticks_per_measure(&self.time_signature);
        let new_ticks = set
            .grid
            .ticks_of(&beat.duration)
            .ok_or_else(|| MeasureError::Unfillable { attempted: 0.0, remaining: 0.0 })?;

        // Primary-beat-aligned tuplet group overflow safety:
        // If we're inserting somewhere inside a primary beat window that already hosts a
        // 3:2 tuplet grid anchored at the start of that primary beat (e.g., eighth-triplet grid),
        // then we must not allow the inserted duration to extend past the end of that primary
        // beat window. This captures the musical constraint in the invalid tuplet insertion tests.
        {
            let onsets = set.compute_onset_ticks(&self.beats);
            if let Some(&onset) = onsets.get(idx) {
                let beat_ticks = set.grid.ticks_per_beat(&self.time_signature);
                if beat_ticks > 0 {
                    let rel_in_primary = onset % beat_ticks;
                    if rel_in_primary != 0 {
                        let window_start_tick = onset - rel_in_primary;
                        if let Some(win_start_idx) =
                            onsets.iter().position(|&t| t == window_start_tick)
                        {
                            if let Duration::Tuplet { n, .. } = self.beats[win_start_idx].duration {
                                // General rule for any n:m tuplet grid anchored at the primary-beat start:
                                // If the primary beat can be divided into n equal canonical slots, and the
                                // first element at the window start fits as an exact subdivision of that
                                // canonical slot, then we treat this primary-beat-aligned tuplet grid as
                                // active. Any mid-window insertion must not extend past the end of the
                                // current primary-beat window.
                                if n > 0 && beat_ticks % (n as u32) == 0 {
                                    let canonical_slot = beat_ticks / (n as u32);
                                    if let Some(elem_ticks) =
                                        set.grid.ticks_of(&self.beats[win_start_idx].duration)
                                    {
                                        if elem_ticks > 0 && canonical_slot % elem_ticks == 0 {
                                            let allowed = beat_ticks - rel_in_primary;
                                            if new_ticks > allowed {
                                                let attempted =
                                                    set.grid.ticks_to_whole_notes(new_ticks);
                                                let remaining =
                                                    set.grid.ticks_to_whole_notes(allowed);
                                                return Err(MeasureError::Unfillable {
                                                    attempted,
                                                    remaining,
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // NEW: Multi-primary-beat tuplet group overflow safety.
                    // Regardless of whether we are exactly on a primary boundary or mid-primary,
                    // we might be inside a tuplet group that spans multiple primary beats (e.g.,
                    // eighth‑triplet group spans two eighths). In that case, disallow inserting a
                    // duration that would extend past the end of the active group window.
                    // Algorithm: Walk backwards over primary beat boundaries up to a small bound and
                    // look for a boundary that hosts a tuplet element establishing a primary‑beat‑
                    // aligned n:m grid whose group length is an integer number of primary beats and
                    // whose span covers the current onset.
                    {
                        // Start from the current primary boundary at or before onset
                        let mut boundary_tick = onset - rel_in_primary; // <= onset
                        let mut steps = 0u8;
                        while boundary_tick < onset && steps < 8 {
                            if let Some(start_idx) = onsets.iter().position(|&t| t == boundary_tick)
                            {
                                if let Duration::Tuplet { n, m, .. } =
                                    self.beats[start_idx].duration
                                {
                                    if n > 0 {
                                        let beat_times_m = (beat_ticks as u64) * (m as u64);
                                        if beat_times_m % (n as u64) == 0 {
                                            let canonical_slot = (beat_times_m / (n as u64)) as u32; // beat_ticks * m / n
                                            if let Some(elem_ticks) =
                                                set.grid.ticks_of(&self.beats[start_idx].duration)
                                            {
                                                if elem_ticks > 0
                                                    && canonical_slot % elem_ticks == 0
                                                {
                                                    let group_ticks =
                                                        canonical_slot.saturating_mul(n as u32); // == beat_ticks * m
                                                    if onset > boundary_tick
                                                        && onset < boundary_tick + group_ticks
                                                    {
                                                        let allowed =
                                                            (boundary_tick + group_ticks) - onset;
                                                        if new_ticks > allowed {
                                                            let attempted = set
                                                                .grid
                                                                .ticks_to_whole_notes(new_ticks);
                                                            let remaining = set
                                                                .grid
                                                                .ticks_to_whole_notes(allowed);
                                                            return Err(MeasureError::Unfillable {
                                                                attempted,
                                                                remaining,
                                                            });
                                                        }
                                                        break; // we are inside this group; done.
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            // If we've reached tick 0, stop; otherwise step one primary beat back.
                            if boundary_tick < beat_ticks {
                                break;
                            }
                            boundary_tick -= beat_ticks;
                            steps += 1;
                        }
                    }

                    // NEW: Insertion-driven group window safety for generic n:m tuplets.
                    // If we're inserting a tuplet and our onset lies strictly inside an n:m
                    // group window aligned to primary beats (period = beat_ticks * m), then the
                    // inserted duration must not exceed the remaining ticks to that group end —
                    // but only if such a group is actually active (i.e., started at the group
                    // window start with a compatible tuplet).
                    if let Duration::Tuplet { n: n_ins, m: m_ins, .. } = beat.duration {
                        if n_ins > 0 && m_ins > 0 {
                            let group_period = (beat_ticks as u64) * (m_ins as u64);
                            let onset_u = onset as u64;
                            if group_period > 0 {
                                let r = onset_u % group_period;
                                if r != 0 {
                                    let group_start = (onset_u - r) as u32;
                                    let group_end = (group_start as u64 + group_period) as u32;
                                    // Verify the group is active: a tuplet with same n:m at group_start
                                    if let Some(start_idx) =
                                        onsets.iter().position(|&t| t == group_start)
                                    {
                                        if let Duration::Tuplet { n, m, .. } =
                                            self.beats[start_idx].duration
                                        {
                                            if n as u64 == n_ins as u64 && m as u64 == m_ins as u64
                                            {
                                                let allowed = group_end - onset;
                                                if new_ticks > allowed {
                                                    let attempted =
                                                        set.grid.ticks_to_whole_notes(new_ticks);
                                                    let remaining =
                                                        set.grid.ticks_to_whole_notes(allowed);
                                                    return Err(MeasureError::Unfillable {
                                                        attempted,
                                                        remaining,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Tuplet slot boundary safety (contextual): if the current index sits inside an existing
        // tuplet element slot (because a prior beat subdivided this slot), then the new duration may
        // not extend past the end of that slot. This is the generalized form of the earlier narrow
        // rule and matches the intuition “don’t overflow the current tuplet boundary”.
        {
            let old_dur = self.beats[idx].duration;
            if let Duration::Tuplet { .. } = old_dur {
                if let Some(slot_ticks) = set.grid.ticks_of(&old_dur) {
                    let onsets = set.compute_onset_ticks(&self.beats);
                    if let Some(&onset) = onsets.get(idx) {
                        let rel = onset % slot_ticks;
                        if rel != 0 {
                            let allowed = slot_ticks - rel;
                            if new_ticks > allowed {
                                let attempted = set.grid.ticks_to_whole_notes(new_ticks);
                                let remaining = set.grid.ticks_to_whole_notes(allowed);
                                return Err(MeasureError::Unfillable { attempted, remaining });
                            }
                        }
                    }
                }
            }
        }

        // Slot divisibility rule: if we're replacing an intact tuplet slot, only allow
        // durations whose tick length evenly divides that slot. This prevents inserting invalid
        // durations into tuplet slots.
        {
            let old_dur = self.beats[idx].duration;
            if let Duration::Tuplet { n: _, m: _, base: _ } = old_dur {
                // Generic tuplet element slot: one note of n-in-the-time-of-m over base.
                // Enforce divisibility only when shrinking (replacing with a duration not longer than the slot)
                // to prevent mixing incompatible grids inside an intact tuplet element.
                // Allow growth; boundary overflow and group-window safety are enforced elsewhere.
                if let Some(slot_ticks) = set.grid.ticks_of(&old_dur) {
                    if slot_ticks == 0 || new_ticks == 0 {
                        let attempted = set.grid.ticks_to_whole_notes(new_ticks);
                        let remaining = set.grid.ticks_to_whole_notes(slot_ticks);
                        return Err(MeasureError::Unfillable { attempted, remaining });
                    }

                    // Shrinking: new tick must divide slot
                    if new_ticks <= slot_ticks && (slot_ticks % new_ticks != 0) {
                        let attempted = set.grid.ticks_to_whole_notes(new_ticks);
                        let remaining = set.grid.ticks_to_whole_notes(slot_ticks);
                        return Err(MeasureError::Unfillable { attempted, remaining });
                    }
                    // If growing beyond the slot and the new duration is non-tuplet (simple/dotted),
                    // require it to be an integer multiple of the slot to avoid crossing fractional
                    // tuplet boundaries with incompatible grids. Allow growth for tuplet insertions
                    // (handled by group-window safety elsewhere).
                    if new_ticks > slot_ticks {
                        match beat.duration {
                            Duration::Tuplet { .. } => { /* allowed */ }
                            _ => {
                                if new_ticks % slot_ticks != 0 {
                                    let attempted = set.grid.ticks_to_whole_notes(new_ticks);
                                    let remaining = set.grid.ticks_to_whole_notes(slot_ticks);
                                    return Err(MeasureError::Unfillable { attempted, remaining });
                                }
                            }
                        }
                    }
                } else {
                    // If the grid cannot represent the old tuplet slot (should not happen), reject.
                    let attempted = set.grid.ticks_to_whole_notes(new_ticks);
                    return Err(MeasureError::Unfillable { attempted, remaining: 0.0 });
                }
            }
        }

        let old_dur_prev = self.beats[idx].duration;
        let old_ticks = set.grid.ticks_of(&old_dur_prev).unwrap_or(0);
        let new_total_ticks = current_ticks - old_ticks + new_ticks;
        if new_total_ticks > max_ticks {
            // Attempt to expand into subsequent contiguous rests to accommodate growth
            let need = new_ticks - old_ticks; // extra ticks required
            let mut k = idx + 1;
            let mut absorb_ticks = 0u32;
            while need > 0 && k < self.beats.len() {
                let b = self.beats[k];
                let t = set.grid.ticks_of(&b.duration).unwrap_or(0);
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
                Ok(())
            } else {
                let available_ticks = (max_ticks - (current_ticks - old_ticks)).max(0);
                let available = (available_ticks as f64) / (set.grid.ticks_per_whole as f64);
                let attempted = (new_ticks as f64) / (set.grid.ticks_per_whole as f64);
                Err(MeasureError::Overflow { attempted, available })
            };
        }
        let remaining_ticks = max_ticks - new_total_ticks;
        if remaining_ticks != 0 && !Self::is_remainder_fillable(remaining_ticks) {
            let remaining = set.grid.ticks_to_whole_notes(remaining_ticks);
            let attempted = set.grid.ticks_to_whole_notes(new_ticks);
            return Err(MeasureError::Unfillable { attempted, remaining });
        }
        // Perform replacement at idx
        self.beats[idx] = beat;

        // If the new beat is shorter than the old one, split the leftover time
        // into concrete rest beats inserted immediately after idx so that subsequent
        // positions exist (elegant progression for add_beat at idx+1).
        if new_ticks < old_ticks {
            let leftover = old_ticks - new_ticks;
            // If the original slot was a tuplet element, restrict the refill to divisors of that slot
            // to keep the remainder within the same tuplet grid.
            let allowed: Vec<Duration> = if let Duration::Tuplet { .. } = self.beats[idx].duration {
                // Note: self.beats[idx] is still the old value here (we haven't assigned the new beat yet).
                if let Some(slot_ticks) = set.grid.ticks_of(&self.beats[idx].duration) {
                    default_duration_set()
                        .durations
                        .iter()
                        .copied()
                        .filter(|d| {
                            set.grid.ticks_of(d).map_or(false, |t| {
                                t > 0 && leftover % t == 0 && slot_ticks % t == 0
                            })
                        })
                        .collect()
                } else {
                    vec![]
                }
            } else {
                vec![]
            };
            let allowed_slice: &[Duration] = if allowed.is_empty() { &[] } else { &allowed };
            if let Some(fill) = best_fill_for_gap(leftover, allowed_slice) {
                let mut insert_at = idx + 1;
                for d in fill {
                    self.beats.insert(insert_at, Beat::rest(d));
                    insert_at += 1;
                }
            }
        }

        self.recompute_beams();
        //TODO revisit later. shouldn't the current_ticks() always be max_ticks?
        //assert_eq!(self.current_ticks(), max_ticks);
        Ok(())
    }

    /// Expose the time signature (clone)
    pub fn time_signature(&self) -> TimeSignature { self.time_signature.clone() }

    /// Expose the beaming plan for this measure
    pub fn beam_plan(&self) -> Option<&BeamPlan> { self.beam_plan.as_ref() }

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

    /// Adds a beat to this measure at the current internal insertion pointer (left-to-right
    /// progression independent of UI). After a successful insertion, the pointer advances by 1
    /// his preserves existing tests that relied on sequential addition without embedding a UI cursor in the model.
    pub fn add_beat(&mut self, beat: Beat) -> Result<(), MeasureError> {
        // Clamp pointer to available range
        match self.set_beat_at(self.next_insert, beat) {
            Ok(()) => {
                self.next_insert += 1;
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
        self.fill_measure(BeatKind::Rest);
        self.minimize_remainder_rests_from(idx);
        println!("{:#}", self)
    }

    pub fn fill_measure(&mut self, kind: BeatKind) {
        let remaining_ticks = self.remaining_ticks();
        if remaining_ticks <= 0 {
            return; // nothing to commit
        }
        if let Some(fill) = best_fill_for_gap(remaining_ticks, &[]) {
            let take = fill.len();
            for duration in fill.into_iter().take(take) {
                let beat = Beat { duration, kind, tremolo: None };
                self.beats.push(beat);
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
                self.minimize_remainder_rests_from(idx);
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
        // Decide where to start minimizing:
        // - If the deletion point `start_idx` currently points to a rest, include it in the minimization span
        //   so adjacent rests merge even when there are non-rests earlier in the measure.
        // - Otherwise, fall back to the computed `trailing_start`.
        let start = if start_idx < self.beats.len() && self.beats[start_idx].kind == BeatKind::Rest
        {
            start_idx
        } else {
            trailing_start
        };
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
    use crate::duration::{Duration, NoteValue, e, q, qt16, s, t8, t16, t32, th, qt8};

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
        assert!(measure.add_beat(Beat::note(q())).is_err());
        assert!(measure.add_beat(Beat::note(e())).is_err());
        assert!(measure.add_beat(Beat::note(s())).is_err());
        assert!(measure.add_beat(Beat::note(th())).is_err());

        assert!(measure.add_beat(Beat::rest(t8())).is_ok());
        assert!(measure.add_beat(Beat::note(q())).is_err());
        assert!(measure.add_beat(Beat::note(e())).is_err());
        assert!(measure.add_beat(Beat::note(s())).is_err());
        assert!(measure.add_beat(Beat::note(th())).is_err());

        assert!(measure.add_beat(Beat::rest(t8())).is_ok());
    }

    #[test]
    fn test_invalid_tuplet_insertion_0() {
        let mut measure = Measure::new(TimeSignature::TWO_FOUR);

        assert!(measure.add_beat(Beat::note(t8())).is_ok());
        assert!(measure.add_beat(Beat::note(t8())).is_ok());
        assert!(measure.add_beat(Beat::note(t16())).is_ok());
        // The next triplet 1/8 overfills this tuplet group, which has only space for one triplet
        // 1/6 note left (or two triplet 1/32 subdivisions).
        assert!(measure.add_beat(Beat::note(t8())).is_err());
        assert!(measure.add_beat(Beat::note(t32())).is_ok());
        // The next triplet 1/16 overfills this tuplet group, which has only space for one triplet
        // 1/32 note left.
        assert!(measure.add_beat(Beat::note(t16())).is_err());
        assert!(measure.add_beat(Beat::note(t32())).is_ok());

        // The next beat starts a new tuplet group, so this is valid.
        assert!(measure.add_beat(Beat::note(t8())).is_ok());
    }

    #[test]
    fn test_invalid_tuplet_insertion_1() {
        let mut measure = Measure::new(TimeSignature::ONE_FOUR);

        assert!(measure.add_beat(Beat::note(t16())).is_ok());
        assert!(measure.add_beat(Beat::note(t8())).is_ok());
        assert!(measure.add_beat(Beat::note(t8())).is_ok());
        // The next triplet 1/8 overfills this tuplet group, which has only space for one triplet
        // 1/6 note left (or two triplet 1/32 subdivisions).
        assert!(measure.add_beat(Beat::note(t8())).is_err());
        assert!(measure.add_beat(Beat::note(t32())).is_ok());
        assert!(measure.add_beat(Beat::note(t32())).is_ok());

        // The next beat starts a new tuplet group, but we don't have enough space in our measure.
        assert!(measure.add_beat(Beat::note(t32())).is_err());
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
        measure.add_beat(Beat::note(Duration::Dotted { base: ThirtySecond, dots: 1 })).unwrap();
        measure.add_beat(Beat::rest(Duration::Dotted { base: Eighth, dots: 1 })).unwrap();
    }

    #[test]
    fn test_remove_middle() {
        // 4/4: q, e, e -> delete index 1 yields q, e
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        let q = Duration::Simple(NoteValue::Quarter);
        let e = Duration::Simple(Eighth);
        m.add_beat(Beat::note(q)).unwrap();
        m.add_beat(Beat::note(e)).unwrap();
        m.add_beat(Beat::rest(e)).unwrap();
        m.remove(1);
        assert_eq!(m.beats()[0].duration, q);
        assert_eq!(m.beats()[1].duration, q); // because of minimization remaining rests
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
    fn test_tuplets_0() {
        let mut measure = Measure::new(TimeSignature::SEVEN_EIGHT);

        assert!(measure.add_beat(Beat::note(s())).is_ok());
        assert!(measure.add_beat(Beat::note(qt16())).is_ok());
        // Can't add triplet notes to a quintuplet group:
        assert!(measure.add_beat(Beat::note(t8())).is_err());
        assert!(measure.add_beat(Beat::note(t16())).is_err());
        // Can add tuplet notes in tuplet groups when they are not overfilling:
        assert!(measure.add_beat(Beat::note(qt8())).is_ok());
    }
}
