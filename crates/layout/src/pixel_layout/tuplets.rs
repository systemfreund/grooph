use super::{LayoutOpts, Line, NoteLayout, TupletLayout, c, centered_run, ink_of};
use crate::glyphs;
use crate::tuplet_plan::TupletPlan;
use egui::Pos2;
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
    let digit_size = opts.em * c::TUPLET_DIGIT_FONT_FACTOR;
    // Away from the heads, past the stem tips.
    let out = opts.stem_dir();

    // A bracket spans from the left edge of its first head or rest to the
    // right edge of its last, whichever side the stems are on.
    let left_of = |idx: usize| note_layout[idx].head.ink().left();
    let right_of = |idx: usize| note_layout[idx].head.ink().right();

    let mut tuplets_out: Vec<TupletLayout> = Vec::new();
    for t in tuplet_plan {
        let mut x_l = left_of(t.start);
        let mut x_r = right_of(t.end);
        let margin = opts.em * c::TUPLET_MARGIN_EM;
        x_l -= margin;
        x_r += margin;

        // The bracket line past the stems of the outermost note, which the
        // number is centered on. Rests don't count: a lower voice's hang
        // below the line.
        let outermost = opts.outermost(
            (t.start..=t.end)
                .filter(|&i| note_layout[i].kind == BeatKind::Note)
                .map(|i| note_layout[i].staff_pos),
        );
        let y_base = opts.stem_tip_y(outermost.unwrap_or(0)) + out * bracket_gap;

        let xc = 0.5 * (x_l + x_r);
        let digits = glyphs::tuplet_glyphs(t.count);
        let number_at = |y: f32| centered_run(&digits, xc, y, digit_size);
        let num_width = ink_of(&number_at(0.0)).map_or(0.0, |r| r.width());
        let mut gap_half = 0.5 * num_width + c::TUPLET_NUMBER_CLEARANCE_SS * opts.staff_space();
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
            let y_bracket = y_base + out * accent_clearance;

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
                p2: Pos2::new(x_l, y_bracket - out * hook_dy),
            });
            bracket.push(Line {
                p1: Pos2::new(x_r, y_bracket),
                p2: Pos2::new(x_r, y_bracket - out * hook_dy),
            });

            tuplets_out.push(TupletLayout {
                count: t.count,
                number: number_at(y_bracket),
                bracket,
            });
        } else {
            // Number-only: only lift the number if an accent's ink would
            // reach under it.
            let collides = !opts.accent_below
                && ink_of(&number_at(0.0)).is_some_and(|num| {
                    note_layout[t.start..=t.end].iter().filter_map(|nl| nl.accent).any(|a| {
                        let a = a.ink();
                        a.left() < num.right() && a.right() > num.left()
                    })
                });

            let close_clearance = c::TUPLET_NUMBER_CLOSE_SS * opts.staff_space();
            let raised_clearance = c::TUPLET_ACCENT_RAISE_SS * opts.staff_space();
            let clearance = if collides { raised_clearance } else { close_clearance };
            tuplets_out.push(TupletLayout {
                count: t.count,
                number: number_at(y_base + out * clearance),
                bracket: Vec::new(),
            });
        }
    }

    tuplets_out
}
