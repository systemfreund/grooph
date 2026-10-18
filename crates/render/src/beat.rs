use eframe::egui;
use eframe::egui::{Color32, Stroke};
use grooph_layout::pixel_layout::{LayoutOpts, NoteLayout};

use crate::glyph::draw_glyph;

pub(super) fn draw_beat(
    painter: &egui::Painter,
    note: &NoteLayout,
    opts: &LayoutOpts,
    color: Color32,
) {
    if let Some(stem) = &note.stem {
        painter.line_segment([stem.p1, stem.p2], Stroke::new(opts.stem_thickness(), color));
    }
    for glyph in note.glyphs() {
        draw_glyph(painter, glyph, &opts.font_id.family, color);
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
