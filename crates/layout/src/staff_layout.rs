//! Multi-measure layout layer.
//!
//! Sits above [`crate::pixel_layout::build_measure_layout`] and arranges multiple
//! [`Measure`]s along a single horizontal staff (scrolling). Cross-measure concerns
//! (per-measure width allocation, clef/time-signature repeat rules, total size for
//! the host's scroll area) live here; per-measure pixel geometry stays in
//! `MeasureLayout`.
//!
//! Wrapping into multiple systems is not implemented yet, but the data shape
//! (`StaffLayout.systems: Vec<SystemLayout>`) is ready for it.

use crate::pixel_layout::{GlyphMetrics, LayoutOpts, MeasureLayout, build_measure_layout};
use egui::{FontId, Pos2, Rect, Vec2, pos2, vec2};
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
    /// Vertical spacing between systems in em (reserved for line-wrap mode).
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

/// One system (line of music). For now we always produce exactly one.
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

/// Minimum natural width (px) for a measure given its content and which
/// header glyphs (clef / time signature) it will draw.
fn min_measure_width(
    score: &Score,
    measure_idx: MeasureIdx,
    show_clef: bool,
    show_ts: bool,
    opts: &StaffOpts,
) -> f32 {
    let m = &score.measures[measure_idx];
    let beats = m.beats().len().max(1) as f32;
    let body = (opts.min_measure_width_em + beats * opts.note_width_em) * opts.em;
    let clef = if show_clef { CLEF_WIDTH_EM * opts.em } else { 0.0 };
    let ts = if show_ts { time_sig_width_em(&m.time_signature()) * opts.em } else { 0.0 };
    body + clef + ts
}

/// Build the pixel layout for an entire score.
///
/// Scroll mode: all measures go into a single system. If the natural total
/// width fits inside `opts.rect.width()`, measures are scaled up proportionally
/// to fill it (this preserves single-measure equivalence with
/// `build_measure_layout`). Otherwise each measure takes its natural width and
/// the caller is expected to host the result in a horizontal `ScrollArea`.
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

    // 2. natural widths and overall scale.
    let widths_min: Vec<f32> = (0..score.len())
        .map(|i| min_measure_width(score, i, show_clef[i], show_ts[i], opts))
        .collect();
    let total_min: f32 = widths_min.iter().sum();
    let available = opts.rect.width().max(0.0);
    let scale = if total_min > 0.0 && total_min < available { available / total_min } else { 1.0 };
    let widths: Vec<f32> = widths_min.iter().map(|w| w * scale).collect();

    // 3. lay out left-to-right.
    let top = opts.rect.top();
    let height = opts.rect.height();
    let mut x_acc = opts.rect.left();
    let mut measures: Vec<PlacedMeasure> = Vec::with_capacity(score.len());

    for i in 0..score.len() {
        let rect = Rect::from_min_size(pos2(x_acc, top), vec2(widths[i], height));
        let per_opts = opts.measure_opts(rect, show_clef[i], show_ts[i]);
        let layout = build_measure_layout(&score.measures[i], &per_opts);
        measures.push(PlacedMeasure {
            measure_idx: i,
            rect,
            layout,
            show_clef: show_clef[i],
            show_time_signature: show_ts[i],
        });
        x_acc += widths[i];
    }

    let system_rect =
        Rect::from_min_size(pos2(opts.rect.left(), top), vec2(x_acc - opts.rect.left(), height));
    let system = SystemLayout {
        y_baseline: opts.rect.center().y + opts.y_offset,
        rect: system_rect,
        measures,
    };

    StaffLayout { total_size: vec2(x_acc - opts.rect.left(), height), systems: vec![system] }
}

/// Find `(measure_idx, beat_idx)` of the beat closest to `x`. `x` is the
/// pointer X in the same coordinate space as `staff` (i.e. the inner space of
/// the ScrollArea content). Returns `None` if the score has no notes anywhere.
pub fn hit_test_staff(staff: &StaffLayout, x: f32) -> Option<(MeasureIdx, BeatIdx)> {
    // Find the placed measure whose horizontal span contains x. If x is
    // outside all measures, clamp to the first or last.
    let placed: Vec<&PlacedMeasure> =
        staff.systems.iter().flat_map(|s| s.measures.iter()).collect();
    if placed.is_empty() {
        return None;
    }

    let target = if x <= placed[0].rect.left() {
        placed[0]
    } else if x >= placed[placed.len() - 1].rect.right() {
        placed[placed.len() - 1]
    } else {
        let mut found = placed[placed.len() - 1];
        for p in &placed {
            if x < p.rect.right() {
                found = p;
                break;
            }
        }
        found
    };

    let notes = &target.layout.notes;
    if notes.is_empty() {
        // Try neighbors with notes.
        let with_notes = placed.iter().find(|p| !p.layout.notes.is_empty())?;
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
            system_spacing_em: 4.0,
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
            staff.systems[0].measures.iter().map(|m| m.show_time_signature).collect();
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
        let flags: Vec<bool> = staff.systems[0].measures.iter().map(|m| m.show_clef).collect();
        assert_eq!(flags, vec![true, false, false]);
    }

    #[test]
    fn staff_layout_measures_placed_horizontally() {
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
        let placed = &staff.systems[0].measures;
        for w in placed.windows(2) {
            assert!(
                w[0].rect.right() <= w[1].rect.left() + 0.01,
                "measure {} overlaps {}: {} > {}",
                w[0].measure_idx,
                w[1].measure_idx,
                w[0].rect.right(),
                w[1].rect.left(),
            );
        }
    }

    #[test]
    fn staff_layout_total_size_monotonic() {
        let em = 20.0;
        // small rect so the layout does not get scaled up to fill it
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(50.0, 100.0));
        let staff_opts = opts(em, rect);

        let one = Score::single(measure_with_quarters(TimeSignature::FOUR_FOUR));
        let two = Score {
            measures: vec![
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::FOUR_FOUR),
            ],
        };
        let four = Score {
            measures: vec![
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::FOUR_FOUR),
            ],
        };

        let w1 = build_staff_layout(&one, &staff_opts).total_size.x;
        let w2 = build_staff_layout(&two, &staff_opts).total_size.x;
        let w4 = build_staff_layout(&four, &staff_opts).total_size.x;
        assert!(w1 < w2, "{w1} should be < {w2}");
        assert!(w2 < w4, "{w2} should be < {w4}");
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

        // far left -> first beat of first measure
        assert_eq!(hit_test_staff(&staff, -100.0), Some((0, 0)));
        // far right -> last beat of last measure (3 quarters -> beats=4, last_idx=3)
        let last = staff.systems[0].measures.last().unwrap();
        assert_eq!(
            hit_test_staff(&staff, 1e6),
            Some((last.measure_idx, last.layout.notes.len() - 1))
        );

        // click in the middle of measure 1 should hit measure 1
        let m1 = &staff.systems[0].measures[1];
        let hit_x = m1.rect.center().x;
        let hit = hit_test_staff(&staff, hit_x).expect("should hit");
        assert_eq!(hit.0, 1);
    }
}
