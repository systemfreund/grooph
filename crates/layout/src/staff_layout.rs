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
//! second score on a line of its own below, stems down, its beams running
//! over its rests as stemlets, spaced like the upper voice so notes at the
//! same time line up — like the feet under the hands of a drum part. With
//! [`StaffOpts::one_line`] both voices share one line instead, the upper one
//! above it and the lower one below.
//!
//! Each measure grows to fill the row. If a measure doesn't fit, glyphs shrink
//! toward [`LEGIBILITY_FLOOR_EM`]; a measure that doesn't fit even at the floor
//! overflows its row — the caller is expected to host the result in a
//! horizontally scrollable area for that case.

use crate::pixel_layout::{
    LayoutOpts, MeasureLayout, build_measure_layout_with_plan, clef_width, min_note_area_width,
    time_sig_width,
};
use crate::render_plan::{RenderPlan, plan_measure_with};
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

    /// Minimum width of a measure in em, before per-beat additions.
    pub min_measure_width_em: f32,
    /// Width added per beat in em, to grow dense measures.
    pub note_width_em: f32,
    /// Vertical spacing between systems (rows) in em.
    pub system_spacing_em: f32,
    /// Whether to show the clef on the first measure of each system.
    pub layout_clef_first: bool,
    /// Room below the staff line in em, at least: rests, accents and timing
    /// marks need [`ROOM_BELOW_EM`], count labels
    /// [`ROOM_BELOW_WITH_LABELS_EM`].
    pub room_below_em: f32,
    /// Room under a lower voice's notes in em, for count labels: 0 or
    /// [`LOWER_VOICE_LABEL_ROOM_EM`].
    pub lower_label_room_em: f32,
    /// A lower voice shares the staff line instead of having its own: the
    /// upper voice's heads sit on the line, the lower one's hang from it
    /// (see [`LayoutOpts::shared_line`]).
    pub one_line: bool,
}

impl StaffOpts {
    /// Build per-measure [`LayoutOpts`] for a placed measure (renderer entry).
    pub fn measure_opts_for(&self, placed: &PlacedMeasure) -> LayoutOpts {
        let opts = self.measure_opts(
            placed.rect,
            placed.y_offset,
            placed.show_clef,
            placed.show_time_signature,
        );
        self.upper_of(opts, placed.lower.is_some())
    }

    /// Per-measure [`LayoutOpts`] of the lower voice of a placed measure:
    /// stems down, on its own line [`LOWER_VOICE_GAP_EM`] below or below
    /// the shared one.
    pub fn lower_opts_for(&self, placed: &PlacedMeasure) -> LayoutOpts {
        self.lower_of(self.measure_opts_for(placed))
    }

    /// `opts` for the upper voice, which shares its line if `with_lower`.
    fn upper_of(&self, opts: LayoutOpts, with_lower: bool) -> LayoutOpts {
        LayoutOpts { shared_line: self.one_line && with_lower, ..opts }
    }

    fn lower_of(&self, opts: LayoutOpts) -> LayoutOpts {
        LayoutOpts {
            y_offset: opts.y_offset + self.lower_voice_gap_em() * opts.em,
            stem_down: true,
            shared_line: self.one_line,
            accent_below: false,
            ..opts
        }
    }

    /// Distance from the staff line down to the lower voice's, in em.
    fn lower_voice_gap_em(&self) -> f32 {
        if self.one_line { 0.0 } else { LOWER_VOICE_GAP_EM }
    }

    /// Build per-measure [`LayoutOpts`] with a measure-local rect, the staff
    /// line `y_offset` below its center, and clef/TS flags.
    fn measure_opts(
        &self,
        rect: Rect,
        y_offset: f32,
        show_clef: bool,
        show_ts: bool,
    ) -> LayoutOpts {
        LayoutOpts {
            rect,
            font_id: self.font_id.clone(),
            pixels_per_point: self.pixels_per_point,
            em: self.em,
            layout_clef: show_clef,
            layout_time_signature: show_ts,
            y_offset,
            stem_length_factor: self.stem_length_factor,
            stem_thickness_factor: self.stem_thickness_factor,
            accent_displacement: self.accent_displacement,
            accent_below: self.accent_below,
            stem_down: false,
            shared_line: false,
            proportional_spacing: self.proportional_spacing,
        }
    }

    /// Distance between two lines of a five-line staff: a quarter of `em`.
    pub fn staff_space(&self) -> f32 {
        self.em * 0.25
    }

    /// Return a copy of these options scaled by `factor`: `em` and the
    /// `font_id` size move together.
    pub fn rescaled(&self, factor: f32) -> StaffOpts {
        let mut font_id = self.font_id.clone();
        font_id.size *= factor;
        StaffOpts { em: self.em * factor, font_id, ..self.clone() }
    }
}

/// One measure placed in a system.
#[derive(Debug, Clone)]
pub struct PlacedMeasure {
    pub measure_idx: MeasureIdx,
    pub rect: Rect,
    /// How far the staff line sits below the center of `rect`.
    pub y_offset: f32,
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
    /// How far every row reaches above and below its staff line.
    pub room: RowRoom,
}

impl StaffLayout {
    /// Find the placed measure for a given measure index across all systems.
    pub fn placed(&self, measure_idx: MeasureIdx) -> Option<&PlacedMeasure> {
        self.systems.iter().flat_map(|s| s.measures.iter()).find(|m| m.measure_idx == measure_idx)
    }
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
    let body =
        body.max(min_note_area_width(m, plan, &opts.measure_opts(opts.rect, 0.0, false, false)));
    let clef = if show_clef { clef_width(opts.em) } else { 0.0 };
    let ts = if show_ts { time_sig_width(&m.time_signature(), opts.em) } else { 0.0 };
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

/// Room past the tips of full-length stems, in em: enough for the beams and
/// flags at the tips.
const STEM_MARGIN_EM: f32 = 0.15;
/// Space kept between the notes' ink and the edge of a row, in em.
const INK_MARGIN_EM: f32 = 0.1;

/// Distance from the staff line down to the lower voice's line, in em: three
/// staff spaces, about two lines of a five-line staff apart. The lower
/// voice's rests hang below its line to clear the upper voice's.
pub const LOWER_VOICE_GAP_EM: f32 = 0.75;
/// Room below a lower voice for count labels, in em.
pub const LOWER_VOICE_LABEL_ROOM_EM: f32 = 0.5;

/// How far the rows of a staff reach above and below the staff line, in em.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowRoom {
    pub above: f32,
    pub below: f32,
}

impl RowRoom {
    /// The height of a row, in em.
    pub fn height(&self) -> f32 {
        self.above + self.below
    }
}

impl RowRoom {
    const NONE: RowRoom = RowRoom { above: 0.0, below: 0.0 };

    fn max(self, other: RowRoom) -> RowRoom {
        RowRoom { above: self.above.max(other.above), below: self.below.max(other.below) }
    }
}

/// How far the notes of `layout` reach above and below the line of its
/// voice, in em.
fn ink_reach_em(layout: &MeasureLayout, opts: &LayoutOpts) -> RowRoom {
    layout.notes_ink().map_or(RowRoom::NONE, |ink| {
        let y = opts.y_center();
        RowRoom { above: (y - ink.top()) / opts.em, below: (ink.bottom() - y) / opts.em }
    })
}

/// The room the rows of a staff need, from how far each measure's notes
/// reach above and below their line ([`ink_reach_em`]), with the lower
/// voice's if the staff has one. Every row gets the same room, so rows don't
/// shift from one to the next. It always has room for full-length stems
/// (`stem_length_factor` is user configurable) and `opts.room_below_em`, and
/// grows for whatever reaches further: accents, tuplet brackets and numbers.
fn row_room(reaches: &[(RowRoom, Option<RowRoom>)], opts: &StaffOpts) -> RowRoom {
    let past_stems =
        |reach: f32| (opts.stem_length_factor + STEM_MARGIN_EM).max(reach + INK_MARGIN_EM);
    let upper = reaches.iter().map(|r| r.0).fold(RowRoom::NONE, RowRoom::max);
    let mut room_below = opts.room_below_em.max(upper.below + INK_MARGIN_EM);
    if let Some(lower) = reaches.iter().filter_map(|r| r.1).reduce(RowRoom::max) {
        room_below = room_below
            .max(opts.lower_voice_gap_em() + past_stems(lower.below) + opts.lower_label_room_em);
    }
    RowRoom { above: past_stems(upper.above), below: room_below }
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
/// score's, one per row, each as wide as the wider of the two. The rows grow
/// below the staff line to hold it.
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
    // On a shared line the upper voice's beamed rests are stemlets too, clear
    // of the line.
    let lower_measures = |i: usize| lower.and_then(|l| l.measures.get(i));
    let shares_line = |i: usize| opts.one_line && lower_measures(i).is_some();
    let plans: Vec<RenderPlan> = score
        .measures
        .iter()
        .enumerate()
        .map(|(i, m)| plan_measure_with(m, shares_line(i)))
        .collect();
    let lower_plans: Vec<Option<RenderPlan>> =
        (0..score.len()).map(|i| lower_measures(i).map(|m| plan_measure_with(m, true))).collect();
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

    // 4. Lay out every measure once to measure the room the rows need, which
    // places the staff line in its row, then again in its row. Nothing is
    // snapped vertically, so the second layout only moves the first.
    let left = opts.rect.left();
    let widths: Vec<f32> = widths_min.iter().map(|w| (w * em_scale).max(available)).collect();
    let lay_out = |i: usize, rect: Rect, y_offset: f32| {
        let per_opts = effective_opts.measure_opts(rect, y_offset, show_clef[i], show_ts[i]);
        let per_opts = effective_opts.upper_of(per_opts, lower_measures(i).is_some());
        let layout = build_measure_layout_with_plan(&score.measures[i], &plans[i], &per_opts);
        let lower_opts = effective_opts.lower_of(per_opts.clone());
        let lower = lower_measures(i)
            .zip(lower_plans[i].as_ref())
            .map(|(m, plan)| build_measure_layout_with_plan(m, plan, &lower_opts));
        (layout, per_opts, lower, lower_opts)
    };
    let reaches: Vec<(RowRoom, Option<RowRoom>)> = (0..score.len())
        .map(|i| {
            let rect = Rect::from_min_size(pos2(left, 0.0), vec2(widths[i], 0.0));
            let (layout, per_opts, lower, lower_opts) = lay_out(i, rect, 0.0);
            (ink_reach_em(&layout, &per_opts), lower.map(|l| ink_reach_em(&l, &lower_opts)))
        })
        .collect();
    let room = row_room(&reaches, opts);

    // 5. Stack one measure per row, each stretched to fill `available`.
    let row_height = room.height() * effective_opts.em;
    // Rows are taller above the line (stems, beams, tuplets) than below it.
    let y_offset = effective_opts.y_offset + (room.above - room.below) * 0.5 * effective_opts.em;
    let spacing = opts.system_spacing_em * effective_opts.em;
    let mut y_acc = opts.rect.top();
    let mut systems: Vec<SystemLayout> = Vec::with_capacity(score.len());
    let mut max_row_width = 0.0f32;

    for (i, &width) in widths.iter().enumerate() {
        let rect = Rect::from_min_size(pos2(left, y_acc), vec2(width, row_height));
        let (layout, _, lower_layout, lower_opts) = lay_out(i, rect, y_offset);
        max_row_width = max_row_width.max(width);
        systems.push(SystemLayout {
            y_baseline: rect.center().y + y_offset,
            lower_baseline: lower_layout.as_ref().map(|_| lower_opts.y_center()),
            rect,
            measures: vec![PlacedMeasure {
                measure_idx: i,
                rect,
                y_offset,
                layout,
                lower: lower_layout,
                show_clef: show_clef[i],
                show_time_signature: show_ts[i],
            }],
        });
        y_acc += row_height + spacing;
    }

    let total_height = (y_acc - spacing - opts.rect.top()).max(row_height);
    StaffLayout { total_size: vec2(max_row_width, total_height), systems, scale: em_scale, room }
}

/// One of the staff's two voices: the upper one on the staff line, or the
/// lower one on its own line below (or below the shared one).
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
/// staff with a lower voice, below halfway between the two lines (below the
/// line both share) picks the lower one and the beat among its notes.
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
            min_measure_width_em: 6.0,
            note_width_em: 0.6,
            system_spacing_em: 0.5,
            layout_clef_first: true,
            room_below_em: ROOM_BELOW_EM,
            lower_label_room_em: 0.0,
            one_line: false,
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
            &staff_opts.measure_opts(rect, 0.0, true, false),
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
        let staff_opts = opts(em, rect);
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
        let staff_opts = opts(em, rect);
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
    fn one_line_puts_the_voices_on_either_side_of_it() {
        use grooph_measure::notation::parse_score;
        let em = 20.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 100.0));
        let upper = parse_score("4/4 >q e r:e q q").unwrap();
        let lower = parse_score("4/4 t4 r:t4 (t4) r:t4 (t4) r:t4").unwrap();
        let two_lines = opts(em, rect);
        let staff_opts = StaffOpts { one_line: true, ..two_lines.clone() };
        let staff = build_staff_layout_with_lower(&upper, Some(&lower), &staff_opts);
        let system = &staff.systems[0];
        let y = system.y_baseline;
        assert_eq!(system.lower_baseline, Some(y));
        let placed = &system.measures[0];
        let below = placed.lower.as_ref().unwrap();
        // Every head and rest keeps to its voice's side of the line.
        for n in &placed.layout.notes {
            let ink = n.stem.map_or(n.head.ink(), |s| Rect::from_two_pos(s.p1, s.p2));
            assert!(ink.bottom() <= y + 0.01 && n.staff_pos > 0, "{n:?}");
        }
        for n in &below.notes {
            assert!(n.head.ink().top() >= y - 0.01 && n.staff_pos < 0, "{n:?}");
        }
        // The upper voice's beamed rest is a stemlet, like the lower one's.
        assert!(placed.layout.notes[2].stemlet);
        // Beats at the same time still line up.
        assert!((placed.layout.notes[0].center.x - below.notes[0].center.x).abs() < 0.5);
        // The row is shorter by the second line's gap, at most.
        let room = build_staff_layout_with_lower(&upper, Some(&lower), &two_lines).room;
        assert!(staff.room.below < room.below);
        assert!(staff.room.below >= room.below - LOWER_VOICE_GAP_EM - 0.01);
        // Above the line hits the upper voice, below it the lower one.
        let x = below.notes[3].center.x;
        assert_eq!(hit_test_voice(&staff, Pos2::new(x, y + 2.0)), Some((Voice::Lower, 0, 3)));
        assert_eq!(hit_test_voice(&staff, Pos2::new(x, y - 2.0)), Some((Voice::Upper, 0, 3)));
        // Without a lower voice the line is the upper voice's alone.
        let single = build_staff_layout(&upper, &staff_opts);
        assert!(single.systems[0].measures[0].layout.notes.iter().all(|n| n.staff_pos == 0));
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
    fn rows_make_room_for_what_reaches_past_the_stems() {
        use grooph_measure::notation::parse_score;
        let em = 20.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 100.0));
        let room = |text: &str, accent_below: bool| {
            let score = parse_score(text).unwrap();
            build_staff_layout(&score, &StaffOpts { accent_below, ..opts(em, rect) }).room
        };
        // Full-length stems and the room below, whatever the notes.
        let plain = room("4/4 q q q q", false);
        assert!((plain.above - (3.5 + STEM_MARGIN_EM)).abs() < 1e-4);
        assert_eq!(room("4/4 s s s s s s s s s s s s s s s s", false), plain);
        assert!((plain.below - ROOM_BELOW_EM).abs() < 1e-4);
        // Accents above, and more for a tuplet over them.
        let accented = room("4/4 >e e e e e e e e", false);
        assert!(accented.above > plain.above);
        assert_eq!(room("4/4 >e e e e e e e e", true).above, plain.above);
        let tuplet = room("4/4 >t8 r:t8 t8 q q q", false);
        assert!(tuplet.above > accented.above);
        // Each measured to its ink.
        let staff =
            build_staff_layout(&parse_score("4/4 >t8 r:t8 t8 q q q").unwrap(), &opts(em, rect));
        let system = &staff.systems[0];
        let ink = system.measures[0].layout.notes_ink().unwrap();
        assert!((system.y_baseline - ink.top() - (tuplet.above - INK_MARGIN_EM) * em).abs() < 0.01);
        assert!((ink.top() - system.rect.top() - INK_MARGIN_EM * em).abs() < 0.01);
    }

    #[test]
    fn rows_with_a_lower_voice_hold_its_stems_and_labels() {
        use grooph_measure::notation::parse_score;
        let em = 20.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 100.0));
        let upper = parse_score("4/4 q q q q").unwrap();
        let lower = parse_score("4/4 e e e e e e e e").unwrap();
        let without = build_staff_layout_with_lower(&upper, Some(&lower), &opts(em, rect)).room;
        assert!((without.below - (LOWER_VOICE_GAP_EM + 3.5 + STEM_MARGIN_EM)).abs() < 1e-4);
        let labels = StaffOpts { lower_label_room_em: LOWER_VOICE_LABEL_ROOM_EM, ..opts(em, rect) };
        let with = build_staff_layout_with_lower(&upper, Some(&lower), &labels).room;
        assert!((with.below - without.below - LOWER_VOICE_LABEL_ROOM_EM).abs() < 1e-4);
    }
}
