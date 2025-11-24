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

    // Draw stem
    if let Some(stem) = &note.stem {
        painter.line_segment([stem.p1, stem.p2], Stroke::new(opts.stem_thickness(), color));
    }

    // Draw flag
    if let Some(pos) = note.flag_pos
        && let Some(flag) = glyphs::flag_glyph_for_duration(note.duration)
    {
        let flag_font = FontId::new(opts.font_id.size * 1.0, opts.font_id.family.clone());
        painter.text(pos, Align2::LEFT_CENTER, flag.to_string(), flag_font, color);
    }

    // Draw notehead
    painter.text(note.center, Align2::CENTER_CENTER, glyph.to_string(), glyph_font, color);

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
        let accent_font = FontId::new(opts.font_id.size * 0.8, opts.font_id.family.clone());
        painter.text(
            p,
            Align2::CENTER_CENTER,
            glyphs::GLYPH_ACCENT_ABOVE.to_string(),
            accent_font,
            color,
        );
    }
}
