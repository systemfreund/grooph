use crate::layout::beam_plan::{BeamPlan, compute_beam_plan, BeamGroup};
use crate::measure::duration::{Duration, NoteValue};
use crate::measure::{Beat, BeatKind, Measure};
use eframe::egui::{FontId, Pos2, Rect};

/// Logical Beat-Index within a measure (0-based)
pub type BeatIdx = usize;

// Removed legacy logical NoteLayout; geometries are derived directly at pixel level now.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TupletPlan {
    /// z. B. 3 für Triplet
    pub count: u8,
    /// inklusive Endindex (geschlossenes Intervall): start..=end
    pub start: BeatIdx,
    pub end: BeatIdx,
    /// Basis-Notenwert (für ggf. spätere Darstellungen nützlich)
    pub base: NoteValue,
    /// true, wenn die Gruppe vollständig mit Balken verbunden ist (keine Klammer nötig)
    pub fully_beamed: bool,
    /// true, wenn irgendeine Pause innerhalb der Gruppe liegt (dann immer Klammer)
    pub contains_rest: bool,
    /// Verbindung der Tuplet-Gruppe nach außen über Balken
    pub edge_connection: EdgeConnection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeConnection {
    None,
    Left,
    Right,
    Both,
}

impl TupletPlan {
    pub fn is_externally_connected(&self) -> bool { self.edge_connection != EdgeConnection::None }

    pub fn number_only(&self) -> bool {
        self.fully_beamed && !self.contains_rest && !self.is_externally_connected()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderPlan {
    pub beams: Vec<BeamGroup>,
    pub tuplets: Vec<TupletPlan>,
}

pub fn plan_measure(measure: &Measure) -> RenderPlan {
    let BeamPlan { groups: beams } = compute_beam_plan(measure);
    let tuplets = discover_tuplets(measure, &beams);

    RenderPlan { beams, tuplets }
}

// ========================
// Phase A: Pixel layout scaffolding (x-centers + beam segments)
// ========================

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

#[derive(Debug, Clone, PartialEq)]
pub struct MeasureLayoutPx {
    pub inner_rect: Rect,
    pub em: f32,
    pub font_id: FontId,
    pub x_centers: Vec<f32>, // absolute pixel centers per beat (offset applied)
    pub beams: Vec<BeamSegmentPx>,
    pub notes: Vec<NoteLayoutPx>,
    // Phase C additions
    pub clef_pos: Option<Pos2>,
    pub time_sig_top: Vec<Pos2>,
    pub time_sig_bottom: Vec<Pos2>,
    pub tuplets: Vec<TupletLayoutPx>,
    pub content_left: f32,
    pub content_right: f32,
}

/// Build a first-stage pixel layout: absolute x centers and beam segments.
/// Note: This intentionally avoids any dependency on render/glyphs to keep the module graph acyclic.
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

    // Phase C: concrete positions for clef and time signature digits
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
                beams_out.push(BeamSegmentPx { p1: Pos2::new(x1, y), p2: Pos2::new(x2, y), thickness: beam_thickness });
            }
        }
    }

    // Partial beams (stubs) where a note's beam count exceeds continuity
    let stub_len = em * 0.20; // policy
    for group in &render_plan.beams {
        if group.beat_indices.len() < 2 { continue; }
        let note_idxs = &group.beat_indices;
        let counts = &group.beam_counts; // per note
        let cont = &group.continuity; // between neighbors

        for (local_k, &global_i) in note_idxs.iter().enumerate() {
            let count = *counts.get(local_k).unwrap_or(&0);
            if count <= 0 { continue; }
            let left_cont = if local_k > 0 { *cont.get(local_k - 1).unwrap_or(&0) } else { 0 };
            let right_cont = if local_k + 1 < note_idxs.len() { *cont.get(local_k).unwrap_or(&0) } else { 0 };
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
                            beams_out.push(BeamSegmentPx { p1: Pos2::new(stem_x, y), p2: Pos2::new(stem_x + stub_len, y), thickness: beam_thickness });
                        } else if is_last {
                            beams_out.push(BeamSegmentPx { p1: Pos2::new(stem_x - stub_len, y), p2: Pos2::new(stem_x, y), thickness: beam_thickness });
                        } else {
                            if left_cont > right_cont {
                                beams_out.push(BeamSegmentPx { p1: Pos2::new(stem_x - stub_len, y), p2: Pos2::new(stem_x, y), thickness: beam_thickness });
                            } else if right_cont > left_cont {
                                beams_out.push(BeamSegmentPx { p1: Pos2::new(stem_x, y), p2: Pos2::new(stem_x + stub_len, y), thickness: beam_thickness });
                            } else {
                                // equal continuity → prefer left by policy
                                beams_out.push(BeamSegmentPx { p1: Pos2::new(stem_x - stub_len, y), p2: Pos2::new(stem_x, y), thickness: beam_thickness });
                            }
                        }
                    }
                }
            }
        }
    }

    // ========================
    // Phase B: Per-note geometry (stems/flags/dots/tremolo/accent)
    // ========================
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
    // let staff_space = em * 0.25; // currently unused in Phase B metrics here

    fn requires_flag(d: Duration) -> bool {
        match d.base_note() {
            NoteValue::Eighth | NoteValue::Sixteenth | NoteValue::ThirtySecond => true,
            _ => false,
        }
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
        let has_flag_tail = b.kind == BeatKind::Note && !in_beam_flags.get(i).copied().unwrap_or(false) && requires_flag(b.duration);
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
            if let Some(trem) = b.tremolo {
                if trem.measured {
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

    // ========================
    // Phase C: Tuplet geometry at pixel level
    // ========================
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
            let has_accent_in_group = beats.iter().enumerate().any(|(i, b)| {
                i >= t.start && i <= t.end && b.kind == BeatKind::Note && b.accented
            });
            let accent_clearance = (if has_accent_in_group { 1.4 } else { -0.4 }) * staff_space;
            let y_bracket = y_base - accent_clearance;

            let x_gap_l = (xc - gap_half).max(x_l);
            let x_gap_r = (xc + gap_half).min(x_r);

            let mut bracket: Vec<LinePx> = Vec::new();
            if x_gap_l > x_l {
                bracket.push(LinePx { p1: Pos2::new(x_l, y_bracket), p2: Pos2::new(x_gap_l, y_bracket), thickness: 2.0 });
            }
            if x_r > x_gap_r {
                bracket.push(LinePx { p1: Pos2::new(x_gap_r, y_bracket), p2: Pos2::new(x_r, y_bracket), thickness: 2.0 });
            }
            bracket.push(LinePx { p1: Pos2::new(x_l, y_bracket), p2: Pos2::new(x_l, y_bracket + hook_dy), thickness: 2.0 });
            bracket.push(LinePx { p1: Pos2::new(x_r, y_bracket), p2: Pos2::new(x_r, y_bracket + hook_dy), thickness: 2.0 });

            let y_num = y_bracket + 0.5 * staff_space;
            tuplets_out.push(TupletLayoutPx { count: t.count, number_center: Pos2::new(0.5 * (x_l + x_r), y_num), number_font: number_font.clone(), bracket });
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
            tuplets_out.push(TupletLayoutPx { count: t.count, number_center: Pos2::new(0.5 * (x_l + x_r), y_num), number_font: number_font.clone(), bracket: Vec::new() });
        }
    }

    MeasureLayoutPx { inner_rect, em, font_id, x_centers, beams: beams_out, notes: notes_out, clef_pos, time_sig_top, time_sig_bottom, tuplets: tuplets_out, content_left, content_right }
}

fn digit_count(mut n: u32) -> usize {
    if n == 0 { return 1; }
    let mut c = 0usize;
    while n > 0 { c += 1; n /= 10; }
    c
}

fn discover_tuplets(measure: &Measure, beams: &Vec<BeamGroup>) -> Vec<TupletPlan> {
    let beats = measure.beats();
    let set = crate::measure::duration::default_duration_set();

    #[derive(Debug)]
    struct TupGroupTmp {
        start: BeatIdx,
        end: BeatIdx,
        n: u8,
        m: u8,
        base: NoteValue,
        contains_rest: bool,
    }

    let mut tmp: Vec<TupGroupTmp> = Vec::new();
    let mut i = 0usize;
    while i < beats.len() {
        let Duration::Tuplet { n, m, .. } = beats[i].duration else {
            i += 1;
            continue;
        };
        // Maximalen Lauf gleicher (n,m) finden (Basis darf variieren)
        let mut k = i;
        while k < beats.len() {
            match beats[k].duration {
                Duration::Tuplet { n: nn, m: mm, .. } if nn == n && mm == m => k += 1,
                _ => break,
            }
        }

        // Bestimme die kleinste Basisnote innerhalb des Laufs (feinste Unterteilung)
        let mut run_min_base = beats[i].duration.base_note();
        let mut run_min_ticks =
            set.grid.ticks_of(&Duration::Simple(run_min_base)).unwrap_or(u32::MAX);
        for idx in i..k {
            let b = beats[idx].duration.base_note();
            if let Some(t) = set.grid.ticks_of(&Duration::Simple(b)) {
                if t < run_min_ticks {
                    run_min_ticks = t;
                    run_min_base = b;
                }
            }
        }

        // Segment the run: each segment is oriented to the base of the first slot
        // (not to the finest base of the entire run). This prevents phantom segments
        // when a t16 group begins immediately after a t32 group.
        let mut start = i;
        while start < k {
            // Ziel dynamisch anhand der feinsten Basis innerhalb des Segments bestimmen
            let mut seg_min_base = beats[start].duration.base_note();
            let mut seg_min_ticks = set.grid.ticks_of(&Duration::Simple(seg_min_base)).unwrap_or(0);

            let mut acc_ticks: u32 = 0;
            let mut end = start;
            let mut has_rest = false;
            let mut reached_target = false;
            while end < k {
                // Update minimaler Basiswert
                let b = beats[end].duration.base_note();
                if let Some(bt) = set.grid.ticks_of(&Duration::Simple(b)) {
                    if bt < seg_min_ticks {
                        seg_min_ticks = bt;
                        seg_min_base = b;
                    }
                }

                if beats[end].kind == BeatKind::Rest {
                    has_rest = true;
                }
                let dt = set.grid.ticks_of(&beats[end].duration).unwrap_or(0);
                acc_ticks = acc_ticks.saturating_add(dt);
                let target_per_group_ticks = seg_min_ticks.saturating_mul(m as u32);
                if acc_ticks >= target_per_group_ticks {
                    reached_target = true;
                    break;
                }
                end += 1;
            }

            // Wie viele Noten enthält [start..=end]? (nur für fully_beamed später relevant)
            // Für die Segment-Erstellung selbst akzeptieren wir auch Segmente mit nur Rests,
            // da Tuplet-Klammern über reine Pausen hinweg ebenfalls semantisch sinnvoll sind.
            if reached_target {
                // Wichtig: Bewahre die Slot‑Grenzen (inkl. evtl. Rests) — das entspricht der logischen Spannweite
                tmp.push(TupGroupTmp {
                    start,
                    end,
                    n,
                    m,
                    base: seg_min_base,
                    contains_rest: has_rest,
                });
                start = end + 1;
            } else {
                // unvollständig/zu klein → versuche ab nächstem Slot erneut
                start += 1;
            }
        }
        i = k;
    }

    // fully_beamed bestimmen: wenn alle Noten (min. 2) der Gruppe innerhalb einer BeamGroup liegen
    // und alle benachbarten Paare continuity >= 1 haben.
    let mut out: Vec<TupletPlan> = Vec::with_capacity(tmp.len());
    for g in tmp.into_iter() {
        let note_idxs: Vec<BeatIdx> =
            (g.start..=g.end).filter(|&ix| beats[ix].kind == BeatKind::Note).collect();
        let fully = if g.contains_rest || note_idxs.len() < 2 {
            false
        } else {
            // Prüfe gegen jede BeamGroup
            let mut ok_any = false;
            'bg: for bg in beams.iter() {
                // Alle Noten enthalten?
                if note_idxs.iter().all(|ix| bg.beat_indices.contains(ix)) {
                    // Mappe BeatIndex -> Position in der BeamGroup
                    let mut pos_map = std::collections::HashMap::new();
                    for (li, gi2) in bg.beat_indices.iter().enumerate() {
                        pos_map.insert(*gi2, li);
                    }
                    // Prüfe alle benachbarten Paare auf continuity >= 1
                    let mut ok = true;
                    for pair in note_idxs.windows(2) {
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
                        ok_any = true;
                        break 'bg;
                    }
                }
            }
            ok_any
        };

        // Externe Balkenverbindungen links/rechts an den Rändern feststellen
        let mut ext_left = false;
        let mut ext_right = false;
        let first_note = note_idxs.first().copied();
        let last_note = note_idxs.last().copied();

        if let Some(fi) = first_note {
            if fi > 0 && beats[fi - 1].kind == BeatKind::Note {
                for bg in beams.iter() {
                    let pos_prev = bg.beat_indices.iter().position(|&x| x == fi - 1);
                    let pos_cur = bg.beat_indices.iter().position(|&x| x == fi);
                    if let (Some(lp), Some(lc)) = (pos_prev, pos_cur) {
                        // Adjacent in der Gruppe und continuity >=1 zwischen ihnen?
                        let a = lp.min(lc);
                        let b = lp.max(lc);
                        if b == a + 1 {
                            if *bg.continuity.get(a).unwrap_or(&0) >= 1 {
                                ext_left = true;
                                break;
                            }
                        }
                    }
                }
            }
        }

        if let Some(li) = last_note {
            if li + 1 < beats.len() && beats[li + 1].kind == BeatKind::Note {
                for bg in beams.iter() {
                    let pos_cur = bg.beat_indices.iter().position(|&x| x == li);
                    let pos_next = bg.beat_indices.iter().position(|&x| x == li + 1);
                    if let (Some(lc), Some(ln)) = (pos_cur, pos_next) {
                        let a = lc.min(ln);
                        let b = lc.max(ln);
                        if b == a + 1 {
                            if *bg.continuity.get(a).unwrap_or(&0) >= 1 {
                                ext_right = true;
                                break;
                            }
                        }
                    }
                }
            }
        }

        let edge_connection = match (ext_left, ext_right) {
            (false, false) => EdgeConnection::None,
            (true, false) => EdgeConnection::Left,
            (false, true) => EdgeConnection::Right,
            (true, true) => EdgeConnection::Both,
        };

        out.push(TupletPlan {
            // Wichtig: Die dargestellte Tuplet-Zahl ist der Zähler n des (n,m)-Verhältnisses
            // und NICHT die Anzahl der Slots im geschnittenen Segment. Das Segment kann kürzer
            // sein (z. B. wenn der erste Slot zu einer größeren Basis „verschmilzt“), die
            // semantische Tuplet bleibt aber eine „3“ (Triplet), „5“ (Quintuplet), etc.
            count: g.n,
            start: g.start,
            end: g.end,
            base: g.base,
            fully_beamed: fully,
            contains_rest: g.contains_rest,
            edge_connection,
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure::BeatKind::Note;
    use crate::measure::duration::{e, t8, t16, t32};
    use crate::measure::{Beat, Measure, TimeSignature};

    #[test]
    fn beaming_group_within_primary_boundaries_in_seven_eight() {
        // 7/8 mit Achteln, Standardgruppierung 2+3+2.
        // Die mittlere Gruppe (Beats 2..=4, 0-basiert) sollte beamed sein.
        let mut m = Measure::new(TimeSignature::SEVEN_EIGHT);
        for i in 0..7 {
            m.set_beat_at(i, Beat::note(e())).unwrap();
        }

        let plan = plan_measure(&m);
        let has_group = plan.beams.iter().any(|g| {
            let idxs = &g.beat_indices;
            if !(idxs.contains(&2) && idxs.contains(&3) && idxs.contains(&4)) {
                return false;
            }
            let mut pos_map = std::collections::HashMap::new();
            for (i, gi) in idxs.iter().enumerate() {
                pos_map.insert(*gi, i);
            }
            let l2 = *pos_map.get(&2).unwrap();
            let l3 = *pos_map.get(&3).unwrap();
            let l4 = *pos_map.get(&4).unwrap();
            if !(l2 < l3 && l3 < l4) {
                return false;
            }
            g.continuity.get(l2).copied().unwrap_or(0) >= 1
                && g.continuity.get(l3).copied().unwrap_or(0) >= 1
        });
        assert!(has_group, "Beats 3-5 sollten in 7/8 beamed sein (2+3+2, mittlere Gruppe)");
    }

    #[test]
    fn triplet_bracket_over_beats_4_to_6() {
        // Konstruiere einen 4/4, in dem Beats 3..=5 (0-basiert) eine Triole bilden
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Setze zunächst 6 Achtel, dann eine Triole über die nächsten 3 Achtel-Schlitze
        for i in 0..3 {
            m.set_beat_at(i, Beat::note(e())).unwrap();
        }
        // Drei Achtel-Triolett-Noten
        m.set_beat_at(3, Beat::note(t8())).unwrap();
        m.set_beat_at(4, Beat::note(t8())).unwrap();
        m.set_beat_at(5, Beat::note(t8())).unwrap();

        let plan = plan_measure(&m);
        let ok = plan.tuplets.iter().any(|t| t.count == 3 && t.start == 3 && t.end == 5);
        assert!(ok, "Triplet-Klammer sollte über Beats 4–6 (3..=5) liegen");
    }

    #[test]
    fn triplet_bracket_over_beats_when_preceding_beat_is_connected_to_triplet_with_beams_in_7_8() {
        let mut m = Measure::new_init(TimeSignature::SEVEN_EIGHT, Note);
        m.set_beat_at(3, Beat::note(t8())).unwrap();
        m.set_beat_at(4, Beat::note(t8())).unwrap();
        m.set_beat_at(5, Beat::note(t8())).unwrap();

        let plan = plan_measure(&m);

        // Beam connects the triplets (3,4,5) with the preceding beat (2).
        // Expected because of the default 2+3+2 grouping in 7/8.
        assert_eq!(plan.beams[1].beat_indices, vec![2, 3, 4, 5]);

        // We expect a bracket to be drawn over the triplets (3,4,5) to visually distinguish them
        // from the preceding beat.
        let t = plan
            .tuplets
            .iter()
            .find(|t| t.count == 3 && t.start == 3 && t.end == 5)
            .expect("expected triplet over beats 3..=5");

        assert!(t.fully_beamed);
        assert!(!t.contains_rest);
        assert_eq!(t.edge_connection, EdgeConnection::Left);
        assert!(!t.number_only());
    }

    #[test]
    fn triplet_bracket_over_beats_when_following_beat_is_connected_to_triplet_with_beams_in_7_8() {
        let mut m = Measure::new_init(TimeSignature::SEVEN_EIGHT, Note);
        m.set_beat_at(2, Beat::note(t8())).unwrap();
        m.set_beat_at(3, Beat::note(t8())).unwrap();
        m.set_beat_at(4, Beat::note(t8())).unwrap();

        let plan = plan_measure(&m);

        // Beam connects the triplets (2,3,4) with the following beat (5).
        // Expected because of the default 2+3+2 grouping in 7/8.
        assert_eq!(plan.beams[1].beat_indices, vec![2, 3, 4, 5]);

        // We expect a bracket to be drawn over the triplets (3,4,5) to visually distinguish them
        // from the preceding beat.
        let t = plan
            .tuplets
            .iter()
            .find(|t| t.count == 3 && t.start == 2 && t.end == 4)
            .expect("expected triplet over beats 2..=4");

        assert!(t.fully_beamed);
        assert!(!t.contains_rest);
        assert_eq!(t.edge_connection, EdgeConnection::Right);
        assert!(!t.number_only());
    }

    #[test]
    fn triplet_render_plan_0() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        m.set_beat_at(0, Beat::note(t16())).unwrap();
        m.set_beat_at(1, Beat::note(t8())).unwrap();

        let mut tuplets = plan_measure(&m).tuplets;
        assert_eq!(tuplets.len(), 1);
        // The number remains 3 (triplet), but the bracket spans only the two remaining slots
        assert_eq!(tuplets[0].count, 3);
        assert_eq!(tuplets[0].start, 0);
        assert_eq!(tuplets[0].end, 1);

        m.set_beat_at(2, Beat::note(t16())).unwrap();
        tuplets = plan_measure(&m).tuplets;
        assert_eq!(tuplets.len(), 2);
        assert_eq!(tuplets[1].count, 3);
        assert_eq!(tuplets[1].start, 2);
        assert_eq!(tuplets[1].end, 4);
    }

    #[test]
    fn triplet_render_plan_1() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        m.set_beat_at(0, Beat::note(t32())).unwrap();
        m.set_beat_at(0, Beat::note(t16())).unwrap();

        let mut tuplets = plan_measure(&m).tuplets;
        assert_eq!(tuplets.len(), 1, "first tuplet group not found");
        // The number remains 3 (dtriplet), but the bracket spans only the two remaining slots
        assert_eq!(tuplets[0].count, 3);
        assert_eq!(tuplets[0].start, 0);
        assert_eq!(tuplets[0].end, 1);

        // Start a new tuplet group with a t16 immediately after the t32-group.
        m.set_beat_at(2, Beat::rest(t16())).unwrap();
        // Now we expect to have two tuplet groups, and the very last beat must be a simple 1/16 note.
        tuplets = plan_measure(&m).tuplets;
        println!("{:?}", tuplets);
        assert_eq!(tuplets.len(), 2);
        tuplets = plan_measure(&m).tuplets;

        assert_eq!(tuplets[1].count, 3);
        assert_eq!(tuplets[1].start, 2);
        assert_eq!(tuplets[1].end, 4);
    }
}
