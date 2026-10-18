//! Multi-measure layout layer.
//!
//! Sits above [`crate::pixel_layout::build_measure_layout`] and arranges the
//! [`Measure`]s of a score top-to-bottom, one measure per row ("system") —
//! the app targets phones, where a predictable one-bar-per-line layout reads
//! best. Cross-measure concerns (per-measure width allocation, clef/time-
//! signature repeat rules, grow/shrink policy, total size for the host's
//! scroll area) live here; per-measure pixel geometry stays in `MeasureLayout`.
//!
//! A staff can have a lower voice (see [`build_staff_layout_with_lower`]): a
//! second score on a line of its own below, stems down, spaced like the
//! upper voice so notes at the same time line up — like the feet under the
//! hands of a drum part.
//!
//! Each measure grows to fill the row. If a measure doesn't fit, glyphs shrink
//! toward [`LEGIBILITY_FLOOR_EM`]; a measure that doesn't fit even at the floor
//! overflows its row — the caller is expected to host the result in a
//! horizontally scrollable area for that case.

use crate::pixel_layout::{
    GlyphMetrics, LayoutOpts, MeasureLayout, build_measure_layout_with_plan, min_note_area_width,
};
use crate::render_plan::{RenderPlan, plan_measure};
use egui::{FontId, Pos2, Rect, Vec2, pos2, vec2};
use grooph_measure::duration::s;
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::{BeatIdx, BeatKind, MeasureIdx, Score, TimeSignature};

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
    /// Room below the staff line in em: rests, accents and timing marks need
    /// [`ROOM_BELOW_EM`], count labels [`ROOM_BELOW_WITH_LABELS_EM`].
    pub room_below_em: f32,
    /// Headroom above the stems in em beyond the usual: see
    /// [`extra_room_above_em`].
    pub room_above_extra_em: f32,
}

impl StaffOpts {
    /// Build per-measure [`LayoutOpts`] for a placed measure (renderer entry).
    pub fn measure_opts_for(&self, placed: &PlacedMeasure) -> LayoutOpts {
        self.measure_opts(placed.rect, placed.show_clef, placed.show_time_signature)
    }

    /// Per-measure [`LayoutOpts`] of the lower voice of a placed measure:
    /// on its own line [`LOWER_VOICE_GAP_EM`] below, stems down.
    pub fn lower_opts_for(&self, placed: &PlacedMeasure) -> LayoutOpts {
        let opts = self.measure_opts(placed.rect, placed.show_clef, placed.show_time_signature);
        Self::lower_of(opts)
    }

    fn lower_of(opts: LayoutOpts) -> LayoutOpts {
        LayoutOpts {
            y_offset: opts.y_offset + LOWER_VOICE_GAP_EM * opts.em,
            stem_down: true,
            accent_below: false,
            ..opts
        }
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
            y_offset: self.y_offset + self.baseline_shift(),
            stem_length_factor: self.stem_length_factor,
            stem_thickness_factor: self.stem_thickness_factor,
            accent_displacement: self.accent_displacement,
            accent_below: self.accent_below,
            stem_down: false,
            proportional_spacing: self.proportional_spacing,
            debug_bbox: self.debug_bbox,
            metrics: self.metrics,
        }
    }

    /// Distance from a row's center down to its staff line: rows are taller
    /// above the line (stems, beams, tuplets) than below it.
    fn baseline_shift(&self) -> f32 {
        (room_above_em(self) - self.room_below_em) * 0.5 * self.em
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
    /// The lower voice's measure, if the staff has one.
    pub lower: Option<MeasureLayout>,
    pub show_clef: bool,
    pub show_time_signature: bool,
}

/// One system (row/line of music). Currently always holds exactly one measure.
#[derive(Debug, Clone)]
pub struct SystemLayout {
    pub y_baseline: f32,
    /// The lower voice's line, if the staff has one.
    pub lower_baseline: Option<f32>,
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
/// that density the width usually depends only on the time signature:
/// editing notes or swapping measures (endless mode) rarely reflows the
/// staff. Denser measures (32nds, large tuplets) grow with their beat count,
/// and measures mixing very short and long notes grow until the short ones,
/// spaced by duration, no longer collide.
fn min_measure_width(
    score: &Score,
    measure_idx: MeasureIdx,
    plan: &RenderPlan,
    show_clef: bool,
    show_ts: bool,
    opts: &StaffOpts,
) -> f32 {
    let m = &score.measures[measure_idx];
    let beats = (m.beats().len() as f32).max(sixteenth_slots(&m.time_signature()));
    let body = (opts.min_measure_width_em + beats * opts.note_width_em) * opts.em;
    // Short notes next to long ones (e.g. 32nds among eighths) get little
    // space when spaced by duration: widen until no glyph overlaps the next.
    let body = body.max(min_note_area_width(m, plan, &opts.measure_opts(opts.rect, false, false)));
    let clef = if show_clef { CLEF_WIDTH_EM * opts.em } else { 0.0 };
    let ts = if show_ts { time_sig_width_em(&m.time_signature()) * opts.em } else { 0.0 };
    body + clef + ts
}

/// Legibility floor: minimum `em` a measure may be rendered at before it
/// overflows its row instead of shrinking further. The prototype behind this
/// found glyphs stop reading as distinct shapes below ~9em.
pub const LEGIBILITY_FLOOR_EM: f32 = 9.0;

/// Room below the staff line for rests, accents and timing marks, in em.
pub const ROOM_BELOW_EM: f32 = 0.65;
/// Room below the staff line when count labels are drawn under the notes.
pub const ROOM_BELOW_WITH_LABELS_EM: f32 = 1.0;

/// Headroom above the staff line, in em: stems (`stem_length_factor` is user
/// configurable) and flags, plus what the score puts above them (see
/// [`extra_room_above_em`]).
fn room_above_em(opts: &StaffOpts) -> f32 {
    opts.stem_length_factor + 0.15 + opts.room_above_extra_em
}

/// Headroom `score` needs above its stems and beams (for
/// [`StaffOpts::room_above_extra_em`]), in em: accents written above the
/// notes, tuplet brackets and numbers, and a tuplet bracket rising over the
/// accents above its notes. Rows stay free of space nothing in the score uses.
pub fn extra_room_above_em(score: &Score, accent_below: bool) -> f32 {
    let notes =
        || score.measures.iter().flat_map(|m| m.beats()).filter(|b| b.kind == BeatKind::Note);
    let accents_above = !accent_below && notes().any(|b| b.accented);
    let tuplets =
        score.measures.iter().flat_map(|m| m.beats()).any(|b| b.tuplet_group_id.is_some());
    let accented_tuplet =
        !accent_below && notes().any(|b| b.accented && b.tuplet_group_id.is_some());
    let mut extra = 0.0;
    if tuplets {
        extra += 0.5;
    } else if accents_above {
        extra += 0.25;
    }
    if accented_tuplet {
        extra += 0.5;
    }
    extra
}

/// Distance from the staff line down to the lower voice's line, in em: three
/// staff spaces, about two lines of a five-line staff apart. The lower
/// voice's rests hang below its line to clear the upper voice's.
pub const LOWER_VOICE_GAP_EM: f32 = 0.75;
/// Room below a lower voice for count labels, in em.
pub const LOWER_VOICE_LABEL_ROOM_EM: f32 = 0.5;

/// Room below the staff line a lower voice `lower` needs (for
/// [`StaffOpts::room_below_em`]), in em: down to its line, then its stems
/// and what hangs below them.
pub fn lower_voice_room_below_em(lower: &Score, opts: &StaffOpts) -> f32 {
    LOWER_VOICE_GAP_EM
        + room_above_em(&StaffOpts {
            room_above_extra_em: extra_room_above_em(lower, false),
            ..opts.clone()
        })
}

/// Vertical budget (in em) for one row's content: the headroom above the
/// staff line plus the room below it, so adjacent rows sit as close together
/// as legibility allows rather than leaving dead space. The staff line sits
/// off-center accordingly (see [`StaffOpts::baseline_shift`]).
pub fn row_height_em(opts: &StaffOpts) -> f32 {
    room_above_em(opts) + opts.room_below_em
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
    build_staff_layout_with_lower(score, None, opts)
}

/// [`build_staff_layout`] with a lower voice: `lower`'s measures under the
/// score's, one per row, each as wide as the wider of the two. The rows need
/// [`lower_voice_room_below_em`] in [`StaffOpts::room_below_em`].
pub fn build_staff_layout_with_lower(
    score: &Score,
    lower: Option<&Score>,
    opts: &StaffOpts,
) -> StaffLayout {
    assert!(!score.is_empty(), "Score must have at least one measure");

    // 1. show-flags: clef only on first. The time signature only shows when
    // the score mixes meters, then on the first measure + every change.
    let show_clef: Vec<bool> = (0..score.len()).map(|i| i == 0 && opts.layout_clef_first).collect();
    let meter_changes_at =
        |i: usize| score.measures[i].time_signature() != score.measures[i - 1].time_signature();
    let mixed_meters = (1..score.len()).any(meter_changes_at);
    let show_ts: Vec<bool> =
        (0..score.len()).map(|i| mixed_meters && (i == 0 || meter_changes_at(i))).collect();

    // 2. natural widths at the baseline em. Each measure is planned (beams,
    // tuplets) once, for its width and its layout.
    let plans: Vec<RenderPlan> = score.measures.iter().map(plan_measure).collect();
    let lower_measures = |i: usize| lower.and_then(|l| l.measures.get(i));
    let lower_plans: Vec<Option<RenderPlan>> =
        (0..score.len()).map(|i| lower_measures(i).map(plan_measure)).collect();
    let widths_min: Vec<f32> = (0..score.len())
        .map(|i| {
            let upper = min_measure_width(score, i, &plans[i], show_clef[i], show_ts[i], opts);
            let lower = match (lower, &lower_plans[i]) {
                (Some(l), Some(plan)) => {
                    min_measure_width(l, i, plan, show_clef[i], show_ts[i], opts)
                }
                _ => 0.0,
            };
            upper.max(lower)
        })
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
        let layout = build_measure_layout_with_plan(&score.measures[i], &plans[i], &per_opts);
        let lower_opts = StaffOpts::lower_of(per_opts);
        let lower_layout = lower_measures(i)
            .zip(lower_plans[i].as_ref())
            .map(|(m, plan)| build_measure_layout_with_plan(m, plan, &lower_opts));
        max_row_width = max_row_width.max(width);
        let y_baseline =
            rect.center().y + effective_opts.y_offset + effective_opts.baseline_shift();
        systems.push(SystemLayout {
            y_baseline,
            lower_baseline: lower_layout.as_ref().map(|_| lower_opts.y_center()),
            rect,
            measures: vec![PlacedMeasure {
                measure_idx: i,
                rect,
                layout,
                lower: lower_layout,
                show_clef: show_clef[i],
                show_time_signature: show_ts[i],
            }],
        });
        y_acc += row_height + spacing;
    }

    let total_height = (y_acc - spacing - opts.rect.top()).max(row_height);
    StaffLayout { total_size: vec2(max_row_width, total_height), systems, scale: em_scale }
}

/// One of the staff's two voices: the upper one on the staff line, or the
/// lower one on its own line below.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Voice {
    #[default]
    Upper,
    Lower,
}

/// Find `(measure_idx, beat_idx)` of the beat closest to `pos`. `pos` is in
/// the same coordinate space as `staff` (i.e. the inner space of the
/// ScrollArea content). The row — and with it the measure, one per row — is
/// picked by `pos.y` (clamped to the first/last row), then the beat by
/// `pos.x`. Returns `None` if the score has no notes anywhere.
pub fn hit_test_staff(staff: &StaffLayout, pos: Pos2) -> Option<(MeasureIdx, BeatIdx)> {
    hit_test_voice(staff, pos).map(|(_, m, b)| (m, b))
}

/// Like [`hit_test_staff`], with the voice whose line is nearer `pos`: on a
/// staff with a lower voice, below halfway between the two lines picks the
/// lower one and the beat among its notes.
pub fn hit_test_voice(staff: &StaffLayout, pos: Pos2) -> Option<(Voice, MeasureIdx, BeatIdx)> {
    let system =
        staff.systems.iter().find(|s| pos.y < s.rect.bottom()).or_else(|| staff.systems.last())?;
    let target = system.measures.first()?;
    if let (Some(lower), Some(y)) = (&target.lower, system.lower_baseline)
        && pos.y > (system.y_baseline + y) / 2.0
        && !lower.notes.is_empty()
    {
        return Some((Voice::Lower, target.measure_idx, nearest_note(lower, pos.x)));
    }

    if target.layout.notes.is_empty() {
        // Try any measure, in any row, with notes.
        let with_notes = staff
            .systems
            .iter()
            .flat_map(|s| s.measures.iter())
            .find(|p| !p.layout.notes.is_empty())?;
        return Some((Voice::Upper, with_notes.measure_idx, 0));
    }
    Some((Voice::Upper, target.measure_idx, nearest_note(&target.layout, pos.x)))
}

/// Index of the note of `layout` nearest `x`.
fn nearest_note(layout: &MeasureLayout, x: f32) -> BeatIdx {
    let mut best_i = 0usize;
    let mut best_d = f32::MAX;
    for (i, nl) in layout.notes.iter().enumerate() {
        let d = (nl.center.x - x).abs();
        if d < best_d {
            best_d = d;
            best_i = i;
        }
    }
    best_i
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
            room_below_em: ROOM_BELOW_EM,
            room_above_extra_em: 0.0,
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
        let direct = crate::pixel_layout::build_measure_layout(
            &m,
            &staff_opts.measure_opts(rect, true, false),
        );

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
    fn lower_voice_lines_up_below() {
        use grooph_measure::notation::parse_score;
        let em = 20.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 100.0));
        let upper = parse_score("4/4 >q q q q").unwrap();
        let lower = parse_score("4/4 t4 r:t4 (t4) r:t4 (t4) r:t4").unwrap();
        let mut staff_opts = opts(em, rect);
        staff_opts.room_below_em = lower_voice_room_below_em(&lower, &staff_opts);
        let staff = build_staff_layout_with_lower(&upper, Some(&lower), &staff_opts);
        let system = &staff.systems[0];
        let gap = system.lower_baseline.unwrap() - system.y_baseline;
        assert!((gap - LOWER_VOICE_GAP_EM * em).abs() < 0.01);
        let placed = &system.measures[0];
        let below = placed.lower.as_ref().unwrap();
        // Both voices start their beats at the same x: beat 1 and beat 3.
        assert!((placed.layout.notes[0].center.x - below.notes[0].center.x).abs() < 0.5);
        assert!((placed.layout.notes[2].center.x - below.notes[3].center.x).abs() < 0.5);
        // The lower voice's stems end inside the row.
        let stem_end = below.notes[0].stem.unwrap().p2.y;
        assert!(stem_end > system.lower_baseline.unwrap() && stem_end < placed.rect.bottom());
        assert!(build_staff_layout(&upper, &opts(em, rect)).systems[0].lower_baseline.is_none());
    }

    #[test]
    fn hit_test_picks_the_nearer_voice() {
        use grooph_measure::notation::parse_score;
        let em = 20.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 100.0));
        let upper = parse_score("4/4 >q q q q").unwrap();
        let lower = parse_score("4/4 t4 r:t4 (t4) r:t4 (t4) r:t4").unwrap();
        let mut staff_opts = opts(em, rect);
        staff_opts.room_below_em = lower_voice_room_below_em(&lower, &staff_opts);
        let staff = build_staff_layout_with_lower(&upper, Some(&lower), &staff_opts);
        let system = &staff.systems[0];
        let below = system.measures[0].lower.as_ref().unwrap();
        let x = below.notes[3].center.x;
        let lower_y = system.lower_baseline.unwrap();
        assert_eq!(hit_test_voice(&staff, Pos2::new(x, lower_y)), Some((Voice::Lower, 0, 3)));
        assert_eq!(
            hit_test_voice(&staff, Pos2::new(x, system.y_baseline)),
            Some((Voice::Upper, 0, 2))
        );
        // Without a lower voice everything hits the upper one.
        let single = build_staff_layout(&upper, &opts(em, rect));
        let y = single.systems[0].rect.bottom() - 1.0;
        assert_eq!(hit_test_voice(&single, Pos2::new(x, y)).map(|h| h.0), Some(Voice::Upper));
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
    fn staff_layout_hides_time_signature_for_single_meter() {
        let em = 20.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(4000.0, 100.0));
        let staff_opts = opts(em, rect);

        let score = Score {
            measures: vec![
                measure_with_quarters(TimeSignature::THREE_FOUR),
                measure_with_quarters(TimeSignature::THREE_FOUR),
            ],
        };

        let staff = build_staff_layout(&score, &staff_opts);
        let placed: Vec<_> = staff.systems.iter().flat_map(|s| &s.measures).collect();
        assert!(placed.iter().all(|m| !m.show_time_signature && m.layout.time_signature.is_none()));
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
        let score = Score::single(measure_with_quarters(TimeSignature::new(64, 4)));

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
        let dense_ts = TimeSignature::new(16, 4);
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

    #[test]
    fn staff_line_sits_on_note_heads_with_off_center_rows() {
        let em = 20.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(4000.0, 100.0));
        let score = Score {
            measures: vec![
                measure_with_quarters(TimeSignature::FOUR_FOUR),
                measure_with_quarters(TimeSignature::FOUR_FOUR),
            ],
        };
        for below in [ROOM_BELOW_EM, ROOM_BELOW_WITH_LABELS_EM] {
            let staff_opts = StaffOpts { room_below_em: below, ..opts(em, rect) };
            let staff = build_staff_layout(&score, &staff_opts);
            for system in &staff.systems {
                let placed = &system.measures[0];
                let note_y = placed.layout.notes[0].center.y;
                assert!((note_y - system.y_baseline).abs() < 0.01);
                let below_px = system.rect.bottom() - system.y_baseline;
                assert!((below_px - below * em).abs() < 0.01);
                // The renderer rebuilds the opts from the placed rect.
                let render_opts = staff_opts.measure_opts_for(placed);
                assert!((render_opts.y_center() - system.y_baseline).abs() < 0.01);
            }
        }
    }

    #[test]
    fn staff_layout_counts_a_grouping_change_as_a_meter_change() {
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(4000.0, 100.0));
        let grouped = TimeSignature::SEVEN_EIGHT.with_grouping(1);
        let score = Score {
            measures: vec![
                Measure::new(TimeSignature::SEVEN_EIGHT),
                Measure::new(grouped),
                Measure::new(grouped),
            ],
        };

        let staff = build_staff_layout(&score, &opts(20.0, rect));
        let flags: Vec<bool> =
            staff.systems.iter().flat_map(|s| &s.measures).map(|m| m.show_time_signature).collect();
        assert_eq!(flags, vec![true, true, false]);
    }

    #[test]
    fn headroom_only_for_what_the_score_puts_above_the_stems() {
        use grooph_measure::duration::t8;
        let plain = Score { measures: vec![measure_with_quarters(TimeSignature::FOUR_FOUR)] };
        assert_eq!(extra_room_above_em(&plain, false), 0.0);

        let mut accented = measure_with_quarters(TimeSignature::FOUR_FOUR);
        accented.toggle_accent(0).unwrap();
        let accented = Score { measures: vec![accented] };
        assert_eq!(extra_room_above_em(&accented, false), 0.25);
        assert_eq!(extra_room_above_em(&accented, true), 0.0);

        let mut triplet = measure_with_quarters(TimeSignature::FOUR_FOUR);
        triplet.set_beat(0, Beat::note(t8())).unwrap();
        let triplet = Score { measures: vec![triplet] };
        assert_eq!(extra_room_above_em(&triplet, false), 0.5);
    }
}
