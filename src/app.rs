mod glyphs;

use crate::duration::{Duration, NoteValue};
use crate::measure::{Measure, TimeSignature};

use crate::app::glyphs::{
    GLYPH_AUGMENTATION_DOT, GLYPH_CLEF_PERCUSSION, GLYPH_NOTEHEAD_BLACK, flag_glyph_for_duration,
    rest_glyph_for_duration, ts_glyphs, tuplet_glyphs,
};
use crate::beaming::primary_boundaries;
use crate::duration;
use crate::duration::NoteValue::*;
use crate::measure::{Beat, BeatKind};
use eframe::egui::{Align2, Context, Key, Rangef, Stroke, global_theme_preference_buttons, pos2, Label};
use eframe::emath::Pos2;
use eframe::epaint::text::{FontInsert, InsertFontFamily};
use eframe::epaint::{Color32, FontFamily, FontId};
use eframe::{App, CreationContext, egui};
use egui::Rect;
use egui::containers::Frame;

pub struct Grooph {
    font_family: FontFamily,
    font_id: FontId,
    measure: Measure,
}

fn add_font(ctx: &Context) {
    ctx.add_font(FontInsert::new(
        "Bravura",
        egui::FontData::from_static(include_bytes!("../assets/fonts/Bravura.otf")),
        vec![InsertFontFamily {
            family: FontFamily::Name("music".into()),
            priority: egui::epaint::text::FontPriority::Highest,
        }],
    ));
}

fn dot_count_for_duration(d: Duration) -> u8 {
    match d {
        Duration::Dotted { dots, .. } => dots,
        _ => 0,
    }
}

// Beam-aware note rendering options
struct NoteRenderOpts {
    font_id: FontId,
    color: Color32,
    in_beam: bool,
    stem_dx: f32,
    stem_thickness: f32,
}

fn draw_beat(painter: &egui::Painter, pos: Pos2, beat: Beat, opts: NoteRenderOpts) {
    let duration = beat.duration;
    let glyph = match beat.kind {
        BeatKind::Note => GLYPH_NOTEHEAD_BLACK,
        BeatKind::Rest => rest_glyph_for_duration(duration),
    };

    // Render rests a bit smaller than notes
    let font_id = if beat.kind == BeatKind::Rest {
        &FontId::new(opts.font_id.size * 0.8, opts.font_id.family.clone())
    } else {
        &opts.font_id
    };

    // Draw the glyph (notehead or rest)
    painter.text(pos, Align2::CENTER_CENTER, glyph.to_string(), font_id.clone(), opts.color);

    // Draw augmentation dots for dotted durations (notes and rests)
    let dots = dot_count_for_duration(duration);
    if dots > 0 {
        // Horizontal spacing tuned by eye relative to font size
        // If this is a flagged note (not in a beam), push dots a bit further right so they don't collide with the flag tail.
        let has_flag_tail = beat.kind == BeatKind::Note
            && !opts.in_beam
            && flag_glyph_for_duration(duration).is_some();
        let first_dx = if has_flag_tail { font_id.size * 0.5 } else { font_id.size * 0.28 };
        let step_dx = font_id.size * 0.26;
        for i in 0..dots {
            let x = pos.x + first_dx + (i as f32) * step_dx;
            painter.text(
                pos2(x, pos.y - font_id.size * 0.1),
                Align2::CENTER_CENTER,
                GLYPH_AUGMENTATION_DOT.to_string(),
                font_id.clone(),
                opts.color,
            );
        }
    }

    // If this is a Note, draw a stem and possibly flags/tremolo
    if beat.kind == BeatKind::Note {
        let start = pos2(pos.x + opts.stem_dx, pos.y);
        let flag_glyph = flag_glyph_for_duration(duration);
        // It's visually more appealing to reduce the stem length a bit for notes that are neither
        // in a beam nor flagged.
        let stem_len_factor = if opts.in_beam || flag_glyph.is_some() { 1.0 } else { 0.85 };
        let default_stem_len = get_default_stem_length(font_id) * stem_len_factor;
        let end = pos2(start.x, pos.y - default_stem_len);
        painter.line_segment([start, end], Stroke::new(opts.stem_thickness, opts.color));

        // Flag glyph at the stem tip for short durations, only if not in a beam
        if !opts.in_beam {
            if let Some(flag) = flag_glyph {
                let flag_font = FontId::new(font_id.size * 1.0, font_id.family.clone());
                painter.text(
                    pos2(
                        start.x - opts.stem_thickness * 0.5,
                        pos.y - get_default_stem_length(font_id),
                    ),
                    Align2::LEFT_CENTER,
                    flag.to_string(),
                    flag_font,
                    opts.color,
                );
            }
        }

        // Tremolo slashes (single-note measured tremolo)
        if let Some(trem) = beat.tremolo {
            if trem.measured {
                let sl = trem.slashes.min(3);
                let dx = font_id.size * 0.12; // slight right offset per slash
                let dy = font_id.size * 0.12; // spacing along stem
                let ang = 0.6; // tilt factor (down-right)
                for i in 0..sl {
                    let y0 = (pos.y - default_stem_len) + (i as f32) * dy;
                    let x0 = start.x + (i as f32) * dx;
                    let len = font_id.size * 0.45;
                    painter.line_segment(
                        [pos2(x0, y0), pos2(x0 + len, y0 - len * ang)],
                        Stroke::new(2.0, opts.color),
                    );
                }
            }
        }
    }
}

fn draw_measure(
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
    let ts_digit_w = em * 0.7; // width per time-signature digit column

    // Draw clef
    let clef_x = inner_rect.left() + clef_w * 0.4;
    painter.text(
        pos2(clef_x, y),
        Align2::CENTER_CENTER,
        GLYPH_CLEF_PERCUSSION.to_string(),
        font_id.clone(),
        color,
    );

    // Time signature digits (SMuFL)
    let ts = measure.time_signature();
    let top_digits = ts_glyphs(ts.beats as u32);
    let bot_digits = ts_glyphs(ts.beat_unit as u32);

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
    let cap_ticks = ts.measure_duration_ticks();

    // 1) Two-pass layout with per-beat extras (dots/flags/rest pad) and normalization
    // Compute beaming flags first so we can budget extra width for flags only when not beamed
    let mut in_beam_flags: Vec<bool> = vec![false; measure.beats().len()];
    if let Some(bp) = measure.beam_plan() {
        for g in &bp.groups {
            // Only consider groups with at least two notes as "beamed".
            // Singleton groups should render with flags, not beams.
            if g.note_indices.len() >= 2 {
                for &idx in &g.note_indices {
                    if idx < in_beam_flags.len() {
                        in_beam_flags[idx] = true;
                    }
                }
            }
        }
    }

    // Build flat lists for layout; offset each beat by content_left
    let x_centers = calculate_x_centers(measure, content_w)
        .iter()
        .map(|&cx| cx + content_left)
        .collect::<Vec<_>>();

    // 2) Metrics for beams and stems
    let beam_render_opts = bream_render_opts(em, y, color, &font_id);
    let stem_dx = font_id.size * 0.13;
    // Precompute stem x positions for all beats (noteheads + stem offset)
    let stem_xs: Vec<f32> = x_centers.iter().map(|&cx| cx + stem_dx).collect();
    let stem_thickness = font_id.size * 0.03;

    // 3) Pass: draw beats (noteheads/rests) with beam-aware stems (flags suppressed when in beam)
    for (i, beat) in measure.beats().iter().copied().enumerate() {
        let in_beam = *in_beam_flags.get(i).unwrap_or(&false);
        let opts =
            NoteRenderOpts { font_id: font_id.clone(), color, in_beam, stem_dx, stem_thickness };
        if cap_ticks > 0 {
            draw_beat(&painter, pos2(x_centers[i], y), beat, opts);
        }
    }

    // 4) Draw beams per group (horizontal beams for stems up)
    if let Some(bp) = measure.beam_plan() {
        for group in &bp.groups {
            // Full beams between adjacent stems according to continuity
            for (pair_idx, win) in group.note_indices.windows(2).enumerate() {
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
    }

    // 4b) Draw partial beams where a note's beam count exceeds continuity
    if let Some(bp) = measure.beam_plan() {
        let stub_len = em * 0.20; // tune by eye
        for group in &bp.groups {
            if group.note_indices.is_empty() {
                continue;
            }

            let note_idxs = &group.note_indices;
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

    // 4c) Tuplet indicators (number and optional bracket) — stems up only, no staggering
    // Helper: compute onset ticks per beat and primary boundaries (replicated logic)
    let beats = measure.beats();
    let onsets = set.compute_onset_ticks(beats);
    let boundaries = primary_boundaries(set, &ts);

    struct TupGroup {
        start: usize,
        end: usize,
        n: u8,
        m: u8,
        base: NoteValue,
        contains_rest: bool,
    }
    let mut groups: Vec<TupGroup> = Vec::new();
    let mut i = 0usize;
    while i < beats.len() {
        let Duration::Tuplet { n, m, base } = beats[i].duration else {
            i += 1;
            continue;
        };
        // Find maximal run of same-spec tuplets starting at i
        let mut k = i;
        while k < beats.len() {
            match beats[k].duration {
                Duration::Tuplet { n: nn, m: mm, base: bb } if nn == n && mm == m && bb == base => {
                    k += 1;
                }
                _ => break,
            }
        }
        // Split the run [i..k) into consecutive groups of exactly n elements when possible
        let group_size = n as usize;
        let mut start = i;
        while start < k {
            let end = (start + group_size).min(k) - 1;
            let mut has_rest = false;
            for t in start..=end {
                if beats[t].kind == BeatKind::Rest {
                    has_rest = true;
                    break;
                }
            }
            groups.push(TupGroup { start, end, n, m, base, contains_rest: has_rest });
            start = end + 1;
        }
        i = k;
    }

    if !groups.is_empty() {
        let beam_plan = measure.beam_plan();
        let staff_space = em * 0.25;
        let bracket_gap = 0.9 * staff_space;
        let hook_len = 0.8 * staff_space;
        let hook_dy = hook_len * 0.85;
        let number_font = FontId::new(font_id.size * 0.75, font_id.family.clone());

        for g in groups {
            // Derive properties
            // Span crosses primary boundary?
            let start_on = *onsets.get(g.start).unwrap_or(&0);
            let end_on = *onsets.get(g.end).unwrap_or(&start_on)
                + set.grid.ticks_of(&beats[g.end].duration).unwrap_or(0);
            let spans_primary = boundaries.iter().any(|&bd| bd > start_on && bd < end_on);

            // Collect note indices participating (exclude rests)
            let mut tup_note_idxs: Vec<usize> = Vec::new();
            for k in g.start..=g.end {
                if beats[k].kind == BeatKind::Note {
                    tup_note_idxs.push(k);
                }
            }

            // Determine fully_beamed
            let mut fully_beamed = false;
            if !g.contains_rest && tup_note_idxs.len() >= 2 {
                if let Some(bp) = &beam_plan {
                    'outer: for bg in &bp.groups {
                        // All tuplet notes must be inside this beam group
                        if tup_note_idxs.iter().all(|idx| bg.note_indices.contains(idx)) {
                            // Build a map from note index to local position
                            let mut pos_map = std::collections::HashMap::new();
                            for (li, gi) in bg.note_indices.iter().enumerate() {
                                pos_map.insert(*gi, li);
                            }
                            // Verify continuity (>=1) along the chain between consecutive tuplet notes
                            let mut ok = true;
                            for pair in tup_note_idxs.windows(2) {
                                let a = pair[0];
                                let b = pair[1];
                                let la = *pos_map.get(&a).unwrap();
                                let lb = *pos_map.get(&b).unwrap();
                                if la >= lb {
                                    ok = false;
                                    break;
                                }
                                // Require continuity >=1 across every adjacent link from la..lb-1
                                for cidx in la..lb {
                                    if *bg.continuity.get(cidx).unwrap_or(&0) < 1 {
                                        ok = false;
                                        break;
                                    }
                                }
                                if !ok {
                                    break;
                                }
                            }
                            if ok {
                                fully_beamed = true;
                                break 'outer;
                            }
                        }
                    }
                }
            }

            // Auto policy: number-only when fully_beamed and no rest (ignore primary boundary crossing for consistency)
            let number_only = fully_beamed && !g.contains_rest;

            // Horizontal span
            let (mut x_l, mut x_r) = (stem_xs[g.start], stem_xs[g.end]);
            // Add a small outward margin
            let margin = em * 0.25;
            x_l -= margin;
            x_r += margin;

            let y_bracket = beam_render_opts.beam_y - beam_render_opts.thickness - bracket_gap;

            // Draw bracket if needed
            if !number_only {
                let x1 = x_l;
                let x2 = x_r;
                painter.line_segment(
                    [pos2(x1, y_bracket), pos2(x2, y_bracket)],
                    Stroke::new(2.0, color),
                );
                // Hooks (downwards toward notes)
                painter.line_segment(
                    [pos2(x1, y_bracket), pos2(x1, y_bracket + hook_dy)],
                    Stroke::new(2.0, color),
                );
                painter.line_segment(
                    [pos2(x2, y_bracket), pos2(x2, y_bracket + hook_dy)],
                    Stroke::new(2.0, color),
                );
            }

            // Draw number (Bravura tuplet digits), centered
            let digits = tuplet_glyphs(g.n);
            // Place slightly above bracket (or above beam line if number-only)
            let y_num = if number_only {
                y_bracket + 0.5 * (em * 0.25)
            } else {
                y_bracket - 0.50 * (em * 0.25)
            };
            painter.text(
                pos2(0.5 * (x_l + x_r), y_num),
                Align2::CENTER_CENTER,
                digits,
                number_font.clone(),
                color,
            );
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
            let top = inner_rect.top();
            let bottom = inner_rect.bottom();
            let base = if ui.visuals().dark_mode { Color32::WHITE } else { Color32::BLACK };
            let cursor_color = Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha);
            painter.vline(x, Rangef::new(top, bottom), Stroke::new(2.0, cursor_color));
            // Ensure animation progresses even without input
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
}

fn calculate_x_centers(measure: &Measure, content_w: f32) -> Vec<f32> {
    let durations: Vec<Duration> = measure.beats().iter().map(|b| b.duration).collect();

    // Normalize to fit the content box
    let total: f32 = durations.len() as f32;
    let scale = if total > 0.0 { content_w / total } else { 1.0 };

    let mut x_centers: Vec<f32> = vec![0.0; durations.len()];
    let mut run = 0.0_f32;
    for i in 0..durations.len() {
        let density = 0.8; // lower for denser layout
        let cell_w = density * scale;
        x_centers[i] = run + cell_w * 0.5;
        run += cell_w;
    }
    x_centers
}

// Beaming metrics and helpers
#[derive(Copy, Clone)]
struct BeamRenderOpts {
    thickness: f32,
    gap: f32,
    beam_y: f32, // primary beam baseline (closest to notehead)
    color: Color32,
}

impl BeamRenderOpts {
    fn get_y_level(&self, lvl: u8) -> f32 {
        self.beam_y + (lvl as f32) * (self.thickness + self.gap)
    }
}

fn bream_render_opts(em: f32, y_center: f32, color: Color32, font_id: &FontId) -> BeamRenderOpts {
    // Approximate staff space relative to font size for a single-line staff context
    let staff_space = em * 0.25; // tuned by eye
    let thickness = 0.5 * staff_space; // Bravura ~0.5 sp
    let gap = 0.25 * staff_space; // distance between beams
    // Offset the top beam such that its top edge aligns with the end of the stem.
    // Because sometimes the beam's (left|right) edge does not perfectly align with the (left|right)
    // stem's (right|left) edge, it looks a bit off:
    let offset = thickness * 0.95;
    let beam_y = y_center - get_default_stem_length(font_id) + offset;
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

fn get_default_stem_length(font_id: &FontId) -> f32 {
    font_id.size * 0.9 // proportional stem length
}

impl App for Grooph {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            Frame::canvas(ui.style()).show(ui, |ui| {
                global_theme_preference_buttons(ui);
                ui.label(
                    "Keybindings: \n\
                Arrow keys: Move cursor\n\
                Del/Backspace: Remove note\n\
                Space: Toggle between note and rest\n\
                Num1/Num2: Split/merge note\n\
                Period: Toggle dotted\n\
                Q: Insert 1/4 note\n",
                );
                ui.input(|i| {
                    let beats_len = self.measure.beats().len();
                    let total_len = beats_len;
                    if total_len > 0 {
                        // Navigation over committed beats only
                        let mut pos = self.measure.position();
                        if i.key_pressed(Key::ArrowLeft) {
                            pos = pos.saturating_sub(1);
                        }
                        if i.key_pressed(Key::ArrowRight) {
                            let max_idx = total_len.saturating_sub(1);
                            if pos < max_idx {
                                pos += 1;
                            }
                        }
                        if i.key_pressed(Key::Home) {
                            pos = 0;
                        }
                        if i.key_pressed(Key::End) {
                            pos = total_len.saturating_sub(1);
                        }
                        self.measure.set_position(pos);

                        // Edits apply only when cursor is on a committed beat
                        let idx = self.measure.position().min(beats_len.saturating_sub(1));
                        if i.key_pressed(Key::Delete) {
                            // Delete beat at cursor and shift subsequent beats left
                            self.measure.remove(idx);
                            // Do not move cursor; it now points to the next beat (like text editors)
                            let new_len = self.measure.beats().len();
                            let new_pos = self.measure.position().min(new_len.saturating_sub(1));
                            self.measure.set_position(new_pos);
                        }
                        if i.key_pressed(Key::Space) {
                            // Toggle between note and rest at cursor (preserve duration)
                            self.measure.toggle_beat_kind(idx);
                        }
                        if i.key_pressed(Key::Backspace) {
                            // Remove beat at cursor
                            self.measure.remove(idx);
                            // Move cursor left, like a text editor caret
                            let new_len = self.measure.beats().len();
                            let new_pos = self
                                .measure
                                .position()
                                .saturating_sub(1)
                                .min(new_len.saturating_sub(1));
                            self.measure.set_position(new_pos);
                        }
                        if i.key_pressed(Key::Q) {
                            // Set a quarter note at the current cursor position. If it cannot be set, ignore.
                            let quarter = Duration::Simple(Quarter);
                            let _ = self.measure.set_beat_at(idx, Beat::note(quarter));
                        }
                        if i.key_pressed(Key::Num2) {
                            // Split the beat at the current cursor into two halves (e.g., 1/4 -> 1/8 + 1/8). If not possible, ignore.
                            let _ = self.measure.split_beat_by_two(idx);
                        }
                        if i.key_pressed(Key::Num1) {
                            // Unsplit (merge) the beat at the current cursor with the next one if possible (inverse of split by two).
                            let _ = self.measure.unsplit_beat_by_two(idx);
                        }
                        if i.key_pressed(Key::Period) {
                            // Toggle dotted (1 dot) for the current beat. If it cannot be changed (would overflow or unfillable), ignore.
                            let _ = self.measure.toggle_dotted_at(idx);
                        }
                    }
                });
                let idx_opt = Some(self.measure.position());

                // Top-right overlay label showing absolute beat position at the cursor
                let mut beat_text = String::from("-");
                let idx = self.measure.position();
                let positions = self.measure.beat_positions();
                if idx < positions.len() {
                    let v = positions[idx] as f32;
                    let mut s = format!("{:.3}", v);
                    // Trim trailing zeros and optional dot for a cleaner look
                    while s.ends_with('0') {
                        s.pop();
                    }
                    if s.ends_with('.') {
                        s.pop();
                    }
                    beat_text = s;
                }
                ui.add(Label::new(format!("Beat: {}", beat_text)));

                let (_id, rect) = ui.allocate_space(ui.available_size());
                draw_measure(ui, &self.font_id, &self.measure, rect, idx_opt);
            });
        });
    }
}

impl Grooph {
    pub fn new(cc: &CreationContext) -> Self {
        add_font(&cc.egui_ctx);
        let ff = FontFamily::Name("music".into());
        let mut measure = Measure::new(TimeSignature::SEVEN_EIGHT);
        measure.add_beat(Beat::note(Duration::Tuplet { n: 3, m: 2, base: Eighth })).unwrap();
        measure.add_beat(Beat::note(Duration::Tuplet { n: 3, m: 2, base: Eighth })).unwrap();
        measure.add_beat(Beat::note(Duration::Tuplet { n: 3, m: 2, base: Sixteenth })).unwrap();
        measure.add_beat(Beat::rest(Duration::Tuplet { n: 3, m: 2, base: Sixteenth })).unwrap();

        // measure.add_beat(Beat::note(Duration::Tuplet { n: 3, m: 2, base: Eighth })).unwrap();
        // measure.add_beat(Beat::note(Duration::Tuplet { n: 3, m: 2, base: Eighth })).unwrap();
        // measure.add_beat(Beat::note(Duration::Tuplet { n: 3, m: 2, base: Eighth })).unwrap();
        //
        // measure.add_beat(Beat::note(Duration::Tuplet { n: 3, m: 2, base: Eighth })).unwrap();
        // measure.add_beat(Beat::note(Duration::Tuplet { n: 3, m: 2, base: Eighth })).unwrap();
        // measure.add_beat(Beat::note(Duration::Tuplet { n: 3, m: 2, base: Eighth })).unwrap();
        //
        // measure.add_beat(Beat::note(Duration::Tuplet { n: 3, m: 2, base: Sixteenth })).unwrap();
        // measure.add_beat(Beat::note(Duration::Tuplet { n: 3, m: 2, base: Sixteenth })).unwrap();
        // measure.add_beat(Beat::note(Duration::Tuplet { n: 3, m: 2, base: Sixteenth })).unwrap();

        Self { font_family: ff.clone(), font_id: FontId::new(16.0, ff), measure }
    }
}
