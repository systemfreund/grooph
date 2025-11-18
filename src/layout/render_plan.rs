use crate::layout::beam_plan::{BeamGroup, BeamPlan, compute_beam_plan};
use crate::layout::tuplet_plan::{TupletPlan, compute_tuplet_plan};
use crate::measure::{Beat, Measure};

/// Logical Beat-Index within a measure (0-based)
pub type BeatIdx = usize;

/// Logical, device-independent render plan derived from a `Measure`.
///
/// Purpose and scope:
/// - Encodes musical layout decisions that do not depend on pixels, DPI, or fonts.
/// - Contains only logical structures such as beaming groups and tuplet spans.
/// - Carries no absolute coordinates, sizes, or stroke thicknesses.
///
/// Relationship to `MeasureLayoutPx`:
/// - `RenderPlan` is consumed by `build_measure_layout_px(..)` together with the target
///   `Rect`, `FontId`, and UI scaling to produce a pixel-resolved `MeasureLayoutPx`.
/// - The renderer uses `MeasureLayoutPx` exclusively to draw; it makes no further
///   geometry decisions.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderPlan {
    /// Beaming groups with continuity per adjacent pair. This defines which neighboring notes
    /// are connected at how many beam levels, purely logically (by beat indices).
    pub beams: Vec<BeamGroup>,
    /// Tuplet runs (logical start..=end in beat indices). Whether the tuplet can be shown as
    /// number-only or requires a bracket is decided here based on musical semantics, not pixels.
    pub tuplets: Vec<TupletPlan>,
}

/// Build the logical `RenderPlan` for a measure.
///
/// This step analyzes the musical content (meter-aware grouping, durations) and produces
/// beaming groups and tuplet segments. No pixel geometry is computed here.
pub fn plan_measure(measure: &Measure) -> RenderPlan {
    let BeamPlan { groups: beams } = compute_beam_plan(measure);
    let tuplets = compute_tuplet_plan(measure, &beams);

    RenderPlan { beams, tuplets }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::tuplet_plan::EdgeConnection;
    use crate::measure::BeatKind::Note;
    use crate::measure::duration::{e, t8, t16, t32};
    use crate::measure::{Beat, Measure, TimeSignature};

    #[test]
    fn beaming_group_within_primary_boundaries_in_seven_eight() {
        // 7/8 mit Achteln, Standardgruppierung 2+3+2.
        // Die mittlere Gruppe (Beats 2..=4, 0-basiert) sollte beamed sein.
        let mut m = Measure::new(TimeSignature::SEVEN_EIGHT);
        for i in 0..7 {
            m.set_beat_at(i, Beat::note(e())).unwrap();
        }

        let plan = plan_measure(&m);
        let has_group = plan.beams.iter().any(|g| {
            let idxs = &g.beat_indices;
            if !(idxs.contains(&2) && idxs.contains(&3) && idxs.contains(&4)) {
                return false;
            }
            let mut pos_map = std::collections::HashMap::new();
            for (i, gi) in idxs.iter().enumerate() {
                pos_map.insert(*gi, i);
            }
            let l2 = *pos_map.get(&2).unwrap();
            let l3 = *pos_map.get(&3).unwrap();
            let l4 = *pos_map.get(&4).unwrap();
            if !(l2 < l3 && l3 < l4) {
                return false;
            }
            g.continuity.get(l2).copied().unwrap_or(0) >= 1
                && g.continuity.get(l3).copied().unwrap_or(0) >= 1
        });
        assert!(has_group, "Beats 3-5 sollten in 7/8 beamed sein (2+3+2, mittlere Gruppe)");
    }

    #[test]
    fn triplet_bracket_over_beats_4_to_6() {
        // Konstruiere einen 4/4, in dem Beats 3..=5 (0-basiert) eine Triole bilden
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Setze zunächst 6 Achtel, dann eine Triole über die nächsten 3 Achtel-Schlitze
        for i in 0..3 {
            m.set_beat_at(i, Beat::note(e())).unwrap();
        }
        // Drei Achtel-Triolett-Noten
        m.set_beat_at(3, Beat::note(t8())).unwrap();
        m.set_beat_at(4, Beat::note(t8())).unwrap();
        m.set_beat_at(5, Beat::note(t8())).unwrap();

        let plan = plan_measure(&m);
        let ok = plan.tuplets.iter().any(|t| t.count == 3 && t.start == 3 && t.end == 5);
        assert!(ok, "Triplet-Klammer sollte über Beats 4–6 (3..=5) liegen");
    }

    #[test]
    fn triplet_bracket_over_beats_when_preceding_beat_is_connected_to_triplet_with_beams_in_7_8() {
        let mut m = Measure::new_init(TimeSignature::SEVEN_EIGHT, Note);
        m.set_beat_at(3, Beat::note(t8())).unwrap();
        m.set_beat_at(4, Beat::note(t8())).unwrap();
        m.set_beat_at(5, Beat::note(t8())).unwrap();

        let plan = plan_measure(&m);

        // Beam connects the triplets (3,4,5) with the preceding beat (2).
        // Expected because of the default 2+3+2 grouping in 7/8.
        assert_eq!(plan.beams[1].beat_indices, vec![2, 3, 4, 5]);

        // We expect a bracket to be drawn over the triplets (3,4,5) to visually distinguish them
        // from the preceding beat.
        let t = plan
            .tuplets
            .iter()
            .find(|t| t.count == 3 && t.start == 3 && t.end == 5)
            .expect("expected triplet over beats 3..=5");

        assert!(t.fully_beamed);
        assert!(!t.contains_rest);
        assert_eq!(t.edge_connection, EdgeConnection::Left);
        assert!(!t.number_only());
    }

    #[test]
    fn triplet_bracket_over_beats_when_following_beat_is_connected_to_triplet_with_beams_in_7_8() {
        let mut m = Measure::new_init(TimeSignature::SEVEN_EIGHT, Note);
        m.set_beat_at(2, Beat::note(t8())).unwrap();
        m.set_beat_at(3, Beat::note(t8())).unwrap();
        m.set_beat_at(4, Beat::note(t8())).unwrap();

        let plan = plan_measure(&m);

        // Beam connects the triplets (2,3,4) with the following beat (5).
        // Expected because of the default 2+3+2 grouping in 7/8.
        assert_eq!(plan.beams[1].beat_indices, vec![2, 3, 4, 5]);

        // We expect a bracket to be drawn over the triplets (3,4,5) to visually distinguish them
        // from the preceding beat.
        let t = plan
            .tuplets
            .iter()
            .find(|t| t.count == 3 && t.start == 2 && t.end == 4)
            .expect("expected triplet over beats 2..=4");

        assert!(t.fully_beamed);
        assert!(!t.contains_rest);
        assert_eq!(t.edge_connection, EdgeConnection::Right);
        assert!(!t.number_only());
    }

    #[test]
    fn triplet_render_plan_0() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        m.set_beat_at(0, Beat::note(t16())).unwrap();
        m.set_beat_at(1, Beat::note(t8())).unwrap();

        let mut tuplets = plan_measure(&m).tuplets;
        assert_eq!(tuplets.len(), 1);
        // The number remains 3 (triplet), but the bracket spans only the two remaining slots
        assert_eq!(tuplets[0].count, 3);
        assert_eq!(tuplets[0].start, 0);
        assert_eq!(tuplets[0].end, 1);

        m.set_beat_at(2, Beat::note(t16())).unwrap();
        tuplets = plan_measure(&m).tuplets;
        assert_eq!(tuplets.len(), 2);
        assert_eq!(tuplets[1].count, 3);
        assert_eq!(tuplets[1].start, 2);
        assert_eq!(tuplets[1].end, 4);
    }

    #[test]
    fn triplet_render_plan_1() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        m.set_beat_at(0, Beat::note(t32())).unwrap();
        m.set_beat_at(0, Beat::note(t16())).unwrap();

        let mut tuplets = plan_measure(&m).tuplets;
        assert_eq!(tuplets.len(), 1, "first tuplet group not found");
        // The number remains 3 (dtriplet), but the bracket spans only the two remaining slots
        assert_eq!(tuplets[0].count, 3);
        assert_eq!(tuplets[0].start, 0);
        assert_eq!(tuplets[0].end, 1);

        // Start a new tuplet group with a t16 immediately after the t32-group.
        m.set_beat_at(2, Beat::rest(t16())).unwrap();
        // Now we expect to have two tuplet groups, and the very last beat must be a simple 1/16 note.
        tuplets = plan_measure(&m).tuplets;
        println!("{:?}", tuplets);
        assert_eq!(tuplets.len(), 2);
        tuplets = plan_measure(&m).tuplets;

        assert_eq!(tuplets[1].count, 3);
        assert_eq!(tuplets[1].start, 2);
        assert_eq!(tuplets[1].end, 4);
    }

    #[test]
    fn triplet_render_plan_2() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        m.set_beat_at(0, Beat::note(t8())).unwrap();

        let mut tuplets = plan_measure(&m).tuplets;
        assert_eq!(tuplets.len(), 1, "first tuplet group not found");
        // The number remains 3 (dtriplet), but the bracket spans only the two remaining slots
        assert_eq!(tuplets[0].count, 3);
        assert_eq!(tuplets[0].start, 0);
        assert_eq!(tuplets[0].end, 1);

        // Start a new tuplet group with a t16 immediately after the t32-group.
        m.set_beat_at(2, Beat::rest(t16())).unwrap();
        // Now we expect to have two tuplet groups, and the very last beat must be a simple 1/16 note.
        tuplets = plan_measure(&m).tuplets;
        println!("{:?}", tuplets);
        assert_eq!(tuplets.len(), 2);
        tuplets = plan_measure(&m).tuplets;

        assert_eq!(tuplets[1].count, 3);
        assert_eq!(tuplets[1].start, 2);
        assert_eq!(tuplets[1].end, 4);
    }
}
