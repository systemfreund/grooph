use super::{c, requires_flag, Line, LayoutOpts, NoteLayout};
use crate::render_plan::RenderPlan;
use egui::{Pos2, Rect};
use grooph_measure::duration::Duration;
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::{Beat, BeatKind, Measure};

/// Number of augmentation dots on a duration (0 for non-dotted).
const fn dot_count_of(duration: Duration) -> u8 {
    match duration {
        Duration::Dotted { dots, .. } => dots,
        _ => 0,
    }
}

/// X offset of the first augmentation dot, relative to the note center.
fn dot_first_dx(opts: &LayoutOpts, has_flag_tail: bool) -> f32 {
    opts.em
        * if has_flag_tail {
            c::DOT_FIRST_DX_WITH_FLAG_EM
        } else {
            c::DOT_FIRST_DX_NO_FLAG_EM
        }
}

/// Absolute screen positions of the augmentation dots for a note centered
/// at `(center_x, cy)`. Returns empty if `dot_count == 0`.
fn dot_positions(
    center_x: f32,
    cy: f32,
    dot_count: u8,
    has_flag_tail: bool,
    opts: &LayoutOpts,
) -> Vec<Pos2> {
    if dot_count == 0 {
        return Vec::new();
    }
    let first_dx = dot_first_dx(opts, has_flag_tail);
    let step_dx = opts.em * c::DOT_STEP_DX_EM;
    let y = cy - opts.em * c::DOT_Y_OFFSET_EM;
    (0..dot_count).map(|d| Pos2::new(center_x + first_dx + (d as f32) * step_dx, y)).collect()
}

/// Pre-layout horizontal extents (relative to a note's center) used by the
/// shift pass. Estimates the rightward overhang from head/rest base width
/// plus dots and an ungrouped flag. Stems are not yet known here, so the
/// flag right edge is approximated from `opts.stem_offset()`.
fn compute_x_extents(beat: &Beat, in_beam: bool, opts: &LayoutOpts) -> (f32, f32) {
    let m = &opts.metrics;
    let is_note = beat.kind == BeatKind::Note;
    let needs_flag = requires_flag(beat.duration);

    let (left, mut right) = if is_note {
        let h = m.head_size.x * 0.5;
        (-h, h)
    } else {
        let w = m.rest_sizes[beat.duration.base_note().rest_index()].x;
        (-w * 0.5, w * 0.5)
    };

    let dot_count = dot_count_of(beat.duration);
    if dot_count > 0 {
        let has_flag_tail = is_note && !in_beam && needs_flag;
        let first_dx = dot_first_dx(opts, has_flag_tail);
        let step_dx = opts.em * c::DOT_STEP_DX_EM;
        let last_dot_center_rel = first_dx + ((dot_count - 1) as f32) * step_dx;
        let last_dot_right_rel = last_dot_center_rel + m.dot_size.x * 0.5;
        if last_dot_right_rel > right {
            right = last_dot_right_rel;
        }
    }

    if is_note && !in_beam && needs_flag {
        let flag_left_rel = opts.stem_offset() - opts.stem_thickness() * 0.5;
        let fw = m.flag_size_for(beat.duration.base_note()).x;
        let flag_right_rel = flag_left_rel + fw;
        if flag_right_rel > right {
            right = flag_right_rel;
        }
    }

    (left, right)
}

/// Post-layout vertical extents in absolute screen coordinates, derived
/// from the already-computed stem/flag/dot geometry.
fn compute_vertical_extents(
    beat: &Beat,
    cy: f32,
    stem: Option<&Line>,
    flag_pos: Option<Pos2>,
    dot_count: u8,
    opts: &LayoutOpts,
) -> (f32, f32) {
    let m = &opts.metrics;
    let is_note = beat.kind == BeatKind::Note;

    let base_h = if is_note {
        m.head_size.y * 0.5
    } else {
        m.rest_sizes[beat.duration.base_note().rest_index()].y
    };
    let mut top = cy - base_h;
    let mut bottom = cy + base_h;

    if let Some(s) = stem {
        top = top.min(s.p2.y).min(s.p1.y);
        bottom = bottom.max(s.p2.y).max(s.p1.y);
    }
    if dot_count > 0 {
        let dot_y = cy - opts.em * c::DOT_Y_OFFSET_EM;
        let h = m.dot_size.y * 0.5;
        top = top.min(dot_y - h);
        bottom = bottom.max(dot_y + h);
    }
    if let Some(fp) = flag_pos {
        let fs = m.flag_size_for(beat.duration.base_note());
        top = top.min(fp.y - fs.y * 0.5);
        bottom = bottom.max(fp.y + fs.y * 0.5);
    }
    (top, bottom)
}

pub(super) fn build_note_layout(
    measure: &Measure,
    render_plan: &RenderPlan,
    rect: &Rect,
    opts: &LayoutOpts,
) -> Vec<NoteLayout> {
    let beats = measure.beats();
    let beams = &render_plan.beams;

    // Basisverteilung (rhythmisch gleichmäßig über die verfügbare Breite)
    let x_centers =
        crate::calculate_x_centers(beats, rect.width(), opts.proportional_spacing)
            .into_iter()
            .map(|cx| cx + rect.left())
            .collect::<Vec<_>>();

    // Determine which beats are inside any beamed group (for flag suppression)
    let mut in_beam_flags: Vec<bool> = vec![false; beats.len()];
    for g in beams {
        if g.beat_indices.len() >= 2 {
            for &idx in &g.beat_indices {
                if idx < in_beam_flags.len() {
                    in_beam_flags[idx] = true;
                }
            }
        }
    }

    // 1) Kollisionen vermeiden: metrik-basierte Bounding-Box je Element und
    //    greedy nach rechts schieben (nur X‑Richtung).
    let em = opts.em;
    let min_gap = c::COLLISION_MIN_GAP_EM * em;

    // Map beat index to beam group index to detect connected notes
    let mut beat_to_beam_group = vec![None; beats.len()];
    for (g_idx, g) in beams.iter().enumerate() {
        for &b_idx in &g.beat_indices {
            if b_idx < beat_to_beam_group.len() {
                beat_to_beam_group[b_idx] = Some(g_idx);
            }
        }
    }

    // Primary group info
    let onsets = DEFAULT_GRID.compute_onset_ticks(beats);
    let boundaries = DEFAULT_GRID.primary_boundaries(&measure.time_signature());

    // Track shift per beat (cx - base_cx) to propagate within beam groups
    let mut shifts = vec![0.0; beats.len()];

    // (cx, left_rel, right_rel)
    let mut shifted_layout_info: Vec<(f32, f32, f32)> = Vec::with_capacity(beats.len());
    let mut prev_right: Option<f32> = None;

    for (i, b) in beats.iter().enumerate() {
        let base_cx = *x_centers.get(i).unwrap_or(&rect.center().x);

        let in_beam = in_beam_flags.get(i).copied().unwrap_or(false);
        let (left, right) = compute_x_extents(b, in_beam, opts);

        // Greedy-Verschiebung, um Mindestabstand zur vorherigen Box zu sichern
        let mut cx = base_cx;

        // Propagate shift if connected to previous beat
        let mut connected = false;
        if i > 0 {
            // 1. Beam
            if let Some(bg) = beat_to_beam_group[i]
                && beat_to_beam_group[i - 1] == Some(bg)
            {
                connected = true;
            }
            // 2. Tuplet
            else if render_plan
                .tuplets
                .iter()
                .any(|t| t.start <= (i - 1) && t.end >= i)
            {
                connected = true;
            }
            // 3. Primary Group (only if proportional spacing is enabled)
            else if opts.proportional_spacing {
                let t = onsets[i];
                if !boundaries.contains(&t) {
                    connected = true;
                }
            }
        }

        if connected {
            cx += shifts[i - 1];
        }

        if let Some(prev_r) = prev_right {
            let curr_left_abs = cx + left;
            let overlap = (prev_r + min_gap) - curr_left_abs;
            if overlap > 0.0 {
                cx += overlap;
            }
        }

        shifts[i] = cx - base_cx;

        // Update rechter Rand dieser Box in absoluten Koordinaten
        prev_right = Some(cx + right);
        shifted_layout_info.push((cx, left, right));
    }

    let mut note_layout: Vec<NoteLayout> = Vec::with_capacity(beats.len());
    for (i, b) in beats.iter().enumerate() {
        let (ideal_cx, left_rel, right_rel) = *shifted_layout_info.get(i).unwrap();
        let cy = opts.y_center();

        // 1. Calculate Stem (if any) and determining pixel-snapping offset.
        // We do this first because the notehead and dots must align with
        // the snapped stem.
        let mut stem: Option<Line> = None;

        let needs_flag = requires_flag(b.duration);
        let in_beam = in_beam_flags.get(i).copied().unwrap_or(false);
        let is_note = b.kind == BeatKind::Note;

        if is_note {
            let stem_len_factor = if in_beam || needs_flag { 1.0 } else { 0.85 };
            let stem_len = opts.stem_length() * stem_len_factor;
            let ideal_stem_x = ideal_cx + opts.stem_offset();
            let snapped_stem_x = opts.snap_x(ideal_stem_x, opts.stem_thickness());

            let start = Pos2::new(snapped_stem_x, cy - opts.em * 0.05);
            let end = Pos2::new(snapped_stem_x, cy - stem_len);
            stem = Some(Line { p1: start, p2: end });
        }

        // 2. Adjust Notehead Center
        let center = Pos2::new(ideal_cx, cy);

        // 3. Dots
        let dot_count = dot_count_of(b.duration);
        let has_flag_tail = is_note && !in_beam && needs_flag;
        let dots = dot_positions(center.x, cy, dot_count, has_flag_tail, opts);

        // 4. Flag (if needed)
        let mut flag_pos: Option<Pos2> = None;
        if is_note
            && !in_beam
            && needs_flag
            && let Some(s) = &stem
        {
            // Align flag to the left edge of the snapped stem
            let flag_x = s.p1.x - opts.stem_thickness() * 0.5;
            flag_pos = Some(Pos2::new(flag_x, cy - opts.stem_length()));
        }

        // 5. Accent and Debug Bounding Box.
        // We calculate the content bounding box first to position the
        // accent relative to it.
        let (content_top, content_bottom) =
            compute_vertical_extents(b, cy, stem.as_ref(), flag_pos, dot_count, opts);

        let mut accent_pos: Option<Pos2> = None;
        let mut accent_debug_bbox: Option<Rect> = None;

        if is_note && b.accented {
            let displacement = opts.em * opts.accent_displacement;
            let accent_half_h = opts.metrics.accent_size.y * 0.5;

            let logical_y = if opts.accent_below {
                content_bottom + displacement + accent_half_h
            } else {
                content_top - displacement - accent_half_h
            };

            accent_pos = Some(Pos2::new(center.x, logical_y + accent_half_h));

            if opts.debug_bbox {
                accent_debug_bbox = Some(Rect::from_center_size(
                    Pos2::new(center.x, logical_y),
                    opts.metrics.accent_size,
                ));
            }
        }

        let debug_bbox = if opts.debug_bbox {
            Some(Rect::from_min_max(
                Pos2::new(ideal_cx + left_rel, content_top),
                Pos2::new(ideal_cx + right_rel, content_bottom),
            ))
        } else {
            None
        };

        note_layout.push(NoteLayout {
            center,
            duration: b.duration,
            kind: b.kind,
            dots,
            stem,
            flag_pos,
            accent_pos,
            debug_bbox,
            accent_debug_bbox,
        });
    }

    note_layout
}
