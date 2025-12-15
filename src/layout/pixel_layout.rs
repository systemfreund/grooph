use crate::layout::beam_plan::BeamGroup;
use crate::layout::render_plan::plan_measure;
use crate::layout::tuplet_plan::TupletPlan;
use crate::measure::duration::{Duration, NoteValue};
use crate::measure::{Beat, BeatKind, Measure, TimeSignature};
use eframe::egui::{self, FontId, Pos2, Rect};

pub fn compute_em(rect: &Rect, width_cap_factor: f32, ui: &egui::Ui) -> f32 {
    // Derive font size mainly from the available height, modulated by width caps
    let min_size = 12.0 * ui.ctx().pixels_per_point();
    let width_cap = (rect.width() * width_cap_factor).max(min_size);
    let max_size = rect.height().max(min_size);
    min_size.max(max_size.min(width_cap))
}

pub(crate) struct LayoutOpts {
    pub rect: Rect,
    pub font_id: FontId,
    pub pixels_per_point: f32,
    pub em: f32,
    pub layout_clef: bool,
    pub layout_time_signature: bool,

    pub y_offset: f32,
    pub stem_length_factor: f32,
    pub stem_thickness_factor: f32,

    pub accent_displacement: f32,
    pub accent_below: bool,
    pub debug_bbox: bool,
}

impl LayoutOpts {
    const fn staff_space(&self) -> f32 { self.em * 0.25 }

    const fn stem_length(&self) -> f32 { self.em * self.stem_length_factor }

    pub(crate) const fn stem_thickness(&self) -> f32 {
        self.font_id.size * self.stem_thickness_factor
    }

    const fn stem_offset(&self) -> f32 { self.font_id.size * 0.129 }

    pub(crate) const fn beam_thickness(&self) -> f32 {
        // Bravura ~0.5 sp
        0.5 * self.staff_space()
    }

    const fn beam_gap(&self) -> f32 { 0.25 * self.staff_space() }

    const fn stub_length(&self) -> f32 { self.em * 0.20 }

    pub(crate) const fn bracket_thickness(&self) -> f32 { self.font_id.size * 0.02 }

    fn y_center(&self) -> f32 { self.rect.center().y + self.y_offset }

    pub(crate) fn snap_thickness(&self, t: f32) -> f32 {
        let ppp = self.pixels_per_point;
        (t * ppp).round().max(1.0) / ppp
    }

    pub(crate) fn snap_x(&self, x: f32, thickness: f32) -> f32 {
        let ppp = self.pixels_per_point;
        let px_thickness = (thickness * ppp).round() as i32;
        if px_thickness % 2 != 0 {
            // Odd width: center on half-pixel
            ((x * ppp).round() + 0.5) / ppp
        } else {
            // Even width: center on integer pixel
            (x * ppp).round() / ppp
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BeamLayout {
    pub rect: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line {
    pub p1: Pos2,
    pub p2: Pos2,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TupletLayout {
    /// Ziffer (z. B. 3 für Triole)
    pub count: u8,
    /// Zentrum der Zahl in Pixelkoordinaten
    pub number_center: Pos2,
    /// Font für die Zahl (vom Layout vorgegeben)
    pub number_font: FontId,
    /// Klammersegmente inkl. Haken; leer bei number-only Fall
    pub bracket: Vec<Line>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NoteLayout {
    pub center: Pos2,
    pub duration: Duration,
    pub kind: BeatKind,
    pub dots: Vec<Pos2>,
    pub stem: Option<Line>,
    pub flag_pos: Option<Pos2>,
    pub accent_pos: Option<Pos2>,
    pub debug_bbox: Option<Rect>,
}

/// Pixel-level layout for a measure.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasureLayout {
    pub beams: Vec<BeamLayout>,
    pub notes: Vec<NoteLayout>,
    pub tuplets: Vec<TupletLayout>,
    pub clef_pos: Option<Pos2>,
    pub time_signature: Option<TimeSignatureLayout>,
    // Left boundary of the notes drawing area (excludes clef and time signature)
    pub notes_left_edge: f32,
}

/// Build the pixel layout (`MeasureLayout`) from a `Measure`.
pub fn build_measure_layout(measure: &Measure, opts: &LayoutOpts) -> MeasureLayout {
    let mut x_offset_acc = opts.rect.left();

    let clef_pos = if opts.layout_clef {
        let clef_w = opts.em * 1.1; // reserved width for percussion clef
        x_offset_acc += clef_w;
        Some(Pos2::new(opts.rect.left() + clef_w * 0.4, opts.y_center()))
    } else {
        None
    };

    // Time signature
    let time_signature_layout = if opts.layout_time_signature {
        let ts_layout = build_time_sig_layout(&measure.time_signature(), x_offset_acc, opts);
        x_offset_acc += ts_layout.width;
        Some(ts_layout)
    } else {
        None
    };

    // Notes
    let notes_left_edge = x_offset_acc;
    let note_rect =
        Rect::from_min_max(Pos2::new(notes_left_edge, opts.rect.top()), opts.rect.right_bottom());
    let render_plan = plan_measure(measure);
    let note_layout = build_note_layout(measure.beats(), &render_plan.beams, &note_rect, opts);
    let beam_layout = build_beam_layout(&note_layout, &render_plan.beams, opts);
    let tuplet_layout =
        build_tuplet_layout(measure.beats(), &note_layout, &render_plan.tuplets, opts);

    MeasureLayout {
        beams: beam_layout,
        notes: note_layout,
        tuplets: tuplet_layout,
        clef_pos,
        time_signature: time_signature_layout,
        notes_left_edge,
    }
}

fn digit_count(mut n: u32) -> usize {
    let mut c = 0usize;
    while n > 0 {
        c += 1;
        n /= 10;
    }
    c
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimeSignatureLayout {
    pub beats: Vec<Pos2>,
    pub beat_unit: Vec<Pos2>,
    pub width: f32,
}

pub(crate) fn build_time_sig_layout(
    time_signature: &TimeSignature,
    x: f32,
    opts: &LayoutOpts,
) -> TimeSignatureLayout {
    let ts_digit_w = opts.em * 0.35; // per column
    let top_digits = digit_count(time_signature.beats as u32);
    let bot_digits = digit_count(time_signature.beat_unit as u32);
    let ts_cols = top_digits.max(bot_digits) as f32;

    // Compute centered columns for both rows
    let mut time_sig_top: Vec<Pos2> = Vec::with_capacity(top_digits);
    let mut time_sig_bottom: Vec<Pos2> = Vec::with_capacity(bot_digits);

    let x_offset = top_digits.max(bot_digits) as f32 * ts_digit_w / 2.0;

    if top_digits > 0 {
        let offset = (ts_cols - top_digits as f32) * 0.5;
        for i in 0..top_digits {
            let cx = x - x_offset + ((i as f32) + 0.5 + offset) * ts_digit_w;
            time_sig_top.push(Pos2::new(cx, opts.y_center() - opts.em * 0.25));
        }
    }
    if bot_digits > 0 {
        let offset = (ts_cols - bot_digits as f32) * 0.5;
        for i in 0..bot_digits {
            let cx = x - x_offset + ((i as f32) + 0.5 + offset) * ts_digit_w;
            time_sig_bottom.push(Pos2::new(cx, opts.y_center() + opts.em * 0.25));
        }
    }

    TimeSignatureLayout {
        beats: time_sig_top,
        beat_unit: time_sig_bottom,
        width: ts_cols * ts_digit_w,
    }
}

fn build_note_layout(
    beats: &[Beat],
    beams: &[BeamGroup],
    rect: &Rect,
    opts: &LayoutOpts,
) -> Vec<NoteLayout> {
    // Basisverteilung (rhythmisch gleichmäßig über die verfügbare Breite)
    let x_centers = crate::layout::calculate_x_centers(beats, rect.width())
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

    // 1) Kollisionen vermeiden: heuristische Bounding-Box je Element und greedy nach rechts schieben
    //    (nur X‑Richtung). Dadurch werden insbesondere Punkte (dotted) und Fahnen berücksichtigt.
    let em = opts.em;
    let head_half = 0.2 * em; // ~Notehead-Hälfte
    let rest_half = 0.2 * em; // Restbreite (Heuristik)
    let dot_half = 0.15 * em;  // Punkt-Hälfte
    let min_gap = 0.0 * em;   // optischer Mindestabstand
    let flag_overhang = 0.20 * em; // kleine Ausladung einer einzelnen Fahne

    // (cx, left_rel, right_rel)
    let mut shifted_layout_info: Vec<(f32, f32, f32)> = Vec::with_capacity(beats.len());
    let mut prev_right: Option<f32> = None;

    for (i, b) in beats.iter().enumerate() {
        let base_cx = *x_centers.get(i).unwrap_or(&rect.center().x);

        let needs_flag = requires_flag(b.duration);
        let in_beam = in_beam_flags.get(i).copied().unwrap_or(false);
        let is_note = b.kind == BeatKind::Note;

        // Heuristische BBox relativ zur Center‑X
        let left = if is_note { -head_half } else { -rest_half };
        let mut right = if is_note { head_half } else { rest_half };

        // Dots berücksichtigen (rechts vom Kopf)
        let dot_count = match b.duration { Duration::Dotted { dots, .. } => dots, _ => 0 };
        if dot_count > 0 {
            // äußerster Punkt‑Rand
            let dots_right = dot_count as f32 * dot_half;
            right += dots_right;
        }

        // Einzelfahne (nicht beamed) ragt leicht nach rechts
        if is_note && !in_beam && needs_flag {
            right += flag_overhang;
        }

        // Greedy-Verschiebung, um Mindestabstand zur vorherigen Box zu sichern
        let mut cx = base_cx;
        if let Some(prev_r) = prev_right {
            let curr_left_abs = cx + left;
            let overlap = (prev_r + min_gap) - curr_left_abs;
            if overlap > 0.0 { cx += overlap; }
        }

        // Update rechter Rand dieser Box in absoluten Koordinaten
        prev_right = Some(cx + right);
        shifted_layout_info.push((cx, left, right));
    }

    let mut note_layout: Vec<NoteLayout> = Vec::with_capacity(beats.len());
    for (i, b) in beats.iter().enumerate() {
        // Benutze die kollisionsbereinigte Center‑X als Idealwert für das Rendering
        let (ideal_cx, left_rel, right_rel) = *shifted_layout_info.get(i).unwrap_or(&(rect.center().x, -head_half, head_half));
        let cy = opts.y_center();

        // 1. Calculate Stem (if any) and determining pixel-snapping offset
        // We do this first because the notehead and dots must align with the snapped stem.
        let mut stem: Option<Line> = None;
        let mut stem_x_offset = 0.0;
        let mut stem_width_snapped = 0.0;

        let needs_flag = requires_flag(b.duration);
        let in_beam = in_beam_flags.get(i).copied().unwrap_or(false);
        let is_note = b.kind == BeatKind::Note;

        if is_note {
            let stem_len_factor = if in_beam || needs_flag { 1.0 } else { 0.85 };
            let stem_len = opts.stem_length() * stem_len_factor;
            let ideal_stem_x = ideal_cx + opts.stem_offset();

            // Snap stem
            stem_width_snapped = opts.snap_thickness(opts.stem_thickness());
            let snapped_stem_x = opts.snap_x(ideal_stem_x, stem_width_snapped);
            stem_x_offset = snapped_stem_x - ideal_stem_x;

            let start = Pos2::new(snapped_stem_x, cy - opts.em * 0.05);
            let end = Pos2::new(snapped_stem_x, cy - stem_len);
            stem = Some(Line { p1: start, p2: end });
        }

        // 2. Adjust Notehead Center
        let center = Pos2::new(ideal_cx + stem_x_offset, cy);

        // 3. Dots
        let dot_count = match b.duration {
            Duration::Dotted { dots, .. } => dots,
            _ => 0,
        };
        let mut dots: Vec<Pos2> = Vec::with_capacity(dot_count as usize);
        if dot_count > 0 {
            let has_flag_tail = is_note && !in_beam && needs_flag;
            let first_dx =
                if has_flag_tail { opts.font_id.size * 0.5 } else { opts.font_id.size * 0.26 };
            let step_dx = opts.font_id.size * 0.26;
            // The dots start relative to the shifted center
            for d in 0..dot_count {
                let x = center.x + first_dx + (d as f32) * step_dx;
                let y = cy - opts.font_id.size * 0.1;
                dots.push(Pos2::new(x, y));
            }
        }

        // 4. Flag (if needed)
        let mut flag_pos: Option<Pos2> = None;
        if is_note
            && !in_beam
            && needs_flag
            && let Some(s) = &stem
        {
            // Align flag to the left edge of the snapped stem
            let flag_x = s.p1.x - stem_width_snapped * 0.5;
            flag_pos = Some(Pos2::new(flag_x, cy - opts.stem_length()));
        }

        // 5. Accent
        let mut accent_pos: Option<Pos2> = None;
        if is_note && b.accented {
            let dy = opts.em * opts.accent_displacement;
            let y = if opts.accent_below { cy + dy } else { cy - dy };
            accent_pos = Some(Pos2::new(center.x, y));
        }

        let debug_bbox = if opts.debug_bbox {
            let mut top = cy - em * 0.0;
            let mut bottom = cy + em * 0.0;

            // Expand to include stem if present
            if let Some(s) = &stem {
                top = top.min(s.p2.y).min(s.p1.y);
                bottom = bottom.max(s.p2.y).max(s.p1.y);
            }

            Some(Rect::from_min_max(
                Pos2::new(ideal_cx + left_rel, top),
                Pos2::new(ideal_cx + right_rel, bottom),
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
        });
    }

    note_layout
}

fn requires_flag(d: Duration) -> bool {
    matches!(d.base_note(), NoteValue::Eighth | NoteValue::Sixteenth | NoteValue::ThirtySecond)
}

fn build_beam_layout(
    note_layout: &[NoteLayout],
    beam_groups: &[BeamGroup],
    opts: &LayoutOpts,
) -> Vec<BeamLayout> {
    // align top edge with stem tip ⇒ use bottom y with slight offset to hide seam
    let base_y = opts.y_center() - opts.stem_length() + opts.beam_thickness() * 0.95;

    // We can rely on note_layout having snapped stems now.
    // Use stem center X if available, else fallback (though fallback shouldn't happen for beamed notes).
    let stem_xs: Vec<f32> = note_layout
        .iter()
        .map(|nl| nl.stem.map(|s| s.p1.x).unwrap_or(nl.center.x + opts.stem_offset()))
        .collect();

    // Helper: compute y for level
    let y_level =
        |lvl: u8| -> f32 { base_y + (lvl as f32) * (opts.beam_thickness() + opts.beam_gap()) };

    let beam_h = opts.beam_thickness();
    // To ensure beams cover the stems, we extend them by half the stem width.
    let stem_w = opts.snap_thickness(opts.stem_thickness());
    let half_stem = stem_w * 0.5;

    let mut beams_out: Vec<BeamLayout> = Vec::new();

    // Full beams between adjacent stems according to continuity
    for group in beam_groups {
        for (pair_idx, win) in group.beat_indices.windows(2).enumerate() {
            let i = win[0];
            let j = win[1];
            let levels = *group.continuity.get(pair_idx).unwrap_or(&0);
            if levels == 0 {
                continue;
            }

            let x1 = stem_xs[i];
            let x2 = stem_xs[j];
            let left = x1.min(x2) - half_stem;
            let right = x1.max(x2) + half_stem;

            for lvl in 0..levels {
                let y = y_level(lvl); // This is the bottom Y of the beam?
                // In original code: p1 = (x, y). Rect top = y - thickness.
                // So y is the BOTTOM edge.
                let top = y - beam_h;
                let bottom = y;

                beams_out.push(BeamLayout {
                    rect: Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, bottom)),
                });
            }
        }
    }

    // Partial beams (stubs) where a note's beam count exceeds continuity
    for group in beam_groups {
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

            for lvl in 0..count {
                let connects_left = lvl < left_cont;
                let connects_right = lvl < right_cont;

                if !connects_left && !connects_right {
                    let y = y_level(lvl);
                    let top = y - beam_h;
                    let bottom = y;

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

                    beams_out.push(BeamLayout {
                        rect: Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, bottom)),
                    });
                }
            }
        }
    }

    beams_out
}

fn build_tuplet_layout(
    beats: &[Beat],
    note_layout: &[NoteLayout],
    tuplet_plan: &[TupletPlan],
    opts: &LayoutOpts,
) -> Vec<TupletLayout> {
    let bracket_gap = 1.8 * opts.staff_space();
    let hook_len = 0.8 * opts.staff_space();
    let hook_dy = hook_len * 0.85;
    let digit_font = FontId::new(opts.font_id.size * 0.75, opts.font_id.family.clone());
    // Approximate baseline above stems
    let y_base = opts.y_center() - opts.stem_length() - 0.5 * opts.staff_space() - bracket_gap;

    let x_from_idx = |idx: usize| -> f32 {
        let n = note_layout.get(idx).unwrap();
        if let Some(stem) = &n.stem { stem.p1.x } else { n.center.x }
    };

    // Helper: count decimal digits of tuplet number
    let digit_len = |n: u8| -> usize { digit_count(n as u32) };

    let mut tuplets_out: Vec<TupletLayout> = Vec::new();
    for t in tuplet_plan {
        let mut x_l = x_from_idx(t.start);
        let mut x_r = x_from_idx(t.end);
        let margin = opts.em * 0.15;
        x_l -= margin;
        x_r += margin;

        // Number width approximation in pixels based on em
        let num_chars = digit_len(t.count) as f32;
        let num_width = num_chars * 0.6 * opts.em;
        let pad = 0.25 * opts.em; // horizontal padding around digits inside the bracket gap
        let xc = 0.5 * (x_l + x_r);
        let mut gap_half = 0.5 * (num_width + 2.0 * pad);
        let min_seg = 0.5 * opts.staff_space();
        let half_span = 0.5 * (x_r - x_l);
        if gap_half > half_span - min_seg {
            gap_half = (half_span - min_seg).max(0.0);
        }

        if !t.number_only() {
            // Bracketed case: raise whole bracket+number if any accent exists in span.
            let has_accent_in_group = !opts.accent_below
                && beats.iter().enumerate().any(|(i, b)| {
                    i >= t.start && i <= t.end && b.kind == BeatKind::Note && b.accented
                });
            let accent_clearance =
                (if has_accent_in_group { 1.4 } else { -0.4 }) * opts.staff_space();
            let y_bracket = y_base - accent_clearance;

            let x_gap_l = (xc - gap_half).max(x_l);
            let x_gap_r = (xc + gap_half).min(x_r);

            let mut bracket: Vec<Line> = Vec::new();
            if x_gap_l > x_l {
                bracket.push(Line {
                    p1: Pos2::new(x_l, y_bracket),
                    p2: Pos2::new(x_gap_l, y_bracket),
                });
            }
            if x_r > x_gap_r {
                bracket.push(Line {
                    p1: Pos2::new(x_gap_r, y_bracket),
                    p2: Pos2::new(x_r, y_bracket),
                });
            }
            bracket.push(Line {
                p1: Pos2::new(x_l, y_bracket),
                p2: Pos2::new(x_l, y_bracket + hook_dy),
            });
            bracket.push(Line {
                p1: Pos2::new(x_r, y_bracket),
                p2: Pos2::new(x_r, y_bracket + hook_dy),
            });

            let y_num = y_bracket + 0.5 * opts.staff_space();
            tuplets_out.push(TupletLayout {
                count: t.count,
                number_center: Pos2::new(0.5 * (x_l + x_r), y_num),
                number_font: digit_font.clone(),
                bracket,
            });
        } else {
            // Number-only case: only lift the number if it would collide with an accent horizontally.
            let num_cx = 0.5 * (x_l + x_r);
            let num_half_w = 0.5 * num_width;
            let collides = !opts.accent_below
                && (t.start..=t.end).any(|i| {
                    let b = beats[i];
                    b.kind == BeatKind::Note
                        && b.accented
                        && note_layout
                            .get(i)
                            .map(|nl| {
                                nl.center.x >= num_cx - num_half_w
                                    && nl.center.x <= num_cx + num_half_w
                            })
                            .unwrap_or(false)
                });

            // Choose vertical clearance based on potential collision
            let close_clearance = -opts.staff_space(); // closer to the beam
            let raised_clearance = 1.4 * opts.staff_space(); // high enough to clear accent
            let clearance = if collides { raised_clearance } else { close_clearance };
            let y_num = (y_base - clearance) + 0.5 * opts.staff_space();
            tuplets_out.push(TupletLayout {
                count: t.count,
                number_center: Pos2::new(0.5 * (x_l + x_r), y_num),
                number_font: digit_font.clone(),
                bracket: Vec::new(),
            });
        }
    }

    tuplets_out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure::{Measure, TimeSignature, Beat};
    use crate::measure::duration::q;
    use eframe::egui::{Rect, Pos2, FontId, FontFamily};

    #[test]
    fn test_debug_bbox_includes_stem_length() {
        use crate::measure::duration::e;
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Add one eighth note (has flag -> stem_len_factor = 1.0)
        m.set_beat(0, Beat::note(e())).unwrap();

        let em = 20.0;
        let opts = LayoutOpts {
            rect: Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(100.0, 100.0)),
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset: 0.0,
            stem_length_factor: 2.0, // Long stem: 2.0 * 20.0 = 40.0
            stem_thickness_factor: 0.1,
            accent_displacement: 0.0,
            accent_below: false,
            debug_bbox: true,
        };

        let layout = build_measure_layout(&m, &opts);
        let note = &layout.notes[0];
        let bbox = note.debug_bbox.expect("Debug bbox should be present");
        
        let cy = opts.rect.center().y; // 50.0
        let stem_len = em * opts.stem_length_factor; // 40.0 (factor 1.0 due to flag)
        
        // Assert bbox top covers the stem
        // Allow small margin for snapping
        assert!(bbox.min.y <= cy - stem_len + 1.0, 
            "BBox top ({}) should cover stem top ({})", bbox.min.y, cy - stem_len);
    }
}
