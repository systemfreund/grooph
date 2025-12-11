use crate::layout::pixel_layout::{LayoutOpts, NoteLayout};
use crate::measure::BeatKind;
use crate::render::glyphs;
use eframe::egui;
use eframe::egui::{Align2, Color32, FontId, Stroke};

pub(super) fn draw_beat(
    painter: &egui::Painter,
    note: &NoteLayout,
    opts: &LayoutOpts,
    color: Color32,
) {
    let glyph = match note.kind == BeatKind::Rest {
        true => glyphs::rest_glyph_for_duration(note.duration),
        false => glyphs::GLYPH_NOTEHEAD_BLACK,
    };

    let glyph_font = if note.kind == BeatKind::Rest {
        FontId::new(opts.font_id.size * 0.8, opts.font_id.family.clone())
    } else {
        opts.font_id.clone()
    };

    // Pixel snapping helpers to ensure sharp stem lines and perfect flag alignment
    let ppp = painter.ctx().pixels_per_point();
    let snap_thickness = |t: f32| -> f32 { (t * ppp).round().max(1.0) / ppp };
    let snap_x = |x: f32, thickness: f32| -> f32 {
        let px_thickness = (thickness * ppp).round() as i32;
        if px_thickness % 2 != 0 {
            // Odd width: center on half-pixel
            ((x * ppp).round() + 0.5) / ppp
        } else {
            // Even width: center on integer pixel
            (x * ppp).round() / ppp
        }
    };

    // Calculate snapped stem properties if stem exists
    let (stem_stroke_width, stem_x_offset) = if let Some(stem) = &note.stem {
        let width = snap_thickness(opts.stem_thickness());
        let snapped_x = snap_x(stem.p1.x, width);
        (width, snapped_x - stem.p1.x)
    } else {
        (opts.stem_thickness(), 0.0)
    };

    // Draw stem
    if let Some(stem) = &note.stem {
        let mut p1 = stem.p1;
        let mut p2 = stem.p2;
        p1.x += stem_x_offset;
        p2.x += stem_x_offset;
        painter.line_segment([p1, p2], Stroke::new(stem_stroke_width, color));
    }

    // Draw flag
    if let Some(pos) = note.flag_pos
        && let Some(flag) = glyphs::flag_glyph_for_duration(note.duration)
    {
        let flag_font = FontId::new(opts.font_id.size * 1.0, opts.font_id.family.clone());
        let mut p = pos;

        // If we have a stem, align flag to the snapped stem edge
        if let Some(stem) = &note.stem {
            // The flag should start exactly at the left edge of the stem.
            // stem.p1.x + stem_x_offset is the center of the snapped stem.
            p.x = (stem.p1.x + stem_x_offset) - stem_stroke_width * 0.5;
        }

        painter.text(p, Align2::LEFT_CENTER, flag.to_string(), flag_font, color);
    }

    // Draw notehead
    let mut note_center = note.center;
    note_center.x += stem_x_offset;
    painter.text(note_center, Align2::CENTER_CENTER, glyph.to_string(), glyph_font, color);

    // Draw dots
    if !note.dots.is_empty() {
        for p in &note.dots {
            let mut dot_pos = *p;
            dot_pos.x += stem_x_offset;
            painter.text(
                dot_pos,
                Align2::CENTER_CENTER,
                glyphs::GLYPH_AUGMENTATION_DOT.to_string(),
                opts.font_id.clone(),
                color,
            );
        }
    }

    // Draw accent
    if let Some(p) = note.accent_pos {
        let mut accent_pos = p;
        accent_pos.x += stem_x_offset;
        let accent_font = FontId::new(opts.font_id.size * 0.8, opts.font_id.family.clone());
        painter.text(
            accent_pos,
            Align2::CENTER_CENTER,
            glyphs::GLYPH_ACCENT_ABOVE.to_string(),
            accent_font,
            color,
        );
    }
}
