use eframe::egui::{Color32, FontFamily, FontId, Painter, vec2};
use grooph_layout::pixel_layout::PlacedGlyph;

/// Draws `glyph` with its origin, on the font's baseline, at the laid-out
/// point.
pub fn draw_glyph(painter: &Painter, glyph: &PlacedGlyph, family: &FontFamily, color: Color32) {
    let font = FontId::new(glyph.size, family.clone());
    let galley = painter.layout_no_wrap(glyph.glyph.ch.to_string(), font, color);
    let baseline = galley
        .rows
        .first()
        .and_then(|row| row.row.glyphs.first().map(|g| row.pos.y + g.pos.y))
        .unwrap_or(0.0);
    painter.galley(glyph.origin - vec2(0.0, baseline), galley, color);
}
