use crate::duration::{COMMON_DURATIONS, Duration, NoteValue, TupletSpec};
use crate::grouping::default_groups_for;
use crate::math::lcm;
use crate::{Beat, TimeSignature};

pub const DEFAULT_GRID: Grid = Grid::from_durations(&COMMON_DURATIONS);

/// Most notes of a tuplet outside [`COMMON_DURATIONS`].
pub const MAX_GENERAL_TUPLET: u8 = 16;

const NOTE_VALUES: [NoteValue; 6] = [
    NoteValue::Whole,
    NoteValue::Half,
    NoteValue::Quarter,
    NoteValue::Eighth,
    NoteValue::Sixteenth,
    NoteValue::ThirtySecond,
];

/// A tuplet of `n` notes in the time of `m` that is not a plain value:
/// `n` up to [`MAX_GENERAL_TUPLET`], more than half and fewer than twice
/// `m`, like 7:5 or 5:7.
pub fn is_general_tuplet(d: &Duration) -> bool {
    let Duration::Tuplet(TupletSpec { n, m, .. }) = *d else {
        return false;
    };
    let (n, m) = (u32::from(n), u32::from(m));
    (2..=u32::from(MAX_GENERAL_TUPLET)).contains(&n) && n != m && 2 * n > m && n < 2 * m
}

/// A tick grid provider. Build dynamically from the set of supported durations.
#[derive(Debug)]
pub struct Grid<'a> {
    pub ticks_per_whole: u32,
    pub durations: &'a [Duration],
}

impl Grid<'_> {
    /// Build a dynamic grid as the LCM of the denominators of the given
    /// durations and of 64: eight clicks over an odd number of eighths (8:5
    /// in 5/8) fall on sixty-fourths.
    pub const fn from_durations(durs: &[Duration]) -> Grid<'_> {
        let mut l = 64u32;
        let mut i = 0usize;
        while i < durs.len() {
            let f = durs[i].as_fraction();
            l = lcm(l, f.den);
            i += 1;
        }
        Grid { ticks_per_whole: l, durations: durs }
    }

    pub const fn ticks_from_fraction(&self, num: u32, den: u32) -> Option<u32> {
        if den == 0 {
            return None;
        }
        if !self.ticks_per_whole.is_multiple_of(den) {
            return None;
        }
        Some((self.ticks_per_whole / den) * num)
    }

    pub const fn ticks_of(&self, d: &Duration) -> Option<u32> {
        let f = d.as_fraction();
        self.ticks_from_fraction(f.num, f.den)
    }

    pub const fn ticks_per_beat(&self, time_signature: &TimeSignature) -> u32 {
        self.ticks_per_whole / (time_signature.beat_unit as u32)
    }

    /// Returns a measure's total duration in integer ticks
    pub const fn ticks_per_measure(&self, time_signature: &TimeSignature) -> u32 {
        (time_signature.beats as u32) * self.ticks_per_beat(time_signature)
    }

    pub const fn ticks_to_whole_notes(&self, ticks: u32) -> f64 {
        (ticks as f64) / (self.ticks_per_whole as f64)
    }

    /// Compute the primary grouping stride in ticks for a time signature.
    pub fn primary_boundaries(&self, ts: &TimeSignature) -> Vec<u32> {
        let subbeat = self.ticks_per_beat(ts); // ticks per beat_unit
        let measure_ticks = self.ticks_per_measure(ts); // ticks per measure

        let groups = default_groups_for(ts);

        let beats_sum: u32 = groups.iter().map(|&g| g as u32).sum();
        if beats_sum != ts.beats as u32 {
            // Invalid grouping for ts; safe fallback: no in‑measure boundaries
            return Vec::new();
        }

        let mut acc = 0u32;
        let mut bounds = Vec::new();
        for &cnt in groups.iter() {
            acc += (cnt as u32) * subbeat;
            if acc < measure_ticks {
                bounds.push(acc);
            }
        }
        bounds
    }

    /// Whether a measure may hold `d`: one of [`Self::durations`], or a
    /// tuplet of any other ratio (see [`is_general_tuplet`]) on the grid.
    pub fn supports(&self, d: &Duration) -> bool {
        self.durations.contains(d) || (is_general_tuplet(d) && self.ticks_of(d).is_some())
    }

    /// All supported durations on the tuplet grid identified by `(n, m)`.
    /// Used to constrain fills that must stay inside an existing tuplet group.
    /// A ratio outside [`Self::durations`] takes every note value on the grid.
    pub fn tuplet_grid_durations(&self, n: u8, m: u8) -> Vec<Duration> {
        let listed: Vec<Duration> = self
            .durations
            .iter()
            .copied()
            .filter(|d| matches!(d, Duration::Tuplet(TupletSpec { n: dn, m: dm, .. }) if *dn == n && *dm == m))
            .collect();
        if !listed.is_empty() {
            return listed;
        }
        NOTE_VALUES
            .into_iter()
            .map(|base| Duration::Tuplet(TupletSpec { n, m, base }))
            .filter(|d| self.supports(d))
            .collect()
    }

    pub fn compute_onset_ticks(&self, beats: &[Beat]) -> Vec<u32> {
        let mut onsets: Vec<u32> = Vec::with_capacity(beats.len());
        let mut t = 0;
        for b in beats.iter() {
            onsets.push(t);
            if let Some(dt) = self.ticks_of(&b.duration) {
                t += dt;
            }
        }
        onsets
    }
}
