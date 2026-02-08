use crate::TimeSignature;

pub(super) fn default_groups_for(ts: &TimeSignature) -> Vec<u8> {
    // Common conventional defaults
    match (ts.beats, ts.beat_unit) {
        (2, 2) => vec![1, 1],
        (3, 2) => vec![1, 1, 1],

        (2, 8) => vec![2],
        (3, 8) => vec![3],
        (4, 8) => vec![4],
        (5, 8) => vec![3, 2], // 5/8 → default 3+2 (other feel 2+3 is possible)
        (6, 8) => vec![3, 3], // 6/8 → 2 big beats
        (7, 8) => vec![2, 3, 2], // 7/8 → default 2+3+2 (other feels 2+2+3, 2+3+2)
        (9, 8) => vec![3, 3, 3], // 9/8 → 3 big beats
        (12, 8) => vec![3, 3, 3, 3], // 12/8 → 4 big beats
        (n, 8) => vec![n],
        (n, 16) => vec![n],

        // Fallback: simple — one primary beat per beat_unit
        _ => vec![1u8; ts.beats as usize],
    }
}
