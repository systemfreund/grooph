use eframe::egui;
use eframe::egui::{Align2, Color32, Stroke};
use grooph_layout::glyphs;
use grooph_layout::pixel_layout::{LayoutOpts, NoteLayout};
use grooph_measure::BeatKind;

/// Gap between a soft note's head and its parentheses, in em.
const SOFT_PAREN_GAP_EM: f32 = 0.04;

pub(super) fn draw_beat(
    painter: &egui::Painter,
    note: &NoteLayout,
    opts: &LayoutOpts,
    color: Color32,
) {
    let glyph = match note.kind == BeatKind::Rest {
        true => glyphs::rest_glyph_for_duration(note.duration),
        false => glyphs::notehead_glyph_for_duration(note.duration),
    };

    // Draw stem
    if let Some(stem) = &note.stem {
        painter.line_segment([stem.p1, stem.p2], Stroke::new(opts.stem_thickness(), color));
    }

    // Draw flag
    if let Some(pos) = note.flag_pos
        && let Some(flag) = glyphs::flag_glyph_for_duration(note.duration, opts.stem_down)
    {
        painter.text(pos, Align2::LEFT_CENTER, flag.to_string(), opts.font_id.clone(), color);
    }

    // Draw notehead. With a stem it hangs off the stem's right edge (left
    // edge for a down stem): both are rounded to pixels on their own, and
    // centering the head could leave a hairline between them on small staffs.
    let (pos, align) = match &note.stem {
        Some(stem) if note.kind == BeatKind::Note && opts.stem_down => (
            egui::pos2(stem.p1.x - opts.stem_thickness() * 0.5, note.center.y),
            Align2::LEFT_CENTER,
        ),
        Some(stem) if note.kind == BeatKind::Note => (
            egui::pos2(stem.p1.x + opts.stem_thickness() * 0.5, note.center.y),
            Align2::RIGHT_CENTER,
        ),
        _ => (note.center, Align2::CENTER_CENTER),
    };
    painter.text(pos, align, glyph.to_string(), opts.font_id.clone(), color);

    // Parentheses of a soft note
    if note.soft && note.kind == BeatKind::Note {
        let dx = opts.metrics.head_size.x * 0.5 + opts.em * SOFT_PAREN_GAP_EM;
        for (glyph, x, align) in [
            (glyphs::GLYPH_NOTEHEAD_PAREN_LEFT, note.center.x - dx, Align2::RIGHT_CENTER),
            (glyphs::GLYPH_NOTEHEAD_PAREN_RIGHT, note.center.x + dx, Align2::LEFT_CENTER),
        ] {
            painter.text(
                egui::pos2(x, note.center.y),
                align,
                glyph.to_string(),
                opts.font_id.clone(),
                color,
            );
        }
    }

    // Draw dots
    if !note.dots.is_empty() {
        for p in &note.dots {
            painter.text(
                *p,
                Align2::CENTER_CENTER,
                glyphs::GLYPH_AUGMENTATION_DOT.to_string(),
                opts.font_id.clone(),
                color,
            );
        }
    }

    // Draw accent
    if let Some(p) = note.accent_pos {
        painter.text(
            p,
            Align2::CENTER_CENTER,
            glyphs::GLYPH_ACCENT_ABOVE.to_string(),
            opts.font_id.clone(),
            color,
        );
    }

    if opts.debug_bbox {
        if let Some(rect) = note.debug_bbox {
            painter.rect(
                rect,
                0.0,
                Color32::TRANSPARENT,
                Stroke::new(1.0_f32, Color32::RED),
                egui::StrokeKind::Outside,
            );
        }
        if let Some(rect) = note.accent_debug_bbox {
            painter.rect(
                rect,
                0.0,
                Color32::TRANSPARENT,
                Stroke::new(1.0_f32, Color32::GREEN),
                egui::StrokeKind::Outside,
            );
        }
    }
}
