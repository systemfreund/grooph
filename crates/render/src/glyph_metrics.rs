//! Resolve per-glyph pixel metrics from a live `egui::Ui`.
//!
//! Lives in `render` (not `layout`) because measuring depends on the egui font
//! cache. The `layout` crate keeps the `GlyphMetrics` data struct and a
//! `debug()` constructor for tests — callers (UI code) construct real metrics
//! via [`measure_glyph_metrics`] and hand them to layout via `LayoutOpts`.

use eframe::egui;
use eframe::egui::{FontId, Vec2};
use grooph_layout::glyphs;
use grooph_layout::pixel_layout::GlyphMetrics;

/// Per-glyph height factor (× em). Widths are measured from the font; heights
/// are heuristic to avoid SMuFL bounding boxes that span far above/below the
/// visible glyph (Bravura tightly hugs the cap line, not the ink).
mod h {
    pub const HEAD: f32 = 0.25;
    pub const DOT: f32 = 0.20;
    pub const ACCENT: f32 = 0.25;
    pub const FLAG_8TH: f32 = 0.20;
    pub const FLAG_16TH: f32 = 0.20;
    pub const FLAG_32ND: f32 = 0.40;
    pub const REST_WHOLE: f32 = 0.25;
    pub const REST_HALF: f32 = 0.25;
    pub const REST_QUARTER: f32 = 0.45;
    pub const REST_EIGHTH: f32 = 0.30;
    pub const REST_SIXTEENTH: f32 = 0.50;
    pub const REST_32ND: f32 = 0.55;
}

/// Measure widths of the SMuFL glyphs used by the renderer and build a
/// [`GlyphMetrics`] table sized in pixels for `font_id`.
pub fn measure_glyph_metrics(ui: &egui::Ui, font_id: &FontId) -> GlyphMetrics {
    let em = font_id.size;
    let w = |c: char| -> f32 {
        ui.painter()
            .layout_no_wrap(c.to_string(), font_id.clone(), egui::Color32::WHITE)
            .rect
            .width()
    };

    GlyphMetrics {
        head_size: Vec2::new(w(glyphs::GLYPH_NOTEHEAD_BLACK), h::HEAD * em),
        dot_size: Vec2::new(w(glyphs::GLYPH_AUGMENTATION_DOT), h::DOT * em),
        accent_size: Vec2::new(w(glyphs::GLYPH_ACCENT_ABOVE), h::ACCENT * em),
        flag_8th_size: Vec2::new(w(glyphs::GLYPH_FLAG_8TH_UP), h::FLAG_8TH * em),
        flag_16th_size: Vec2::new(w(glyphs::GLYPH_FLAG_16TH_UP), h::FLAG_16TH * em),
        flag_32nd_size: Vec2::new(w(glyphs::GLYPH_FLAG_32ND_UP), h::FLAG_32ND * em),
        rest_sizes: [
            Vec2::new(w(glyphs::GLYPH_REST_WHOLE), h::REST_WHOLE * em),
            Vec2::new(w(glyphs::GLYPH_REST_HALF), h::REST_HALF * em),
            Vec2::new(w(glyphs::GLYPH_REST_QUARTER), h::REST_QUARTER * em),
            Vec2::new(w(glyphs::GLYPH_REST_EIGHTH), h::REST_EIGHTH * em),
            Vec2::new(w(glyphs::GLYPH_REST_SIXTEENTH), h::REST_SIXTEENTH * em),
            Vec2::new(w(glyphs::GLYPH_REST_32ND), h::REST_32ND * em),
        ],
    }
}
