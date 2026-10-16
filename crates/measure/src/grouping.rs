use crate::TimeSignature;

pub(super) fn default_groups_for(ts: &TimeSignature) -> Vec<u8> {
    // Common conventional defaults
    match (ts.beats, ts.beat_unit) {
        (2, 2) => vec![1, 1],
        (3, 2) => vec![1, 1, 1],

        (n, 8) => compound_groups(n),
        // Below 5/16 (only used internally, e.g. for tuplet buttons) one group.
        (n @ 5.., 16) => compound_groups(n),
        (n, 16) => vec![n],

        // Fallback: simple — one primary beat per beat_unit
        _ => vec![1u8; ts.beats as usize],
    }
}

/// Groups of two and three for an `n/8` or `n/16` meter: the usual
/// groupings up to 13 (5 = 3+2, 7 = 2+3+2, 8 = 3+3+2, ...), beyond that as
/// many threes as fit with the rest in twos.
fn compound_groups(n: u8) -> Vec<u8> {
    match n {
        0 => vec![],
        1 => vec![1],
        2 => vec![2],
        3 => vec![3],
        4 => vec![2, 2],
        5 => vec![3, 2],    // other feel 2+3 is possible
        6 => vec![3, 3],    // 2 big beats
        7 => vec![2, 3, 2], // other feels 2+2+3, 3+2+2
        8 => vec![3, 3, 2],
        9 => vec![3, 3, 3], // 3 big beats
        10 => vec![3, 3, 2, 2],
        11 => vec![3, 3, 3, 2],
        12 => vec![3, 3, 3, 3], // 4 big beats
        13 => vec![3, 3, 3, 2, 2],
        n => {
            let twos = match n % 3 {
                0 => 0,
                1 => 2,
                _ => 1,
            };
            let threes = (n - 2 * twos) / 3;
            let mut groups = vec![3; threes as usize];
            groups.extend(std::iter::repeat_n(2, twos as usize));
            groups
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compound_groups_add_up_and_use_twos_and_threes() {
        for n in 2..=19 {
            let groups = compound_groups(n);
            assert_eq!(groups.iter().map(|&g| g as u32).sum::<u32>(), n as u32, "{n}");
            assert!(groups.iter().all(|&g| g == 2 || g == 3), "{n}: {groups:?}");
        }
        assert_eq!(compound_groups(7), [2, 3, 2]);
        assert_eq!(compound_groups(14), [3, 3, 3, 3, 2]);
        assert_eq!(compound_groups(16), [3, 3, 3, 3, 2, 2]);
        assert_eq!(compound_groups(19), [3, 3, 3, 3, 3, 2, 2]);
    }

    #[test]
    fn sixteenth_meters_use_the_same_groups_as_eighth_meters() {
        for n in 5..=13 {
            assert_eq!(
                default_groups_for(&TimeSignature { beats: n, beat_unit: 16 }),
                default_groups_for(&TimeSignature { beats: n, beat_unit: 8 })
            );
        }
    }
}
