use crate::duration::{Duration, default_duration_set};

/// Compute the best exact spelling for a gap measured in ticks.
/// Optimization priority:
/// 1) Minimal number of tokens (durations)
/// 2) Minimal total weight (sum of denominators) to prefer simpler durations
/// 3) Prefer larger steps on ties for determinism
pub(crate) fn best_fill_for_gap(gap_ticks: i32) -> Option<Vec<Duration>> {
    if gap_ticks <= 0 {
        return None;
    }

    let set = default_duration_set();

    // Build coin list: (ticks, duration, weight)
    let mut coins: Vec<(i32, Duration, i32)> = set
        .durations
        .iter()
        .filter_map(|d| {
            let den = d.denominator();
            set.grid.ticks_of(d).map(|t| (t, *d, den))
        })
        .collect();
    // Prefer smaller coin as the last step → small duration ends up at the end of the returned sequence
    coins.sort_unstable_by(|a, b| a.0.cmp(&b.0));

    let target = gap_ticks as usize;
    #[derive(Clone, Copy)]
    struct Cell {
        len: u16,
        weight: i32,
        prev: i32,
        choice_idx: u8,
    }
    let mut dp: Vec<Option<Cell>> = vec![None; target + 1];
    dp[0] = Some(Cell { len: 0, weight: 0, prev: -1, choice_idx: 0 });

    for i in 1..=target {
        let mut best: Option<Cell> = None;
        for (idx, (ticks, _d, w)) in coins.iter().enumerate() {
            let t = *ticks as usize;
            if t <= i {
                if let Some(prev) = dp[i - t] {
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
        }
        dp[i] = best;
    }

    if dp[target].is_none() {
        return None;
    }

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
    let result: Vec<Duration> = seq_idxs.into_iter().map(|ci| coins[ci].1).collect();
    Some(result)
}
