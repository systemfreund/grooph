use crate::tuplet_plan::{TupletSpan, detect_tuplet_spans};
use grooph_measure::duration::{Duration, NoteValue, e, q};
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::{Beat, BeatIdx, TimeSignature, VoiceBar};

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
/// - Rests between two notes of a group sit under its beams; a rest too long
///   for a beam (a quarter or longer) splits the group
/// - In 2/4 and 4/4 the eighths of a half bar are beamed in fours when the
///   half holds nothing shorter than an eighth and no tuplet (a group of
///   eighths reaches up to a half); never over the middle of a 4/4 bar
/// - The secondary beams break two levels above the shortest value of a
///   group, on the levels of its beat (two-level rhythmic parsing): with
///   thirty-seconds only the eighth beam runs over the eighths of a quarter
///   or a dotted quarter, while a group of three sixteenths runs through
/// - Cross-measure beams are exposed via the `continues_*` flags but left as false here; a higher level
///   can link adjacent measures and set these appropriately.
#[cfg(test)]
pub(super) fn compute_beam_plan(measure: &VoiceBar) -> BeamPlan {
    compute_beam_plan_with(measure, false)
}

/// [`compute_beam_plan`], with `beam_rests` beaming rests like notes of their
/// length, for stemlets: the beams of a group then run over its rests, also
/// in a group of rests alone.
pub(super) fn compute_beam_plan_with(measure: &VoiceBar, beam_rests: bool) -> BeamPlan {
    let beats = measure.beats();
    let ts = measure.time_signature();
    let onsets = DEFAULT_GRID.compute_onset_ticks(beats);

    // Compute primary boundaries (tick positions inside the measure where groups should break by default)
    let boundaries = DEFAULT_GRID.primary_boundaries(&ts);
    let measure_ticks = DEFAULT_GRID.ticks_per_measure(&ts);
    let boundaries = join_eighth_halves(beats, &onsets, &ts, boundaries);

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
        if (beam_rests || b.is_note()) && beam_count(&b.duration) > 0 {
            note_idxs.push(i);
        }
    }

    let mut groups: Vec<BeamGroup> = Vec::new();
    if note_idxs.is_empty() {
        return BeamPlan { groups };
    }

    // Build groups based on a single decision function `should_break`
    let mut cur: Vec<BeatIdx> = vec![note_idxs[0]];
    for w in note_idxs.windows(2) {
        let a = w[0];
        let b = w[1];
        let break_group = should_break(a, b, beats, &onsets, &boundaries, &span_of_idx, &spans);

        if break_group {
            finalize_group(&mut groups, beats, &onsets, &boundaries, measure_ticks, &cur);
            cur = vec![b];
        } else {
            cur.push(b);
        }
    }
    finalize_group(&mut groups, beats, &onsets, &boundaries, measure_ticks, &cur);

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

/// Drop the beat between the two quarters of a half bar of 2/4 or 4/4 from
/// `boundaries` when the half holds only eighths or longer values and no
/// tuplet, and notes meet on that beat: its eighths are beamed in fours. A
/// rest on either side of the beat keeps it, so no beam hides the beat
/// over a rest.
fn join_eighth_halves(
    beats: &[Beat],
    onsets: &[u32],
    ts: &TimeSignature,
    boundaries: Vec<u32>,
) -> Vec<u32> {
    if ts.beat_unit != 4 || !matches!(ts.beats, 2 | 4) {
        return boundaries;
    }
    let quarter = DEFAULT_GRID.ticks_of(&q()).unwrap_or(0);
    let eighth = DEFAULT_GRID.ticks_of(&e()).unwrap_or(0);
    let in_eighths = |start: u32, end: u32| {
        beats.iter().zip(onsets).all(|(b, &on)| {
            let len = DEFAULT_GRID.ticks_of(&b.duration).unwrap_or(0);
            let outside = on + len <= start || on >= end;
            outside || (!b.duration.is_tuplet() && len >= eighth)
        })
    };
    let notes_meet = |bd: u32| {
        onsets
            .iter()
            .position(|&on| on == bd)
            .is_some_and(|k| k > 0 && beats[k].is_note() && beats[k - 1].is_note())
    };
    boundaries
        .into_iter()
        .filter(|&bd| {
            // The second beat of a half: 2 and 4 in 4/4, 2 in 2/4.
            let middle_of_half = (bd / quarter) % 2 == 1;
            !(middle_of_half && in_eighths(bd - quarter, bd + quarter) && notes_meet(bd))
        })
        .collect()
}

/// Where the secondary beams of a group break inside a beat of `beat_len`
/// ticks whose shortest value has `shortest` beams (two-level rhythmic
/// parsing): `(unit, beams)` when they break at every `unit` ticks from the
/// beat start and only `beams` beams run on there, `None` when the beams run
/// through the beat.
///
/// The levels of a beat go down by halves, except that a dotted beat (three
/// parts, as in 6/8 or a group of three sixteenths) first divides in three.
/// Notes may be grouped up to two levels above the shortest value: a quarter
/// with thirty-seconds breaks at its eighths, a group of three sixteenths
/// with thirty-seconds runs through.
fn break_level(beat_len: u32, shortest: u8) -> Option<(u32, u8)> {
    let eighth = DEFAULT_GRID.ticks_of(&e())?;
    if shortest == 0 || beat_len == 0 {
        return None;
    }
    let shortest_len = eighth >> (shortest - 1);
    // A plain (undotted) note value: a power of two times the shortest.
    let plain = |t: u32| t.is_multiple_of(shortest_len) && (t / shortest_len).is_power_of_two();
    let mut levels = vec![beat_len];
    while let Some(&last) = levels.last() {
        let next = if last.is_multiple_of(3) && plain(last / 3) { last / 3 } else { last / 2 };
        if last <= shortest_len || !plain(next) {
            break;
        }
        levels.push(next);
    }
    let unit = levels[levels.len().saturating_sub(3)];
    if unit >= beat_len {
        return None;
    }
    // Beams of a value of `unit` ticks: an eighth (dotted or not) has one.
    let beams = (1..=shortest).find(|&b| eighth >> (b - 1) <= unit).unwrap_or(shortest);
    Some((unit, beams))
}

fn finalize_group(
    groups: &mut Vec<BeamGroup>,
    beats: &[Beat],
    onsets: &[u32],
    boundaries: &[u32],
    measure_ticks: u32,
    cur: &[BeatIdx],
) {
    if cur.len() <= 1 {
        // No beamable notes in this group
        return;
    }
    let mut beam_counts: Vec<u8> = Vec::with_capacity(cur.len());
    for &i in cur.iter() {
        beam_counts.push(beam_count(&beats[i].duration));
    }

    // Secondary beams break two levels above the shortest value of the
    // group, on the levels of its beat (see `break_level`).
    let shortest = beats[cur[0]..=cur[cur.len() - 1]]
        .iter()
        .map(|b| beam_count(&b.duration))
        .max()
        .unwrap_or(0);
    let beat_of = |t: u32| {
        let start = boundaries.iter().copied().filter(|&bd| bd <= t).max().unwrap_or(0);
        let end = boundaries.iter().copied().filter(|&bd| bd > t).min().unwrap_or(measure_ticks);
        (start, end - start)
    };
    let breaks_at = |t: u32| {
        let (start, len) = beat_of(t);
        break_level(len, shortest)
            .filter(|&(unit, _)| (t - start).is_multiple_of(unit))
            .map(|(_, beams)| beams)
    };

    let mut continuity: Vec<u8> = Vec::new();
    for w in cur.windows(2) {
        let i = w[0];
        let j = w[1];
        // The beams run on over the rests between, as many as all of them share.
        let shared = beats[i..=j].iter().map(|b| beam_count(&b.duration)).min().unwrap_or(0);
        // The note or rest after `i` starts the next part of its beat.
        let cont = breaks_at(onsets[i + 1]).map_or(shared, |beams| shared.min(beams));
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

/// Whether a beat between two beamable notes is too long for a beam, a note
/// or a rest: it splits their group.
fn has_unbeamable_between(beats: &[Beat], i: BeatIdx, j: BeatIdx) -> bool {
    beats.iter().take(j).skip(i + 1).any(|b| beam_count(&b.duration) == 0)
}

fn is_primary_boundary_between(a_on: u32, b_on: u32, boundaries: &[u32]) -> bool {
    boundaries.iter().any(|&bd| bd > a_on && bd <= b_on)
}

fn tuplet_last_note_crosses_into_non_tuplet(
    a: BeatIdx,
    b: BeatIdx,
    beats: &[Beat],
    onsets: &[u32],
    boundaries: &[u32],
    span_of_idx: &[Option<usize>],
    spans: &[TupletSpan],
) -> bool {
    let Some(sa) = span_of_idx[a] else {
        return false;
    };
    // a must be the last index in its tuplet span
    if a != spans[sa].end {
        return false;
    }
    // b must not be a tuplet and must be beamable
    if span_of_idx[b].is_some() || beam_count(&beats[b].duration) == 0 {
        return false;
    }

    let a_end = onsets[a].saturating_add(DEFAULT_GRID.ticks_of(&beats[a].duration).unwrap_or(0));
    let b_on = onsets[b];
    let crosses_boundary = boundaries.iter().copied().any(|bd| bd > onsets[a] && bd < a_end);
    crosses_boundary && a_end == b_on
}

fn should_break(
    a: BeatIdx,
    b: BeatIdx,
    beats: &[Beat],
    onsets: &[u32],
    boundaries: &[u32],
    span_of_idx: &[Option<usize>],
    spans: &[TupletSpan],
) -> bool {
    // 1) Hard splits first
    if has_unbeamable_between(beats, a, b) {
        return true;
    }
    // Never merge across two different TupletSpans
    match (span_of_idx[a], span_of_idx[b]) {
        (Some(sa), Some(sb)) if sa != sb => return true,
        _ => {}
    }

    let boundary_between = is_primary_boundary_between(onsets[a], onsets[b], boundaries);

    if boundary_between {
        // Exception 1: both in same TupletSpan → keep together
        if let (Some(sa), Some(sb)) = (span_of_idx[a], span_of_idx[b])
            && sa == sb
        {
            return false;
        }
        // Exception 2: last tuplet note crosses into non‑tuplet that starts exactly at its end
        if tuplet_last_note_crosses_into_non_tuplet(
            a,
            b,
            beats,
            onsets,
            boundaries,
            span_of_idx,
            spans,
        ) {
            return false;
        }
        return true;
    }

    // No primary boundary and no hard split → keep together
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use grooph_measure::duration::{e, s, t8, t16, t32};
    use grooph_measure::{Beat, Measure, TimeSignature};

    #[test]
    fn beaming_simple_eighths_group_by_half_bars() {
        // 4/4 with eight eighth notes -> 2 groups of four, one per half bar
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        for i in 0..8 {
            m.first_voice_mut().set_beat(i, Beat::note(e())).unwrap();
        }
        let plan = compute_beam_plan(m.first_voice());
        assert_eq!(plan.groups.len(), 2, "expected 2 groups of eighths in 4/4");
        for (gi, g) in plan.groups.iter().enumerate() {
            assert_eq!(g.beat_indices.len(), 4, "group {} should have 4 notes", gi);
            assert_eq!(g.beam_counts, vec![1, 1, 1, 1]);
            assert_eq!(g.continuity, vec![1, 1, 1]);
        }
    }

    #[test]
    fn eighths_join_in_fours_only_inside_a_half_of_eighths() {
        use grooph_measure::notation::parse_score;
        let groups_of = |s: &str| -> Vec<Vec<usize>> {
            let plan = compute_beam_plan(parse_score(s).unwrap().measures.remove(0).first_voice());
            plan.groups.into_iter().map(|g| g.beat_indices).collect()
        };
        // 2/4: the whole bar is one half.
        assert_eq!(groups_of("2/4 e e e e"), [vec![0, 1, 2, 3]]);
        // Five eighths and a quarter on the "and" of 3: four and a flag.
        assert_eq!(groups_of("4/4 e e e e e q e"), [vec![0, 1, 2, 3]]);
        // Never over the middle of the bar.
        assert_eq!(groups_of("4/4 q e e e e q"), [vec![1, 2], vec![3, 4]]);
        // A half with sixteenths stays in beats, the other half joins.
        assert_eq!(
            groups_of("4/4 e e s s e e e e e"),
            [vec![0, 1], vec![2, 3, 4], vec![5, 6, 7, 8]]
        );
        // Triplets keep their beats.
        assert_eq!(groups_of("2/4 e e t8 t8 t8"), [vec![0, 1], vec![2, 3, 4]]);
        // A rest on either side of the beat keeps it.
        assert_eq!(groups_of("2/4 e r:e e e"), [vec![2, 3]]);
        assert_eq!(groups_of("2/4 e e r:e e"), [vec![0, 1]]);
        // Rests at the ends of a half stay out of its beam.
        assert_eq!(groups_of("2/4 e e e r:e"), [vec![0, 1, 2]]);
        assert_eq!(groups_of("2/4 r:e e e e"), [vec![1, 2, 3]]);
        // Other meters keep their beats.
        assert_eq!(groups_of("3/4 e e e e e e").len(), 3);
        assert_eq!(groups_of("6/8 e e e e e e").len(), 2);
        assert_eq!(groups_of("5/4 e e e e e e e e e e").len(), 5);
    }

    #[test]
    fn rests_between_notes_sit_under_the_beam() {
        use grooph_measure::notation::parse_score;
        let plan_of =
            |s: &str| compute_beam_plan(parse_score(s).unwrap().measures.remove(0).first_voice());
        // The beams run on over the rests between two notes.
        let plan = plan_of("1/4 s r:s r:s s");
        assert_eq!(plan.groups.len(), 1);
        assert_eq!(plan.groups[0].beat_indices, vec![0, 3]);
        assert_eq!(plan.groups[0].continuity, vec![2]);
        // As many as the notes and the rests between share.
        let plan = plan_of("1/4 s r:e s");
        assert_eq!(plan.groups[0].beat_indices, vec![0, 2]);
        assert_eq!(plan.groups[0].continuity, vec![1]);
        // Rests at either end stay out of the beam.
        let plan = plan_of("1/4 r:s s s r:s");
        assert_eq!(plan.groups[0].beat_indices, vec![1, 2]);
        // A note with only rests beside it keeps its flag.
        assert!(plan_of("1/4 s r:s r:s r:s").groups.is_empty());
        // Nor do the beams run over a rest across a beat.
        let plan = plan_of("2/4 e r:e r:e e");
        assert!(plan.groups.is_empty());
    }

    #[test]
    fn beamed_rests_carry_the_beams_of_their_beat() {
        use grooph_measure::notation::parse_score;
        let m = parse_score("2/4 s r:s r:s r:s r:s r:s r:s r:s").unwrap().measures.remove(0);
        assert!(compute_beam_plan(m.first_voice()).groups.is_empty());
        // Each beat's rests run on under its beams, also without a note.
        let plan = compute_beam_plan_with(m.first_voice(), true);
        assert_eq!(plan.groups.len(), 2);
        assert_eq!(plan.groups[0].beat_indices, vec![0, 1, 2, 3]);
        assert_eq!(plan.groups[1].beat_indices, vec![4, 5, 6, 7]);
        assert_eq!(plan.groups[1].continuity, vec![2, 2, 2]);
    }

    #[test]
    fn thirty_seconds_break_the_secondary_beams_at_each_eighth() {
        use grooph_measure::notation::parse_score;
        let plan_of =
            |s: &str| compute_beam_plan(parse_score(s).unwrap().measures.remove(0).first_voice());
        // Eight thirty-seconds: 4 + 4 under one eighth beam.
        let plan = plan_of("1/4 th th th th th th th th");
        assert_eq!(plan.groups.len(), 1);
        assert_eq!(plan.groups[0].continuity, vec![3, 3, 3, 1, 3, 3, 3]);
        // Sixteenths beside thirty-seconds break at the eighth too.
        let plan = plan_of("1/4 s s th th th th");
        assert_eq!(plan.groups[0].continuity, vec![2, 1, 3, 3, 3]);
        // Over a rest on the eighth as well.
        let plan = plan_of("1/4 th th th th r:th th th th");
        assert_eq!(plan.groups[0].continuity, vec![3, 3, 3, 1, 3, 3]);
        // Without thirty-seconds the sixteenths keep both beams.
        assert_eq!(plan_of("1/4 s s s s").groups[0].continuity, vec![2, 2, 2]);
        // Eighths count from each beat, also in a dotted-quarter group.
        let plan = plan_of("6/8 th th th th s s e r:q.");
        assert_eq!(plan.groups[0].continuity, vec![3, 3, 3, 1, 2, 1]);
        // Thirty-second triplets keep their beams inside an eighth.
        let plan = plan_of("1/4 t32 t32 t32 s e");
        assert_eq!(plan.groups[0].continuity, vec![3, 3, 2, 1]);
    }

    #[test]
    fn thirty_seconds_run_through_a_group_of_three_sixteenths() {
        use grooph_measure::notation::parse_score;
        let plan_of =
            |s: &str| compute_beam_plan(parse_score(s).unwrap().measures.remove(0).first_voice());
        // Two levels above the thirty-second in a dotted-eighth beat is the
        // whole beat: six thirty-seconds under three beams, not 4 + 2.
        let plan = plan_of("6/16 th th th th th th th th th th th th");
        let continuity: Vec<_> = plan.groups.iter().map(|g| g.continuity.clone()).collect();
        assert_eq!(continuity, vec![vec![3; 5], vec![3; 5]]);
        // 8/16 as 3 + 2 + 3: every group runs through.
        let plan = plan_of("3+2+3/16 th th th th th th th th th th th th th th th th");
        let continuity: Vec<_> = plan.groups.iter().map(|g| g.continuity.clone()).collect();
        assert_eq!(continuity, vec![vec![3; 5], vec![3; 3], vec![3; 5]]);
        // A group of three eighths with thirty-seconds still breaks at its eighths.
        let plan = plan_of("3/8 th th th th th th th th e");
        assert_eq!(plan.groups[0].continuity, vec![3, 3, 3, 1, 3, 3, 3, 1]);
    }

    #[test]
    fn beaming_tuplet_crosses_primary_boundary_as_single_group() {
        // In 4/4: Eighth rest, then three eighth‑tuplets starting on the offbeat, which can span a quarter boundary.
        // The three tuplet notes must remain in one BeamGroup even if a primary boundary lies between them.
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        m.first_voice_mut().set_beat(0, Beat::rest(e())).unwrap(); // offset by an eighth
        m.first_voice_mut().set_beat(1, Beat::note(t8())).unwrap();
        m.first_voice_mut().set_beat(2, Beat::note(t8())).unwrap();
        m.first_voice_mut().set_beat(3, Beat::note(t8())).unwrap();
        m.first_voice_mut().set_beat(4, Beat::note(e())).unwrap();
        let plan = compute_beam_plan(m.first_voice());

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
        m.first_voice_mut().set_beat(0, Beat::note(s())).unwrap(); // offset by a sixteenth
        m.first_voice_mut().set_beat(1, Beat::note(t8())).unwrap();
        m.first_voice_mut().set_beat(2, Beat::note(t8())).unwrap();
        m.first_voice_mut().set_beat(3, Beat::note(t8())).unwrap();
        m.first_voice_mut().set_beat(4, Beat::note(e())).unwrap();
        m.first_voice_mut().set_beat(5, Beat::rest(e())).unwrap();
        let plan = compute_beam_plan(m.first_voice());

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
            m.first_voice_mut().set_beat(i, Beat::note(e())).unwrap();
        }

        let plan = compute_beam_plan(m.first_voice());
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
    fn beaming_follows_the_picked_grouping_in_7_8() {
        let mut m = Measure::new(TimeSignature::SEVEN_EIGHT.with_grouping(1));
        for i in 0..7 {
            m.first_voice_mut().set_beat(i, Beat::note(e())).unwrap();
        }
        let plan = compute_beam_plan(m.first_voice());
        let groups: Vec<_> = plan.groups.iter().map(|g| g.beat_indices.clone()).collect();
        assert_eq!(groups, [vec![0, 1], vec![2, 3], vec![4, 5, 6]]);
    }

    #[test]
    fn beaming_with_eighth_note_tuplet_precedence_in_seven_eight() {
        // Expected primary beaming by 2+3+2 with tuplet precedence -> groups:
        // [0,1,2], [3,4,5], [6,7]
        let mut m = Measure::new(TimeSignature::SEVEN_EIGHT);
        m.first_voice_mut().set_beat(0, Beat::note(e())).unwrap();
        m.first_voice_mut().set_beat(1, Beat::note(e())).unwrap();
        m.first_voice_mut().set_beat(2, Beat::note(e())).unwrap();
        m.first_voice_mut().set_beat(3, Beat::note(t8())).unwrap();
        m.first_voice_mut().set_beat(4, Beat::note(t8())).unwrap();
        m.first_voice_mut().set_beat(5, Beat::note(t8())).unwrap();
        m.first_voice_mut().set_beat(6, Beat::note(e())).unwrap();
        m.first_voice_mut().set_beat(7, Beat::note(e())).unwrap();

        let plan = compute_beam_plan(m.first_voice());
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
        m.first_voice_mut().set_beat(0, Beat::note(t16())).unwrap();
        m.first_voice_mut().set_beat(1, Beat::note(t16())).unwrap();
        m.first_voice_mut().set_beat(2, Beat::note(t16())).unwrap();
        m.first_voice_mut().set_beat(3, Beat::note(e())).unwrap();
        m.first_voice_mut().set_beat(4, Beat::note(e())).unwrap();
        m.first_voice_mut().set_beat(5, Beat::note(e())).unwrap();
        m.first_voice_mut().set_beat(6, Beat::note(e())).unwrap();
        m.first_voice_mut().set_beat(7, Beat::note(e())).unwrap();
        m.first_voice_mut().set_beat(8, Beat::note(e())).unwrap();

        let plan = compute_beam_plan(m.first_voice());
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

    #[test]
    fn beaming_with_t8_triplet_where_all_beats_are_subdivided_in_t16() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        // First, create a t8 tuplet group
        m.first_voice_mut().set_beat(0, Beat::note(t8())).unwrap();
        // Subdivide all 8 eighths into 16th notes
        for i in 0..=5 {
            m.first_voice_mut().set_beat(i, Beat::note(t16())).unwrap();
        }

        let plan = compute_beam_plan(m.first_voice());
        assert_eq!(plan.groups[0].beat_indices, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn beaming_with_t32_triplet_with_merged_t16() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        m.first_voice_mut().set_beat(0, Beat::note(t32())).unwrap();
        m.first_voice_mut().set_beat(1, Beat::note(t32())).unwrap();
        m.first_voice_mut().set_beat(2, Beat::note(t32())).unwrap();
        m.first_voice_mut().set_beat(0, Beat::note(t16())).unwrap();

        let plan = compute_beam_plan(m.first_voice());
        assert_eq!(plan.groups[0].beat_indices, vec![0, 1]);
        assert_eq!(plan.groups[0].beam_counts, vec![2, 3]);
        assert_eq!(plan.groups[0].continuity, vec![2]);
        assert!(!plan.groups[0].continues_from_previous);
        assert!(!plan.groups[0].continues_into_next);
    }

    #[test]
    fn beaming_two_consecutive_eighth_tuplets_not_joined() {
        // 2/4 with two consecutive 1/8 tuplet groups (triplet eighths).
        // Expectation: No beam between note-idx 2 and 3; i.e., the two tuplet groups are not connected.
        let mut m = Measure::new(TimeSignature::TWO_FOUR);
        // First tuplet group (fills one quarter)
        m.first_voice_mut().set_beat(0, Beat::note(t8())).unwrap();
        m.first_voice_mut().set_beat(1, Beat::note(t8())).unwrap();
        m.first_voice_mut().set_beat(2, Beat::note(t8())).unwrap();
        // Second tuplet group (fills the second quarter)
        m.first_voice_mut().set_beat(3, Beat::note(t8())).unwrap();
        m.first_voice_mut().set_beat(4, Beat::note(t8())).unwrap();
        m.first_voice_mut().set_beat(5, Beat::note(t8())).unwrap();

        let plan = compute_beam_plan(m.first_voice());

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
        m.first_voice_mut().set_beat(0, Beat::rest(t8())).unwrap();
        m.first_voice_mut().set_beat(1, Beat::note(t8())).unwrap();
        m.first_voice_mut().set_beat(2, Beat::note(t8())).unwrap();

        // The next note is not a triplet
        m.first_voice_mut().set_beat(3, Beat::note(e())).unwrap();

        let mut plan = compute_beam_plan(m.first_voice());
        assert_eq!(plan.groups[0].beat_indices, vec![1, 2]);

        // Now change the note after the 1st triplet to a triplet
        m.first_voice_mut().set_beat(3, Beat::note(t8())).unwrap();
        plan = compute_beam_plan(m.first_voice());
        assert_eq!(plan.groups[0].beat_indices, vec![1, 2]);
    }

    #[test]
    fn beaming_two_consecutive_eighth_tuplets_not_joined_when_first_tuplet_is_subdivided() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Create two triplet groups
        m.first_voice_mut().set_beat(0, Beat::rest(t8())).unwrap();
        m.first_voice_mut().set_beat(1, Beat::note(t8())).unwrap();
        m.first_voice_mut().set_beat(2, Beat::note(t8())).unwrap();
        m.first_voice_mut().set_beat(3, Beat::note(t8())).unwrap();

        let mut plan = compute_beam_plan(m.first_voice());
        assert_eq!(plan.groups[0].beat_indices, vec![1, 2]);

        // Subdivide the first note of the first triplet into two notes
        m.first_voice_mut().set_beat(0, Beat::rest(t16())).unwrap();

        plan = compute_beam_plan(m.first_voice());
        // We expect the last two 8th notes of the first triplet not to be joined with the next triplet
        assert_eq!(plan.groups[0].beat_indices, vec![2, 3]);
    }

    #[test]
    fn fully_beamed_when_t8_triplet_is_completely_subdivided_into_t16() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        m.first_voice_mut().set_beat(0, Beat::rest(e())).unwrap(); // offset by an eigth
        m.first_voice_mut().set_beat(1, Beat::note(t8())).unwrap();
        for i in 1..=6 {
            m.first_voice_mut().set_beat(i, Beat::note(t16())).unwrap();
        }

        let plan = compute_beam_plan(m.first_voice());
        assert_eq!(plan.groups[0].beat_indices, vec![1, 2, 3, 4, 5, 6]);
    }
}
