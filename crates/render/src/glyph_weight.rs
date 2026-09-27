use eframe::egui::{Align2, Color32, FontId, Painter, Pos2, Vec2};

/// Radial offsets (unit vectors) used to fake a bolder glyph weight by
/// drawing the same glyph several times, slightly displaced, around its
/// true position.
const OFFSETS: [(f32, f32); 8] = [
    (1.0, 0.0),
    (-1.0, 0.0),
    (0.0, 1.0),
    (0.0, -1.0),
    (0.7071, 0.7071),
    (-0.7071, 0.7071),
    (0.7071, -0.7071),
    (-0.7071, -0.7071),
];

/// Draws `text` at `pos`, faking a bolder weight when `weight > 1.0` by
/// drawing it again several times at small radial offsets. Used for
/// notation glyphs (noteheads, flags, dots, accents, clef, time signature,
/// tuplet numbers) whose font (Bravura) has no bold variant of its own.
/// `weight <= 1.0` draws the glyph once, matching the original look.
pub fn draw_weighted_text(
    painter: &Painter,
    pos: Pos2,
    align: Align2,
    text: &str,
    font: FontId,
    color: Color32,
    weight: f32,
) {
    painter.text(pos, align, text, font.clone(), color);
    if weight <= 1.0 {
        return;
    }
    // Scales with the glyph's own size, not a fixed pixel offset, so the
    // effect stays proportionate at any em.
    let radius = font.size * 0.02 * (weight - 1.0);
    for (dx, dy) in OFFSETS {
        painter.text(pos + Vec2::new(dx, dy) * radius, align, text, font.clone(), color);
    }
}
