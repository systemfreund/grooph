use super::session::{GlobalBeatOnset, NoteKey, neighbour_gaps, note_index};

/// Clamp the `diff_ticks` of a hit on the note `key` to its window as
/// written: up to the neighbouring notes of its instrument (on one line,
/// the notes next to it), so the visual marker stays between them. A hit
/// can count for a note that far away (see [`super::align`]); a connector
/// shows which note it belongs to. Unclamped for a note `beats` does not
/// hold.
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
    let (before, after) = neighbour_gaps(beats, idx, total_loop_ticks);
    diff_ticks.clamp(-before, after)
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
    fn a_mark_stays_within_the_gap_to_its_neighbours() {
        let score = parse_score("4/4 q q h").unwrap();
        let timing = ScoreTiming::from_score(&score, 120);
        let beats = compute_global_beat_onsets(&score, &timing);
        let q = quarter();
        let total = 4.0 * q;
        let at = |tick: f64| NoteKey::onset(tick as u64);
        // The second quarter: a quarter to either side.
        assert_eq!(clamp_diff_to_note_window(0.7 * q, &beats, at(q), total), 0.7 * q);
        assert_eq!(clamp_diff_to_note_window(1.5 * q, &beats, at(q), total), q);
        assert_eq!(clamp_diff_to_note_window(-1.5 * q, &beats, at(q), total), -q);
        // The half: on to the first quarter round the loop, a half; back to
        // the second quarter, one.
        assert_eq!(clamp_diff_to_note_window(3.0 * q, &beats, at(2.0 * q), total), 2.0 * q);
        assert_eq!(clamp_diff_to_note_window(-3.0 * q, &beats, at(2.0 * q), total), -q);
        // The only note of the loop: the whole loop.
        let one = parse_score("4/4 w").unwrap();
        let beats = compute_global_beat_onsets(&one, &timing);
        assert_eq!(clamp_diff_to_note_window(2.0 * total, &beats, at(0.0), total), total);
    }

    #[test]
    fn a_kit_mark_stays_within_the_gap_to_its_instruments_neighbours() {
        let score = parse_score("4/4 H-K:e H:e H:e H:e H-K:e H:e H:e H:e").unwrap();
        let timing = ScoreTiming::from_score(&score, 120);
        let beats = compute_global_beat_onsets(&score, &timing);
        let e = quarter() / 2.0;
        let total = 8.0 * e;
        let kick = NoteKey { tick: 0, instrument: Instrument::Kick };
        let hi_hat = NoteKey { tick: 0, instrument: Instrument::HiHat };
        // The kicks are four eighths apart, the hi-hats one.
        assert_eq!(clamp_diff_to_note_window(3.0 * e, &beats, kick, total), 3.0 * e);
        assert_eq!(clamp_diff_to_note_window(5.0 * e, &beats, kick, total), 4.0 * e);
        assert_eq!(clamp_diff_to_note_window(-5.0 * e, &beats, kick, total), -4.0 * e);
        assert_eq!(clamp_diff_to_note_window(3.0 * e, &beats, hi_hat, total), e);
        assert_eq!(clamp_diff_to_note_window(-3.0 * e, &beats, hi_hat, total), -e);
    }
}
