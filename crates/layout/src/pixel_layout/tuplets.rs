use super::{BeamLayout, LayoutOpts, Line, NoteLayout, TupletLayout, c, centered_run, ink_of};
use crate::glyphs;
use crate::tuplet_plan::TupletPlan;
use egui::{Pos2, Rect};
use grooph_measure::BeatKind;

/// The notes' ink a tuplet has to clear: every glyph, stem and beam, each
/// as its bounding box.
fn ink_boxes(note_layout: &[NoteLayout], beams: &[BeamLayout]) -> Vec<Rect> {
    let glyphs = note_layout.iter().flat_map(NoteLayout::glyphs).map(|g| g.ink());
    let stems = note_layout.iter().filter_map(|n| n.stem).map(|s| Rect::from_two_pos(s.p1, s.p2));
    glyphs.chain(stems).chain(beams.iter().map(|b| b.rect)).collect()
}

pub(super) fn build_tuplet_layout(
    note_layout: &[NoteLayout],
    beams: &[BeamLayout],
    tuplet_plan: &[TupletPlan],
    opts: &LayoutOpts,
) -> Vec<TupletLayout> {
    let hook_len = c::TUPLET_HOOK_LEN_SS * opts.staff_space();
    let hook_dy = hook_len * c::TUPLET_HOOK_DY_FACTOR;
    let digit_size = opts.em * c::TUPLET_DIGIT_FONT_FACTOR;
    let ink_gap = c::TUPLET_INK_GAP_SS * opts.staff_space();
    // Away from the heads, past the stem tips.
    let out = opts.stem_dir();
    // The one of two heights further out.
    let further = |a: f32, b: f32| if out < 0.0 { a.min(b) } else { a.max(b) };

    // A skyline of the notes: how far out their ink reaches over `x0..x1`,
    // widened by the gap on both sides.
    let boxes = ink_boxes(note_layout, beams);
    let reach = |x0: f32, x1: f32| {
        boxes
            .iter()
            .filter(|r| r.right() > x0 - ink_gap && r.left() < x1 + ink_gap)
            .map(|r| if out < 0.0 { r.top() } else { r.bottom() })
            .reduce(further)
    };
    // Move `y` out until it is `clearance` past the ink over `x0..x1`.
    let clear = |y: f32, x0: f32, x1: f32, clearance: f32| {
        reach(x0, x1).map_or(y, |r| further(y, r + out * (ink_gap + clearance)))
    };

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
        let tip = opts.stem_tip_y(outermost.unwrap_or(0));

        let xc = 0.5 * (x_l + x_r);
        let digits = glyphs::tuplet_glyphs(t.count);
        let number_at = |y: f32| centered_run(&digits, xc, y, digit_size);
        let num_ink = ink_of(&number_at(0.0)).unwrap_or(Rect::NOTHING);
        let num_width = num_ink.width().max(0.0);
        // The number centered on `y` clears the ink under it.
        let clear_number =
            |y: f32| clear(y, num_ink.left(), num_ink.right(), 0.5 * num_ink.height());
        let mut gap_half = 0.5 * num_width + c::TUPLET_NUMBER_CLEARANCE_SS * opts.staff_space();
        let min_seg = c::TUPLET_MIN_SEG_SS * opts.staff_space();
        let half_span = 0.5 * (x_r - x_l);
        if gap_half > half_span - min_seg {
            gap_half = (half_span - min_seg).max(0.0);
        }

        if !t.number_only() {
            // The line past the ink under it, its hooks past the ink at the
            // ends, and the number past the ink under the number.
            let mut y_bracket = tip + out * c::TUPLET_BRACKET_GAP_SS * opts.staff_space();
            y_bracket = clear(y_bracket, x_l, x_r, 0.0);
            y_bracket = clear(y_bracket, x_l, x_l, hook_dy);
            y_bracket = clear(y_bracket, x_r, x_r, hook_dy);
            y_bracket = clear_number(y_bracket);

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
            let y = tip + out * c::TUPLET_NUMBER_GAP_SS * opts.staff_space();
            tuplets_out.push(TupletLayout {
                count: t.count,
                number: number_at(clear_number(y)),
                bracket: Vec::new(),
            });
        }
    }

    tuplets_out
}
