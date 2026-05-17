//! Layout pipeline for measures and full scores.
//!
//! The pipeline is layered; each stage consumes the previous one:
//!
//! ```text
//!   Measure ──► RenderPlan ──► MeasureLayout ──► (egui drawing)
//!               (logical)       (pixel)            in `grooph_render`
//! ```
//!
//! * [`render_plan::RenderPlan`] (built via [`render_plan::plan_measure`]) is the
//!   *logical* stage — beaming groups, tuplet spans. No pixels, no fonts.
//! * [`pixel_layout::MeasureLayout`] (built via [`pixel_layout::build_measure_layout`])
//!   is the *pixel* stage. It internally calls `plan_measure` and turns the result
//!   plus a `Rect`, `FontId`, and `GlyphMetrics` into concrete coordinates.
//! * [`staff_layout::StaffLayout`] arranges multiple `MeasureLayout`s along a
//!   horizontal staff (clef/TS repeat rules, scrolling width).
//!
//! The renderer (`grooph_render`) consumes `MeasureLayout` / `StaffLayout` and
//! makes no further geometry decisions. `GlyphMetrics::measure` lives in
//! `grooph_render` (it needs a live `egui::Ui`); this crate keeps
//! `GlyphMetrics` as a plain data struct plus `GlyphMetrics::debug` for tests.

mod basis;
pub mod beam_plan;
pub mod glyphs;
pub mod pixel_layout;
pub mod render_plan;
pub mod staff_layout;
pub mod tuplet_plan;

pub use basis::calculate_x_centers;
pub use beam_plan::*;
pub use pixel_layout::*;
pub use render_plan::*;
pub use staff_layout::*;
pub use tuplet_plan::*;
