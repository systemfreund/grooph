use crate::measure::duration::Duration;
use crate::measure::grid::default_grid;

pub enum SortOrder {
    Ascending,
    Descending,
}

/// Compute the best exact spelling for a gap measured in ticks.
/// Optimization priority:
/// 1) Minimal number of tokens (durations)
/// 2) Minimal total weight (sum of denominators) to prefer simpler durations
/// 3) Prefer larger steps on ties for determinism
pub(crate) fn best_fill_for_gap(gap_ticks: u32, allowed: &[Duration]) -> Option<Vec<Duration>> {
    if gap_ticks == 0 {
        return None;
    }

    let grid = default_grid();

    // Build coin list: (ticks, duration, weight)
    let mut coins: Vec<(u32, Duration, u32)> = if !allowed.is_empty() {
        allowed
            .iter()
            .copied()
            .filter_map(|d| {
                let den = d.denominator();
                grid.ticks_of(&d).map(|t| (t, d, den))
            })
            .collect()
    } else {
        grid.durations
            .iter()
            .copied()
            .filter(|d| !matches!(d, Duration::Dotted { .. }))
            .filter_map(|d| {
                let den = d.denominator();
                grid.ticks_of(&d).map(|t| (t, d, den))
            })
            .collect()
    };
    coins.sort_unstable_by(|a, b| b.0.cmp(&a.0));

    let target = gap_ticks as usize;
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
            let t = *ticks as usize;
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
        let ta = grid.ticks_of(a).unwrap();
        let tb = grid.ticks_of(b).unwrap();
        ta.cmp(&tb)
    });
    Some(result)
}
