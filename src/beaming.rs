use crate::duration::{Duration, NoteValue, default_duration_set};
use crate::measure::{Beat, BeatKind, Measure, TimeSignature};

/// Number of beams implied by a duration (eighth = 1, sixteenth = 2, 32nd = 3).
/// Tuplets map to their base note value for beam count purposes.
pub fn beam_count(d: &Duration) -> u8 {
    match d.base_note() {
        NoteValue::Whole | NoteValue::Half | NoteValue::Quarter => 0,
        NoteValue::Eighth => 1,
        NoteValue::Sixteenth => 2,
        NoteValue::ThirtySecond => 3,
    }
}

#[derive(Debug, Clone)]
pub struct BeamPlan {
    pub groups: Vec<BeamGroup>,
}

/// Link cross-measure beams between two adjacent measures by setting continuation flags on the
/// outermost beam groups if both sides have at least one group. This is a simple default rule:
/// if the previous measure ends with a beamable group and the next measure starts with a
/// beamable group, we mark continuation across the barline.
pub fn link_cross_measure_beams(prev: &Measure, next: &Measure, prev_plan: &mut BeamPlan, next_plan: &mut BeamPlan) {
    if prev_plan.groups.is_empty() || next_plan.groups.is_empty() { return; }
    let last_idx = prev_plan.groups.len() - 1;
    let first_idx = 0;
    // Since groups only contain beamable notes, it's safe to mark continuation directly.
    prev_plan.groups[last_idx].continues_into_next = true;
    next_plan.groups[first_idx].continues_from_previous = true;
}

#[derive(Debug, Clone)]
pub struct BeamGroup {
    /// Stable id within the measure for selection
    pub group_index: usize,
    /// Indices into Measure.beats() of notes participating in the group (rests are not listed)
    pub note_indices: Vec<usize>,
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
/// - Tuplets (if contiguous) are treated like normal beamable notes (future: with explicit group id)
/// - Rests inside a group do not break the group; continuity across a rest is 0 (broken beam/hooks)
/// - Cross-measure beams are exposed via the `continues_*` flags but left as false here; a higher level
///   can link adjacent measures and set these appropriately.
pub fn compute_beam_plan(measure: &Measure) -> BeamPlan {
    let set = default_duration_set();
    let beats = measure.beats();
    let ts = measure.time_signature();
    let capacity = ts.measure_duration_ticks();

    // Compute onset tick for each beat
    let mut onsets: Vec<i32> = Vec::with_capacity(beats.len());
    let mut t = 0;
    for b in beats.iter() {
        onsets.push(t);
        if let Some(dt) = set.grid.ticks_of(&b.duration) { t += dt; }
    }

    // Compute primary boundaries (tick positions inside the measure where groups should break by default)
    let boundaries = primary_boundaries(&ts);

    // Collect indices of beamable notes
    let mut note_idxs: Vec<usize> = Vec::new();
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
    let mut cur: Vec<usize> = vec![note_idxs[0]];
    for w in note_idxs.windows(2) {
        let a = w[0];
        let b = w[1];

        let a_on = onsets[a];
        let b_on = onsets[b];

        let mut break_group = false;
        // default: break if any primary boundary lies between these onsets (exclusive of a_on, inclusive of b_on)
        if boundaries.iter().any(|&bd| bd > a_on && bd <= b_on) {
            break_group = true;
        }
        // Check if any non-beamable NOTE exists between a..b (rests allowed)
        if !break_group {
            for k in (a + 1)..b {
                let bk = &beats[k];
                if bk.kind == BeatKind::Note && beam_count(&bk.duration) == 0 {
                    break_group = true;
                    break;
                }
            }
        }

        if break_group {
            finalize_group(&mut groups, &beats, &onsets, &cur);
            cur = vec![b];
        } else {
            cur.push(b);
        }
    }
    finalize_group(&mut groups, &beats, &onsets, &cur);

    BeamPlan { groups: groups.into_iter().enumerate().map(|(i, mut g)| { g.group_index = i; g }).collect() }
}

fn finalize_group(groups: &mut Vec<BeamGroup>, beats: &Vec<Beat>, onsets: &Vec<i32>, cur: &Vec<usize>) {
    if cur.is_empty() { return; }
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
        note_indices: cur.clone(),
        beam_counts,
        continuity,
        continues_from_previous: false,
        continues_into_next: false,
    });
}

fn has_rest_between(beats: &Vec<Beat>, i: usize, j: usize) -> bool {
    if j <= i + 1 { return false; }
    for k in (i + 1)..j {
        if beats[k].kind == BeatKind::Rest { return true; }
    }
    false
}

/// Compute the primary grouping stride in ticks for a time signature.
/// For simple meters (x/4), group by quarter; for compound (x/8 where x%3==0), group by dotted quarter;
/// For 7/8 default to 3+2+2 pattern -> groups of 3, then 2, then 2 eighths. Here we return the smallest
/// unit (in ticks) where primary breaks may occur frequently; we handle 7/8 specially by returning an eighth
/// and letting the group builder break at pattern boundaries via onset comparisons.
fn primary_boundaries(ts: &TimeSignature) -> Vec<i32> {
    let set = default_duration_set();
    let mut bounds: Vec<i32> = Vec::new();
    let ticks_per_whole = set.grid.ticks_per_whole;
    match (ts.beats as i32, ts.beat_unit as i32) {
        // Simple meters: boundaries at each beat (exclude 0 and end)
        (b, 4) => {
            let stride = ticks_per_whole / 4;
            for i in 1..b { bounds.push(i * stride); }
        }
        // Compound meters by dotted quarter: 6/8, 9/8, 12/8
        (6, 8) | (9, 8) | (12, 8) => {
            let eighth = ticks_per_whole / 8;
            let group = 3 * eighth; // dotted quarter
            let total = (ts.beats as i32) * eighth;
            let mut acc = group;
            while acc < total { bounds.push(acc); acc += group; }
        }
        // 7/8 default pattern 3+2+2 -> boundaries after 3 and 5 eighths
        (7, 8) => {
            let eighth = ticks_per_whole / 8;
            bounds.push(3 * eighth);
            bounds.push(5 * eighth);
        }
        // Fallback: boundaries at each beat unit
        (b, den) => {
            if den > 0 {
                let stride = ticks_per_whole / den;
                for i in 1..b { bounds.push(i * stride); }
            }
        }
    }
    bounds
}
