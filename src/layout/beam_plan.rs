use crate::layout::render_plan::BeatIdx;
use crate::layout::tuplet_plan::detect_tuplet_spans;
use crate::measure::duration::{Duration, NoteValue, TupletSpec};
use crate::measure::{Beat, BeatKind, Measure, TimeSignature};

/// Number of beams implied by a duration (eighth = 1, sixteenth = 2, 32nd = 3).
/// Tuplets map to their base note value for beam count purposes.
fn beam_count(d: &Duration) -> u8 {
    match d.base_note() {
        NoteValue::Whole | NoteValue::Half | NoteValue::Quarter => 0,
        NoteValue::Eighth => 1,
        NoteValue::Sixteenth => 2,
        NoteValue::ThirtySecond => 3,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BeamPlan {
    pub groups: Vec<BeamGroup>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BeamGroup {
    /// Stable id within the measure for selection
    pub group_index: usize,
    /// Indices into Measure.beats() of notes participating in the group
    pub beat_indices: Vec<BeatIdx>,
    /// Per-note beam count (same length as note_indices)
    pub beam_counts: Vec<u8>,
    /// For each adjacent pair (i -> i+1) inside note_indices, how many beams continue between the stems
    /// Length = note_indices.len() - 1
    pub continuity: Vec<u8>,
    /// True if this group visually continues a beam from a previous measure across the left barline
    pub continues_from_previous: bool,
    /// True if this group visually continues into the next measure across the right barline
    pub continues_into_next: bool,
}

/// Compute a default beaming plan for a single measure according to common rules:
/// - Group by primary beat boundaries of the time signature
/// - Contiguous tuplets (same n, m, base; no intervening rests or non-matching notes) take precedence
///   over primary boundaries and remain within a single BeamGroup
/// - Rests break beams: any rest between two notes splits the BeamGroup
/// - Cross-measure beams are exposed via the `continues_*` flags but left as false here; a higher level
///   can link adjacent measures and set these appropriately.
pub(super) fn compute_beam_plan(measure: &Measure) -> BeamPlan {
    let beats = measure.beats();
    let ts = measure.time_signature();
    let onsets = measure.grid.compute_onset_ticks(beats);

    // Compute primary boundaries (tick positions inside the measure where groups should break by default)
    let boundaries = measure.grid.primary_boundaries(&ts);

    // Build tuplet span map (beam-independent segmentation) for boundary decisions
    let spans = detect_tuplet_spans(measure);
    let mut span_of_idx: Vec<Option<usize>> = vec![None; beats.len()];
    for (sid, s) in spans.iter().enumerate() {
        for ix in span_of_idx.iter_mut().take(s.end + 1).skip(s.start) {
            *ix = Some(sid);
        }
    }

    // Collect indices of beamable notes
    let mut note_idxs: Vec<BeatIdx> = Vec::new();
    for (i, b) in beats.iter().enumerate() {
        if b.kind == BeatKind::Note && beam_count(&b.duration) > 0 {
            note_idxs.push(i);
        }
    }

    let mut groups: Vec<BeamGroup> = Vec::new();
    if note_idxs.is_empty() {
        return BeamPlan { groups };
    }

    // Build groups: start new when crossing primary boundary or encountering a non-beamable NOTE between
    let mut cur: Vec<BeatIdx> = vec![note_idxs[0]];
    for w in note_idxs.windows(2) {
        let a = w[0];
        let b = w[1];

        let a_on = onsets[a];
        let b_on = onsets[b];

        let boundary_between = boundaries.iter().any(|&bd| bd > a_on && bd <= b_on);
        let mut break_group = false;
        // First: never merge across boundaries between two different tuplet spans
        if let (Some(sa), Some(sb)) = (span_of_idx[a], span_of_idx[b])
            && sa != sb
        {
            break_group = true;
        }

        // By default we break at primary boundaries, unless a..b belong to the SAME logical tuplet group
        // (e.g., inside the same triplet of 3 notes). Contiguous same-spec tuplets across a boundary should
        // NOT be merged if they represent two adjacent tuplet groups (e.g., two triplets in 2/4).
        if !break_group && boundary_between && !is_same_tuplet_group(beats, a, b) {
            break_group = true;
        }

        // Exception: Allow carrying the beam from the LAST note of a tuplet span across a primary boundary
        // into a following non‑tuplet note IF the tuplet note extends across the boundary and its end aligns
        // exactly with the onset of the following note, and there are no rests in between. This respects
        // primary grouping while capturing the musical continuity described in the test comment.
        if break_group && boundary_between {
            if let Some(sa) = span_of_idx[a] {
                // a must be the last index of its tuplet span
                if a == spans[sa].end {
                    // b must NOT be a tuplet and must be beamable
                    if span_of_idx[b].is_none() && beam_count(&beats[b].duration) > 0 {
                        // a must cross a primary boundary and end exactly at b's onset
                        let a_end = onsets[a]
                            .saturating_add(measure.grid.ticks_of(&beats[a].duration).unwrap_or(0));
                        let b_on = onsets[b];
                        // Check there exists a boundary strictly between a's onset and its end
                        let crosses_boundary = boundaries
                            .iter()
                            .copied()
                            .any(|bd| bd > onsets[a] && bd < a_end);
                        if crosses_boundary && a_end == b_on && !has_rest_between(beats, a, b) {
                            break_group = false;
                        }
                    }
                }
            }
        }
        // Additionally: even without a primary boundary, never merge ACROSS a boundary between two
        // different tuplet groups. This prevents [1,2,3] beaming when 2 is the last of one triplet
        // and 3 the first of the next triplet within the same primary beat.
        if !break_group {
            let ta = tuplet_spec(&beats[a].duration);
            let tb = tuplet_spec(&beats[b].duration);
            if matches!((ta, tb), (Some(_), Some(_))) && !is_same_tuplet_group(beats, a, b) {
                break_group = true;
            }
        }
        // Check if any non-beamable NOTE exists between a..b
        if !break_group {
            for beat in beats.iter().take(b).skip(a + 1) {
                match beat.kind {
                    BeatKind::Note => {
                        if beat.kind == BeatKind::Note && beam_count(&beat.duration) == 0 {
                            break_group = true;
                            break;
                        }
                    }
                    BeatKind::Rest => {
                        break_group = true;
                        break;
                    }
                }
            }
        }

        if break_group {
            finalize_group(&mut groups, beats, &cur);
            cur = vec![b];
        } else {
            cur.push(b);
        }
    }
    finalize_group(&mut groups, beats, &cur);

    BeamPlan {
        groups: groups
            .into_iter()
            .enumerate()
            .map(|(i, mut g)| {
                g.group_index = i;
                g
            })
            .collect(),
    }
}

fn finalize_group(groups: &mut Vec<BeamGroup>, beats: &[Beat], cur: &[BeatIdx]) {
    if cur.is_empty() {
        return;
    }
    let mut beam_counts: Vec<u8> = Vec::with_capacity(cur.len());
    for &i in cur.iter() {
        beam_counts.push(beam_count(&beats[i].duration));
    }

    let mut continuity: Vec<u8> = Vec::new();
    for w in cur.windows(2) {
        let i = w[0];
        let j = w[1];
        // Determine if there was any content between i and j; if any rest or other item exists, continuity can be reduced
        let min_beams = beam_count(&beats[i].duration).min(beam_count(&beats[j].duration));
        // Broken beams (rests between) -> continuity 0, otherwise full min_beams
        let between_has_rest = has_rest_between(beats, i, j);
        let cont = if between_has_rest { 0 } else { min_beams };
        continuity.push(cont);
    }

    groups.push(BeamGroup {
        group_index: 0, // temporary; will be set by caller after push
        beat_indices: cur.to_vec(),
        beam_counts,
        continuity,
        continues_from_previous: false,
        continues_into_next: false,
    });
}

fn has_rest_between(beats: &[Beat], i: BeatIdx, j: BeatIdx) -> bool {
    if j <= i + 1 {
        return false;
    }
    for beat in beats.iter().take(j).skip(i + 1) {
        if beat.kind == BeatKind::Rest {
            return true;
        }
    }
    false
}

fn tuplet_spec(d: &Duration) -> Option<TupletSpec> {
    match *d {
        Duration::Tuplet(spec) => Some(spec),
        _ => None,
    }
}

/// Returns true if both indices `i` and `j` are notes that belong to the SAME logical tuplet group
/// (same (n, m, base) spec and within the same consecutive chunk of size `n`).
/// Example: For triplet eighths (n=3), indices 0,1,2 are group 0; 3,4,5 are group 1.
fn is_same_tuplet_group(beats: &[Beat], i: BeatIdx, j: BeatIdx) -> bool {
    if j < i {
        return false;
    }
    // Both positions must have identical tuplet specs (same n,m,base)
    let si = tuplet_spec(&beats[i].duration);
    let sj = tuplet_spec(&beats[j].duration);
    let Some(spec) = (match (si, sj) {
        (Some(a), Some(b)) if a == b => Some(a),
        _ => None,
    }) else {
        return false;
    };

    // Find the start of the contiguous same-spec block that contains `i`.
    // Important: contiguity is defined by duration spec, not by kind (Note/Rest),
    // because tuplets can contain rests in their slots.
    let mut start = i;
    while start > 0 {
        if tuplet_spec(&beats[start - 1].duration) == Some(spec) {
            start -= 1;
        } else {
            break;
        }
    }

    // Ensure that the span start..=j is a contiguous run of this same tuplet spec
    for beat in beats.iter().take(j).skip(start) {
        if tuplet_spec(&beat.duration) != Some(spec) {
            return false;
        }
    }

    // Compute zero-based positions within the block
    let pos_i = i - start;
    let pos_j = j - start;

    // Same tuplet group if floor(pos/n) equals
    let n_usize = spec.n as usize;
    (pos_i / n_usize) == (pos_j / n_usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure::duration::{e, s, t8, t16, t32};
    use crate::measure::{Beat, Measure, TimeSignature};

    #[test]
    fn beaming_simple_eighths_group_by_quarters() {
        // 4/4 with eight eighth notes -> 4 groups, each two notes
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        for i in 0..8 {
            m.set_beat(i, Beat::note(e())).unwrap();
        }
        let plan = compute_beam_plan(&m);
        assert_eq!(plan.groups.len(), 4, "expected 4 groups of eighths in 4/4");
        for (gi, g) in plan.groups.iter().enumerate() {
            assert_eq!(g.beat_indices.len(), 2, "group {} should have 2 notes", gi);
            assert_eq!(g.beam_counts, vec![1, 1]);
            assert_eq!(g.continuity, vec![1]);
        }
    }

    #[test]
    fn beaming_rest_breaks_group() {
        // Note 16th, Rest 16th, Note 16th, Rest 16th within the first quarter
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        m.add_beat(Beat::note(s())).unwrap();
        m.add_beat(Beat::rest(s())).unwrap();
        m.add_beat(Beat::note(s())).unwrap();
        m.add_beat(Beat::rest(s())).unwrap();
        let plan = compute_beam_plan(&m);
        assert_eq!(plan.groups.len(), 2, "rests should split into two singleton groups");
        let g0 = &plan.groups[0];
        let g1 = &plan.groups[1];
        assert_eq!(g0.beat_indices, vec![0]);
        assert_eq!(g1.beat_indices, vec![2]);
        assert_eq!(g0.beam_counts, vec![2]);
        assert_eq!(g1.beam_counts, vec![2]);
        assert!(g0.continuity.is_empty());
        assert!(g1.continuity.is_empty());
    }

    #[test]
    fn beaming_tuplet_crosses_primary_boundary_as_single_group() {
        // In 4/4: Eighth rest, then three eighth‑tuplets starting on the offbeat, which can span a quarter boundary.
        // The three tuplet notes must remain in one BeamGroup even if a primary boundary lies between them.
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        m.set_beat(0, Beat::rest(e())).unwrap(); // offset by an eighth
        m.set_beat(1, Beat::note(t8())).unwrap();
        m.set_beat(2, Beat::note(t8())).unwrap();
        m.set_beat(3, Beat::note(t8())).unwrap();
        m.set_beat(4, Beat::note(e())).unwrap();
        let plan = compute_beam_plan(&m);

        // Find the group that contains note index 1 (first tuplet note)
        let tuplet_group =
            plan.groups.iter().find(|g| g.beat_indices.contains(&1)).expect("tuplet group");
        // It must contain the three tuplet notes (indices 1,2,3) contiguously at the start of the group
        assert!(tuplet_group.beat_indices.starts_with(&[1, 2, 3]));
        // Continuity should be full min beams (1) between the tuplet notes since there are no rests between them
        assert!(tuplet_group.beam_counts.len() >= 3);
        assert_eq!(&tuplet_group.beam_counts[0..3], &[1, 1, 1]);
        assert!(tuplet_group.continuity.len() >= 2);
        assert_eq!(&tuplet_group.continuity[0..2], &[1, 1]);
    }

    #[test]
    fn last_note_of_tuplet_crosses_primary_boundary() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        m.set_beat(0, Beat::note(s())).unwrap(); // offset by a sixteenth
        m.set_beat(1, Beat::note(t8())).unwrap();
        m.set_beat(2, Beat::note(t8())).unwrap();
        m.set_beat(3, Beat::note(t8())).unwrap();
        m.set_beat(4, Beat::note(e())).unwrap();
        m.set_beat(5, Beat::rest(e())).unwrap();
        let plan = compute_beam_plan(&m);

        // The last beat (t8 @ 3) of the tuplet group crosses the primary boundary,
        // because its absolute location+duration 1.917 + 0.333 = 2.25.
        // The first primary boundary for 4/4 measures is at 2.0. But the last t8's duration does
        // not 'stop' before 2.0. In this particular case, we expect the beam to be connected with
        // the simple 1/8 note at index 4 (absolute position 2.25), too.
        assert_eq!(plan.groups[0].beat_indices, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn beaming_by_2_3_2_pattern_in_7_8() {
        let mut m = Measure::new(TimeSignature::SEVEN_EIGHT);
        for i in 0..7 {
            m.set_beat(i, Beat::note(e())).unwrap();
        }

        let plan = compute_beam_plan(&m);
        assert_eq!(plan.groups.len(), 3);

        let g0 = &plan.groups[0];
        let g1 = &plan.groups[1];
        let g2 = &plan.groups[2];

        assert_eq!(g0.beat_indices, vec![0, 1]);
        assert_eq!(g0.beam_counts, vec![1, 1]);
        assert_eq!(g0.continuity, vec![1]);

        assert_eq!(g1.beat_indices, vec![2, 3, 4]);
        assert_eq!(g1.beam_counts, vec![1, 1, 1]);
        assert_eq!(g1.continuity, vec![1, 1]);

        assert_eq!(g2.beat_indices, vec![5, 6]);
        assert_eq!(g2.beam_counts, vec![1, 1]);
        assert_eq!(g2.continuity, vec![1]);
    }

    #[test]
    fn beaming_with_eighth_note_tuplet_precedence_in_seven_eight() {
        // Expected primary beaming by 2+3+2 with tuplet precedence -> groups:
        // [0,1,2], [3,4,5], [6,7]
        let mut m = Measure::new(TimeSignature::SEVEN_EIGHT);
        m.set_beat(0, Beat::note(e())).unwrap();
        m.set_beat(1, Beat::note(e())).unwrap();
        m.set_beat(2, Beat::note(e())).unwrap();
        m.set_beat(3, Beat::note(t8())).unwrap();
        m.set_beat(4, Beat::note(t8())).unwrap();
        m.set_beat(5, Beat::note(t8())).unwrap();
        m.set_beat(6, Beat::note(e())).unwrap();
        m.set_beat(7, Beat::note(e())).unwrap();

        let plan = compute_beam_plan(&m);
        assert_eq!(plan.groups.len(), 3);

        let g0 = &plan.groups[0];
        let g1 = &plan.groups[1];
        let g2 = &plan.groups[2];

        assert_eq!(g0.beat_indices, vec![0, 1]);
        assert_eq!(g0.beam_counts, vec![1, 1]);
        assert_eq!(g0.continuity, vec![1]);

        assert_eq!(g1.beat_indices, vec![2, 3, 4, 5]);
        assert_eq!(g1.beam_counts, vec![1, 1, 1, 1]);
        assert_eq!(g1.continuity, vec![1, 1, 1]);

        assert_eq!(g2.beat_indices, vec![6, 7]);
        assert_eq!(g2.beam_counts, vec![1, 1]);
        assert_eq!(g2.continuity, vec![1]);
    }

    #[test]
    fn beaming_with_sixteenth_note_tuplet_precedence_in_seven_eight() {
        // The eighth notes are expected to be part of the first beam group, since the measure still
        // fits the 2+3+2 pattern, despite the 16th-note triplet at the beginning.
        let mut m = Measure::new(TimeSignature::SEVEN_EIGHT);
        m.set_beat(0, Beat::note(t16())).unwrap();
        m.set_beat(1, Beat::note(t16())).unwrap();
        m.set_beat(2, Beat::note(t16())).unwrap();
        m.set_beat(3, Beat::note(e())).unwrap();
        m.set_beat(4, Beat::note(e())).unwrap();
        m.set_beat(5, Beat::note(e())).unwrap();
        m.set_beat(6, Beat::note(e())).unwrap();
        m.set_beat(7, Beat::note(e())).unwrap();
        m.set_beat(8, Beat::note(e())).unwrap();

        let plan = compute_beam_plan(&m);
        assert_eq!(plan.groups.len(), 3);

        let g0 = &plan.groups[0];
        let g1 = &plan.groups[1];
        let g2 = &plan.groups[2];

        assert_eq!(g0.beat_indices, vec![0, 1, 2, 3]);
        assert_eq!(g0.beam_counts, vec![2, 2, 2, 1]);

        assert_eq!(g1.beat_indices, vec![4, 5, 6]);
        assert_eq!(g1.beam_counts, vec![1, 1, 1]);
        assert_eq!(g1.continuity, vec![1, 1]);

        assert_eq!(g2.beat_indices, vec![7, 8]);
        assert_eq!(g2.beam_counts, vec![1, 1]);
        assert_eq!(g2.continuity, vec![1]);
    }

    // #[test]
    // fn beaming_with_t32_triplet_with_merged_t16() {
    //     let mut m = Measure::new(TimeSignature::ONE_FOUR);
    //     m.add_beat(Beat::note(t32())).unwrap();
    //     m.add_beat(Beat::note(t32())).unwrap();
    //     m.add_beat(Beat::note(t32())).unwrap();
    //     m.set_beat(0, Beat::note(t16())).unwrap();
    //
    //     let plan = compute_beam_plan(&m);
    //     assert_eq!(plan.groups[0].beat_indices, vec![0, 1]);
    //     assert_eq!(plan.groups[0].beam_counts, vec![2, 3]);
    //     assert_eq!(plan.groups[0].continuity, vec![2]);
    //     assert!(!plan.groups[0].continues_from_previous);
    //     assert!(!plan.groups[0].continues_into_next);
    // }

    #[test]
    fn beaming_two_consecutive_eighth_tuplets_not_joined() {
        // 2/4 with two consecutive 1/8 tuplet groups (triplet eighths).
        // Expectation: No beam between note-idx 2 and 3; i.e., the two tuplet groups are not connected.
        let mut m = Measure::new(TimeSignature::TWO_FOUR);
        // First tuplet group (fills one quarter)
        m.set_beat(0, Beat::note(t8())).unwrap();
        m.set_beat(1, Beat::note(t8())).unwrap();
        m.set_beat(2, Beat::note(t8())).unwrap();
        // Second tuplet group (fills the second quarter)
        m.set_beat(3, Beat::note(t8())).unwrap();
        m.set_beat(4, Beat::note(t8())).unwrap();
        m.set_beat(5, Beat::note(t8())).unwrap();

        let plan = compute_beam_plan(&m);

        // We expect two separate groups: [0,1,2] and [3,4,5]
        assert_eq!(plan.groups.len(), 2, "zwei tuplet-gruppen sollten nicht verbunden werden");

        let g0 = &plan.groups[0];
        let g1 = &plan.groups[1];

        assert_eq!(g0.beat_indices, vec![0, 1, 2]);
        assert_eq!(g1.beat_indices, vec![3, 4, 5]);

        // Each tuplet note is based on eighth -> 1 beam, continuity inside each group is [1,1]
        assert_eq!(g0.beam_counts, vec![1, 1, 1]);
        assert_eq!(g1.beam_counts, vec![1, 1, 1]);
        assert_eq!(g0.continuity, vec![1, 1]);
        assert_eq!(g1.continuity, vec![1, 1]);

        // Explicitly ensure there is no beam between indices 2 and 3 by virtue of the group split
        assert_eq!(g0.beat_indices.last(), Some(&2));
        assert_eq!(g1.beat_indices.first(), Some(&3));
    }

    #[test]
    fn beaming_two_consecutive_eighth_tuplets_not_joined_when_first_tuplet_contains_rest() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Triplet 1
        m.set_beat(0, Beat::rest(t8())).unwrap();
        m.set_beat(1, Beat::note(t8())).unwrap();
        m.set_beat(2, Beat::note(t8())).unwrap();

        // Next note is not a triplet
        m.set_beat(3, Beat::note(e())).unwrap();

        let mut plan = compute_beam_plan(&m);
        assert_eq!(plan.groups[0].beat_indices, vec![1, 2]);

        // Now change the note after the 1st triplet to a triplet
        m.set_beat(3, Beat::note(t8())).unwrap();
        plan = compute_beam_plan(&m);
        assert_eq!(plan.groups[0].beat_indices, vec![1, 2]);
    }

    #[test]
    fn beaming_two_consecutive_eighth_tuplets_not_joined_when_first_tuplet_is_subdivided() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Create two triplet groups
        m.set_beat(0, Beat::rest(t8())).unwrap();
        m.set_beat(1, Beat::note(t8())).unwrap();
        m.set_beat(2, Beat::note(t8())).unwrap();
        m.set_beat(3, Beat::note(t8())).unwrap();

        let mut plan = compute_beam_plan(&m);
        assert_eq!(plan.groups[0].beat_indices, vec![1, 2]);

        // Subdivide the first note of the first triplet into two notes
        m.set_beat(0, Beat::rest(t16())).unwrap();

        plan = compute_beam_plan(&m);
        // We expect the last two 8th notes of the first triplet not to be joined with the next triplet
        assert_eq!(plan.groups[0].beat_indices, vec![2, 3]);
    }
}
