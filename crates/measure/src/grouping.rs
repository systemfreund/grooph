use crate::TimeSignature;

/// Primary beat groups of `ts`, in beat units: the grouping `ts` picks
/// (see [`TimeSignature::groupings`]), the usual one if that is out of range.
pub(super) fn default_groups_for(ts: &TimeSignature) -> Vec<u8> {
    let usual = usual_groups(ts.beats, ts.beat_unit);
    match ts.grouping {
        0 => usual,
        i => alternative_groups(ts.beats, ts.beat_unit)
            .get(i as usize - 1)
            .map_or(usual, |groups| groups.to_vec()),
    }
}

/// Every grouping offered for `beats/beat_unit`, the usual one first.
pub(super) fn groupings(beats: u8, beat_unit: u8) -> Vec<Vec<u8>> {
    let mut all = vec![usual_groups(beats, beat_unit)];
    all.extend(alternative_groups(beats, beat_unit).iter().map(|g| g.to_vec()));
    all
}

/// Other common groupings of the odd x/8 and x/16 meters (the usual one is
/// [`usual_groups`]). Empty for every other meter.
fn alternative_groups(beats: u8, beat_unit: u8) -> &'static [&'static [u8]] {
    if !matches!(beat_unit, 8 | 16) {
        return &[];
    }
    match beats {
        5 => &[&[2, 3]],
        7 => &[&[2, 2, 3], &[3, 2, 2]],
        8 => &[&[3, 2, 3], &[2, 3, 3]],
        10 => &[&[3, 2, 3, 2], &[2, 3, 2, 3], &[2, 2, 3, 3]],
        11 => &[&[2, 2, 3, 2, 2], &[2, 3, 3, 3]],
        _ => &[],
    }
}

fn usual_groups(beats: u8, beat_unit: u8) -> Vec<u8> {
    // Common conventional defaults
    match (beats, beat_unit) {
        (2, 2) => vec![1, 1],
        (3, 2) => vec![1, 1, 1],

        (n, 8) => compound_groups(n),
        // Below 5/16 (only used internally, e.g. for tuplet buttons) one group.
        (n @ 5.., 16) => compound_groups(n),
        (n, 16) => vec![n],

        // Fallback: simple — one primary beat per beat_unit
        _ => vec![1u8; beats as usize],
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
    fn groupings_add_up_and_start_with_the_usual_one() {
        for unit in [8, 16] {
            for n in 5..=13 {
                let all = groupings(n, unit);
                assert_eq!(all[0], compound_groups(n), "{n}/{unit}");
                for g in &all {
                    assert_eq!(g.iter().map(|&b| b as u32).sum::<u32>(), n as u32, "{n}/{unit}");
                }
                let mut unique = all.clone();
                unique.sort();
                unique.dedup();
                assert_eq!(unique.len(), all.len(), "{n}/{unit}");
            }
        }
        assert_eq!(groupings(7, 8), [vec![2, 3, 2], vec![2, 2, 3], vec![3, 2, 2]]);
        assert_eq!(groupings(7, 4).len(), 1);
        assert_eq!(groupings(6, 8).len(), 1);
    }

    #[test]
    fn picked_grouping_drives_the_groups() {
        let ts = TimeSignature::SEVEN_EIGHT.with_grouping(2);
        assert_eq!(default_groups_for(&ts), [3, 2, 2]);
        // Out of range falls back to the usual grouping.
        let ts = TimeSignature { grouping: 9, ..TimeSignature::SEVEN_EIGHT };
        assert_eq!(default_groups_for(&ts), [2, 3, 2]);
    }

    #[test]
    fn sixteenth_meters_use_the_same_groups_as_eighth_meters() {
        for n in 5..=13 {
            assert_eq!(
                default_groups_for(&TimeSignature::new(n, 16)),
                default_groups_for(&TimeSignature::new(n, 8))
            );
        }
    }
}
