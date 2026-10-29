use super::{BeamLayout, LayoutOpts, NoteLayout, c};
use crate::beam_plan::BeamGroup;
use egui::{Pos2, Rect};

/// The rectangle of beam level `lvl` (0 is the outermost, at the stem tips)
/// over `left..right`. The beams stack from the tips `tip_y` toward the
/// heads; the outermost reaches a little past the tips to hide the
/// stem/beam seam.
fn beam_rect(left: f32, right: f32, lvl: u8, tip_y: f32, opts: &LayoutOpts) -> BeamLayout {
    let d = opts.stem_dir();
    let step = opts.beam_thickness() + opts.beam_gap();
    let inner =
        tip_y - d * (opts.beam_thickness() * c::BEAM_BASELINE_OFFSET + f32::from(lvl) * step);
    let outer = inner + d * opts.beam_thickness();
    BeamLayout { rect: Rect::from_two_pos(Pos2::new(left, inner), Pos2::new(right, outer)) }
}

/// Emit full beams between adjacent stems of a beam group, one rectangle
/// per continuity level.
fn emit_full_beams(
    group: &BeamGroup,
    stem_xs: &[f32],
    tip_y: f32,
    half_stem: f32,
    opts: &LayoutOpts,
    out: &mut Vec<BeamLayout>,
) {
    for (pair_idx, win) in group.beat_indices.windows(2).enumerate() {
        let levels = *group.continuity.get(pair_idx).unwrap_or(&0);
        if levels == 0 {
            continue;
        }
        let x1 = stem_xs[win[0]];
        let x2 = stem_xs[win[1]];
        let left = x1.min(x2) - half_stem;
        let right = x1.max(x2) + half_stem;
        for lvl in 0..levels {
            out.push(beam_rect(left, right, lvl, tip_y, opts));
        }
    }
}

/// Emit stub beams (partial beams) at notes whose beam count exceeds the
/// continuity to either neighbor. Skips groups with fewer than two notes.
fn emit_stub_beams(
    group: &BeamGroup,
    stem_xs: &[f32],
    tip_y: f32,
    half_stem: f32,
    opts: &LayoutOpts,
    out: &mut Vec<BeamLayout>,
) {
    if group.beat_indices.len() < 2 {
        return;
    }
    let note_idxs = &group.beat_indices;
    let counts = &group.beam_counts;
    let cont = &group.continuity;

    for (local_k, &global_i) in note_idxs.iter().enumerate() {
        let count = *counts.get(local_k).unwrap_or(&0);
        if count == 0 {
            continue;
        }
        let left_cont = if local_k > 0 { *cont.get(local_k - 1).unwrap_or(&0) } else { 0 };
        let right_cont =
            if local_k + 1 < note_idxs.len() { *cont.get(local_k).unwrap_or(&0) } else { 0 };
        let stem_x = stem_xs[global_i];

        for lvl in 0..count {
            let connects_left = lvl < left_cont;
            let connects_right = lvl < right_cont;
            if connects_left || connects_right {
                continue;
            }
            // First note in group → stub points right; last → left;
            // middle → the side with more outgoing beam levels.
            let stub_right = if local_k == 0 {
                true
            } else if local_k + 1 == note_idxs.len() {
                false
            } else {
                right_cont > left_cont
            };
            let (left, right) = if stub_right {
                (stem_x - half_stem, stem_x + opts.stub_length())
            } else {
                (stem_x - opts.stub_length(), stem_x + half_stem)
            };
            out.push(beam_rect(left, right, lvl, tip_y, opts));
        }
    }
}

/// Let every stem of a beamed group reach the group's beams, at the tip of
/// its outermost head: on the kit's staff the heads of a group sit at
/// different heights. A stemlet keeps its length.
pub(super) fn reach_beams(notes: &mut [NoteLayout], beam_groups: &[BeamGroup], opts: &LayoutOpts) {
    for g in beam_groups.iter().filter(|g| g.beat_indices.len() >= 2) {
        let positions = g.beat_indices.iter().map(|&i| notes[i].staff_pos);
        let tip = opts.stem_tip_y(opts.outermost(positions).unwrap_or(0));
        for &i in &g.beat_indices {
            let stemlet = notes[i].stemlet;
            if let Some(stem) = &mut notes[i].stem {
                if stemlet {
                    stem.p1.y = tip - (stem.p2.y - stem.p1.y);
                }
                stem.p2.y = tip;
            }
        }
    }
}

pub(super) fn build_beam_layout(
    note_layout: &[NoteLayout],
    beam_groups: &[BeamGroup],
    opts: &LayoutOpts,
) -> Vec<BeamLayout> {
    // The beams hang off the stem tips of the group's outermost head.
    let tip_y = |g: &BeamGroup| {
        let positions = g.beat_indices.iter().map(|&i| note_layout[i].staff_pos);
        opts.stem_tip_y(opts.outermost(positions).unwrap_or(0))
    };

    // Stems are already snapped in `note_layout`; fall back for un-stemmed
    // entries (shouldn't actually happen for beamed notes).
    let stem_xs: Vec<f32> = note_layout
        .iter()
        .map(|nl| nl.stem.map(|s| s.p1.x).unwrap_or(nl.center.x + opts.stem_offset()))
        .collect();
    let half_stem = opts.stem_thickness() * 0.5;

    let mut out: Vec<BeamLayout> = Vec::new();
    for g in beam_groups {
        emit_full_beams(g, &stem_xs, tip_y(g), half_stem, opts, &mut out);
    }
    for g in beam_groups {
        emit_stub_beams(g, &stem_xs, tip_y(g), half_stem, opts, &mut out);
    }
    out
}
