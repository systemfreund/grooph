//! Multi-measure layout layer.
//!
//! Sits above [`crate::pixel_layout::build_measure_layout`] and arranges multiple
//! [`Measure`]s across one or more rows ("systems"). Cross-measure concerns
//! (per-measure width allocation, clef/time-signature repeat rules, row
//! packing, grow/shrink/wrap policy, total size for the host's scroll area)
//! live here; per-measure pixel geometry stays in `MeasureLayout`.
//!
//! Measures grow to fill a row, shrink toward [`LEGIBILITY_FLOOR_EM`] before
//! that, and wrap into a new row (`StaffLayout.systems`) once a row can't fit
//! more measures even at the floor. A single measure that alone doesn't fit
//! at the floor is left to overflow its row — the caller is expected to host
//! the result in a horizontally scrollable area for that case.

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

/// One system (row/line of music).
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
    /// scales glyph size down, but growing to fill a row only stretches note
    /// spacing, not glyph size (growing never threatens legibility). Callers
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

/// Legibility floor: minimum `em` a measure may be rendered at before the
/// layout wraps to a new row instead of shrinking further. The prototype
/// behind the original 24 found glyphs stop reading as distinct shapes below
/// ~9em; 48 is a deliberately larger margin than that measurement alone
/// implies, above the em `compute_em` typically produces in normal use
/// (~16-20).
pub const LEGIBILITY_FLOOR_EM: f32 = 48.0;

/// Vertical budget (in em) for one row's content, symmetric around the
/// baseline — generous enough for stems (`stem_length_factor` is user
/// configurable), flags, and tuplet brackets above, and rests/accents below,
/// so adjacent rows never visually collide.
fn row_height_em(opts: &StaffOpts) -> f32 { 2.0 * (opts.stem_length_factor + 3.0) }

/// Build the pixel layout for an entire score.
///
/// Measures are packed into rows (systems) left-to-right, greedily: a
/// measure joins the current row as long as the row could still be scaled to
/// fit `opts.rect.width()` without dropping below [`LEGIBILITY_FLOOR_EM`];
/// otherwise it starts a new row (a row always gets at least one measure,
/// even one that alone can't fit at the floor). One scale factor — governed
/// by the most tightly-packed row — is then shared by every row and measure,
/// so rows grow to fill the available width, shrink toward the floor before
/// that, and only overflow (the caller is expected to host the result in a
/// scrollable area) when a single measure alone doesn't fit even at the
/// floor.
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

    // 3. Greedy row assignment. `shrink_floor_scale` is the smallest scale
    // factor a row may ever be asked for: it protects the legibility floor
    // when shrinking, and is 1.0 (no shrink permitted at all) if the
    // baseline em is already at or below the floor.
    let shrink_floor_scale =
        if opts.em > 0.0 { (LEGIBILITY_FLOOR_EM / opts.em).min(1.0) } else { 1.0 };
    let row_capacity =
        if shrink_floor_scale > 0.0 { available / shrink_floor_scale } else { f32::INFINITY };

    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut current_total = 0.0f32;
    for i in 0..score.len() {
        let w = widths_min[i];
        if !current.is_empty() && current_total + w > row_capacity {
            rows.push(std::mem::take(&mut current));
            current_total = 0.0;
        }
        current.push(i);
        current_total += w;
    }
    if !current.is_empty() {
        rows.push(current);
    }

    // 4. One global (width) scale, governed by the row that needs the most
    // shrinking (or least growth) to exactly fill `available` — clamped so
    // no row is ever asked to shrink past the floor, even one that can't fit
    // at all. Growth (`global_scale > 1`) only ever stretches note spacing —
    // realized below purely as extra measure width, glyph size unaffected —
    // since growing never threatens legibility the way shrinking does.
    // Shrinking scales glyph size (`em`) down together with width, since a
    // narrower measure with full-size glyphs would just collide instead.
    let row_totals: Vec<f32> =
        rows.iter().map(|r| r.iter().map(|&i| widths_min[i]).sum::<f32>()).collect();
    let min_required_scale = row_totals
        .iter()
        .filter(|&&t| t > 0.0)
        .map(|&t| available / t)
        .fold(f32::INFINITY, f32::min);
    let global_scale = if min_required_scale.is_finite() {
        min_required_scale.max(shrink_floor_scale)
    } else {
        1.0
    };
    let em_scale = global_scale.min(1.0);

    let effective_opts = opts.rescaled(em_scale);
    let widths: Vec<f32> = widths_min.iter().map(|w| w * global_scale).collect();

    // 5. Stack rows top-to-bottom, laying out each left-to-right.
    let row_height = row_height_em(opts) * effective_opts.em;
    let spacing = opts.system_spacing_em * effective_opts.em;
    let left = opts.rect.left();
    let mut y_acc = opts.rect.top();
    let mut systems: Vec<SystemLayout> = Vec::with_capacity(rows.len());
    let mut max_row_width = 0.0f32;

    for row in &rows {
        let row_top = y_acc;
        let mut x_acc = left;
        let mut measures: Vec<PlacedMeasure> = Vec::with_capacity(row.len());
        for &i in row {
            let rect = Rect::from_min_size(pos2(x_acc, row_top), vec2(widths[i], row_height));
            let per_opts = effective_opts.measure_opts(rect, show_clef[i], show_ts[i]);
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
        let row_width = x_acc - left;
        max_row_width = max_row_width.max(row_width);
        let system_rect = Rect::from_min_size(pos2(left, row_top), vec2(row_width, row_height));
        systems.push(SystemLayout {
            y_baseline: system_rect.center().y + opts.y_offset,
            rect: system_rect,
            measures,
        });
        y_acc = row_top + row_height + spacing;
    }

    let total_height = (y_acc - spacing - opts.rect.top()).max(row_height);
    StaffLayout { total_size: vec2(max_row_width, total_height), systems, scale: em_scale }
}

/// Find `(measure_idx, beat_idx)` of the beat closest to `pos`. `pos` is in
/// the same coordinate space as `staff` (i.e. the inner space of the
/// ScrollArea content). The row (system) is picked by `pos.y` first, then
/// the measure within that row by `pos.x`; both clamp to the nearest edge
/// when `pos` falls outside every row/measure. Returns `None` if the score
/// has no notes anywhere.
pub fn hit_test_staff(staff: &StaffLayout, pos: Pos2) -> Option<(MeasureIdx, BeatIdx)> {
    if staff.systems.is_empty() {
        return None;
    }
    let last_system = staff.systems.len() - 1;
    let system = if pos.y <= staff.systems[0].rect.top() {
        &staff.systems[0]
    } else if pos.y >= staff.systems[last_system].rect.bottom() {
        &staff.systems[last_system]
    } else {
        staff
            .systems
            .iter()
            .find(|s| pos.y < s.rect.bottom())
            .unwrap_or(&staff.systems[last_system])
    };

    let placed = &system.measures;
    if placed.is_empty() {
        return None;
    }

    let x = pos.x;
    let target = if x <= placed[0].rect.left() {
        &placed[0]
    } else if x >= placed[placed.len() - 1].rect.right() {
        &placed[placed.len() - 1]
    } else {
        let mut found = &placed[placed.len() - 1];
        for p in placed {
            if x < p.rect.right() {
                found = p;
                break;
            }
        }
        found
    };

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
    fn staff_layout_fits_one_row_grows_to_fill() {
        // Content comfortably narrower than the available width: stays a
        // single system, scaled up to fill it exactly — matches the
        // pre-wrap single-system grow-to-fill behavior.
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
        assert_eq!(staff.systems.len(), 1);
        assert!(
            (staff.total_size.x - rect.width()).abs() < 0.5,
            "should grow to exactly fill the available width: {}",
            staff.total_size.x
        );
    }

    #[test]
    fn staff_layout_wraps_into_multiple_rows() {
        // em=96 is double the legibility floor (48), so rows may shrink to
        // half size before wrapping. A row holds measure 0 (with clef+TS,
        // wider) plus measure 1 within that shrink budget, but a third
        // measure would drop the row below the floor — it starts a new row.
        let em = 96.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1000.0, 100.0));
        let staff_opts = opts(em, rect);

        let score = Score {
            measures: vec![
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::FOUR_FOUR),
            ],
        };

        let staff = build_staff_layout(&score, &staff_opts);
        assert_eq!(staff.systems.len(), 2, "expected content to wrap into 2 rows");
        assert_eq!(staff.systems[0].measures.len(), 2);
        assert_eq!(staff.systems[1].measures.len(), 1);
        assert_eq!(staff.systems[1].measures[0].measure_idx, 2);

        // The floor was never actually needed here — the governing row's
        // required scale was already above it.
        let floor_scale = LEGIBILITY_FLOOR_EM / em;
        assert!(staff.scale > floor_scale);

        // Second row sits below the first, separated by system_spacing_em.
        assert!(staff.systems[1].rect.top() > staff.systems[0].rect.bottom());
    }

    #[test]
    fn staff_layout_single_measure_wider_than_floor_scrolls() {
        // A measure so dense that even at the legibility floor it doesn't
        // fit `rect.width()` — the only case allowed to overflow into
        // horizontal scroll rather than shrink further or wrap.
        let em = 96.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 100.0));
        let staff_opts = opts(em, rect);

        // A 20/4 measure gives 20 valid quarter-note slots (a plain 4/4
        // measure only has 4 — indexing beyond that panics).
        let score = Score::single(measure_with_quarters(TimeSignature { beats: 20, beat_unit: 4 }));

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

        // far left -> first beat of first measure
        assert_eq!(hit_test_staff(&staff, Pos2::new(-100.0, y)), Some((0, 0)));
        // far right -> last beat of last measure (3 quarters -> beats=4, last_idx=3)
        let last = staff.systems[0].measures.last().unwrap();
        assert_eq!(
            hit_test_staff(&staff, Pos2::new(1e6, y)),
            Some((last.measure_idx, last.layout.notes.len() - 1))
        );

        // click in the middle of measure 1 should hit measure 1
        let m1 = &staff.systems[0].measures[1];
        let hit_pos = Pos2::new(m1.rect.center().x, y);
        let hit = hit_test_staff(&staff, hit_pos).expect("should hit");
        assert_eq!(hit.0, 1);
    }

    #[test]
    fn hit_test_staff_picks_row_by_y() {
        // Same wrap scenario as staff_layout_wraps_into_multiple_rows: 3
        // measures, 2 rows. A click within the second row's vertical span
        // should hit measure 2, even though its X also lies under row 1's
        // measures (rows can differ in width).
        let em = 96.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1000.0, 100.0));
        let staff_opts = opts(em, rect);

        let score = Score {
            measures: vec![
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::FOUR_FOUR),
            ],
        };
        let staff = build_staff_layout(&score, &staff_opts);
        assert_eq!(staff.systems.len(), 2);

        let row1_y = staff.systems[1].rect.center().y;
        let x = staff.systems[1].measures[0].rect.center().x;
        let hit = hit_test_staff(&staff, Pos2::new(x, row1_y)).expect("should hit");
        assert_eq!(hit.0, 2);
    }
}
