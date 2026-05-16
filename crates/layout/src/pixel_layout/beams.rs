use super::{BeamLayout, LayoutOpts, NoteLayout, c};
use crate::beam_plan::BeamGroup;
use egui::{Pos2, Rect};

/// Y coordinate of the bottom edge of a beam level (`lvl == 0` is the
/// outermost/topmost beam, closest to the stem tip).
fn beam_y_level(lvl: u8, base_y: f32, opts: &LayoutOpts) -> f32 {
    base_y + (lvl as f32) * (opts.beam_thickness() + opts.beam_gap())
}

/// Construct a beam rectangle from its bottom edge `y` and horizontal span.
fn beam_rect(left: f32, right: f32, bottom_y: f32, opts: &LayoutOpts) -> BeamLayout {
    let top = bottom_y - opts.beam_thickness();
    BeamLayout { rect: Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, bottom_y)) }
}

/// Emit full beams between adjacent stems of a beam group, one rectangle
/// per continuity level.
fn emit_full_beams(
    group: &BeamGroup,
    stem_xs: &[f32],
    base_y: f32,
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
            out.push(beam_rect(left, right, beam_y_level(lvl, base_y, opts), opts));
        }
    }
}

/// Emit stub beams (partial beams) at notes whose beam count exceeds the
/// continuity to either neighbor. Skips groups with fewer than two notes.
fn emit_stub_beams(
    group: &BeamGroup,
    stem_xs: &[f32],
    base_y: f32,
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
            out.push(beam_rect(left, right, beam_y_level(lvl, base_y, opts), opts));
        }
    }
}

pub(super) fn build_beam_layout(
    note_layout: &[NoteLayout],
    beam_groups: &[BeamGroup],
    opts: &LayoutOpts,
) -> Vec<BeamLayout> {
    // Align top edge with stem tip; small downward offset hides the
    // stem/beam seam.
    let base_y =
        opts.y_center() - opts.stem_length() + opts.beam_thickness() * c::BEAM_BASELINE_OFFSET;

    // Stems are already snapped in `note_layout`; fall back for un-stemmed
    // entries (shouldn't actually happen for beamed notes).
    let stem_xs: Vec<f32> = note_layout
        .iter()
        .map(|nl| nl.stem.map(|s| s.p1.x).unwrap_or(nl.center.x + opts.stem_offset()))
        .collect();
    let half_stem = opts.stem_thickness() * 0.5;

    let mut out: Vec<BeamLayout> = Vec::new();
    for g in beam_groups {
        emit_full_beams(g, &stem_xs, base_y, half_stem, opts, &mut out);
    }
    for g in beam_groups {
        emit_stub_beams(g, &stem_xs, base_y, half_stem, opts, &mut out);
    }
    out
}
