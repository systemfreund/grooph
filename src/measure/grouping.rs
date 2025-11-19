use crate::measure::TimeSignature;

pub(super) fn default_groups_for(ts: &TimeSignature) -> Vec<u8> {
    // Common conventional defaults
    match (ts.beats, ts.beat_unit) {
        // Compound meters in x/8 felt as dotted quarters (3 eighths per primary beat)
        (6, 8) => vec![3, 3],        // 6/8 → 2 big beats
        (9, 8) => vec![3, 3, 3],     // 9/8 → 3 big beats
        (12, 8) => vec![3, 3, 3, 3], // 12/8 → 4 big beats

        // Additive meters in x/8 (choose the most common defaults)
        (5, 8) => vec![3, 2], // 5/8 → default 3+2 (other feel 2+3 is possible)
        (7, 8) => vec![2, 3, 2], // 7/8 → default 2+3+2 (other feels 2+2+3, 2+3+2)

        // Fallback: simple — one primary beat per beat_unit
        _ => vec![1u8; ts.beats as usize],
    }
}
