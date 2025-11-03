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

    pub const FOUR_FOUR: Self = Self {
        beats: 4,
        beat_unit: 4,
    };


    /// Returns the total duration in integer ticks
    pub fn measure_duration_ticks(&self) -> i32 {
        // number of whole-note fractions: beats / beat_unit of a whole note
        // Convert to ticks: (beats * TICKS_PER_WHOLE) / beat_unit
        ((self.beats as i32) * Duration::TICKS_PER_WHOLE) / (self.beat_unit as i32)
    }
}

#[derive(Copy, Clone)]
pub enum Sticking {
    R,
    L,
}

pub enum BeatKind {
    Note(Option<Sticking>),
    Rest,
}

pub struct Beat {
    pub duration: Duration,
    pub kind: BeatKind,
}

impl Beat {
    /// Creates a new note with the given duration and sticking
    pub fn note_with_sticking(duration: Duration, sticking: Sticking) -> Self {
        Self {
            duration,
            kind: BeatKind::Note(Some(sticking)),
        }
    }

    /// Creates a new note with the given duration and no sticking
    pub fn note(duration: Duration) -> Self {
        Self {
            duration,
            kind: BeatKind::Note(None),
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

    /// Internal: compute best fill of exactly `gap_ticks` using durations, optimizing:
    /// primary -> minimal token count; secondary -> minimal total weight; tertiary -> prefer larger last step.
    fn best_fill_for_gap(gap_ticks: i32) -> Option<Vec<Duration>> {
        if gap_ticks < 0 { return None; }
        if gap_ticks == 0 { return Some(Vec::new()); }
        // Precompute coins and weights
        let mut coins: Vec<(i32, Duration, i32)> = Duration::DURATIONS
            .iter()
            .map(|&d| {
                let ticks = Duration::TICKS_PER_WHOLE / Duration::denominator_of(d);
                let weight = Duration::denominator_of(d); // smaller denominator preferred
                (ticks, d, weight)
            })
            .collect();
        // Sort by descending tick size to help tertiary tie-break towards larger steps
        coins.sort_unstable_by(|a,b| b.0.cmp(&a.0));

        let target = gap_ticks as usize;
        #[derive(Clone, Copy)]
        struct Cell { len: u16, weight: i32, prev: i32, choice_idx: u8 }
        let mut dp: Vec<Option<Cell>> = vec![None; target + 1];
        dp[0] = Some(Cell { len: 0, weight: 0, prev: -1, choice_idx: 0 });

        for i in 1..=target {
            let mut best: Option<Cell> = None;
            for (idx, (ticks, _d, w)) in coins.iter().enumerate() {
                let t = *ticks as usize;
                if t <= i {
                    if let Some(prev) = dp[i - t] {
                        let cand = Cell { len: prev.len.saturating_add(1), weight: prev.weight + *w, prev: (i - t) as i32, choice_idx: idx as u8 };
                        best = match best {
                            None => Some(cand),
                            Some(cur) => {
                                // Compare (len, weight); if equal, prefer larger step (since coins sorted desc, smaller idx is larger)
                                if cand.len < cur.len || (cand.len == cur.len && (cand.weight < cur.weight || (cand.weight == cur.weight && (cand.choice_idx as i32) < (cur.choice_idx as i32)))) {
                                    Some(cand)
                                } else { Some(cur) }
                            }
                        };
                    }
                }
            }
            dp[i] = best;
        }

        if dp[target].is_none() { return None; }
        // Reconstruct durations in forward order
        let mut seq_idxs: Vec<usize> = Vec::new();
        let mut i = target as i32;
        while i > 0 {
            let cell = dp[i as usize].unwrap();
            let ci = cell.choice_idx as usize;
            seq_idxs.push(ci);
            i = cell.prev;
        }
        seq_idxs.reverse();
        let result: Vec<Duration> = seq_idxs.into_iter().map(|ci| coins[ci].1).collect();
        Some(result)
    }

    /// Normalize the current measure by reconstructing a simpler equivalent that preserves onsets.
    /// Strategy: rebuild from onset set; for each span between onsets (and edges), fill with minimal-token durations.
    pub fn normalize(&mut self) {
        let max_ticks = self.time_signature.measure_duration_ticks();
        // Collect onset positions and their sticking (if any)
        let mut onsets: Vec<(i32, Option<Sticking>)> = Vec::new();
        let mut pos = 0;
        for beat in &self.beats {
            match beat.kind {
                BeatKind::Note(stick) => {
                    onsets.push((pos, stick));
                }
                BeatKind::Rest => {}
            }
            pos += beat.duration.ticks();
        }
        // Build boundaries: always start at 0; ensure end boundary
        let mut boundaries: Vec<i32> = vec![0];
        for (p, _) in &onsets { if *p > 0 { boundaries.push(*p); } }
        if boundaries.last().copied() != Some(max_ticks) { boundaries.push(max_ticks); }
        boundaries.sort_unstable();
        boundaries.dedup();

        // We'll iterate over spans between consecutive boundaries, but we need to know which are onsets.
        // Build a map from position to sticking for quick lookup (last specified sticking wins if duplicates).
        use std::collections::BTreeMap;
        let mut onset_map: BTreeMap<i32, Option<Sticking>> = BTreeMap::new();
        for (p, s) in onsets { onset_map.insert(p, s); }

        let mut new_beats: Vec<Beat> = Vec::new();
        for w in boundaries.windows(2) {
            let start = w[0];
            let end = w[1];
            let gap = end - start;
            if gap <= 0 { continue; }
            let is_onset = onset_map.contains_key(&start);
            let sticking = onset_map.get(&start).copied().flatten();
            if let Some(seq) = Self::best_fill_for_gap(gap) {
                if is_onset {
                    // First token is Note with sticking; rest are Rests
                    if let Some((first, rest)) = seq.split_first() {
                        if let Some(stick) = sticking {
                            new_beats.push(Beat::note_with_sticking(*first, stick));
                        } else {
                            new_beats.push(Beat::note(*first));
                        }
                        for d in rest { new_beats.push(Beat::rest(*d)); }
                    }
                } else {
                    // Entire span is rest
                    for d in seq { new_beats.push(Beat::rest(d)); }
                }
            } else {
                // Should not happen because original measure was valid; fall back to original micro-chunk
                // Emit as a single rest for safety if possible
                // Try to find a duration exactly equal to gap
                let mut matched = false;
                for &d in Duration::DURATIONS.iter() {
                    if d.ticks() == gap { new_beats.push(Beat::rest(d)); matched = true; break; }
                }
                if !matched {
                    // In worst case, fill with smallest durations
                    let smallest = Duration::DURATIONS.iter().min_by_key(|d| d.ticks()).copied().unwrap();
                    let mut rem = gap;
                    while rem > 0 { new_beats.push(Beat::rest(smallest)); rem -= smallest.ticks(); }
                }
            }
        }

        self.beats = new_beats;
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
    use crate::duration::Duration::{Quarter, Eighth, Sixteenth, TripletEighth, SextupletSixteenth, QuintupletSixteenth, ThirtySecond};
    use super::*;

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

    #[test]
    fn normalize_sextuplet_alternating_to_triplets() {
        let mut measure = Measure::new(TimeSignature::ONE_FOUR);
        // Build: (N 1/24, R 1/24) x 3
        for _ in 0..3 {
            assert!(measure.add_beat(Beat::note(SextupletSixteenth)).is_ok());
            assert!(measure.add_beat(Beat::rest(SextupletSixteenth)).is_ok());
        }
        // At this point, measure is exactly full
        assert_eq!(measure.current_ticks(), measure.time_signature.measure_duration_ticks());

        // Normalize
        measure.normalize();

        // Expect three TripletEighth notes
        let beats = measure.beats();
        assert_eq!(beats.len(), 3);
        for b in beats {
            assert_eq!(b.duration.ticks(), TripletEighth.ticks());
            match b.kind {
                BeatKind::Note(_) => {}
                _ => panic!("expected notes after normalization"),
            }
        }
    }
}
