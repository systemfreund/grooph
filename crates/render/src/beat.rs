use crate::glyph_weight::draw_weighted_text;
use eframe::egui;
use eframe::egui::{Align2, Color32, FontId, Stroke};
use grooph_layout::glyphs;
use grooph_layout::pixel_layout::{LayoutOpts, NoteLayout};
use grooph_measure::BeatKind;

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

    // Draw stem
    if let Some(stem) = &note.stem {
        painter.line_segment([stem.p1, stem.p2], Stroke::new(opts.stem_thickness(), color));
    }

    // Draw flag
    if let Some(pos) = note.flag_pos
        && let Some(flag) = glyphs::flag_glyph_for_duration(note.duration)
    {
        draw_weighted_text(
            painter,
            pos,
            Align2::LEFT_CENTER,
            &flag.to_string(),
            opts.font_id.clone(),
            color,
            opts.notation_weight,
        );
    }

    // Draw notehead
    draw_weighted_text(
        painter,
        note.center,
        Align2::CENTER_CENTER,
        &glyph.to_string(),
        opts.font_id.clone(),
        color,
        opts.notation_weight,
    );

    // Draw dots
    if !note.dots.is_empty() {
        for p in &note.dots {
            draw_weighted_text(
                painter,
                *p,
                Align2::CENTER_CENTER,
                &glyphs::GLYPH_AUGMENTATION_DOT.to_string(),
                opts.font_id.clone(),
                color,
                opts.notation_weight,
            );
        }
    }

    // Draw accent
    if let Some(p) = note.accent_pos {
        draw_weighted_text(
            painter,
            p,
            Align2::CENTER_CENTER,
            &glyphs::GLYPH_ACCENT_ABOVE.to_string(),
            opts.font_id.clone(),
            color,
            opts.notation_weight,
        );
    }

    if opts.debug_bbox {
        if let Some(rect) = note.debug_bbox {
            painter.rect(
                rect,
                0.0,
                Color32::TRANSPARENT,
                Stroke::new(1.0, Color32::RED),
                egui::StrokeKind::Outside,
            );
        }
        if let Some(rect) = note.accent_debug_bbox {
            painter.rect(
                rect,
                0.0,
                Color32::TRANSPARENT,
                Stroke::new(1.0, Color32::GREEN),
                egui::StrokeKind::Outside,
            );
        }
    }
}
