use crate::VoiceBar;
use crate::duration::Duration;
use crate::grid::DEFAULT_GRID;
use crate::math::gcd;

impl VoiceBar {
    /// Compute the best exact spelling for a gap measured in ticks.
    /// Optimization priority:
    /// 1) Minimal number of tokens (durations)
    /// 2) Minimal total weight (sum of denominators) to prefer simpler durations
    /// 3) Prefer larger steps on ties for determinism
    pub(crate) fn best_fill_for_gap(
        &self,
        gap_ticks: u32,
        allowed: &[Duration],
    ) -> Option<Vec<Duration>> {
        if gap_ticks == 0 {
            return None;
        }

        // Build coin list: (ticks, duration, weight)
        let mut coins: Vec<(u32, Duration, u32)> = if !allowed.is_empty() {
            allowed
                .iter()
                .copied()
                .filter_map(|d| {
                    let den = d.denominator();
                    DEFAULT_GRID.ticks_of(&d).map(|t| (t, d, den))
                })
                .collect()
        } else {
            DEFAULT_GRID
                .durations
                .iter()
                .copied()
                // Up to quarters, as before halves were written: the editor
                // has no tools for half rests.
                .filter(|d| matches!(d, Duration::Simple(v) if v.denominator() >= 4))
                .filter_map(|d| {
                    let den = d.denominator();
                    DEFAULT_GRID.ticks_of(&d).map(|t| (t, d, den))
                })
                .collect()
        };
        coins.sort_unstable_by_key(|c| std::cmp::Reverse(c.0));

        // Every sum of coins is a multiple of their common divisor, so the
        // table only needs those steps: a 4/4 bar has 4 * 2^n * 3^m * .. ticks
        // but few steps of the shortest coin.
        let unit = coins.iter().fold(gap_ticks, |g, c| gcd(g, c.0));
        let target = (gap_ticks / unit) as usize;
        #[derive(Clone, Copy)]
        struct Cell {
            len: u16,
            weight: u32,
            prev: i32,
            choice_idx: u8,
        }
        let mut dp: Vec<Option<Cell>> = vec![None; target + 1];
        dp[0] = Some(Cell { len: 0, weight: 0, prev: -1, choice_idx: 0 });

        for i in 1..=target {
            let mut best: Option<Cell> = None;
            for (idx, (ticks, _d, w)) in coins.iter().enumerate() {
                let t = (*ticks / unit) as usize;
                if t <= i
                    && let Some(prev) = dp[i - t]
                {
                    let cand = Cell {
                        len: prev.len.saturating_add(1),
                        weight: prev.weight + *w,
                        prev: (i - t) as i32,
                        choice_idx: idx as u8,
                    };
                    best = match best {
                        None => Some(cand),
                        Some(cur) => {
                            if cand.len < cur.len
                                || (cand.len == cur.len
                                    && (cand.weight < cur.weight
                                        || (cand.weight == cur.weight
                                            && (cand.choice_idx as i32) < (cur.choice_idx as i32))))
                            {
                                Some(cand)
                            } else {
                                Some(cur)
                            }
                        }
                    };
                }
            }
            dp[i] = best;
        }

        dp[target]?;

        // Reconstruct sequence
        let mut seq_idxs: Vec<usize> = Vec::new();
        let mut i = target as i32;
        while i > 0 {
            let cell = dp[i as usize].unwrap();
            let ci = cell.choice_idx as usize;
            seq_idxs.push(ci);
            i = cell.prev;
        }
        seq_idxs.reverse();
        let mut result: Vec<Duration> = seq_idxs.into_iter().map(|ci| coins[ci].1).collect();
        result.sort_by(|a, b| {
            let ta = DEFAULT_GRID.ticks_of(a).unwrap();
            let tb = DEFAULT_GRID.ticks_of(b).unwrap();
            ta.cmp(&tb)
        });
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use crate::duration::{h, q};
    use crate::grid::DEFAULT_GRID;
    use crate::{Measure, TimeSignature};

    #[test]
    fn gaps_fill_with_quarters_not_halves() {
        let m = Measure::new(TimeSignature::FOUR_FOUR);
        let half = DEFAULT_GRID.ticks_of(&h()).unwrap();
        assert_eq!(m.first_voice().best_fill_for_gap(half, &[]), Some(vec![q(), q()]));
    }
}
