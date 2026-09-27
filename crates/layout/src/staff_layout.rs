//! Multi-measure layout layer.
//!
//! Sits above [`crate::pixel_layout::build_measure_layout`] and arranges the
//! [`Measure`]s of a score top-to-bottom, one measure per row ("system") —
//! the app targets phones, where a predictable one-bar-per-line layout reads
//! best. Cross-measure concerns (per-measure width allocation, clef/time-
//! signature repeat rules, grow/shrink policy, total size for the host's
//! scroll area) live here; per-measure pixel geometry stays in `MeasureLayout`.
//!
//! Each measure grows to fill the row. If a measure doesn't fit, glyphs shrink
//! toward [`LEGIBILITY_FLOOR_EM`]; a measure that doesn't fit even at the floor
//! overflows its row — the caller is expected to host the result in a
//! horizontally scrollable area for that case.

use crate::pixel_layout::{GlyphMetrics, LayoutOpts, MeasureLayout, build_measure_layout};
use egui::{FontId, Pos2, Rect, Vec2, pos2, vec2};
use grooph_measure::duration::s;
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::{BeatIdx, MeasureIdx, Score, TimeSignature};

/// Configuration for laying out an entire [`Score`].
///
/// Mirrors the per-measure fields of [`LayoutOpts`], plus the cross-measure
/// knobs (`min_measure_width_em`, `note_width_em`, `system_spacing_em`,
/// `layout_clef_first`). The per-measure `rect`, `layout_clef` and
/// `layout_time_signature` are derived per measure during layout.
#[derive(Clone)]
pub struct StaffOpts {
    pub rect: Rect,
    pub font_id: FontId,
    pub pixels_per_point: f32,
    pub em: f32,
    pub y_offset: f32,

    pub stem_length_factor: f32,
    pub stem_thickness_factor: f32,
    pub accent_displacement: f32,
    pub accent_below: bool,
    pub proportional_spacing: bool,
    pub debug_bbox: bool,
    pub metrics: GlyphMetrics,

    /// Minimum width of a measure in em, before per-beat additions.
    pub min_measure_width_em: f32,
    /// Width added per beat in em, to grow dense measures.
    pub note_width_em: f32,
    /// Vertical spacing between systems (rows) in em.
    pub system_spacing_em: f32,
    /// Whether to show the clef on the first measure of each system.
    pub layout_clef_first: bool,
}

impl StaffOpts {
    /// Build per-measure [`LayoutOpts`] for a placed measure (renderer entry).
    pub fn measure_opts_for(&self, placed: &PlacedMeasure) -> LayoutOpts {
        self.measure_opts(placed.rect, placed.show_clef, placed.show_time_signature)
    }

    /// Build per-measure [`LayoutOpts`] with a measure-local rect and clef/TS flags.
    fn measure_opts(&self, rect: Rect, show_clef: bool, show_ts: bool) -> LayoutOpts {
        LayoutOpts {
            rect,
            font_id: self.font_id.clone(),
            pixels_per_point: self.pixels_per_point,
            em: self.em,
            layout_clef: show_clef,
            layout_time_signature: show_ts,
            y_offset: self.y_offset,
            stem_length_factor: self.stem_length_factor,
            stem_thickness_factor: self.stem_thickness_factor,
            accent_displacement: self.accent_displacement,
            accent_below: self.accent_below,
            proportional_spacing: self.proportional_spacing,
            debug_bbox: self.debug_bbox,
            metrics: self.metrics,
        }
    }

    /// Return a copy of these options scaled by `factor` — `em`, `font_id`
    /// size, and `metrics` move together, since `metrics` is measured against
    /// `font_id`'s original size and would otherwise no longer match it.
    /// Vector-font metrics scale ~linearly with size, close enough to avoid
    /// needing a live `egui::Ui` to re-measure.
    pub fn rescaled(&self, factor: f32) -> StaffOpts {
        let mut font_id = self.font_id.clone();
        font_id.size *= factor;
        StaffOpts {
            em: self.em * factor,
            font_id,
            metrics: self.metrics.scaled(factor),
            ..self.clone()
        }
    }
}

/// One measure placed in a system.
#[derive(Debug, Clone)]
pub struct PlacedMeasure {
    pub measure_idx: MeasureIdx,
    pub rect: Rect,
    pub layout: MeasureLayout,
    pub show_clef: bool,
    pub show_time_signature: bool,
}

/// One system (row/line of music). Currently always holds exactly one measure.
#[derive(Debug, Clone)]
pub struct SystemLayout {
    pub y_baseline: f32,
    pub rect: Rect,
    pub measures: Vec<PlacedMeasure>,
}

/// Full pixel layout of a [`Score`].
#[derive(Debug, Clone)]
pub struct StaffLayout {
    pub systems: Vec<SystemLayout>,
    /// Logical size occupied by the score (input for the host's ScrollArea).
    pub total_size: Vec2,
    /// The factor `opts.em` was scaled by to produce this layout — 1.0 means
    /// unscaled. Only ever `<= 1.0`: shrinking toward the legibility floor
    /// scales glyph size down (shared by all rows, governed by the widest
    /// measure), but growing to fill a row only stretches note spacing, not
    /// glyph size (growing never threatens legibility). Callers
    /// that render the layout need to rescale their own `StaffOpts` by this
    /// same factor (see [`StaffOpts::rescaled`]) so glyph size matches what
    /// was used to compute positions here.
    pub scale: f32,
}

impl StaffLayout {
    /// Find the placed measure for a given measure index across all systems.
    pub fn placed(&self, measure_idx: MeasureIdx) -> Option<&PlacedMeasure> {
        self.systems.iter().flat_map(|s| s.measures.iter()).find(|m| m.measure_idx == measure_idx)
    }
}

/// Reserved clef width in em (matches `build_measure_layout`).
const CLEF_WIDTH_EM: f32 = 1.1;

/// Reserved width per time-signature digit column in em (matches
/// `build_time_sig_layout`).
const TS_DIGIT_WIDTH_EM: f32 = 0.35;

fn digit_count(mut n: u32) -> usize {
    if n == 0 {
        return 0;
    }
    let mut c = 0usize;
    while n > 0 {
        c += 1;
        n /= 10;
    }
    c
}

fn time_sig_width_em(ts: &TimeSignature) -> f32 {
    let top = digit_count(ts.beats as u32);
    let bot = digit_count(ts.beat_unit as u32);
    top.max(bot) as f32 * TS_DIGIT_WIDTH_EM
}

/// Number of sixteenth notes that fit into a measure of `ts` (at least 1).
fn sixteenth_slots(ts: &TimeSignature) -> f32 {
    let per_measure = DEFAULT_GRID.ticks_per_measure(ts);
    let per_sixteenth = DEFAULT_GRID.ticks_of(&s()).unwrap_or(1).max(1);
    (per_measure / per_sixteenth).max(1) as f32
}

/// Minimum natural width (px) for a measure given its content and which
/// header glyphs (clef / time signature) it will draw.
///
/// The body is sized for at least a measure full of sixteenths, so up to
/// that density the width depends only on the time signature: editing notes
/// or swapping measures (endless mode) never reflows the staff. Denser
/// measures (32nds, large tuplets) grow with their beat count so nothing
/// collides.
fn min_measure_width(
    score: &Score,
    measure_idx: MeasureIdx,
    show_clef: bool,
    show_ts: bool,
    opts: &StaffOpts,
) -> f32 {
    let m = &score.measures[measure_idx];
    let beats = (m.beats().len() as f32).max(sixteenth_slots(&m.time_signature()));
    let body = (opts.min_measure_width_em + beats * opts.note_width_em) * opts.em;
    let clef = if show_clef { CLEF_WIDTH_EM * opts.em } else { 0.0 };
    let ts = if show_ts { time_sig_width_em(&m.time_signature()) * opts.em } else { 0.0 };
    body + clef + ts
}

/// Legibility floor: minimum `em` a measure may be rendered at before it
/// overflows its row instead of shrinking further. The prototype behind this
/// found glyphs stop reading as distinct shapes below ~9em.
pub const LEGIBILITY_FLOOR_EM: f32 = 9.0;

/// Vertical budget (in em) for one row's content, symmetric around the
/// baseline (per-measure note positioning is center-based — see
/// `LayoutOpts::y_center` — so the budget can't be asymmetric without also
/// shifting where notes render) — just enough headroom for stems
/// (`stem_length_factor` is user configurable), flags, and tuplet brackets
/// above, and rests/accents below, so adjacent rows sit as close together as
/// legibility allows rather than leaving dead space.
fn row_height_em(opts: &StaffOpts) -> f32 {
    let above = opts.stem_length_factor + 0.75;
    let below = 0.5;
    2.0 * above.max(below)
}

/// Build the pixel layout for an entire score.
///
/// Every measure gets its own row (system), stacked top-to-bottom, and is
/// stretched to fill `opts.rect.width()`. Glyph size is shared by all rows:
/// it shrinks as needed for the widest measure to fit, never below
/// [`LEGIBILITY_FLOOR_EM`]. A measure that doesn't fit even fully shrunk
/// overflows its row (the caller is expected to host the result in a
/// scrollable area) — the rare, genuinely too-dense case, not the common one.
pub fn build_staff_layout(score: &Score, opts: &StaffOpts) -> StaffLayout {
    assert!(!score.is_empty(), "Score must have at least one measure");

    // 1. show-flags: clef only on first, TS on first + every change.
    let show_clef: Vec<bool> = (0..score.len()).map(|i| i == 0 && opts.layout_clef_first).collect();
    let show_ts: Vec<bool> = (0..score.len())
        .map(|i| {
            if i == 0 {
                true
            } else {
                score.measures[i].time_signature() != score.measures[i - 1].time_signature()
            }
        })
        .collect();

    // 2. natural widths at the baseline em.
    let widths_min: Vec<f32> = (0..score.len())
        .map(|i| min_measure_width(score, i, show_clef[i], show_ts[i], opts))
        .collect();
    let available = opts.rect.width().max(0.0);

    // 3. One glyph scale for all rows: shrink just enough for the widest
    // measure to fit, but never below the legibility floor. Widths scale
    // linearly with em, so a measure's minimum width at this scale is
    // `w * em_scale`.
    let shrink_floor_scale =
        if opts.em > 0.0 { (LEGIBILITY_FLOOR_EM / opts.em).min(1.0) } else { 1.0 };
    let widest = widths_min.iter().copied().fold(0.0f32, f32::max);
    let em_scale =
        if widest > 0.0 { (available / widest).clamp(shrink_floor_scale, 1.0) } else { 1.0 };
    let effective_opts = opts.rescaled(em_scale);

    // 4. Stack one measure per row, each stretched to fill `available`.
    let row_height = row_height_em(opts) * effective_opts.em;
    let spacing = opts.system_spacing_em * effective_opts.em;
    let left = opts.rect.left();
    let mut y_acc = opts.rect.top();
    let mut systems: Vec<SystemLayout> = Vec::with_capacity(score.len());
    let mut max_row_width = 0.0f32;

    for i in 0..score.len() {
        let width = (widths_min[i] * em_scale).max(available);
        let rect = Rect::from_min_size(pos2(left, y_acc), vec2(width, row_height));
        let per_opts = effective_opts.measure_opts(rect, show_clef[i], show_ts[i]);
        let layout = build_measure_layout(&score.measures[i], &per_opts);
        max_row_width = max_row_width.max(width);
        systems.push(SystemLayout {
            y_baseline: rect.center().y + opts.y_offset,
            rect,
            measures: vec![PlacedMeasure {
                measure_idx: i,
                rect,
                layout,
                show_clef: show_clef[i],
                show_time_signature: show_ts[i],
            }],
        });
        y_acc += row_height + spacing;
    }

    let total_height = (y_acc - spacing - opts.rect.top()).max(row_height);
    StaffLayout { total_size: vec2(max_row_width, total_height), systems, scale: em_scale }
}

/// Find `(measure_idx, beat_idx)` of the beat closest to `pos`. `pos` is in
/// the same coordinate space as `staff` (i.e. the inner space of the
/// ScrollArea content). The row — and with it the measure, one per row — is
/// picked by `pos.y` (clamped to the first/last row), then the beat by
/// `pos.x`. Returns `None` if the score has no notes anywhere.
pub fn hit_test_staff(staff: &StaffLayout, pos: Pos2) -> Option<(MeasureIdx, BeatIdx)> {
    let system =
        staff.systems.iter().find(|s| pos.y < s.rect.bottom()).or_else(|| staff.systems.last())?;
    let target = system.measures.first()?;
    let x = pos.x;

    let notes = &target.layout.notes;
    if notes.is_empty() {
        // Try any measure, in any row, with notes.
        let with_notes = staff
            .systems
            .iter()
            .flat_map(|s| s.measures.iter())
            .find(|p| !p.layout.notes.is_empty())?;
        return Some((with_notes.measure_idx, 0));
    }

    let mut best_i = 0usize;
    let mut best_d = f32::MAX;
    for (i, nl) in notes.iter().enumerate() {
        let d = (nl.center.x - x).abs();
        if d < best_d {
            best_d = d;
            best_i = i;
        }
    }
    Some((target.measure_idx, best_i))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pixel_layout::GlyphMetrics;
    use egui::{FontFamily, FontId, Pos2, Rect};
    use grooph_measure::duration::q;
    use grooph_measure::{Beat, Measure, Score, TimeSignature};

    fn opts(em: f32, rect: Rect) -> StaffOpts {
        StaffOpts {
            rect,
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            y_offset: 0.0,
            stem_length_factor: 3.5,
            stem_thickness_factor: 0.1,
            accent_displacement: 0.0,
            accent_below: false,
            proportional_spacing: true,
            debug_bbox: false,
            metrics: GlyphMetrics::debug(em),
            min_measure_width_em: 6.0,
            note_width_em: 0.6,
            system_spacing_em: 0.5,
            layout_clef_first: true,
        }
    }

    fn measure_with_quarters(ts: TimeSignature) -> Measure {
        let mut m = Measure::new(ts);
        for i in 0..(ts.beats as usize) {
            m.set_beat(i, Beat::note(q())).unwrap();
        }
        m
    }

    #[test]
    fn staff_layout_one_measure_matches_measure_layout() {
        // Equivalence: building a staff for a 1-measure score should produce the
        // same per-beat X positions as calling build_measure_layout directly with
        // the same rect.
        let em = 20.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 100.0));
        let staff_opts = opts(em, rect);

        let m = measure_with_quarters(TimeSignature::FOUR_FOUR);
        let score = Score::single(m.clone());

        let staff = build_staff_layout(&score, &staff_opts);
        assert_eq!(staff.systems.len(), 1);
        assert_eq!(staff.systems[0].measures.len(), 1);

        let placed = &staff.systems[0].measures[0];
        let direct = build_measure_layout(&m, &staff_opts.measure_opts(rect, true, true));

        assert_eq!(placed.layout.notes.len(), direct.notes.len());
        for (a, b) in placed.layout.notes.iter().zip(direct.notes.iter()) {
            assert!(
                (a.center.x - b.center.x).abs() < 0.01,
                "x mismatch: {} vs {}",
                a.center.x,
                b.center.x
            );
        }
    }

    #[test]
    fn staff_layout_show_time_signature_only_on_change() {
        let em = 20.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(4000.0, 100.0));
        let staff_opts = opts(em, rect);

        let score = Score {
            measures: vec![
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::THREE_FOUR),
                measure_with_quarters(TimeSignature::THREE_FOUR),
            ],
        };

        let staff = build_staff_layout(&score, &staff_opts);
        let flags: Vec<bool> =
            staff.systems.iter().flat_map(|s| &s.measures).map(|m| m.show_time_signature).collect();
        assert_eq!(flags, vec![true, false, true, false]);
    }

    #[test]
    fn staff_layout_show_clef_only_first() {
        let em = 20.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(4000.0, 100.0));
        let staff_opts = opts(em, rect);

        let score = Score {
            measures: vec![
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::FOUR_FOUR),
            ],
        };

        let staff = build_staff_layout(&score, &staff_opts);
        let flags: Vec<bool> =
            staff.systems.iter().flat_map(|s| &s.measures).map(|m| m.show_clef).collect();
        assert_eq!(flags, vec![true, false, false]);
    }

    #[test]
    fn staff_layout_one_measure_per_row() {
        // Plenty of width for all three measures side by side — they still
        // each get their own row, stacked top-to-bottom without overlap.
        let em = 20.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(4000.0, 100.0));
        let staff_opts = opts(em, rect);

        let score = Score {
            measures: vec![
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::FOUR_FOUR),
            ],
        };

        let staff = build_staff_layout(&score, &staff_opts);
        assert_eq!(staff.systems.len(), 3);
        for (i, system) in staff.systems.iter().enumerate() {
            assert_eq!(system.measures.len(), 1);
            assert_eq!(system.measures[0].measure_idx, i);
        }
        for w in staff.systems.windows(2) {
            assert!(
                w[0].rect.bottom() < w[1].rect.top(),
                "rows overlap: {} >= {}",
                w[0].rect.bottom(),
                w[1].rect.top(),
            );
        }
    }

    #[test]
    fn staff_layout_each_measure_fills_row() {
        // Measures narrower than the available width stretch to fill it —
        // including the first one, which carries clef + time signature.
        let em = 20.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(4000.0, 100.0));
        let staff_opts = opts(em, rect);

        let score = Score {
            measures: vec![
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::FOUR_FOUR),
            ],
        };

        let staff = build_staff_layout(&score, &staff_opts);
        assert_eq!(staff.scale, 1.0, "growing never scales glyphs");
        for system in &staff.systems {
            let w = system.measures[0].rect.width();
            assert!((w - rect.width()).abs() < 0.5, "measure should fill the row: {w}");
        }
        assert!((staff.total_size.x - rect.width()).abs() < 0.5);
    }

    #[test]
    fn staff_layout_shrinks_to_fit_above_floor() {
        // em=96 is well above the legibility floor, so glyphs may shrink a
        // lot before hitting it. The first measure (with clef + TS, the
        // widest) doesn't fit at the baseline em but does once shrunk:
        // everything shrinks just enough for it to fill the row exactly,
        // nothing overflows.
        let em = 96.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1200.0, 100.0));
        let staff_opts = opts(em, rect);

        let score = Score {
            measures: vec![
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::FOUR_FOUR),
            ],
        };

        let staff = build_staff_layout(&score, &staff_opts);
        let floor_scale = LEGIBILITY_FLOOR_EM / em;
        assert!(staff.scale < 1.0 && staff.scale > floor_scale, "scale: {}", staff.scale);
        for system in &staff.systems {
            let w = system.measures[0].rect.width();
            assert!((w - rect.width()).abs() < 0.5, "measure should fill the row: {w}");
        }
    }

    #[test]
    fn staff_layout_shrinks_to_fit_from_small_mobile_baseline() {
        // Mimics a narrow phone viewport: `compute_em` picks a small
        // baseline em (20) purely from available height/width, well before
        // content is considered. The measure's natural width at that
        // baseline doesn't fit the row — this must still shrink to fit
        // rather than permanently overflowing into forced horizontal
        // scroll, which is the common case on mobile, not an exceptional
        // one.
        let em = 20.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(200.0, 400.0));
        let staff_opts = opts(em, rect);

        let score = Score::single(measure_with_quarters(TimeSignature::FOUR_FOUR));

        let staff = build_staff_layout(&score, &staff_opts);
        assert!(
            staff.scale < 1.0,
            "expected the below-floor baseline to still shrink: {}",
            staff.scale
        );
        let w = staff.systems[0].measures[0].rect.width();
        assert!(
            (w - rect.width()).abs() < 0.5,
            "measure should fill the row instead of overflowing: {w}"
        );
    }

    #[test]
    fn staff_layout_single_measure_wider_than_floor_scrolls() {
        // A measure so dense that even at the legibility floor it doesn't
        // fit `rect.width()` — the only case allowed to overflow into
        // horizontal scroll rather than shrink further or wrap.
        let em = 96.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 100.0));
        let staff_opts = opts(em, rect);

        // A 64/4 measure gives 64 valid quarter-note slots (a plain 4/4
        // measure only has 4 — indexing beyond that panics). Needs to be
        // this dense for the floor (a low, absolute legibility limit) to
        // actually be reached before the measure fits.
        let score = Score::single(measure_with_quarters(TimeSignature { beats: 64, beat_unit: 4 }));

        let staff = build_staff_layout(&score, &staff_opts);
        assert_eq!(staff.systems.len(), 1);
        assert_eq!(staff.systems[0].measures.len(), 1);
        assert!(
            staff.total_size.x > rect.width(),
            "expected the oversized measure to overflow rect width: {} <= {}",
            staff.total_size.x,
            rect.width()
        );

        // Pinned at the floor, not shrunk further.
        let floor_scale = LEGIBILITY_FLOOR_EM / em;
        assert!((staff.scale - floor_scale).abs() < 1e-4, "expected scale pinned at the floor");
    }

    fn measure_with_sixteenths(ts: TimeSignature) -> Measure {
        use grooph_measure::duration::s;
        let mut m = Measure::new(ts);
        let mut idx = 0;
        while idx < m.beats().len() {
            m.set_beat(idx, Beat::note(s())).unwrap();
            idx += 1;
        }
        m
    }

    #[test]
    fn width_ignores_note_density_up_to_sixteenths() {
        // Two scores in the same meter, one sparse and one dense (sixteenths):
        // both get identical rows and measure rects.
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 400.0));
        let o = opts(20.0, rect);
        let sparse = Score {
            measures: vec![
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::FOUR_FOUR),
            ],
        };
        let dense = Score {
            measures: vec![
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_sixteenths(TimeSignature::FOUR_FOUR),
            ],
        };
        let a = build_staff_layout(&sparse, &o);
        let b = build_staff_layout(&dense, &o);
        assert_eq!(a.systems.len(), b.systems.len());
        assert_eq!(a.scale, b.scale);
        for (pa, pb) in a
            .systems
            .iter()
            .flat_map(|s| &s.measures)
            .zip(b.systems.iter().flat_map(|s| &s.measures))
        {
            assert_eq!(pa.rect, pb.rect);
        }
    }

    #[test]
    fn measures_denser_than_sixteenths_overflow() {
        // A measure so dense (32nds packed into a wide 16/4 bar — four times
        // a 4/4 bar's worth) that even shrinking all the way to the
        // legibility floor can't make it fit: it widens past the row
        // instead of cramping its notes further, while the plain sixteenth
        // measure still just fills the row.
        use grooph_measure::duration::th;
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 400.0));
        let o = opts(20.0, rect);
        let dense_ts = TimeSignature { beats: 16, beat_unit: 4 };
        let mut thirty_seconds = Measure::new(dense_ts);
        let mut idx = 0;
        while idx < thirty_seconds.beats().len() {
            thirty_seconds.set_beat(idx, Beat::note(th())).unwrap();
            idx += 1;
        }
        let score = Score {
            measures: vec![
                measure_with_sixteenths(TimeSignature::FOUR_FOUR),
                measure_with_sixteenths(TimeSignature::FOUR_FOUR),
                thirty_seconds,
            ],
        };
        let staff = build_staff_layout(&score, &o);
        let w1 = staff.placed(1).unwrap().rect.width();
        let w2 = staff.placed(2).unwrap().rect.width();
        assert!((w1 - rect.width()).abs() < 0.5, "16th measure should fill the row: {w1}");
        assert!(w2 > w1, "32nd measure should be wider: {w1} vs {w2}");
        assert!((staff.total_size.x - w2).abs() < 0.5);
    }

    #[test]
    fn hit_test_staff_basic() {
        let em = 20.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(4000.0, 100.0));
        let staff_opts = opts(em, rect);

        let score = Score {
            measures: vec![
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::FOUR_FOUR),
            ],
        };
        let staff = build_staff_layout(&score, &staff_opts);
        let y = staff.systems[0].rect.center().y;

        // far left -> first beat of the row's measure
        assert_eq!(hit_test_staff(&staff, Pos2::new(-100.0, y)), Some((0, 0)));
        // far right -> last beat of the row's measure
        let m0 = &staff.systems[0].measures[0];
        assert_eq!(hit_test_staff(&staff, Pos2::new(1e6, y)), Some((0, m0.layout.notes.len() - 1)));
        // above/below every row clamps to the first/last row
        assert_eq!(hit_test_staff(&staff, Pos2::new(0.0, -1e6)).map(|h| h.0), Some(0));
        assert_eq!(hit_test_staff(&staff, Pos2::new(0.0, 1e6)).map(|h| h.0), Some(2));
    }

    #[test]
    fn hit_test_staff_picks_row_by_y() {
        // All rows span the same X range, so only `pos.y` tells them apart.
        let em = 20.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(4000.0, 100.0));
        let staff_opts = opts(em, rect);

        let score = Score {
            measures: vec![
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::FOUR_FOUR),
            ],
        };
        let staff = build_staff_layout(&score, &staff_opts);
        let x = rect.center().x;
        for (i, system) in staff.systems.iter().enumerate() {
            let hit = hit_test_staff(&staff, Pos2::new(x, system.rect.center().y));
            assert_eq!(hit.map(|h| h.0), Some(i));
        }
    }
}
