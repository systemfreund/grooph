use eframe::egui::{Align2, Color32, FontId, Painter, Pos2, Vec2};

/// Cardinal offsets (unit vectors) used to fake a bolder glyph weight by
/// drawing the same glyph a few times, slightly displaced, around its true
/// position. Deliberately just the 4 cardinal directions, not diagonals:
/// diagonal copies land further from thin, curved strokes (like the eighth
/// note flag) than the stroke itself is wide, so instead of thickening the
/// line evenly they show up as a separate fanned-out shape.
const OFFSETS: [(f32, f32); 4] = [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)];

/// Largest offset radius (in logical pixels) `draw_weighted_text` will use,
/// regardless of font size. Kept small and font-size-independent — unlike
/// stroke thickness, which scales with `em`, glyph outlines (particularly
/// thin curves like flags) start looking like a distinct, ghosted shape
/// well before an em-proportional offset would.
const MAX_RADIUS: f32 = 0.9;

/// Draws `text` at `pos`, faking a bolder weight when `weight > 1.0` by
/// drawing it again a few times at small offsets. Used for notation glyphs
/// (noteheads, flags, dots, accents, clef, time signature, tuplet numbers)
/// whose font (Bravura) has no bold variant of its own. `weight <= 1.0`
/// draws the glyph once, matching the original look.
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
    let radius = (MAX_RADIUS * (weight - 1.0) * 0.5).min(MAX_RADIUS);
    for (dx, dy) in OFFSETS {
        painter.text(pos + Vec2::new(dx, dy) * radius, align, text, font.clone(), color);
    }
}
