use crate::measure::duration;
use crate::measure::{BeatKind, Measure};
use crate::render::beat::draw_beat;
use crate::render::{beat, glyphs};
use crate::layout::render_plan::plan_measure;
use eframe::egui;
use eframe::egui::{Align2, Color32, FontId, Rangef, Rect, Stroke, pos2};

pub(crate) fn draw_measure(
    ui: &mut egui::Ui,
    font_id: &FontId,
    measure: &Measure,
    rect: Rect,
    cursor_idx: Option<usize>,
) {
    let color: Color32 = if ui.visuals().dark_mode { Color32::WHITE } else { Color32::BLACK };
    let painter = ui.painter();
    let y = rect.center().y;
    // staff line
    painter.hline(Rangef::new(rect.left(), rect.right()), y, Stroke::new(1.0, color));

    let min_size = 14.0 * ui.ctx().pixels_per_point(); // avoid unreadably small glyphs on HiDPI

    // Make inner rect scale with available height: keep a small vertical padding fraction
    let vpad = (rect.height() * 0.10).clamp(10.0, 200.0);
    let hpad = (rect.width() * 0.10).clamp(10.0, 30.0);
    let inner_rect = Rect::from_min_max(
        pos2(rect.left(), rect.top() + vpad),
        pos2(rect.right() - hpad, rect.bottom() - vpad),
    );

    // Derive font size from available height and width (scaled), keep family from provided font_id
    // Height-first sizing, modulated by window width so very narrow/wide windows adapt.
    let base_size_h = inner_rect.height() * 0.50;
    // Also cap by an estimate from width to prevent overflow on very narrow windows.
    let width_cap = (rect.width() * 0.1).max(min_size);
    let max_size = (inner_rect.height() * 0.80).max(min_size); // avoid overflowing inner rect, but not a fixed cap. also make sure it does not go below min_size
    let target_size = base_size_h.clamp(min_size, max_size.min(width_cap));
    let font_id = FontId::new(target_size, font_id.family.clone());
    let em = target_size;

    // Left-side: percussion clef and stacked time signature
    let clef_w = em * 0.9; // reserved visual width for clef
    let ts_digit_w = em * 0.35; // width per time-signature digit column

    // Draw clef
    let clef_x = inner_rect.left() + clef_w * 0.4;
    painter.text(
        pos2(clef_x, y),
        Align2::CENTER_CENTER,
        glyphs::GLYPH_CLEF_PERCUSSION.to_string(),
        font_id.clone(),
        color,
    );

    // Time signature digits (SMuFL)
    let ts = measure.time_signature();
    let top_digits = glyphs::ts_glyphs(ts.beats as u32);
    let bot_digits = glyphs::ts_glyphs(ts.beat_unit as u32);

    let ts_cols = top_digits.len().max(bot_digits.len()) as f32;
    let ts_w = ts_cols * ts_digit_w;
    let ts_left = inner_rect.left() + clef_w - (em * 0.2);

    // Top row (beats)
    for (i, ch) in top_digits.iter().enumerate() {
        // center narrower row within max columns
        let offset = (ts_cols - top_digits.len() as f32) * 0.5;
        let cx = ts_left + (i as f32 + 0.5 + offset) * ts_digit_w;
        painter.text(
            pos2(cx, y - em * 0.25),
            Align2::CENTER_CENTER,
            ch.to_string(),
            font_id.clone(),
            color,
        );
    }
    // Bottom row (beat unit)
    for (i, ch) in bot_digits.iter().enumerate() {
        let offset = (ts_cols - bot_digits.len() as f32) * 0.5;
        let cx = ts_left + (i as f32 + 0.5 + offset) * ts_digit_w;
        painter.text(
            pos2(cx, y + em * 0.25),
            Align2::CENTER_CENTER,
            ch.to_string(),
            font_id.clone(),
            color,
        );
    }

    // Content area after clef + time signature
    let content_left = ts_left + ts_w + (em * 0.2);
    let content_right = inner_rect.right();
    let content_w = (content_right - content_left).max(1.0);

    // Compute ticks
    let set = duration::default_duration_set();
    let cap_ticks = set.grid.ticks_per_measure(&ts);

    let plan = plan_measure(measure);

    // 1) Two-pass layout with per-beat extras (dots/flags/rest pad) and normalization
    let mut in_beam_flags: Vec<bool> = vec![false; measure.beats().len()];
    for g in &plan.beams {
        if g.beat_indices.len() >= 2 {
            for &idx in &g.beat_indices {
                if idx < in_beam_flags.len() {
                    in_beam_flags[idx] = true;
                }
            }
        }
    }

    // Build flat lists for layout; offset each beat by content_left
    let x_centers = crate::layout::calculate_x_centers(measure, content_w)
        .iter()
        .map(|&cx| cx + content_left)
        .collect::<Vec<_>>();

    // 2) Metrics for beams and stems
    let beam_render_opts = create_beam_render_opts(em, y, color, &font_id);
    let stem_dx = font_id.size * 0.13;
    // Precompute stem x positions for all beats (noteheads + stem offset)
    let stem_xs: Vec<f32> = x_centers.iter().map(|&cx| cx + stem_dx).collect();
    let stem_thickness = font_id.size * 0.03;

    // 3) Pass: draw beats (noteheads/rests) with beam-aware stems (flags suppressed when in beam)
    for (i, beat) in measure.beats().iter().copied().enumerate() {
        let in_beam = *in_beam_flags.get(i).unwrap_or(&false);
        let opts = beat::NoteRenderOpts {
            font_id: font_id.clone(),
            color,
            in_beam,
            stem_dx,
            stem_thickness,
        };
        if cap_ticks > 0 {
            draw_beat(&painter, pos2(x_centers[i], y), beat, opts);
        }
    }

    // 4) Draw beams per group (horizontal beams for stems up)
    for group in &plan.beams {
        // Full beams between adjacent stems according to continuity
        for (pair_idx, win) in group.beat_indices.windows(2).enumerate() {
            let i = win[0];
            let j = win[1];
            let levels = *group.continuity.get(pair_idx).unwrap_or(&0);
            if levels == 0 {
                continue;
            }
            // Extend the beam to the left and right a little bit such that it touches the stem
            // on both sides, otherwise it looks off sometimes.
            let offset = stem_thickness / 3.0;
            let x1 = stem_xs[i] - offset;
            let x2 = stem_xs[j] + offset;
            for lvl in 0..levels {
                draw_full_beam(&painter, x1, x2, lvl, &beam_render_opts);
            }
        }
    }

    // 4b) Draw partial beams where a note's beam count exceeds continuity
    {
        let stub_len = em * 0.20; // tune by eye
        for group in &plan.beams {
            if group.beat_indices.is_empty() {
                continue;
            }

            let note_idxs = &group.beat_indices;
            // Singleton notes should show flags only — no partial beam stubs.
            if note_idxs.len() < 2 {
                continue;
            }
            let counts = &group.beam_counts; // per note
            let cont = &group.continuity; // between neighbors

            for (local_k, &global_i) in note_idxs.iter().enumerate() {
                let count = *counts.get(local_k).unwrap_or(&0);
                if count <= 0 {
                    continue;
                }

                let left_cont = if local_k > 0 { *cont.get(local_k - 1).unwrap_or(&0) } else { 0 };
                let right_cont = if local_k + 1 < note_idxs.len() {
                    *cont.get(local_k).unwrap_or(&0)
                } else {
                    0
                };

                let stem_x = stem_xs[global_i];
                let is_first = local_k == 0;
                let is_last = local_k + 1 == note_idxs.len();

                for lvl in 0..count {
                    let connects_left = lvl < left_cont;
                    let connects_right = lvl < right_cont;

                    match (connects_left, connects_right) {
                        (true, true) => { /* fully connected at this level */ }
                        (true, false) => {
                            // Connected to left neighbor, missing to right
                            // Interior notes: no right stub; full beam terminates cleanly at this stem.
                            // Last note is an outer edge; we also suppress stubs there by policy.
                            // => do nothing
                        }
                        (false, true) => {
                            // Connected to right neighbor, missing to left
                            // Interior notes: no left stub; full beam terminates at this stem.
                            // First note is an outer edge; stubs suppressed by policy.
                            // => do nothing
                        }
                        (false, false) => {
                            // Not connected on either side at this level.
                            // On group edges, draw only the interior-facing stub; on interior notes, choose side by higher continuity (or prefer left if equal).
                            if is_first {
                                // First note: interior faces right
                                draw_full_beam(
                                    &painter,
                                    stem_x,
                                    stem_x + stub_len,
                                    lvl,
                                    &beam_render_opts,
                                );
                            } else if is_last {
                                // Last note: interior faces left
                                draw_full_beam(
                                    &painter,
                                    stem_x - stub_len,
                                    stem_x,
                                    lvl,
                                    &beam_render_opts,
                                );
                            } else {
                                if left_cont > right_cont {
                                    draw_full_beam(
                                        &painter,
                                        stem_x - stub_len,
                                        stem_x,
                                        lvl,
                                        &beam_render_opts,
                                    );
                                } else if right_cont > left_cont {
                                    draw_full_beam(
                                        &painter,
                                        stem_x,
                                        stem_x + stub_len,
                                        lvl,
                                        &beam_render_opts,
                                    );
                                } else {
                                    // Equal continuity: prefer left-facing stubs
                                    draw_full_beam(
                                        &painter,
                                        stem_x - stub_len,
                                        stem_x,
                                        lvl,
                                        &beam_render_opts,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 4c) Tuplet indicators (number and optional bracket)
    if !plan.tuplets.is_empty() {
        let staff_space = em * 0.25;
        let bracket_gap = 1.8 * staff_space;
        let hook_len = 0.8 * staff_space;
        let hook_dy = hook_len * 0.85;
        let number_font = FontId::new(font_id.size * 0.75, font_id.family.clone());

        for t in &plan.tuplets {
            // Horizontal span in Pixeln
            let (mut x_l, mut x_r) = (stem_xs[t.start], stem_xs[t.end]);
            let margin = em * 0.15;
            x_l -= margin;
            x_r += margin;

            // Precompute number glyphs and basic metrics
            let digits = glyphs::tuplet_glyphs(t.count);
            let num_chars = digits.chars().count() as f32;
            let num_width = num_chars * 0.6 * em;
            let pad = 0.25 * staff_space; // horizontal padding around digits inside the bracket gap
            let xc = 0.5 * (x_l + x_r);
            let mut gap_half = 0.5 * (num_width + 2.0 * pad);
            let min_seg = 0.5 * staff_space;
            let half_span = 0.5 * (x_r - x_l);
            if gap_half > half_span - min_seg {
                gap_half = (half_span - min_seg).max(0.0);
            }

            // Base Y without any accent-specific clearance
            let y_base = beam_render_opts.beam_y - beam_render_opts.thickness - bracket_gap;

            if !t.number_only() {
                // Bracketed case: keep previous behavior – raise whole bracket+number if any accent exists in span.
                let has_accent_in_group = measure.beats().iter().enumerate().any(|(i, b)| {
                    i >= t.start && i <= t.end && b.kind == BeatKind::Note && b.accented
                });
                let accent_clearance = (if has_accent_in_group { 1.4 } else { -0.4 }) * staff_space;
                let y_bracket = y_base - accent_clearance;

                let x_gap_l = (xc - gap_half).max(x_l);
                let x_gap_r = (xc + gap_half).min(x_r);
                if x_gap_l > x_l {
                    painter.line_segment(
                        [pos2(x_l, y_bracket), pos2(x_gap_l, y_bracket)],
                        Stroke::new(2.0, color),
                    );
                }
                if x_r > x_gap_r {
                    painter.line_segment(
                        [pos2(x_gap_r, y_bracket), pos2(x_r, y_bracket)],
                        Stroke::new(2.0, color),
                    );
                }
                painter.line_segment(
                    [pos2(x_l, y_bracket), pos2(x_l, y_bracket + hook_dy)],
                    Stroke::new(2.0, color),
                );
                painter.line_segment(
                    [pos2(x_r, y_bracket), pos2(x_r, y_bracket + hook_dy)],
                    Stroke::new(2.0, color),
                );

                // Number sits slightly inside the bracket gap
                let y_num = y_bracket + 0.5 * (em * 0.25);
                painter.text(
                    pos2(0.5 * (x_l + x_r), y_num),
                    Align2::CENTER_CENTER,
                    digits,
                    number_font.clone(),
                    color,
                );
            } else {
                // Number-only case: only lift the number if it would actually collide with an accent horizontally.
                let num_cx = 0.5 * (x_l + x_r);
                let num_half_w = 0.5 * num_width;
                let collides = (t.start..=t.end).any(|i| {
                    let b = measure.beats()[i];
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
                let y_num = (y_base - clearance) + 0.5 * (em * 0.25);
                painter.text(
                    pos2(0.5 * (x_l + x_r), y_num),
                    Align2::CENTER_CENTER,
                    digits,
                    number_font.clone(),
                    color,
                );
            }
        }
    }

    // 5) Cursor at current beat index (does not consume width) — blink over time
    if let Some(idx) = cursor_idx {
        if let Some(&x) = x_centers.get(idx) {
            // Blink parameters
            let blink_period = 1.0_f64; // seconds for a full on+off cycle
            let duty = 0.5_f64; // visible fraction of the period
            let t = ui.input(|i| i.time);
            let phase = (t % blink_period) / blink_period; // 0..1
            let visible = phase < duty;
            // Smooth fade near edges optional; for now a simple square wave with two alpha levels
            let alpha_on = 220u8;
            let alpha_off = 40u8; // faint but still present; set to 0 to hide completely
            let alpha = if visible { alpha_on } else { alpha_off };
            let top = inner_rect.top() + 0.5 * em;
            let bottom = inner_rect.bottom() - 0.5 * em;
            let base = if ui.visuals().dark_mode { Color32::WHITE } else { Color32::BLACK };
            let cursor_color = Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha);
            painter.vline(x, Rangef::new(top, bottom), Stroke::new(2.0, cursor_color));
            // Ensure animation progresses even without input
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
}

// Beaming metrics and helpers
#[derive(Copy, Clone)]
struct BeamRenderOpts {
    thickness: f32,
    gap: f32,
    beam_y: f32, // primary beam baseline (closest to notehead)
    color: Color32,
}

// TODO --> layout?
impl BeamRenderOpts {
    fn get_y_level(&self, lvl: u8) -> f32 {
        self.beam_y + (lvl as f32) * (self.thickness + self.gap)
    }
}

fn create_beam_render_opts(
    em: f32,
    y_center: f32,
    color: Color32,
    font_id: &FontId,
) -> BeamRenderOpts {
    // Approximate staff space relative to font size for a single-line staff context
    let staff_space = em * 0.25; // tuned by eye
    let thickness = 0.5 * staff_space; // Bravura ~0.5 sp
    let gap = 0.25 * staff_space; // distance between beams
    // Offset the top beam such that its top edge aligns with the end of the stem.
    // Because sometimes the beam's (left|right) edge does not perfectly align with the (left|right)
    // stem's (right|left) edge, it looks a bit off:
    let offset = thickness * 0.95;
    let beam_y = y_center - beat::get_default_stem_length(font_id) + offset;
    BeamRenderOpts { thickness, gap, beam_y, color }
}

fn draw_full_beam(p: &egui::Painter, x1: f32, x2: f32, lvl: u8, beam_opts: &BeamRenderOpts) {
    let left = x1.min(x2);
    let right = x1.max(x2);
    let y = beam_opts.get_y_level(lvl);
    let top = y - beam_opts.thickness;
    let rect = Rect::from_min_max(pos2(left, top), pos2(right, y));
    p.rect_filled(rect, 0.0, beam_opts.color);
}
