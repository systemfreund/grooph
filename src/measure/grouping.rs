use crate::duration::Grid;
use crate::measure::TimeSignature;

impl Grid {
    pub(super) fn is_primary_onset(
        &self,
        ts: &TimeSignature,
        onset_ticks: u32,
        groups: Option<&[u8]>,
    ) -> bool {
        let subbeat = self.ticks_per_beat(ts); // one beat_unit
        let measure_ticks = self.ticks_per_measure(ts);

        let g: Vec<u8> = match groups {
            Some(g) => g.to_vec(),
            None => default_groups_for(ts),
        };

        // Validate grouping matches the time signature’s beat count
        if g.iter().map(|&x| x as u32).sum::<u32>() != ts.beats as u32 {
            return false;
        }

        let o = onset_ticks % measure_ticks; // normalize within measure
        if o == 0 {
            return true;
        }

        let mut acc = 0u32;
        for &cnt in &g {
            acc += (cnt as u32) * subbeat;
            if acc == o {
                return true;
            }
        }
        false
    }

    /// Absolute tick of the next primary-beat boundary after `onset_ticks`.
    /// Returns `Some(boundary)` with `boundary > onset_ticks` and `boundary <= measure_ticks`.
    /// Returns `None` only if grouping is invalid (should not happen with our defaults).
    pub(super) fn next_primary_boundary_from(
        &self,
        ts: &TimeSignature,
        onset_ticks: u32,
    ) -> Option<u32> {
        let subbeat = self.ticks_per_beat(ts);
        let measure_ticks = self.ticks_per_measure(ts);

        let groups = default_groups_for(ts);
        if groups.iter().map(|&g| g as u32).sum::<u32>() != ts.beats as u32 {
            return None;
        }

        let o = onset_ticks % measure_ticks; // normalize within bar

        let mut acc = 0u32;
        for &g in &groups {
            acc += (g as u32) * subbeat;
            if acc > o {
                return Some(acc);
            }
        }
        // If onset is at or beyond the last boundary, clamp to the end of the measure
        Some(measure_ticks)
    }

    /// Convenience: how many ticks are left until the next primary boundary (0 if already at/over it).
    pub(super) fn ticks_until_next_primary(&self, ts: &TimeSignature, onset_ticks: u32) -> u32 {
        let measure_ticks = self.ticks_per_measure(ts);
        let o = onset_ticks % measure_ticks;
        match self.next_primary_boundary_from(ts, o) {
            Some(b) if b > o => b - o,
            _ => 0,
        }
    }
}

pub(super) fn default_groups_for(ts: &TimeSignature) -> Vec<u8> {
    // Common conventional defaults
    match (ts.beats, ts.beat_unit) {
        // Compound meters in x/8 felt as dotted quarters (3 eighths per primary beat)
        (6, 8) => vec![3, 3],        // 6/8 → 2 big beats
        (9, 8) => vec![3, 3, 3],     // 9/8 → 3 big beats
        (12, 8) => vec![3, 3, 3, 3], // 12/8 → 4 big beats

        // Additive meters in x/8 (choose the most common defaults)
        (5, 8) => vec![3, 2], // 5/8 → default 3+2 (other feel 2+3 is possible)
        (7, 8) => vec![3, 2, 2], // 7/8 → default 3+2+2 (other feels 2+2+3, 2+3+2)

        // Fallback: simple — one primary beat per beat_unit
        _ => vec![1u8; ts.beats as usize],
    }
}
