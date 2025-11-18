use eframe::egui::{FontId, Pos2, Rect};
use crate::layout::render_plan::plan_measure;
use crate::measure::duration::{Duration, NoteValue};
use crate::measure::{BeatKind, Measure};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeamSegmentPx {
    pub p1: Pos2, // bottom edge of the beam
    pub p2: Pos2, // bottom edge of the beam
    pub thickness: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinePx {
    pub p1: Pos2,
    pub p2: Pos2,
    pub thickness: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TupletLayoutPx {
    /// Ziffer (z. B. 3 für Triole)
    pub count: u8,
    /// Zentrum der Zahl in Pixelkoordinaten
    pub number_center: Pos2,
    /// Font für die Zahl (vom Layout vorgegeben)
    pub number_font: FontId,
    /// Klammersegmente inkl. Haken; leer bei number-only Fall
    pub bracket: Vec<LinePx>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NoteLayoutPx {
    pub center: Pos2,
    pub duration: Duration,
    pub is_rest: bool,
    pub dots: Vec<Pos2>,
    pub stem: Option<LinePx>,
    /// Where to place the flag glyph (if any). The concrete glyph is chosen by the renderer.
    pub flag_pos: Option<Pos2>,
    pub tremolo: Vec<LinePx>,
    /// Where to place the accent glyph (if any).
    pub accent_pos: Option<Pos2>,
}

/// Pixel-level layout for a measure.
///
/// This structure is produced by `build_measure_layout_px(..)` by combining:
/// - the musical semantics (`Measure`),
/// - the logical structures from `RenderPlan` (beaming and tuplets), and
/// - the target device constraints (available `Rect`, `FontId`, scaling).
///
/// It contains absolute positions, sizes and stroke thicknesses for everything the renderer
/// needs to draw, without performing any geometry computations at render time.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasureLayoutPx {
    pub inner_rect: Rect,
    pub em: f32,
    pub font_id: FontId,
    pub x_centers: Vec<f32>, // absolute pixel centers per beat (offset applied)
    pub beams: Vec<BeamSegmentPx>,
    pub notes: Vec<NoteLayoutPx>,
    pub clef_pos: Option<Pos2>,
    pub time_sig_top: Vec<Pos2>,
    pub time_sig_bottom: Vec<Pos2>,
    pub tuplets: Vec<TupletLayoutPx>,
    pub content_left: f32,
    pub content_right: f32,
}

/// Build the pixel layout (`MeasureLayoutPx`) from a `Measure`.
///
/// Responsibilities:
/// - Compute inner/content rectangles and font metrics for the current target rect.
/// - Map logical beat indices to absolute x centers.
/// - Expand `RenderPlan` beaming continuity into concrete `BeamSegmentPx` with y-levels and thickness.
/// - Derive per-note geometry (stems, flags, dots, tremolo, accents) in pixels.
/// - Compute positions for clef and stacked time signature digits.
/// - Expand logical tuplets into bracket segments and number positions at pixel coordinates.
///
/// Note: This intentionally avoids any dependency on `render::glyphs` to keep the module graph acyclic.
pub fn build_measure_layout_px(
    measure: &Measure,
    rect: Rect,
    base_font: &FontId,
    pixels_per_point: f32,
) -> MeasureLayoutPx {
    // 1) Inner rect and font metrics
    let min_size = 14.0 * pixels_per_point; // avoid unreadably small glyphs on HiDPI

    // Keep a small vertical padding fraction
    let vpad = (rect.height() * 0.10).clamp(10.0, 200.0);
    let hpad = (rect.width() * 0.10).clamp(10.0, 30.0);
    let inner_rect = Rect::from_min_max(
        Pos2::new(rect.left(), rect.top() + vpad),
        Pos2::new(rect.right() - hpad, rect.bottom() - vpad),
    );

    // Derive font size mainly from the available height, modulated by width caps
    let base_size_h = inner_rect.height() * 0.50;
    let width_cap = (rect.width() * 0.1).max(min_size);
    let max_size = (inner_rect.height() * 0.80).max(min_size);
    let target_size = base_size_h.clamp(min_size, max_size.min(width_cap));
    let font_id = FontId::new(target_size, base_font.family.clone());
    let em = target_size;

    // 2) Compute left block footprint (clef + stacked time signature) to derive content area
    let clef_w = em * 0.9; // reserved width for percussion clef
    let ts_digit_w = em * 0.35; // per column
    let ts = measure.time_signature();
    let top_digits = digit_count(ts.beats as u32);
    let bot_digits = digit_count(ts.beat_unit as u32);
    let ts_cols = top_digits.max(bot_digits) as f32;
    let ts_w = ts_cols * ts_digit_w;
    let ts_left = inner_rect.left() + clef_w - (em * 0.2);
    let content_left = ts_left + ts_w + (em * 0.2);
    let content_right = inner_rect.right();
    let content_w = (content_right - content_left).max(1.0);

    let y_center = rect.center().y;
    let clef_pos = Some(Pos2::new(inner_rect.left() + clef_w * 0.4, y_center));
    // Compute centered columns for both rows
    let mut time_sig_top: Vec<Pos2> = Vec::with_capacity(top_digits);
    let mut time_sig_bottom: Vec<Pos2> = Vec::with_capacity(bot_digits);
    if top_digits > 0 {
        let offset = (ts_cols - top_digits as f32) * 0.5;
        for i in 0..top_digits {
            let cx = ts_left + ((i as f32) + 0.5 + offset) * ts_digit_w;
            time_sig_top.push(Pos2::new(cx, y_center - em * 0.25));
        }
    }
    if bot_digits > 0 {
        let offset = (ts_cols - bot_digits as f32) * 0.5;
        for i in 0..bot_digits {
            let cx = ts_left + ((i as f32) + 0.5 + offset) * ts_digit_w;
            time_sig_bottom.push(Pos2::new(cx, y_center + em * 0.25));
        }
    }

    // 3) Absolute x centers
    let x_centers = crate::layout::calculate_x_centers(measure, content_w)
        .into_iter()
        .map(|cx| cx + content_left)
        .collect::<Vec<_>>();

    // 4) Beam segments in pixels (bottom edge y with thickness)
    let staff_space = em * 0.25; // tuned by eye; single-line context
    let beam_thickness = 0.5 * staff_space; // Bravura ~0.5 sp
    let beam_gap = 0.25 * staff_space; // distance between beams
    let default_stem_len = em * 0.9; // mirror render::beat::get_default_stem_length
    // align top edge with stem tip ⇒ use bottom y with slight offset to hide seam
    let beam_base_y = y_center - default_stem_len + beam_thickness * 0.95;

    let stem_dx = font_id.size * 0.13;
    let stem_thickness = font_id.size * 0.03;
    let stem_xs: Vec<f32> = x_centers.iter().map(|&cx| cx + stem_dx).collect();

    let render_plan = plan_measure(measure);
    let mut beams_out: Vec<BeamSegmentPx> = Vec::new();

    // Helper: compute y for level
    let y_level = |lvl: u8| -> f32 { beam_base_y + (lvl as f32) * (beam_thickness + beam_gap) };

    // Full beams between adjacent stems according to continuity
    for group in &render_plan.beams {
        for (pair_idx, win) in group.beat_indices.windows(2).enumerate() {
            let i = win[0];
            let j = win[1];
            let levels = *group.continuity.get(pair_idx).unwrap_or(&0);
            if levels == 0 {
                continue;
            }
            let offset = stem_thickness / 3.0; // extend slightly to touch stems nicely
            let x1 = stem_xs[i] - offset;
            let x2 = stem_xs[j] + offset;
            for lvl in 0..levels {
                let y = y_level(lvl);
                beams_out.push(BeamSegmentPx {
                    p1: Pos2::new(x1, y),
                    p2: Pos2::new(x2, y),
                    thickness: beam_thickness,
                });
            }
        }
    }

    // Partial beams (stubs) where a note's beam count exceeds continuity
    let stub_len = em * 0.20; // policy
    for group in &render_plan.beams {
        if group.beat_indices.len() < 2 {
            continue;
        }
        let note_idxs = &group.beat_indices;
        let counts = &group.beam_counts; // per note
        let cont = &group.continuity; // between neighbors

        for (local_k, &global_i) in note_idxs.iter().enumerate() {
            let count = *counts.get(local_k).unwrap_or(&0);
            if count == 0 {
                continue;
            }
            let left_cont = if local_k > 0 { *cont.get(local_k - 1).unwrap_or(&0) } else { 0 };
            let right_cont =
                if local_k + 1 < note_idxs.len() { *cont.get(local_k).unwrap_or(&0) } else { 0 };
            let stem_x = stem_xs[global_i];
            let is_first = local_k == 0;
            let is_last = local_k + 1 == note_idxs.len();

            for lvl in 0..count {
                let connects_left = lvl < left_cont;
                let connects_right = lvl < right_cont;
                match (connects_left, connects_right) {
                    (true, true) => { /* fully connected at this level */ }
                    (true, false) => { /* do nothing */ }
                    (false, true) => { /* do nothing */ }
                    (false, false) => {
                        let y = y_level(lvl);
                        if is_first {
                            beams_out.push(BeamSegmentPx {
                                p1: Pos2::new(stem_x, y),
                                p2: Pos2::new(stem_x + stub_len, y),
                                thickness: beam_thickness,
                            });
                        } else if is_last || left_cont > right_cont {
                            beams_out.push(BeamSegmentPx {
                                p1: Pos2::new(stem_x - stub_len, y),
                                p2: Pos2::new(stem_x, y),
                                thickness: beam_thickness,
                            });
                        } else if right_cont > left_cont {
                            beams_out.push(BeamSegmentPx {
                                p1: Pos2::new(stem_x, y),
                                p2: Pos2::new(stem_x + stub_len, y),
                                thickness: beam_thickness,
                            });
                        } else {
                            // equal continuity → prefer left by policy
                            beams_out.push(BeamSegmentPx {
                                p1: Pos2::new(stem_x - stub_len, y),
                                p2: Pos2::new(stem_x, y),
                                thickness: beam_thickness,
                            });
                        }
                    }
                }
            }
        }
    }

    let beats = measure.beats();

    // Determine which beats are inside any beamed group (for flag suppression)
    let mut in_beam_flags: Vec<bool> = vec![false; beats.len()];
    for g in &render_plan.beams {
        if g.beat_indices.len() >= 2 {
            for &idx in &g.beat_indices {
                if idx < in_beam_flags.len() {
                    in_beam_flags[idx] = true;
                }
            }
        }
    }

    // Metrics/policies
    let stem_dx = font_id.size * 0.13;
    let stem_thickness = font_id.size * 0.03;
    let default_stem_len = em * 0.9; // same base as above

    fn requires_flag(d: Duration) -> bool {
        matches!(d.base_note(), NoteValue::Eighth | NoteValue::Sixteenth | NoteValue::ThirtySecond)
    }

    let mut notes_out: Vec<NoteLayoutPx> = Vec::with_capacity(beats.len());
    for (i, b) in beats.iter().enumerate() {
        let cx = *x_centers.get(i).unwrap_or(&rect.center().x);
        let cy = y_center;
        let center = Pos2::new(cx, cy);

        // Dots (apply to both notes and rests)
        let dot_count = match b.duration {
            Duration::Dotted { dots, .. } => dots,
            _ => 0,
        };
        let has_flag_tail = b.kind == BeatKind::Note
            && !in_beam_flags.get(i).copied().unwrap_or(false)
            && requires_flag(b.duration);
        let first_dx = if has_flag_tail { font_id.size * 0.5 } else { font_id.size * 0.28 };
        let step_dx = font_id.size * 0.26;
        let mut dots: Vec<Pos2> = Vec::with_capacity(dot_count as usize);
        if dot_count > 0 {
            for d in 0..dot_count {
                let x = cx + first_dx + (d as f32) * step_dx;
                let y = cy - font_id.size * 0.1;
                dots.push(Pos2::new(x, y));
            }
        }

        // Stem (notes only)
        let mut stem: Option<LinePx> = None;
        let mut flag_pos: Option<Pos2> = None;
        let mut tremolo: Vec<LinePx> = Vec::new();
        let mut accent_pos: Option<Pos2> = None;

        if b.kind == BeatKind::Note {
            // Accent position
            if b.accented {
                accent_pos = Some(Pos2::new(cx, cy - font_id.size * 1.2));
            }

            let start_x = cx + stem_dx;
            let needs_flag = requires_flag(b.duration);
            let in_beam = in_beam_flags.get(i).copied().unwrap_or(false);
            let stem_len_factor = if in_beam || needs_flag { 1.0 } else { 0.85 };
            let stem_len = default_stem_len * stem_len_factor;
            let start = Pos2::new(start_x, cy);
            let end = Pos2::new(start_x, cy - stem_len);
            stem = Some(LinePx { p1: start, p2: end, thickness: stem_thickness });

            // Flag position at stem tip if not in a beam and duration requires a flag
            if !in_beam && needs_flag {
                flag_pos = Some(Pos2::new(start_x - stem_thickness * 0.5, cy - default_stem_len));
            }

            // Tremolo slashes (single-note measured tremolo)
            if let Some(trem) = b.tremolo
                && trem.measured
            {
                let sl = trem.slashes.min(3);
                let dx = font_id.size * 0.12; // slight right offset per slash
                let dy = font_id.size * 0.12; // spacing along stem
                let ang = 0.6; // tilt factor (down-right)
                for s in 0..sl {
                    let y0 = (cy - stem_len) + (s as f32) * dy;
                    let x0 = start_x + (s as f32) * dx;
                    let len = font_id.size * 0.45;
                    tremolo.push(LinePx {
                        p1: Pos2::new(x0, y0),
                        p2: Pos2::new(x0 + len, y0 - len * ang),
                        thickness: 2.0,
                    });
                }
            }
        }

        notes_out.push(NoteLayoutPx {
            center,
            duration: b.duration,
            is_rest: b.kind == BeatKind::Rest,
            dots,
            stem,
            flag_pos,
            tremolo,
            accent_pos,
        });
    }

    let staff_space = em * 0.25;
    let bracket_gap = 1.8 * staff_space;
    let hook_len = 0.8 * staff_space;
    let hook_dy = hook_len * 0.85;
    let number_font = FontId::new(font_id.size * 0.75, font_id.family.clone());
    let default_stem_len = em * 0.9;
    // Approximate baseline above stems
    let y_base = y_center - default_stem_len - 0.5 * staff_space - bracket_gap;

    let x_from_idx = |idx: usize| -> f32 {
        if let Some(n) = notes_out.get(idx) {
            if let Some(stem) = &n.stem { stem.p1.x } else { n.center.x }
        } else {
            *x_centers.get(idx).unwrap_or(&inner_rect.center().x)
        }
    };

    // Helper: count decimal digits of tuplet number
    let digit_len = |n: u8| -> usize { digit_count(n as u32) };

    let mut tuplets_out: Vec<TupletLayoutPx> = Vec::new();
    for t in &render_plan.tuplets {
        let mut x_l = x_from_idx(t.start);
        let mut x_r = x_from_idx(t.end);
        let margin = em * 0.15;
        x_l -= margin;
        x_r += margin;

        // Number width approximation in pixels based on em
        let num_chars = digit_len(t.count) as f32;
        let num_width = num_chars * 0.6 * em;
        let pad = 0.25 * staff_space; // horizontal padding around digits inside the bracket gap
        let xc = 0.5 * (x_l + x_r);
        let mut gap_half = 0.5 * (num_width + 2.0 * pad);
        let min_seg = 0.5 * staff_space;
        let half_span = 0.5 * (x_r - x_l);
        if gap_half > half_span - min_seg {
            gap_half = (half_span - min_seg).max(0.0);
        }

        if !t.number_only() {
            // Bracketed case: raise whole bracket+number if any accent exists in span.
            let has_accent_in_group = beats
                .iter()
                .enumerate()
                .any(|(i, b)| i >= t.start && i <= t.end && b.kind == BeatKind::Note && b.accented);
            let accent_clearance = (if has_accent_in_group { 1.4 } else { -0.4 }) * staff_space;
            let y_bracket = y_base - accent_clearance;

            let x_gap_l = (xc - gap_half).max(x_l);
            let x_gap_r = (xc + gap_half).min(x_r);

            let mut bracket: Vec<LinePx> = Vec::new();
            if x_gap_l > x_l {
                bracket.push(LinePx {
                    p1: Pos2::new(x_l, y_bracket),
                    p2: Pos2::new(x_gap_l, y_bracket),
                    thickness: 2.0,
                });
            }
            if x_r > x_gap_r {
                bracket.push(LinePx {
                    p1: Pos2::new(x_gap_r, y_bracket),
                    p2: Pos2::new(x_r, y_bracket),
                    thickness: 2.0,
                });
            }
            bracket.push(LinePx {
                p1: Pos2::new(x_l, y_bracket),
                p2: Pos2::new(x_l, y_bracket + hook_dy),
                thickness: 2.0,
            });
            bracket.push(LinePx {
                p1: Pos2::new(x_r, y_bracket),
                p2: Pos2::new(x_r, y_bracket + hook_dy),
                thickness: 2.0,
            });

            let y_num = y_bracket + 0.5 * staff_space;
            tuplets_out.push(TupletLayoutPx {
                count: t.count,
                number_center: Pos2::new(0.5 * (x_l + x_r), y_num),
                number_font: number_font.clone(),
                bracket,
            });
        } else {
            // Number-only case: only lift the number if it would collide with an accent horizontally.
            let num_cx = 0.5 * (x_l + x_r);
            let num_half_w = 0.5 * num_width;
            let collides = (t.start..=t.end).any(|i| {
                let b = beats[i];
                b.kind == BeatKind::Note
                    && b.accented
                    && x_centers
                    .get(i)
                    .map(|&x| x >= num_cx - num_half_w && x <= num_cx + num_half_w)
                    .unwrap_or(false)
            });

            // Choose vertical clearance based on potential collision
            let close_clearance = -0.4 * staff_space; // closer to the beam
            let raised_clearance = 1.4 * staff_space; // high enough to clear accent
            let clearance = if collides { raised_clearance } else { close_clearance };
            let y_num = (y_base - clearance) + 0.5 * staff_space;
            tuplets_out.push(TupletLayoutPx {
                count: t.count,
                number_center: Pos2::new(0.5 * (x_l + x_r), y_num),
                number_font: number_font.clone(),
                bracket: Vec::new(),
            });
        }
    }

    MeasureLayoutPx {
        inner_rect,
        em,
        font_id,
        x_centers,
        beams: beams_out,
        notes: notes_out,
        clef_pos,
        time_sig_top,
        time_sig_bottom,
        tuplets: tuplets_out,
        content_left,
        content_right,
    }
}

fn digit_count(mut n: u32) -> usize {
    if n == 0 {
        return 1;
    }
    let mut c = 0usize;
    while n > 0 {
        c += 1;
        n /= 10;
    }
    c
}
