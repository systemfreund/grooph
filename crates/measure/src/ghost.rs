//! Ghost notes: quiet hits on every grid position of the chosen subdivision
//! that carries no written note onset, so the player hears the subdivision
//! running underneath the rhythm.
//!
//! The grid is laid per beat (the time signature's beat unit):
//! eighths = 2 slots, sixteenths = 4, triplets = 3. `Mixed` uses a triplet
//! grid in beats that contain a tuplet and sixteenths otherwise.

use crate::BeatKind::Note;
use crate::Measure;
use crate::generator::Subdivision;
use crate::grid::DEFAULT_GRID;

/// Local tick positions (within `measure`) where a ghost note sounds.
pub fn ghost_onsets(measure: &Measure, subdivision: Subdivision) -> Vec<u32> {
    let ts = measure.time_signature();
    let beat_ticks = DEFAULT_GRID.ticks_per_beat(&ts);
    let measure_ticks = DEFAULT_GRID.ticks_per_measure(&ts);
    if beat_ticks == 0 {
        return Vec::new();
    }

    let beats = measure.beats();
    let onsets = DEFAULT_GRID.compute_onset_ticks(beats);
    let note_onsets: Vec<u32> =
        beats.iter().zip(&onsets).filter(|(b, _)| b.kind == Note).map(|(_, &t)| t).collect();
    let beat_has_tuplet = |beat_start: u32| {
        beats
            .iter()
            .zip(&onsets)
            .any(|(b, &t)| b.duration.is_tuplet() && t >= beat_start && t < beat_start + beat_ticks)
    };

    let mut out = Vec::new();
    let mut beat_start = 0;
    while beat_start < measure_ticks {
        let slots: u32 = match subdivision {
            Subdivision::Eighths => 2,
            Subdivision::Sixteenths => 4,
            Subdivision::Triplets => 3,
            Subdivision::Mixed if beat_has_tuplet(beat_start) => 3,
            Subdivision::Mixed => 4,
        };
        // Skip grids that don't divide the beat evenly (never happens with
        // the default grid, but keeps the output on exact ticks).
        if beat_ticks.is_multiple_of(slots) {
            let step = beat_ticks / slots;
            for k in 0..slots {
                let t = beat_start + k * step;
                if t < measure_ticks && !note_onsets.contains(&t) {
                    out.push(t);
                }
            }
        }
        beat_start += beat_ticks;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duration::{e, q, s, t8};
    use crate::{Beat, TimeSignature};

    fn tick(d: crate::duration::Duration) -> u32 { DEFAULT_GRID.ticks_of(&d).unwrap() }

    #[test]
    fn quarter_notes_get_sixteenth_ghosts_in_between() {
        let mut m = Measure::new(TimeSignature::TWO_FOUR);
        m.set_beat(0, Beat::note(q())).unwrap();
        m.set_beat(1, Beat::note(q())).unwrap();
        let sx = tick(s());
        assert_eq!(
            ghost_onsets(&m, Subdivision::Sixteenths),
            vec![sx, 2 * sx, 3 * sx, 5 * sx, 6 * sx, 7 * sx]
        );
    }

    #[test]
    fn rests_are_filled_with_ghosts() {
        // A measure of rests: every grid slot is a ghost.
        let m = Measure::new(TimeSignature::FOUR_FOUR);
        assert_eq!(ghost_onsets(&m, Subdivision::Eighths).len(), 8);
        assert_eq!(ghost_onsets(&m, Subdivision::Sixteenths).len(), 16);
        assert_eq!(ghost_onsets(&m, Subdivision::Triplets).len(), 12);
    }

    #[test]
    fn eighth_grid_skips_note_on_the_and() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        m.set_beat(0, Beat::rest(e())).unwrap();
        m.set_beat(1, Beat::note(e())).unwrap();
        assert_eq!(ghost_onsets(&m, Subdivision::Eighths), vec![0]);
    }

    #[test]
    fn mixed_uses_triplet_grid_only_in_tuplet_beats() {
        let mut m = Measure::new(TimeSignature::TWO_FOUR);
        m.set_beat(0, Beat::note(t8())).unwrap(); // triplet group in beat 1
        let beat = tick(q());
        let ghosts = ghost_onsets(&m, Subdivision::Mixed);
        // Beat 1: triplet slots 2 and 3 (note on slot 1).
        assert_eq!(&ghosts[..2], &[beat / 3, 2 * beat / 3]);
        // Beat 2 (rest): four sixteenth slots.
        assert_eq!(&ghosts[2..], &[beat, beat + beat / 4, beat + beat / 2, beat + 3 * beat / 4]);
    }

    #[test]
    fn compound_meter_uses_beat_unit() {
        let m = Measure::new(TimeSignature::SIX_EIGHT);
        // 6 eighth beats * 2 slots each.
        assert_eq!(ghost_onsets(&m, Subdivision::Eighths).len(), 12);
    }
}
