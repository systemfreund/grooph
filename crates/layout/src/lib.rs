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
//!   plus a `Rect` and font size into concrete coordinates: where each glyph's
//!   origin goes, placed by the glyph's metrics from [`glyphs`].
//! * [`staff_layout::StaffLayout`] stacks the `MeasureLayout`s of a score one
//!   per row (clef/TS repeat rules, row width, scroll size).
//!
//! The renderer (`grooph_render`) consumes `MeasureLayout` / `StaffLayout` and
//! makes no further geometry decisions: it draws each glyph at its origin.

mod basis;
pub mod beam_plan;
pub mod glyphs;
pub mod pixel_layout;
pub mod render_plan;
pub mod staff;
pub mod staff_layout;
pub mod tuplet_plan;

pub use basis::{calculate_shared_x_centers, calculate_x_centers, shared_min_width};
pub use beam_plan::*;
pub use pixel_layout::*;
pub use render_plan::*;
pub use staff_layout::*;
pub use tuplet_plan::*;
