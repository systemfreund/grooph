use super::session::{GlobalBeatOnset, NoteKey, TickKind, neighbour_gaps, note_index};

/// Clamp the `diff_ticks` of a hit on the note `key` to its window as
/// written: halfway to the neighbouring entries of its instrument (on one
/// line, the onsets next to it), so the visual marker stays inside the
/// note's hit window. Unclamped for a note `beats` does not hold.
pub(crate) fn clamp_diff_to_note_window(
    diff_ticks: f64,
    beats: &[GlobalBeatOnset],
    key: NoteKey,
    total_loop_ticks: f64,
) -> f64 {
    if total_loop_ticks <= 0.0 {
        return diff_ticks;
    }
    let Some(idx) = note_index(beats, key) else {
        return diff_ticks;
    };
    let (before, after) = neighbour_gaps(beats, idx, total_loop_ticks, TickKind::Written);
    diff_ticks.clamp(-before * 0.5, after * 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accuracy::session::compute_global_beat_onsets;
    use grooph_measure::grid::DEFAULT_GRID;
    use grooph_measure::notation::parse_score;
    use grooph_measure::tempo::ScoreTiming;
    use grooph_measure::{Instrument, TimeSignature};

    fn quarter() -> f64 {
        DEFAULT_GRID.ticks_per_beat(&TimeSignature::FOUR_FOUR) as f64
    }

    #[test]
    fn a_mark_stays_within_halfway_to_its_neighbours() {
        let score = parse_score("4/4 q q h").unwrap();
        let timing = ScoreTiming::from_score(&score, 120);
        let beats = compute_global_beat_onsets(&score, &timing);
        let q = quarter();
        let total = 4.0 * q;
        let at = |tick: f64| NoteKey::onset(tick as u64);
        // The second quarter: a quarter to either side.
        assert_eq!(clamp_diff_to_note_window(q, &beats, at(q), total), q / 2.0);
        assert_eq!(clamp_diff_to_note_window(-q, &beats, at(q), total), -q / 2.0);
        // The half: on to the first quarter round the loop, a half.
        assert_eq!(clamp_diff_to_note_window(2.0 * q, &beats, at(2.0 * q), total), q);
        // The only note of the loop: half the loop.
        let one = parse_score("4/4 w").unwrap();
        let beats = compute_global_beat_onsets(&one, &timing);
        assert_eq!(clamp_diff_to_note_window(total, &beats, at(0.0), total), total / 2.0);
    }

    #[test]
    fn a_kit_mark_stays_within_halfway_to_its_instruments_neighbours() {
        let score = parse_score("4/4 H-K:e H:e H:e H:e H-K:e H:e H:e H:e").unwrap();
        let timing = ScoreTiming::from_score(&score, 120);
        let beats = compute_global_beat_onsets(&score, &timing);
        let e = quarter() / 2.0;
        let total = 8.0 * e;
        let kick = NoteKey { tick: 0, instrument: Instrument::Kick };
        let hi_hat = NoteKey { tick: 0, instrument: Instrument::HiHat };
        // The kicks are four eighths apart, the hi-hats one.
        assert_eq!(clamp_diff_to_note_window(3.0 * e, &beats, kick, total), 2.0 * e);
        assert_eq!(clamp_diff_to_note_window(3.0 * e, &beats, hi_hat, total), e / 2.0);
    }
}
