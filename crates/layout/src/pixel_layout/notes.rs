use super::{LayoutOpts, Line, NoteLayout, PlacedGlyph, c, requires_flag};
use crate::glyphs;
use crate::render_plan::RenderPlan;
use egui::{Pos2, Rect};
use grooph_measure::duration::{Duration, NoteValue};
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::{Beat, BeatKind, Measure};

/// Number of augmentation dots on a duration (0 for non-dotted).
const fn dot_count_of(duration: Duration) -> u8 {
    match duration {
        Duration::Dotted { dots, .. } => dots,
        _ => 0,
    }
}

/// Where `beat` sits, in half staff spaces up from the staff line. Notes are
/// on the line. In the lower voice a rest hangs a staff space lower, clear of
/// an upper voice's, except half and whole rests: they are told apart only by
/// sitting on the line or hanging from it.
fn staff_pos_of(beat: &Beat, opts: &LayoutOpts) -> i8 {
    let keeps_line = matches!(beat.duration.base_note(), NoteValue::Half | NoteValue::Whole);
    match beat.kind {
        BeatKind::Rest if opts.stem_down && !keeps_line => -2,
        _ => 0,
    }
}

/// The head or rest glyph of `beat`.
fn head_glyph(beat: &Beat) -> glyphs::Glyph {
    match beat.kind {
        BeatKind::Note => glyphs::notehead_glyph_for_duration(beat.duration),
        BeatKind::Rest => glyphs::rest_glyph_for_duration(beat.duration),
    }
}

/// From a soft note's center to the outer edge of its parentheses' ink.
fn paren_reach(opts: &LayoutOpts) -> f32 {
    let ss = opts.staff_space();
    opts.head_half_width() + c::SOFT_PAREN_GAP_SS * ss + glyphs::NOTEHEAD_PAREN_LEFT.ink(ss).width()
}

/// The parentheses around a soft note's head centered at `center`.
fn parens_at(center: Pos2, opts: &LayoutOpts) -> Vec<PlacedGlyph> {
    let ss = opts.staff_space();
    let inner = opts.head_half_width() + c::SOFT_PAREN_GAP_SS * ss;
    let left = glyphs::NOTEHEAD_PAREN_LEFT;
    let right = glyphs::NOTEHEAD_PAREN_RIGHT;
    vec![
        PlacedGlyph {
            glyph: left,
            origin: Pos2::new(center.x - inner - left.ink(ss).right(), center.y),
            size: opts.em,
        },
        PlacedGlyph {
            glyph: right,
            origin: Pos2::new(center.x + inner - right.ink(ss).left(), center.y),
            size: opts.em,
        },
    ]
}

/// The flag of a note of `duration` on `stem`, its end at the stem's tip.
pub(super) fn flag_at(
    duration: Duration,
    stem: &Line,
    stem_down: bool,
    opts: &LayoutOpts,
) -> Option<PlacedGlyph> {
    let (glyph, anchor) = glyphs::flag_glyph_for_duration(duration, stem_down)?;
    let ss = opts.staff_space();
    // Flags hang off the stem's left edge, toward the right.
    let x = stem.p1.x - opts.stem_thickness() * 0.5 - anchor[0] * ss;
    Some(PlacedGlyph { glyph, origin: Pos2::new(x, stem.p2.y + anchor[1] * ss), size: opts.em })
}

/// X offset of the first augmentation dot's center, relative to the note
/// center: just right of the head, or of its parentheses for a soft note. An
/// up-stem flag curls down beside the dots and pushes them past it; a
/// down-stem flag hangs below them.
fn dot_first_dx(beat: &Beat, in_beam: bool, opts: &LayoutOpts) -> f32 {
    let is_note = beat.kind == BeatKind::Note;
    let mut dx = opts.em * c::DOT_FIRST_DX_NO_FLAG_EM;
    if is_note && beat.soft {
        dx += paren_reach(opts) - opts.head_half_width();
    }
    if is_note && !in_beam && requires_flag(beat.duration) && !opts.stem_down {
        dx = dx.max(opts.em * c::DOT_FIRST_DX_WITH_FLAG_EM);
    }
    dx
}

/// The augmentation dots of `beat`, a note centered at `(center_x, cy)`.
/// Returns empty if `dot_count == 0`.
fn dot_glyphs(
    beat: &Beat,
    in_beam: bool,
    center_x: f32,
    cy: f32,
    dot_count: u8,
    opts: &LayoutOpts,
) -> Vec<PlacedGlyph> {
    let first_dx = dot_first_dx(beat, in_beam, opts);
    let step_dx = opts.em * c::DOT_STEP_DX_EM;
    let y = cy - opts.em * c::DOT_Y_OFFSET_EM;
    (0..dot_count)
        .map(|d| {
            let x = center_x + first_dx + (d as f32) * step_dx;
            PlacedGlyph::centered(glyphs::AUGMENTATION_DOT, x, y, opts.em)
        })
        .collect()
}

/// Pre-layout horizontal extents (relative to a note's center) used by the
/// shift pass: the head or rest, its parentheses and dots, and an ungrouped
/// flag. Stems are not yet known here, so the flag is placed from
/// `opts.stem_offset()`.
fn compute_x_extents(beat: &Beat, in_beam: bool, opts: &LayoutOpts) -> (f32, f32) {
    let ss = opts.staff_space();
    let is_note = beat.kind == BeatKind::Note;
    let needs_flag = requires_flag(beat.duration);

    let (left, mut right) = if is_note && beat.soft {
        (-paren_reach(opts), paren_reach(opts))
    } else {
        let head = PlacedGlyph::centered(head_glyph(beat), 0.0, 0.0, opts.em).ink();
        (head.left(), head.right())
    };

    let dot_count = dot_count_of(beat.duration);
    if dot_count > 0 {
        let first_dx = dot_first_dx(beat, in_beam, opts);
        let step_dx = opts.em * c::DOT_STEP_DX_EM;
        let last_dot_center_rel = first_dx + ((dot_count - 1) as f32) * step_dx;
        let last_dot_right_rel =
            last_dot_center_rel + glyphs::AUGMENTATION_DOT.ink(ss).width() * 0.5;
        right = right.max(last_dot_right_rel);
    }

    if is_note
        && !in_beam
        && needs_flag
        && let Some((flag, anchor)) = glyphs::flag_glyph_for_duration(beat.duration, opts.stem_down)
    {
        let stem_dx = if opts.stem_down { -opts.stem_offset() } else { opts.stem_offset() };
        let flag_x = stem_dx - opts.stem_thickness() * 0.5 - anchor[0] * ss;
        right = right.max(flag_x + flag.ink(ss).right());
    }

    (left, right)
}

/// Post-layout vertical extents in absolute screen coordinates, from the
/// already-placed glyphs and stem.
/// Top and bottom of a note's head, stem, flag and dots.
pub(super) fn content_extents(note: &NoteLayout) -> (f32, f32) {
    let mut ink = note.head.ink();
    for g in note.flag.iter().chain(&note.dots) {
        ink = ink.union(g.ink());
    }
    if let Some(s) = note.stem {
        ink = ink.union(Rect::from_two_pos(s.p1, s.p2));
    }
    (ink.top(), ink.bottom())
}

/// The debug box of `note`: from `left` to `right`, and over its head, stem,
/// flag and dots.
pub(super) fn debug_bbox(note: &NoteLayout, left: f32, right: f32) -> Rect {
    let (top, bottom) = content_extents(note);
    Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, bottom))
}

/// Put an accent above the note's head, stem, flag and dots (below them
/// when `below`), replacing any accent it had.
pub(super) fn place_accent(note: &mut NoteLayout, below: bool, opts: &LayoutOpts) {
    let (content_top, content_bottom) = content_extents(note);
    let displacement = opts.em * opts.accent_displacement;
    let mut glyph = PlacedGlyph::centered(glyphs::ACCENT_ABOVE, note.center.x, 0.0, opts.em);
    let ink = glyph.ink();
    glyph.origin.y = if below {
        content_bottom + displacement - ink.top()
    } else {
        content_top - displacement - ink.bottom()
    };
    note.accent = Some(glyph);
    note.accent_debug_bbox = opts.debug_bbox.then(|| glyph.ink());
}

/// Whether each beat sits in a beamed group of two or more (no flag).
fn in_beam_flags(beats: &[Beat], render_plan: &RenderPlan) -> Vec<bool> {
    let mut flags = vec![false; beats.len()];
    for g in &render_plan.beams {
        if g.beat_indices.len() >= 2 {
            for &idx in &g.beat_indices {
                if idx < flags.len() {
                    flags[idx] = true;
                }
            }
        }
    }
    flags
}

/// Least distance (px) each beat needs to the next note head, or to the
/// barline for the last one, so their glyphs don't overlap.
fn min_advances(beats: &[Beat], in_beam: &[bool], opts: &LayoutOpts) -> Vec<f32> {
    let extents: Vec<(f32, f32)> =
        beats.iter().zip(in_beam).map(|(b, &ib)| compute_x_extents(b, ib, opts)).collect();
    let gap = c::COLLISION_MIN_GAP_EM * opts.em;
    let needs: Vec<f32> = (0..beats.len())
        .map(|i| match extents.get(i + 1) {
            Some(&(next_left, _)) => extents[i].1 + gap - next_left,
            None => extents[i].1,
        })
        .collect();
    // Notes of the same length get the same room, so a wide neighbour (a
    // soft note's parentheses) doesn't make the plain ones look shorter.
    let ticks: Vec<_> = beats.iter().map(|b| DEFAULT_GRID.ticks_of(&b.duration)).collect();
    ticks
        .iter()
        .map(|t| {
            ticks.iter().zip(&needs).filter(|(u, _)| *u == t).map(|(_, &n)| n).fold(0.0, f32::max)
        })
        .collect()
}

/// Narrowest note area in which no glyph overlaps the next one or the
/// barline (see [`crate::calculate_x_centers`]'s `min_advance`).
pub(super) fn min_note_area_width(
    measure: &Measure,
    render_plan: &RenderPlan,
    opts: &LayoutOpts,
) -> f32 {
    let beats = measure.beats();
    let in_beam = in_beam_flags(beats, render_plan);
    c::NOTE_LEAD_EM * opts.em + min_advances(beats, &in_beam, opts).iter().sum::<f32>()
}

pub(super) fn build_note_layout(
    measure: &Measure,
    render_plan: &RenderPlan,
    rect: &Rect,
    opts: &LayoutOpts,
) -> Vec<NoteLayout> {
    let beats = measure.beats();
    let beams = &render_plan.beams;

    // Base positions, spread evenly by rhythm over the available width
    // Determine which beats are inside any beamed group (for flag suppression)
    let in_beam_flags = in_beam_flags(beats, render_plan);

    let lead = c::NOTE_LEAD_EM * opts.em;
    let min_advance = min_advances(beats, &in_beam_flags, opts);
    let x_centers = crate::calculate_x_centers(
        beats,
        rect.width(),
        opts.proportional_spacing,
        lead,
        &min_advance,
    )
    .into_iter()
    .map(|cx| cx + rect.left())
    .collect::<Vec<_>>();

    // 1) Avoid collisions: a metrics-based bounding box per element, pushed
    //    right greedily (x only).
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

        // Push right to keep the minimum gap to the previous box
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
            else if render_plan.tuplets.iter().any(|t| t.start <= (i - 1) && t.end >= i) {
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
    let ss = opts.staff_space();
    for (i, b) in beats.iter().enumerate() {
        let (ideal_cx, left_rel, right_rel) = *shifted_layout_info.get(i).unwrap();
        let staff_pos = staff_pos_of(b, opts);
        let cy = opts.y_of(staff_pos);

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
            // Down stems hang off the head's left side (flipped later, see
            // `flip_down`).
            let stem_dx = if opts.stem_down { -opts.stem_offset() } else { opts.stem_offset() };
            let ideal_stem_x = ideal_cx + stem_dx;
            let snapped_stem_x = opts.snap_x(ideal_stem_x, opts.stem_thickness());

            let attach = glyphs::NOTEHEAD_BLACK_STEM_UP_SE[1] * ss;
            let start = Pos2::new(snapped_stem_x, cy - attach);
            let end = Pos2::new(snapped_stem_x, cy - stem_len);
            stem = Some(Line { p1: start, p2: end });
        }

        // 2. Notehead or rest. A head hangs off its stem's right edge (left
        // edge for a down stem): both are rounded to pixels on their own,
        // and centering the head could leave a hairline between them on
        // small staffs.
        let center = Pos2::new(ideal_cx, cy);
        let head = match &stem {
            Some(s) if opts.stem_down => {
                let x = s.p1.x
                    - opts.stem_thickness() * 0.5
                    - glyphs::NOTEHEAD_BLACK_STEM_DOWN_NW[0] * ss;
                PlacedGlyph { glyph: head_glyph(b), origin: Pos2::new(x, cy), size: opts.em }
            }
            Some(s) => {
                let x = s.p1.x + opts.stem_thickness() * 0.5
                    - glyphs::NOTEHEAD_BLACK_STEM_UP_SE[0] * ss;
                PlacedGlyph { glyph: head_glyph(b), origin: Pos2::new(x, cy), size: opts.em }
            }
            None => PlacedGlyph::centered(head_glyph(b), center.x, cy, opts.em),
        };
        let parens = if is_note && b.soft { parens_at(center, opts) } else { Vec::new() };

        // 3. Dots
        let dots = dot_glyphs(b, in_beam, center.x, cy, dot_count_of(b.duration), opts);

        // 4. Flag (if needed). The stem still points up here, so it gets an
        // up flag; `flip_down` swaps in the down flag.
        let flag = match &stem {
            Some(s) if !in_beam && needs_flag => flag_at(b.duration, s, false, opts),
            _ => None,
        };

        let mut note = NoteLayout {
            center,
            staff_pos,
            duration: b.duration,
            kind: b.kind,
            head,
            parens,
            dots,
            stem,
            flag,
            accent: None,
            debug_bbox: None,
            accent_debug_bbox: None,
        };

        // 5. Accent and Debug Bounding Box, around the head, stem, flag and dots.
        if is_note && b.accented {
            place_accent(&mut note, opts.accent_below, opts);
        }
        if opts.debug_bbox {
            note.debug_bbox = Some(debug_bbox(&note, ideal_cx + left_rel, ideal_cx + right_rel));
        }

        note_layout.push(note);
    }

    note_layout
}
