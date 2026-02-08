use crate::beam_plan::BeamGroup;
use crate::render_plan::{plan_measure, RenderPlan};
use crate::tuplet_plan::TupletPlan;
use grooph_measure::duration::{Duration, NoteValue};
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::{Beat, BeatKind, Measure, TimeSignature};
use crate::glyphs;
use egui::{self, FontId, Pos2, Rect, Vec2};

#[derive(Debug, Clone, Copy)]
pub struct GlyphMetrics {
    pub head_size: Vec2,
    pub dot_size: Vec2,
    pub accent_size: Vec2,
    pub flag_8th_size: Vec2,
    pub flag_16th_size: Vec2,
    pub flag_32nd_size: Vec2,
    pub rest_sizes: [Vec2; 6],
}

impl GlyphMetrics {
    pub fn measure(ui: &egui::Ui, font_id: &FontId) -> Self {
        let em = font_id.size;
        // Measure only width from font; use heuristics for height to avoid huge SMuFL bounding boxes
        let measure_width = |c: char| -> f32 {
            ui.painter()
                .layout_no_wrap(c.to_string(), font_id.clone(), egui::Color32::WHITE)
                .rect
                .width()
        };

        Self {
            head_size: Vec2::new(measure_width(glyphs::GLYPH_NOTEHEAD_BLACK), 0.25 * em),
            dot_size: Vec2::new(measure_width(glyphs::GLYPH_AUGMENTATION_DOT), 0.2 * em),
            accent_size: Vec2::new(measure_width(glyphs::GLYPH_ACCENT_ABOVE), 0.25 * em),
            flag_8th_size: Vec2::new(measure_width(glyphs::GLYPH_FLAG_8TH_UP), 0.2 * em),
            flag_16th_size: Vec2::new(measure_width(glyphs::GLYPH_FLAG_16TH_UP), 0.2 * em),
            flag_32nd_size: Vec2::new(measure_width(glyphs::GLYPH_FLAG_32ND_UP), 0.4 * em),
            rest_sizes: [
                Vec2::new(measure_width(glyphs::GLYPH_REST_WHOLE), 0.25 * em),
                Vec2::new(measure_width(glyphs::GLYPH_REST_HALF), 0.25 * em),
                Vec2::new(measure_width(glyphs::GLYPH_REST_QUARTER), 0.45 * em),
                Vec2::new(measure_width(glyphs::GLYPH_REST_EIGHTH), 0.30 * em),
                Vec2::new(measure_width(glyphs::GLYPH_REST_SIXTEENTH), 0.50 * em),
                Vec2::new(measure_width(glyphs::GLYPH_REST_32ND), 0.55 * em),
            ],
        }
    }

    pub fn debug(em: f32) -> Self {
        let default_size = Vec2::new(0.4 * em, 0.25 * em);
        Self {
            head_size: default_size,
            dot_size: Vec2::new(0.2 * em, 0.2 * em),
            accent_size: Vec2::new(0.4 * em, 0.2 * em),
            flag_8th_size: Vec2::new(0.3 * em, 1.0 * em),
            flag_16th_size: Vec2::new(0.35 * em, 1.2 * em),
            flag_32nd_size: Vec2::new(0.4 * em, 1.4 * em),
            rest_sizes: [default_size; 6],
        }
    }
}

pub fn compute_em(rect: &Rect, width_cap_factor: f32, ui: &egui::Ui) -> f32 {
    // Derive font size mainly from the available height, modulated by width caps
    let min_size = 12.0 * ui.ctx().pixels_per_point();
    let width_cap = (rect.width() * width_cap_factor).max(min_size);
    let max_size = rect.height().max(min_size);
    min_size.max(max_size.min(width_cap))
}

#[derive(Clone)]
pub struct LayoutOpts {
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
    pub proportional_spacing: bool,
    pub debug_bbox: bool,
    pub metrics: GlyphMetrics,
}

impl LayoutOpts {
    const fn staff_space(&self) -> f32 { self.em * 0.25 }

    const fn stem_length(&self) -> f32 { self.em * self.stem_length_factor }

    pub const fn stem_thickness(&self) -> f32 {
        self.snap_thickness(self.em * self.stem_thickness_factor)
    }

    // const fn stem_offset(&self) -> f32 { self.em * 0.129 }
    const fn stem_offset(&self) -> f32 {
        self.metrics.head_size.x * 0.5 - self.stem_thickness() * 0.5
    }

    pub const fn beam_thickness(&self) -> f32 {
        // Bravura ~0.5 sp
        0.5 * self.staff_space()
    }

    const fn beam_gap(&self) -> f32 { 0.25 * self.staff_space() }

    const fn stub_length(&self) -> f32 { self.em * 0.20 }

    pub const fn bracket_thickness(&self) -> f32 { self.em * 0.02 }

    fn y_center(&self) -> f32 { self.rect.center().y + self.y_offset }

    const fn snap_thickness(&self, t: f32) -> f32 {
        let ppp = self.pixels_per_point;
        (t * ppp).round().max(1.0) / ppp
    }

    pub(crate) const fn snap_x(&self, x: f32, thickness: f32) -> f32 {
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
    pub accent_debug_bbox: Option<Rect>,
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
    let note_layout = build_note_layout(measure, &render_plan, &note_rect, opts);
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

pub fn build_time_sig_layout(
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

    // 1) Kollisionen vermeiden: metrik-basierte Bounding-Box je Element und greedy nach rechts schieben
    //    (nur X‑Richtung).
    let em = opts.em;
    let min_gap = 0.0 * em; // optischer Mindestabstand

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

        let needs_flag = requires_flag(b.duration);
        let in_beam = in_beam_flags.get(i).copied().unwrap_or(false);
        let is_note = b.kind == BeatKind::Note;

        let m = &opts.metrics;

        // Basis-Box (Kopf oder Rest)
        let (left, mut right) = if is_note {
            let h = m.head_size.x * 0.5;
            (-h, h)
        } else {
            let idx = match b.duration.base_note() {
                NoteValue::Whole => 0,
                NoteValue::Half => 1,
                NoteValue::Quarter => 2,
                NoteValue::Eighth => 3,
                NoteValue::Sixteenth => 4,
                NoteValue::ThirtySecond => 5,
            };
            let w = m.rest_sizes[idx].x;
            (-w * 0.5, w * 0.5)
        };

        // Dots berücksichtigen (rechts vom Kopf)
        let dot_count = match b.duration {
            Duration::Dotted { dots, .. } => dots,
            _ => 0,
        };
        if dot_count > 0 {
            // Berechne rechte Kante basierend auf Rendering-Logik
            let has_flag_tail = is_note && !in_beam && needs_flag;
            let first_dx = if has_flag_tail { opts.em * 0.5 } else { opts.em * 0.26 };
            let step_dx = opts.em * 0.26;
            let last_dot_center_rel = first_dx + ((dot_count - 1) as f32) * step_dx;
            let last_dot_right_rel = last_dot_center_rel + m.dot_size.x * 0.5;

            if last_dot_right_rel > right {
                right = last_dot_right_rel;
            }
        }

        // Einzelfahne (nicht beamed) ragt leicht nach rechts
        if is_note && !in_beam && needs_flag {
            // Flag startet links vom Stem (stem_offset - halbe Dicke) und geht flag_width nach rechts
            let stem_offset = opts.stem_offset();
            let stem_thick = opts.stem_thickness();
            let flag_left_rel = stem_offset - stem_thick * 0.5;

            let fw = match b.duration.base_note() {
                NoteValue::Eighth => m.flag_8th_size.x,
                NoteValue::Sixteenth => m.flag_16th_size.x,
                NoteValue::ThirtySecond => m.flag_32nd_size.x,
                _ => m.flag_8th_size.x,
            };
            let flag_right_rel = flag_left_rel + fw;
            if flag_right_rel > right {
                right = flag_right_rel;
            }
        }

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

        // 1. Calculate Stem (if any) and determining pixel-snapping offset
        // We do this first because the notehead and dots must align with the snapped stem.
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
        let dot_count = match b.duration {
            Duration::Dotted { dots, .. } => dots,
            _ => 0,
        };
        let mut dots: Vec<Pos2> = Vec::with_capacity(dot_count as usize);
        if dot_count > 0 {
            let has_flag_tail = is_note && !in_beam && needs_flag;
            let first_dx = opts.em * if has_flag_tail { 0.5 } else { 0.26 };
            let step_dx = opts.em * 0.26;
            // The dots start relative to the shifted center
            for d in 0..dot_count {
                let x = center.x + first_dx + (d as f32) * step_dx;
                let y = cy - opts.em * 0.1;
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
            let flag_x = s.p1.x - opts.stem_thickness() * 0.5;
            flag_pos = Some(Pos2::new(flag_x, cy - opts.stem_length()));
        }

        // 5. Accent and Debug Bounding Box
        // We calculate the content bounding box first to position the accent relative to it.
        let (content_top, content_bottom) = {
            let m = &opts.metrics;
            // Base height from head or rest
            let base_h = if is_note {
                m.head_size.y * 0.5
            } else {
                let idx = match b.duration.base_note() {
                    NoteValue::Whole => 0,
                    NoteValue::Half => 1,
                    NoteValue::Quarter => 2,
                    NoteValue::Eighth => 3,
                    NoteValue::Sixteenth => 4,
                    NoteValue::ThirtySecond => 5,
                };
                m.rest_sizes[idx].y
            };

            let mut top = cy - base_h;
            let mut bottom = cy + base_h;

            // Expand to include stem if present
            if let Some(s) = &stem {
                top = top.min(s.p2.y).min(s.p1.y);
                bottom = bottom.max(s.p2.y).max(s.p1.y);
            }

            // Expand for dots
            if dot_count > 0 {
                let dot_y = cy - opts.em * 0.1;
                let h = m.dot_size.y * 0.5;
                top = top.min(dot_y - h);
                bottom = bottom.max(dot_y + h);
            }

            // Expand for flag
            if let Some(fp) = flag_pos {
                let fs = match b.duration.base_note() {
                    NoteValue::Eighth => m.flag_8th_size,
                    NoteValue::Sixteenth => m.flag_16th_size,
                    NoteValue::ThirtySecond => m.flag_32nd_size,
                    _ => m.flag_8th_size,
                };
                // Flag is drawn at LEFT_CENTER.
                top = top.min(fp.y - fs.y * 0.5);
                bottom = bottom.max(fp.y + fs.y * 0.5);
            }
            (top, bottom)
        };

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
    let stem_w = opts.stem_thickness();
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
    use grooph_measure::duration::q;
    use grooph_measure::{Beat, Measure, TimeSignature};
    use egui::{FontFamily, FontId, Pos2, Rect, Vec2};

    #[test]
    fn test_debug_bbox_includes_stem_length() {
        use grooph_measure::duration::e;
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
            proportional_spacing: true,
            debug_bbox: true,
            metrics: GlyphMetrics::debug(em),
        };

        let layout = build_measure_layout(&m, &opts);
        let note = &layout.notes[0];
        let bbox = note.debug_bbox.expect("Debug bbox should be present");

        let cy = opts.rect.center().y; // 50.0
        let stem_len = em * opts.stem_length_factor; // 40.0 (factor 1.0 due to flag)

        // Assert bbox top covers the stem
        // Allow small margin for snapping
        assert!(
            bbox.min.y <= cy - stem_len + 1.0,
            "BBox top ({}) should cover stem top ({})",
            bbox.min.y,
            cy - stem_len
        );
    }

    #[test]
    fn test_layout_uses_metrics_if_provided() {
        use grooph_measure::duration::q;
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        m.set_beat(0, Beat::note(q())).unwrap();

        let em = 20.0;
        let mut opts = LayoutOpts {
            rect: Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(100.0, 100.0)),
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset: 0.0,
            stem_length_factor: 1.0,
            stem_thickness_factor: 0.1,
            accent_displacement: 0.0,
            accent_below: false,
            proportional_spacing: true,
            debug_bbox: true,
            metrics: GlyphMetrics::debug(em),
        };

        // 1. Standard Metrics (Debug defaults)
        let layout_std = build_measure_layout(&m, &opts);
        let bbox_std = layout_std.notes[0].debug_bbox.unwrap();
        let width_std = bbox_std.width();

        // 2. Mit Metrics (Large Head)
        opts.metrics = GlyphMetrics {
            head_size: Vec2::new(5.0 * em, 1.0 * em), // Huge head
            ..GlyphMetrics::debug(em)
        };

        let layout_metrics = build_measure_layout(&m, &opts);
        let bbox_metrics = layout_metrics.notes[0].debug_bbox.unwrap();
        let width_metrics = bbox_metrics.width();

        assert!(
            width_metrics > width_std * 2.0,
            "Metrics-based width should be significantly larger"
        );
        // Expect width roughly 5.0 * em
        assert!((width_metrics - 5.0 * em).abs() < 1.0);
    }

    #[test]
    fn test_accent_positioning_relative_to_bbox() {
        use grooph_measure::duration::q;
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Quarter note with accent
        let b = Beat::note(q());
        m.set_beat(0, b).unwrap();
        m.toggle_accent(0);

        let em = 20.0;
        let displacement = 0.5; // 0.5 em gap
        let opts = LayoutOpts {
            rect: Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(100.0, 100.0)),
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset: 0.0,
            stem_length_factor: 3.5,
            stem_thickness_factor: 0.1,
            accent_displacement: displacement,
            accent_below: false,
            proportional_spacing: true,
            debug_bbox: true,
            metrics: GlyphMetrics::debug(em),
        };

        // Case 1: Accent Above (default)
        // Should be above the stem (since stem goes up)
        let layout = build_measure_layout(&m, &opts);
        let note = &layout.notes[0];
        let accent_pos = note.accent_pos.expect("Accent should be present");
        let stem = note.stem.expect("Stem should be present");

        // Stem tip is the top-most point of the stem (smaller y)
        let stem_top = stem.p2.y.min(stem.p1.y);

        // Accent should be above stem top by (displacement * em) + half accent height (logical)
        // PLUS visual offset (half accent height)
        // y decreases upwards
        // expected_pos_y = logical_y + visual_offset
        //                = (stem_top - displacement - half_h) + half_h
        //                = stem_top - displacement
        let expected_y = stem_top - (displacement * em);

        assert!(
            (accent_pos.y - expected_y).abs() < 0.001,
            "Accent (y={}) should be exactly at expected (y={})",
            accent_pos.y,
            expected_y
        );

        // Case 2: Accent Below
        let mut opts_below = opts.clone();
        opts_below.accent_below = true;
        let layout_below = build_measure_layout(&m, &opts_below);
        let note_below = &layout_below.notes[0];
        let accent_pos_below = note_below.accent_pos.expect("Accent should be present");

        // Note head bottom
        let head_bottom = opts.y_center() + opts.metrics.head_size.y * 0.5;

        // Accent should be below head bottom
        assert!(accent_pos_below.y > head_bottom, "Accent should be below note head");
    }

    #[test]
    fn test_accent_debug_bbox_separate() {
        use grooph_measure::duration::q;
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        let b = Beat::note(q());
        m.set_beat(0, b).unwrap();
        m.toggle_accent(0);

        let em = 20.0;
        let opts = LayoutOpts {
            rect: Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(100.0, 100.0)),
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset: 0.0,
            stem_length_factor: 3.5,
            stem_thickness_factor: 0.1,
            accent_displacement: 0.5,
            accent_below: false,
            proportional_spacing: true,
            debug_bbox: true,
            metrics: GlyphMetrics::debug(em),
        };

        let layout = build_measure_layout(&m, &opts);
        let note = &layout.notes[0];

        assert!(note.debug_bbox.is_some(), "Main debug bbox should be present");
        assert!(note.accent_debug_bbox.is_some(), "Accent debug bbox should be present");

        let main_bbox = note.debug_bbox.unwrap();
        let accent_bbox = note.accent_debug_bbox.unwrap();

        // Check accent bbox size matches metrics
        let expected_size = opts.metrics.accent_size;
        assert!((accent_bbox.width() - expected_size.x).abs() < 0.001);
        assert!((accent_bbox.height() - expected_size.y).abs() < 0.001);

        // Check they don't intersect (displacement ensures gap)
        assert!(!main_bbox.intersects(accent_bbox), "BBoxes should be separate");
    }

    #[test]
    fn test_beam_group_spacing_preservation() {
        use grooph_measure::duration::{t8, th};
        // Use 2/4 to keep total duration small (matches our notes)
        let mut m = Measure::new(TimeSignature::TWO_FOUR);

        // Group 1: 7x 32nd notes
        for i in 0..7 {
            m.set_beat(i, Beat::note(th())).unwrap();
        }

        // Group 2: 3x triplet 8th
        // Indices 7, 8, 9
        for i in 0..3 {
            m.set_beat(7 + i, Beat::note(t8())).unwrap();
        }

        // Setup layout
        let em = 10.0;
        let rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(50.0, 100.0));

        let opts = LayoutOpts {
            rect,
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset: 0.0,
            stem_length_factor: 3.5,
            stem_thickness_factor: 0.1,
            accent_displacement: 0.0,
            accent_below: false,
            proportional_spacing: true,
            debug_bbox: true,
            metrics: GlyphMetrics::debug(em),
        };

        let layout = build_measure_layout(&m, &opts);

        // Check triplets: indices 7, 8, 9
        let n7 = &layout.notes[7];
        let n8 = &layout.notes[8];
        let n9 = &layout.notes[9];

        let d1 = n8.center.x - n7.center.x;
        let d2 = n9.center.x - n8.center.x;

        println!("Triplet spacing: {:.2} vs {:.2}", d1, d2);

        // We expect d1 < d2 (compressed vs natural)
        assert!(
            (d1 - d2).abs() < 1.0,
            "Spacing in triplet group should be consistent. Got {:.2} vs {:.2}",
            d1,
            d2
        );
    }

    #[test]
    fn test_primary_group_spacing_preservation() {
        use grooph_measure::duration::{Duration, TupletSpec, NoteValue};
        use grooph_measure::{Beat, Measure, TimeSignature};
        
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);

        // Group 1: Insert many 32nd notes to force a shift
        // 8 * 1/32 = 1/4 beat.
        // We need enough to push the next beat.
        // Let's just use a very wide previous element?
        // Or simpler: Use a custom LayoutOpt with huge 'clef' or 'time signature' width?
        // But 'clef' space is added to x_offset, which becomes the base for everything. It shifts everything equally.
        // We need something that pushes specific notes.
        // Greedy shifting happens when 'left' of current note overlaps 'right' of previous.
        // So let's put a VERY WIDE note at beat 0.
        // And then the triplet at beat 1?
        // But beat 1 is a new primary group. If I push beat 0, beat 1 stays at its metric position unless it overlaps.
        
        // Let's try:
        // Beat 0: Quarter note.
        // Beat 1: Quarter note (start of next group).
        // If Beat 0 is very wide, Beat 1 moves.
        // Beat 2: Quarter note.
        // Does Beat 2 move if Beat 1 moves?
        // In "Primary Group" logic: Beat 1 and Beat 2 are separate groups (in 4/4). 
        // So Beat 2 should NOT move just because Beat 1 moved (unless Beat 1 overlaps Beat 2).
        
        // Wait, the user wants "Equal spacing in primary groups".
        // This means INSIDE a primary group.
        // Example: Triplet of Quarters. (Spans 2 beats).
        // This is a Tuplet Group.
        // Example: 4 Eighths in 4/4 (2 beats).
        // They are 2 primary groups (2 eighths + 2 eighths).
        // If I shift 1st eighth. 2nd eighth should move.
        // 3rd eighth (start of next beat) should NOT move (unless collision).
        
        // So let's test a Tuplet of Quarters (3 notes spanning 2 beats).
        // If I shift Note 1, Note 2 should move.
        
        // Setup:
        // Beat 0: A massive note (e.g. with huge 'head_size' metric override for just this note? No, metrics are global).
        // Use a "Dotted" note with many dots?
        // Or just many notes.
        
        // Let's use 4x 32nd notes at start (Beat 0.0 - 0.125).
        // Then the Triplet Quarters.
        // Wait, Tuplet Quarters are huge (0.66 beats).
        
        // Let's use Triplet Eighths (unbeamed).
        // Beat 0: 3x Triplet Eighths. Unbeamed.
        // If I shift the first one (e.g. by having a previous note collide), the others should shift.
        
        // Construct:
        // Pushing element: Beat -1? No.
        // Let's use a TimeSig/Clef that is huge?
        // No, that shifts start position.
        
        // Let's use the same trick as 'test_beam_group_spacing_preservation'.
        // Overflow from previous beats.
        
        // Beat 0: 8x 32nds. (Duration 1/4).
        // Beat 1: 3x Triplet 8ths (Duration 1/4).
        // If 32nds are wide enough, they push the first Triplet 8th.
        
        // Layout width very small.
        let em = 10.0;
        let t8 = Duration::Tuplet(TupletSpec { n: 3, m: 2, base: NoteValue::Eighth });
        let th = Duration::Simple(NoteValue::ThirtySecond);
        
        // 8x 32nds
        for i in 0..8 {
             m.set_beat(i, Beat::note(th)).unwrap();
        }
        // 3x Triplet 8ths (Indices 8, 9, 10)
        for i in 0..3 {
             m.set_beat(8+i, Beat::note(t8)).unwrap();
        }
        
        // Ensure they are NOT beamed. (Default might beam them depending on 'plan_measure' logic).
        // In 4/4, 32nds might be beamed.
        // We want the Triplets to be Unbeamed for this test.
        // But 'plan_measure' usually auto-beams.
        // We can break the beam by inserting a rest? 
        // Or we just rely on the fact that we can force "primary group" logic even if beamed?
        // No, if beamed, they are already handled.
        // We need a case where they are NOT beamed but SHOULD be spaced evenly.
        // Quarter notes are never beamed.
        // Let's use Quarter Triplets.
        
        // Reset measure
        m = Measure::new(TimeSignature::FOUR_FOUR);
        // Beat 0: 4x 16th notes (to create crowding).
        // Beat 1..2: 3x Quarter Triplets.
        
        // 4x 16th
        let s = Duration::Simple(NoteValue::Sixteenth);
        for i in 0..4 {
            m.set_beat(i, Beat::note(s)).unwrap();
        }
        
        // 3x Quarter Triplets
        let tq = Duration::Tuplet(TupletSpec { n: 3, m: 2, base: NoteValue::Quarter });
        m.set_beat(4, Beat::note(tq)).unwrap();
        m.set_beat(5, Beat::note(tq)).unwrap();
        m.set_beat(6, Beat::note(tq)).unwrap();
        
        // Layout
        let opts = LayoutOpts {
            rect: Rect::from_min_max(Pos2::ZERO, Pos2::new(50.0, 100.0)), // Very narrow width
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset: 0.0,
            stem_length_factor: 3.5,
            stem_thickness_factor: 0.1,
            accent_displacement: 0.0,
            accent_below: false,
            proportional_spacing: true,
            debug_bbox: true,
            metrics: GlyphMetrics::debug(em),
        };
        
        let layout = build_measure_layout(&m, &opts);
        
        // Indices 4, 5, 6 are the triplet.
        let n4 = &layout.notes[4];
        let n5 = &layout.notes[5];
        let n6 = &layout.notes[6];
        
        let d1 = n5.center.x - n4.center.x;
        let d2 = n6.center.x - n5.center.x;
        
        // Without fix, n4 is pushed right (compressed against n5). So d1 < d2.
        // With fix, n5 should move too. So d1 == d2.
        assert!(
            (d1 - d2).abs() < 0.1,
            "Spacing in primary/tuplet group should be consistent. Got {:.2} vs {:.2}",
            d1, d2
        );
    }
}
