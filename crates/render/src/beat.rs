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
    let stroke = Stroke::new(opts.staff_line_thickness(), color);
    for ledger in &note.ledgers {
        painter.line_segment([ledger.p1, ledger.p2], stroke);
    }
    for glyph in note.glyphs() {
        draw_glyph(painter, glyph, &opts.font_id.family, color);
    }
}
