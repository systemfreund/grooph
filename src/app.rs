mod glyphs;

use crate::duration::{e, s, t8, Duration, NoteValue};
use crate::measure::{Measure, TimeSignature};

use crate::app::glyphs::{
    GLYPH_AUGMENTATION_DOT, GLYPH_CLEF_PERCUSSION, GLYPH_NOTEHEAD_BLACK, flag_glyph_for_duration,
    rest_glyph_for_duration, ts_glyphs, tuplet_glyphs,
};
use crate::duration;
use crate::duration::NoteValue::*;
use crate::duration::human_readable;
use crate::measure::{Beat, BeatKind};
use eframe::egui::{
    Align2, Context, Key, Label, Rangef, Stroke, global_theme_preference_switch, pos2,
};
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
    cursor_idx: usize,
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
    let cap_ticks = set.grid.ticks_per_measure(&ts);

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
        let Duration::Tuplet { n, m, base: _ } = beats[i].duration else {
            i += 1;
            continue;
        };
        // Find maximal run of tuplets with the same ratio (n, m), ignoring base note value
        let mut k = i;
        while k < beats.len() {
            match beats[k].duration {
                Duration::Tuplet { n: nn, m: mm, base: _ } if nn == n && mm == m => {
                    k += 1;
                }
                _ => break,
            }
        }
        // Split the run [i..k) into logical tuplet groups by accumulating ticks
        let mut start = i;
        while start < k {
            let first_dur = beats[start].duration;
            // One full tuplet group spans `n * ticks(first_element)` ticks.
            let target_ticks: u32 =
                set.grid.ticks_of(&first_dur).unwrap_or(0).saturating_mul(n as u32);
            let mut acc_ticks: u32 = 0;
            let mut end = start;
            let mut has_rest = false;
            while end < k {
                if beats[end].kind == BeatKind::Rest {
                    has_rest = true;
                }
                let dt = set.grid.ticks_of(&beats[end].duration).unwrap_or(0);
                acc_ticks = acc_ticks.saturating_add(dt);
                if acc_ticks >= target_ticks {
                    break;
                }
                end += 1;
            }
            // Push the group [start..=end]. In well-formed rhythms acc_ticks should equal target_ticks.
            groups.push(TupGroup {
                start,
                end,
                n,
                m,
                base: beats[start].duration.base_note(),
                contains_rest: has_rest,
            });
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

        // Precompute per-group note indices and fully_beamed flags
        let mut tup_note_idxs_vec: Vec<Vec<usize>> = Vec::with_capacity(groups.len());
        for g in groups.iter() {
            let mut idxs = Vec::new();
            for k in g.start..=g.end {
                if beats[k].kind == BeatKind::Note {
                    idxs.push(k);
                }
            }
            tup_note_idxs_vec.push(idxs);
        }
        let mut fully_beamed_vec: Vec<bool> = vec![false; groups.len()];
        if let Some(bp) = &beam_plan {
            for gi in 0..groups.len() {
                let g = &groups[gi];
                let idxs = &tup_note_idxs_vec[gi];
                if g.contains_rest || idxs.len() < 2 {
                    continue;
                }
                'outer: for bg in &bp.groups {
                    if idxs.iter().all(|idx| bg.note_indices.contains(idx)) {
                        let mut pos_map = std::collections::HashMap::new();
                        for (li, gi2) in bg.note_indices.iter().enumerate() {
                            pos_map.insert(*gi2, li);
                        }
                        let mut ok = true;
                        for pair in idxs.windows(2) {
                            let a = pair[0];
                            let b = pair[1];
                            let la = *pos_map.get(&a).unwrap();
                            let lb = *pos_map.get(&b).unwrap();
                            if la >= lb {
                                ok = false;
                                break;
                            }
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
                            fully_beamed_vec[gi] = true;
                            break 'outer;
                        }
                    }
                }
            }
        }
        // Initial number-only per group
        let mut number_only_vec: Vec<bool> = groups
            .iter()
            .enumerate()
            .map(|(i, g)| fully_beamed_vec[i] && !g.contains_rest)
            .collect();
        // Second pass: if adjacent groups are beamed together continuously, force brackets for both
        if let Some(bp) = &beam_plan {
            for i in 0..(groups.len().saturating_sub(1)) {
                let g_left = &groups[i];
                let g_right = &groups[i + 1];
                // If groups are immediately adjacent and beamed together, force brackets for both,
                // regardless of whether one of them contains rests (the rest-containing group already
                // brackets by default; this ensures the all-notes neighbor also brackets for clarity).
                if g_left.end + 1 != g_right.start {
                    continue;
                }
                let left_idxs = &tup_note_idxs_vec[i];
                let right_idxs = &tup_note_idxs_vec[i + 1];
                if left_idxs.is_empty() || right_idxs.is_empty() {
                    continue;
                }
                let last_left = *left_idxs.last().unwrap();
                let first_right = right_idxs[0];
                let mut beamed_together = false;
                'bgscan: for bg in &bp.groups {
                    if bg.note_indices.contains(&last_left)
                        && bg.note_indices.contains(&first_right)
                    {
                        // map to local positions
                        let mut pos_map = std::collections::HashMap::new();
                        for (li, gi2) in bg.note_indices.iter().enumerate() {
                            pos_map.insert(*gi2, li);
                        }
                        let la = *pos_map.get(&last_left).unwrap();
                        let lb = *pos_map.get(&first_right).unwrap();
                        if la < lb {
                            let mut ok = true;
                            for cidx in la..lb {
                                if *bg.continuity.get(cidx).unwrap_or(&0) < 1 {
                                    ok = false;
                                    break;
                                }
                            }
                            if ok {
                                beamed_together = true;
                                break 'bgscan;
                            }
                        }
                    }
                }
                if beamed_together {
                    number_only_vec[i] = false;
                    number_only_vec[i + 1] = false;
                }
            }
        }

        // Third pass: if a tuplet group's boundary note is beamed to an external non-tuplet neighbor,
        // force a bracket to visually separate from that neighbor (addresses the case where a preceding
        // eighth note beams into the triplet group).
        if let Some(bp) = &beam_plan {
            for gi in 0..groups.len() {
                if !number_only_vec[gi] {
                    continue;
                }
                let g = &groups[gi];
                let first_idx = g.start;
                let last_idx = g.end;

                let mut external_beam = false;
                // Check left neighbor
                if first_idx > 0 {
                    let left_idx = first_idx - 1;
                    // Only relevant if neighbor is a Note and not part of this tuplet group
                    if beats[left_idx].kind == BeatKind::Note {
                        // Ensure neighbor is not the same tuple group (it's outside by construction);
                        // still, verify it isn't any tuplet with same (n,m) immediately preceding which
                        // would have formed a group earlier (defensive check not strictly necessary).
                        let is_same_tuplet = match beats[left_idx].duration {
                            Duration::Tuplet { n, m, .. } => n == g.n && m == g.m,
                            _ => false,
                        };
                        if !is_same_tuplet {
                            'bgscan_l: for bg in &bp.groups {
                                if bg.note_indices.contains(&left_idx)
                                    && bg.note_indices.contains(&first_idx)
                                {
                                    // Map to local positions and check continuity between them
                                    let mut pos_map = std::collections::HashMap::new();
                                    for (li, gi2) in bg.note_indices.iter().enumerate() {
                                        pos_map.insert(*gi2, li);
                                    }
                                    let la = *pos_map.get(&left_idx).unwrap();
                                    let lb = *pos_map.get(&first_idx).unwrap();
                                    if la < lb {
                                        let mut ok = true;
                                        for cidx in la..lb {
                                            if *bg.continuity.get(cidx).unwrap_or(&0) < 1 {
                                                ok = false;
                                                break;
                                            }
                                        }
                                        if ok {
                                            external_beam = true;
                                            break 'bgscan_l;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Check right neighbor if not already determined
                if !external_beam && last_idx + 1 < beats.len() {
                    let right_idx = last_idx + 1;
                    if beats[right_idx].kind == BeatKind::Note {
                        let is_same_tuplet = match beats[right_idx].duration {
                            Duration::Tuplet { n, m, .. } => n == g.n && m == g.m,
                            _ => false,
                        };
                        if !is_same_tuplet {
                            'bgscan_r: for bg in &bp.groups {
                                if bg.note_indices.contains(&last_idx)
                                    && bg.note_indices.contains(&right_idx)
                                {
                                    let mut pos_map = std::collections::HashMap::new();
                                    for (li, gi2) in bg.note_indices.iter().enumerate() {
                                        pos_map.insert(*gi2, li);
                                    }
                                    let la = *pos_map.get(&last_idx).unwrap();
                                    let lb = *pos_map.get(&right_idx).unwrap();
                                    if la < lb {
                                        let mut ok = true;
                                        for cidx in la..lb {
                                            if *bg.continuity.get(cidx).unwrap_or(&0) < 1 {
                                                ok = false;
                                                break;
                                            }
                                        }
                                        if ok {
                                            external_beam = true;
                                            break 'bgscan_r;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if external_beam {
                    number_only_vec[gi] = false;
                }
            }
        }

        for (gi, g) in groups.iter().enumerate() {
            let number_only = number_only_vec[gi];

            // Horizontal span
            let (mut x_l, mut x_r) = (stem_xs[g.start], stem_xs[g.end]);
            // Add a small outward margin
            let margin = em * 0.25;
            x_l -= margin;
            x_r += margin;

            let y_bracket = beam_render_opts.beam_y - beam_render_opts.thickness - bracket_gap;

            // Prepare number glyphs and measure width to reserve a centered gap in the bracket
            let digits = tuplet_glyphs(g.n);
            // Approximate numeral width: ~0.6em per glyph (good enough for SMuFL digits), supports multi-digit tuplets
            let num_chars = digits.chars().count() as f32;
            let num_width = num_chars * 0.6 * em;
            let pad = 0.25 * staff_space; // horizontal padding around digits inside the bracket gap
            let xc = 0.5 * (x_l + x_r);
            let mut gap_half = 0.5 * (num_width + 2.0 * pad);
            // Ensure we don't exceed span; keep a minimal segment on each side if possible
            let min_seg = 0.5 * staff_space;
            let half_span = 0.5 * (x_r - x_l);
            if gap_half > half_span - min_seg {
                gap_half = (half_span - min_seg).max(0.0);
            }

            // Draw bracket if needed: split into left and right segments with a centered gap for the number
            if !number_only {
                let x_gap_l = (xc - gap_half).max(x_l);
                let x_gap_r = (xc + gap_half).min(x_r);
                // Left segment
                if x_gap_l > x_l {
                    painter.line_segment(
                        [pos2(x_l, y_bracket), pos2(x_gap_l, y_bracket)],
                        Stroke::new(2.0, color),
                    );
                }
                // Right segment
                if x_r > x_gap_r {
                    painter.line_segment(
                        [pos2(x_gap_r, y_bracket), pos2(x_r, y_bracket)],
                        Stroke::new(2.0, color),
                    );
                }
                // Hooks (downwards toward notes) remain at full-span endpoints
                painter.line_segment(
                    [pos2(x_l, y_bracket), pos2(x_l, y_bracket + hook_dy)],
                    Stroke::new(2.0, color),
                );
                painter.line_segment(
                    [pos2(x_r, y_bracket), pos2(x_r, y_bracket + hook_dy)],
                    Stroke::new(2.0, color),
                );
            }

            // Draw number (Bravura tuplet digits), centered at the number-only position even when bracketed
            let y_num = y_bracket + 0.5 * (em * 0.25);
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

fn calculate_x_centers(measure: &Measure, content_w: f32) -> Vec<f32> {
    let durations: Vec<Duration> = measure.beats().iter().map(|b| b.duration).collect();

    // Normalize to fit the content box
    let total: f32 = durations.len() as f32;
    let cell_w = if total > 0.0 { content_w / total } else { 1.0 };

    let mut x_centers: Vec<f32> = vec![0.0; durations.len()];
    let mut run = 0.0_f32;
    for i in 0..durations.len() {
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
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            global_theme_preference_switch(ui);
        });

        egui::TopBottomPanel::top("info").show(ctx, |ui| {
            ui.label(
                "Keybindings: \n\
                Arrow keys: Move cursor\n\
                Del/Backspace: Remove note\n\
                Space: Toggle between note and rest\n\
                0: Toggle all beats to notes/rests\n\
                1-4: Set duration (1=1/4, 2=1/8, 3=1/16, 4=1/32)\n\
                Period: Toggle dotted\n",
            );

            // Label showing absolute beat position at the cursor and human-readable duration/kind
            let mut beat_text = String::from("-");
            let idx = self.cursor_idx;
            let positions = self.measure.beat_positions();
            if idx < positions.len() {
                let v = positions[idx];
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
            let mut label = format!("Beat: {}", beat_text);
            if idx < self.measure.beats().len() {
                let b = self.measure.beats()[idx];
                let desc = human_readable(&b.duration);
                let kind = match b.kind {
                    BeatKind::Note => "note",
                    BeatKind::Rest => "rest",
                };
                label = format!("Beat: {}, {} {}", beat_text, desc, kind);
            }
            ui.add(Label::new(label));
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            Frame::canvas(ui.style()).show(ui, |ui| {
                let (_id, rect) = ui.allocate_space(ui.available_size());
                draw_measure(ui, &self.font_id, &self.measure, rect, Some(self.cursor_idx));
            });
        });

        ctx.input(|i| {
            let beats_len = self.measure.beats().len();
            let total_len = beats_len;
            if total_len > 0 {
                // Navigation over committed beats only
                let mut pos = self.cursor_idx;
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
                self.cursor_idx = pos;

                // Edits apply only when cursor is on a committed beat
                let idx = self.cursor_idx.min(beats_len.saturating_sub(1));
                if i.key_pressed(Key::Delete) {
                    // Remove beat at cursor
                    self.measure.remove(idx);
                    // Move cursor right
                    let new_pos = (self.measure.beats().len() - 1).min(self.cursor_idx + 1);
                    self.cursor_idx = new_pos;
                }
                if i.key_pressed(Key::Backspace) {
                    // Remove beat at cursor
                    self.measure.remove(idx);
                    // Move cursor left
                    let new_len = self.measure.beats().len();
                    let new_pos = self.cursor_idx.saturating_sub(1).min(new_len - 1);
                    self.cursor_idx = new_pos;
                }
                if i.key_pressed(Key::Space) {
                    // Toggle between note and rest at cursor (preserve duration)
                    self.measure.toggle_beat_kind(idx);
                }
                if i.key_pressed(Key::Num0) {
                    // Toggle ALL beats to either notes or rests based on current majority; tie resolved by first beat
                    if beats_len > 0 {
                        // Decide target kind using an immutable snapshot
                        let (target_kind, durs, kinds) = {
                            let beats_view = self.measure.beats();
                            let mut notes = 0usize;
                            let mut rests = 0usize;
                            for b in beats_view.iter() {
                                match b.kind {
                                    BeatKind::Note => notes += 1,
                                    BeatKind::Rest => rests += 1,
                                }
                            }
                            let target = if notes > rests {
                                BeatKind::Rest
                            } else if rests > notes {
                                BeatKind::Note
                            } else {
                                // No majority: decide opposite of the first beat
                                match beats_view[0].kind {
                                    BeatKind::Note => BeatKind::Rest,
                                    BeatKind::Rest => BeatKind::Note,
                                }
                            };
                            let durs: Vec<_> = beats_view.iter().map(|b| b.duration).collect();
                            let kinds: Vec<_> = beats_view.iter().map(|b| b.kind).collect();
                            (target, durs, kinds)
                        };
                        // Apply changes using stored durations to avoid borrow conflicts
                        for (bi, (&dur, &kind)) in durs.iter().zip(kinds.iter()).enumerate() {
                            if kind != target_kind {
                                let new_beat = match target_kind {
                                    BeatKind::Note => Beat::note(dur),
                                    BeatKind::Rest => Beat::rest(dur),
                                };
                                let _ = self.measure.set_beat_at(bi, new_beat);
                            }
                        }
                    }
                }
                // Numeric duration assignment: 1=1/4, 2=1/8, 3=1/16, 4=1/32
                // Preserve BeatKind (note/rest). When current beat is a tuplet, preserve (n,m)
                // and only change base for keys 2–4; key 1 is ignored on tuplets (quarter-tuplets unsupported).
                if i.key_pressed(Key::Num1) {
                    let cur = self.measure.beats()[idx];
                    // If tuplet -> ignore (no quarter tuplet support)
                    let new_dur_opt = match cur.duration {
                        Duration::Tuplet { .. } => None,
                        _ => Some(Duration::Simple(Quarter)),
                    };
                    if let Some(new_dur) = new_dur_opt {
                        let new_beat = match cur.kind {
                            BeatKind::Note => Beat::note(new_dur),
                            BeatKind::Rest => Beat::rest(new_dur),
                        };
                        let _ = self.measure.set_beat_at(idx, new_beat);
                    }
                }
                if i.key_pressed(Key::Num2) {
                    let cur = self.measure.beats()[idx];
                    let new_dur = match cur.duration {
                        Duration::Tuplet { n, m, base: _ } => {
                            Duration::Tuplet { n, m, base: Eighth }
                        }
                        _ => Duration::Simple(Eighth),
                    };
                    let new_beat = match cur.kind {
                        BeatKind::Note => Beat::note(new_dur),
                        BeatKind::Rest => Beat::rest(new_dur),
                    };
                    let _ = self.measure.set_beat_at(idx, new_beat);
                }
                if i.key_pressed(Key::Num3) {
                    let cur = self.measure.beats()[idx];
                    let new_dur = match cur.duration {
                        Duration::Tuplet { n, m, base: _ } => {
                            Duration::Tuplet { n, m, base: Sixteenth }
                        }
                        _ => Duration::Simple(Sixteenth),
                    };
                    let new_beat = match cur.kind {
                        BeatKind::Note => Beat::note(new_dur),
                        BeatKind::Rest => Beat::rest(new_dur),
                    };
                    let _ = self.measure.set_beat_at(idx, new_beat);
                }
                if i.key_pressed(Key::Num4) {
                    let cur = self.measure.beats()[idx];
                    let new_dur = match cur.duration {
                        Duration::Tuplet { n, m, base: _ } => {
                            Duration::Tuplet { n, m, base: ThirtySecond }
                        }
                        _ => Duration::Simple(ThirtySecond),
                    };
                    let new_beat = match cur.kind {
                        BeatKind::Note => Beat::note(new_dur),
                        BeatKind::Rest => Beat::rest(new_dur),
                    };
                    let _ = self.measure.set_beat_at(idx, new_beat);
                }
                if i.key_pressed(Key::Period) {
                    // Toggle dotted (1 dot) for the current beat. If it cannot be changed (would overflow or unfillable), ignore.
                    let _ = self.measure.toggle_dotted_at(idx);
                }
            }
        });
    }
}

impl Grooph {
    pub fn new(cc: &CreationContext) -> Self {
        add_font(&cc.egui_ctx);
        let ff = FontFamily::Name("music".into());
        let mut measure = Measure::new(TimeSignature::SEVEN_EIGHT);
        measure.add_beat(Beat::note(e())).unwrap();
        measure.add_beat(Beat::note(e())).unwrap();
        measure.add_beat(Beat::note(e())).unwrap();
        measure.add_beat(Beat::note(t8())).unwrap();
        measure.add_beat(Beat::note(t8())).unwrap();
        measure.add_beat(Beat::note(t8())).unwrap();
        measure.add_beat(Beat::note(e())).unwrap();
        measure.add_beat(Beat::note(e())).unwrap();

        Self { font_family: ff.clone(), font_id: FontId::new(16.0, ff), measure, cursor_idx: 0 }
    }
}
