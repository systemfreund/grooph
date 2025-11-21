use crate::layout::beam_plan::BeamGroup;
use crate::layout::render_plan::plan_measure;
use crate::layout::tuplet_plan::TupletPlan;
use crate::measure::duration::{Duration, NoteValue};
use crate::measure::{Beat, BeatKind, Measure, TimeSignature};
use eframe::egui::{FontId, Pos2, Rect};

pub(crate) struct LayoutOpts {
    pub font_id: FontId,
    pub pixels_per_point: f32,
    pub em: f32
}

impl LayoutOpts {
    const fn em(&self) -> f32 { self.em }

    const fn stem_length(&self) -> f32 { self.em() * 0.9 }

    const fn stem_thickness(&self) -> f32 { self.font_id.size * 0.03 }

    const fn stem_offset(&self) -> f32 { self.font_id.size * 0.13 }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeamLayout {
    pub p1: Pos2,
    pub p2: Pos2,
    pub thickness: f32, // TODO move to 'render opts'?
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line {
    pub p1: Pos2,
    pub p2: Pos2,
    pub thickness: f32, // TODO move to 'render opts'?
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
    pub tremolo: Vec<Line>,
    pub accent_pos: Option<Pos2>,
}

/// Pixel-level layout for a measure.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasureLayout {
    pub em: f32,
    pub font_id: FontId,
    pub beams: Vec<BeamLayout>,
    pub notes: Vec<NoteLayout>,
    pub tuplets: Vec<TupletLayout>,
    pub clef_pos: Option<Pos2>,
    pub time_signature: TimeSignatureLayout,
    pub content: Rect,
}

/// Build the pixel layout (`MeasureLayout`) from a `Measure`.
/// Note: This intentionally avoids any dependency on `render::glyphs` to keep the module graph acyclic.
pub fn build_measure_layout(measure: &Measure, rect: Rect, mut opts: LayoutOpts) -> MeasureLayout {
    // 1) Inner rect and font metrics
    let min_size = 24.0 * opts.pixels_per_point; // avoid unreadably small glyphs on HiDPI

    // Keep a small vertical padding fraction
    let vpad = (rect.height() * 0.10).clamp(10.0, 200.0);
    let hpad = (rect.width() * 0.10).clamp(10.0, 30.0);
    let inner_rect = Rect::from_min_max(
        Pos2::new(rect.left(), rect.top() + vpad),
        Pos2::new(rect.right() - hpad, rect.bottom() - vpad),
    );

    // Derive font size mainly from the available height, modulated by width caps
    let base_size_h = inner_rect.height() * 0.50;
    let width_cap = (inner_rect.width() * 0.1).max(min_size);
    let max_size = (inner_rect.height() * 0.80).max(min_size);
    let target_size = base_size_h.clamp(min_size, max_size.min(width_cap));
    opts.font_id = FontId::new(target_size, opts.font_id.family.clone());
    opts.em = target_size;

    let y_center = inner_rect.center().y;

    let clef_w = opts.em() * 0.9; // reserved width for percussion clef
    let clef_pos = Some(Pos2::new(inner_rect.left() + clef_w * 0.4, y_center));

    // Time signature
    let time_signature_layout =
        build_time_sig_layout(measure.time_signature(), inner_rect.left() + clef_w, y_center, opts.em());

    // Notes
    let notes_left = inner_rect.left() + time_signature_layout.width + (opts.em() * 1.0);
    let note_rect = Rect::from_min_max(
        Pos2::new(inner_rect.left() + notes_left, inner_rect.top()),
        inner_rect.right_bottom(),
    );
    let render_plan = plan_measure(measure);
    let note_layout = build_note_layout(measure.beats(), &render_plan.beams, &note_rect, &opts);
    let beam_layout = build_beam_layout(&note_layout, &render_plan.beams, y_center, &opts);
    let tuplet_layout = build_tuplet_layout(
        &measure.beats(),
        &note_layout,
        &render_plan.tuplets,
        y_center,
        opts.em(),
        &opts.font_id,
    );

    MeasureLayout {
        em: opts.em(),
        font_id: opts.font_id,
        beams: beam_layout,
        notes: note_layout,
        tuplets: tuplet_layout,
        clef_pos,
        time_signature: time_signature_layout,
        content: inner_rect,
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

fn build_time_sig_layout(
    time_signature: TimeSignature,
    x: f32,
    y_center: f32,
    em: f32,
) -> TimeSignatureLayout {
    let ts_digit_w = em * 0.35; // per column
    let top_digits = digit_count(time_signature.beats as u32);
    let bot_digits = digit_count(time_signature.beat_unit as u32);
    let ts_cols = top_digits.max(bot_digits) as f32;

    // Compute centered columns for both rows
    let mut time_sig_top: Vec<Pos2> = Vec::with_capacity(top_digits);
    let mut time_sig_bottom: Vec<Pos2> = Vec::with_capacity(bot_digits);
    if top_digits > 0 {
        let offset = (ts_cols - top_digits as f32) * 0.5;
        for i in 0..top_digits {
            let cx = x + ((i as f32) + 0.5 + offset) * ts_digit_w;
            time_sig_top.push(Pos2::new(cx, y_center - em * 0.25));
        }
    }
    if bot_digits > 0 {
        let offset = (ts_cols - bot_digits as f32) * 0.5;
        for i in 0..bot_digits {
            let cx = x + ((i as f32) + 0.5 + offset) * ts_digit_w;
            time_sig_bottom.push(Pos2::new(cx, y_center + em * 0.25));
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
    let x_centers = crate::layout::calculate_x_centers(beats, rect.width())
        .into_iter()
        .map(|cx| cx + rect.left())
        .collect::<Vec<_>>();
    let y_center = rect.center().y;

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

    fn requires_flag(d: Duration) -> bool {
        matches!(d.base_note(), NoteValue::Eighth | NoteValue::Sixteenth | NoteValue::ThirtySecond)
    }

    let mut note_layout: Vec<NoteLayout> = Vec::with_capacity(beats.len());
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
        let first_dx =
            if has_flag_tail { opts.font_id.size * 0.5 } else { opts.font_id.size * 0.28 };
        let step_dx = opts.font_id.size * 0.26;
        let mut dots: Vec<Pos2> = Vec::with_capacity(dot_count as usize);
        if dot_count > 0 {
            for d in 0..dot_count {
                let x = cx + first_dx + (d as f32) * step_dx;
                let y = cy - opts.font_id.size * 0.1;
                dots.push(Pos2::new(x, y));
            }
        }

        // Stem (notes only)
        let mut stem: Option<Line> = None;
        let mut flag_pos: Option<Pos2> = None;
        let mut tremolo: Vec<Line> = Vec::new();
        let mut accent_pos: Option<Pos2> = None;

        if b.kind == BeatKind::Note {
            // Accent position
            if b.accented {
                accent_pos = Some(Pos2::new(cx, cy - opts.font_id.size * 1.2));
            }

            let start_x = cx + opts.stem_offset();
            let needs_flag = requires_flag(b.duration);
            let in_beam = in_beam_flags.get(i).copied().unwrap_or(false);
            let stem_len_factor = if in_beam || needs_flag { 1.0 } else { 0.85 };
            let stem_len = opts.stem_length() * stem_len_factor;
            let start = Pos2::new(start_x, cy);
            let end = Pos2::new(start_x, cy - stem_len);
            stem = Some(Line { p1: start, p2: end, thickness: opts.stem_thickness() });

            // Flag position at stem tip if not in a beam and duration requires a flag
            if !in_beam && needs_flag {
                flag_pos =
                    Some(Pos2::new(start_x - opts.stem_thickness() * 0.5, cy - opts.stem_length()));
            }

            // Tremolo slashes (single-note measured tremolo)
            if let Some(trem) = b.tremolo
                && trem.measured
            {
                let sl = trem.slashes.min(3);
                let dx = opts.font_id.size * 0.12; // slight right offset per slash
                let dy = opts.font_id.size * 0.12; // spacing along stem
                let ang = 0.6; // tilt factor (down-right)
                for s in 0..sl {
                    let y0 = (cy - stem_len) + (s as f32) * dy;
                    let x0 = start_x + (s as f32) * dx;
                    let len = opts.font_id.size * 0.45;
                    tremolo.push(Line {
                        p1: Pos2::new(x0, y0),
                        p2: Pos2::new(x0 + len, y0 - len * ang),
                        thickness: 2.0,
                    });
                }
            }
        }

        note_layout.push(NoteLayout {
            center,
            duration: b.duration,
            kind: b.kind,
            dots,
            stem,
            flag_pos,
            tremolo,
            accent_pos,
        });
    }

    note_layout
}

fn build_beam_layout(
    note_layout: &[NoteLayout],
    beam_groups: &[BeamGroup],
    y_center: f32,
    opts: &LayoutOpts,
) -> Vec<BeamLayout> {
    // TODO move top opts all 3
    let staff_space = opts.em() * 0.25; // tuned by eye; single-line context
    let beam_thickness = 0.5 * staff_space; // Bravura ~0.5 sp
    let gap = 0.25 * staff_space; // distance between beams
    // align top edge with stem tip ⇒ use bottom y with slight offset to hide seam
    let base_y = y_center - opts.stem_length() + beam_thickness * 0.95;

    let stem_xs: Vec<f32> = note_layout.iter().map(|nl| nl.center.x + opts.stem_offset()).collect();

    // Helper: compute y for level
    let y_level = |lvl: u8| -> f32 { base_y + (lvl as f32) * (beam_thickness + gap) };

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
            let offset = opts.stem_thickness() / 3.0; // extend slightly to touch stems nicely
            let x1 = stem_xs[i] - offset;
            let x2 = stem_xs[j] + offset;
            for lvl in 0..levels {
                let y = y_level(lvl);
                beams_out.push(BeamLayout {
                    p1: Pos2::new(x1, y),
                    p2: Pos2::new(x2, y),
                    thickness: beam_thickness,
                });
            }
        }
    }

    // Helper: compute y for level
    let y_level = |lvl: u8| -> f32 { base_y + (lvl as f32) * (beam_thickness + gap) };

    // Partial beams (stubs) where a note's beam count exceeds continuity
    let stub_len = opts.em() * 0.20; // TODO move to opts?
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
                            beams_out.push(BeamLayout {
                                p1: Pos2::new(stem_x, y),
                                p2: Pos2::new(stem_x + stub_len, y),
                                thickness: beam_thickness,
                            });
                        } else if is_last || left_cont > right_cont {
                            beams_out.push(BeamLayout {
                                p1: Pos2::new(stem_x - stub_len, y),
                                p2: Pos2::new(stem_x, y),
                                thickness: beam_thickness,
                            });
                        } else if right_cont > left_cont {
                            beams_out.push(BeamLayout {
                                p1: Pos2::new(stem_x, y),
                                p2: Pos2::new(stem_x + stub_len, y),
                                thickness: beam_thickness,
                            });
                        } else {
                            // equal continuity → prefer left by policy
                            beams_out.push(BeamLayout {
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

    beams_out
}

fn build_tuplet_layout(
    beats: &[Beat],
    note_layout: &[NoteLayout],
    tuplet_plan: &[TupletPlan],
    y_center: f32,
    em: f32,
    font_id: &FontId,
) -> Vec<TupletLayout> {
    let staff_space = em * 0.25;
    let bracket_gap = 1.8 * staff_space;
    let hook_len = 0.8 * staff_space;
    let hook_dy = hook_len * 0.85;
    let number_font = FontId::new(font_id.size * 0.75, font_id.family.clone());
    let default_stem_len = em * 0.9;
    // Approximate baseline above stems
    let y_base = y_center - default_stem_len - 0.5 * staff_space - bracket_gap;

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

            let mut bracket: Vec<Line> = Vec::new();
            if x_gap_l > x_l {
                bracket.push(Line {
                    p1: Pos2::new(x_l, y_bracket),
                    p2: Pos2::new(x_gap_l, y_bracket),
                    thickness: 2.0,
                });
            }
            if x_r > x_gap_r {
                bracket.push(Line {
                    p1: Pos2::new(x_gap_r, y_bracket),
                    p2: Pos2::new(x_r, y_bracket),
                    thickness: 2.0,
                });
            }
            bracket.push(Line {
                p1: Pos2::new(x_l, y_bracket),
                p2: Pos2::new(x_l, y_bracket + hook_dy),
                thickness: 2.0,
            });
            bracket.push(Line {
                p1: Pos2::new(x_r, y_bracket),
                p2: Pos2::new(x_r, y_bracket + hook_dy),
                thickness: 2.0,
            });

            let y_num = y_bracket + 0.5 * staff_space;
            tuplets_out.push(TupletLayout {
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
                    && note_layout
                        .get(i)
                        .map(|nl| {
                            nl.center.x >= num_cx - num_half_w && nl.center.x <= num_cx + num_half_w
                        })
                        .unwrap_or(false)
            });

            // Choose vertical clearance based on potential collision
            let close_clearance = -0.4 * staff_space; // closer to the beam
            let raised_clearance = 1.4 * staff_space; // high enough to clear accent
            let clearance = if collides { raised_clearance } else { close_clearance };
            let y_num = (y_base - clearance) + 0.5 * staff_space;
            tuplets_out.push(TupletLayout {
                count: t.count,
                number_center: Pos2::new(0.5 * (x_l + x_r), y_num),
                number_font: number_font.clone(),
                bracket: Vec::new(),
            });
        }
    }

    tuplets_out
}
