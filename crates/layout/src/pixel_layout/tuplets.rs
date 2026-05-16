use super::{LayoutOpts, Line, NoteLayout, TupletLayout, c, digit_count};
use crate::tuplet_plan::TupletPlan;
use egui::{FontId, Pos2};
use grooph_measure::{Beat, BeatKind};

pub(super) fn build_tuplet_layout(
    beats: &[Beat],
    note_layout: &[NoteLayout],
    tuplet_plan: &[TupletPlan],
    opts: &LayoutOpts,
) -> Vec<TupletLayout> {
    let bracket_gap = c::TUPLET_BRACKET_GAP_SS * opts.staff_space();
    let hook_len = c::TUPLET_HOOK_LEN_SS * opts.staff_space();
    let hook_dy = hook_len * c::TUPLET_HOOK_DY_FACTOR;
    let digit_font =
        FontId::new(opts.font_id.size * c::TUPLET_DIGIT_FONT_FACTOR, opts.font_id.family.clone());
    // Approximate baseline above stems
    let y_base = opts.y_center()
        - opts.stem_length()
        - c::TUPLET_NUMBER_BASELINE_LIFT_SS * opts.staff_space()
        - bracket_gap;

    let x_from_idx = |idx: usize| -> f32 {
        let n = note_layout.get(idx).unwrap();
        if let Some(stem) = &n.stem { stem.p1.x } else { n.center.x }
    };

    let digit_len = |n: u8| -> usize { digit_count(n as u32) };

    let mut tuplets_out: Vec<TupletLayout> = Vec::new();
    for t in tuplet_plan {
        let mut x_l = x_from_idx(t.start);
        let mut x_r = x_from_idx(t.end);
        let margin = opts.em * c::TUPLET_MARGIN_EM;
        x_l -= margin;
        x_r += margin;

        // Number width approximation in pixels based on em
        let num_chars = digit_len(t.count) as f32;
        let num_width = num_chars * c::TUPLET_NUM_WIDTH_EM * opts.em;
        let pad = c::TUPLET_NUM_PAD_EM * opts.em;
        let xc = 0.5 * (x_l + x_r);
        let mut gap_half = 0.5 * (num_width + 2.0 * pad);
        let min_seg = c::TUPLET_MIN_SEG_SS * opts.staff_space();
        let half_span = 0.5 * (x_r - x_l);
        if gap_half > half_span - min_seg {
            gap_half = (half_span - min_seg).max(0.0);
        }

        if !t.number_only() {
            // Bracketed: raise the whole bracket+number if any accent exists in span.
            let has_accent_in_group = !opts.accent_below
                && beats.iter().enumerate().any(|(i, b)| {
                    i >= t.start && i <= t.end && b.kind == BeatKind::Note && b.accented
                });
            let accent_clearance = (if has_accent_in_group {
                c::TUPLET_ACCENT_RAISE_SS
            } else {
                c::TUPLET_ACCENT_LOWER_SS
            }) * opts.staff_space();
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

            let y_num = y_bracket + c::TUPLET_NUMBER_BASELINE_LIFT_SS * opts.staff_space();
            tuplets_out.push(TupletLayout {
                count: t.count,
                number_center: Pos2::new(0.5 * (x_l + x_r), y_num),
                number_font: digit_font.clone(),
                bracket,
            });
        } else {
            // Number-only: only lift the number if it would collide with an accent.
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

            let close_clearance = c::TUPLET_NUMBER_CLOSE_SS * opts.staff_space();
            let raised_clearance = c::TUPLET_ACCENT_RAISE_SS * opts.staff_space();
            let clearance = if collides { raised_clearance } else { close_clearance };
            let y_num =
                (y_base - clearance) + c::TUPLET_NUMBER_BASELINE_LIFT_SS * opts.staff_space();
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
