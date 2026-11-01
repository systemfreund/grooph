//! Multi-measure layout layer.
//!
//! Sits above [`crate::pixel_layout::build_measure_layout`] and arranges the
//! [`Measure`]s of a score top-to-bottom, one measure per row ("system") —
//! the app targets phones, where a predictable one-bar-per-line layout reads
//! best. Cross-measure concerns (per-measure width allocation, clef/time-
//! signature repeat rules, grow/shrink policy, total size for the host's
//! scroll area) live here; per-measure pixel geometry stays in `MeasureLayout`.
//!
//! A measure can have a second voice (see [`grooph_measure::Measure::voices`]):
//! on the same line, below it with stems down, while the first voice keeps
//! above it; on the one line beamed rests of both are stemlets (the kit's
//! stay rests), and both are spaced together so notes at the same time line
//! up — like the feet under the hands of a drum part.
//!
//! Each measure grows to fill the row. If a measure doesn't fit, glyphs shrink
//! toward [`LEGIBILITY_FLOOR_EM`]; a measure that doesn't fit even at the floor
//! overflows its row — the caller is expected to host the result in a
//! horizontally scrollable area for that case.

use crate::PlacedGlyph;
use crate::pixel_layout::{
    LayoutOpts, MeasureLayout, build_measure_layout_at, build_measure_layout_with_plan, clef_width,
    min_note_area_width, note_advances, note_area_offset, note_lead, time_sig_width,
};
use crate::render_plan::{RenderPlan, plan_measure_with, rests_only};
use crate::staff::Staff;
use crate::{calculate_shared_x_centers, shared_min_width};
use egui::{FontId, Pos2, Rect, Vec2, pos2, vec2};
use grooph_measure::duration::s;
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::{
    Beat, BeatIdx, BeatKind, Instrument, MeasureIdx, Score, StaffKind, TimeSignature, VoiceBar,
    VoiceIdx,
};

/// Configuration for laying out an entire [`Score`].
///
/// Mirrors the per-measure fields of [`LayoutOpts`], plus the cross-measure
/// knobs (`min_measure_width_em`, `note_width_em`, `system_spacing_em`,
/// `layout_clef_first`, `time_signature_first`). The per-measure `rect`, `layout_clef` and
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

    /// Minimum width of a measure in em, before per-beat additions.
    pub min_measure_width_em: f32,
    /// Width added per beat in em, to grow dense measures.
    pub note_width_em: f32,
    /// Vertical spacing between systems (rows) in em.
    pub system_spacing_em: f32,
    /// Whether to show the clef on the first measure of each system.
    pub layout_clef_first: bool,
    /// Whether the first measure shows its time signature even when the
    /// score has only one meter (a preview standing on its own). Mixed
    /// meters show it on the first measure and every change regardless.
    pub time_signature_first: bool,
    /// Room below the staff line in em, at least: rests, accents and timing
    /// marks need [`ROOM_BELOW_EM`], count labels
    /// [`ROOM_BELOW_WITH_LABELS_EM`].
    pub room_below_em: f32,
    /// Room under a lower voice's notes in em, for count labels: 0 or
    /// [`LOWER_VOICE_LABEL_ROOM_EM`].
    pub lower_label_room_em: f32,
    /// Draw a measure of rests only as one whole rest (see
    /// [`RenderPlan::bar_rest`]), unless a lower voice shares its line. Off
    /// while editing, so each rest shows and can be picked.
    pub bar_rests: bool,
}

impl StaffOpts {
    /// Per-measure [`LayoutOpts`] of `voice` of a placed measure (renderer
    /// entry).
    pub fn voice_opts_for(&self, placed: &PlacedMeasure, voice: VoiceIdx) -> LayoutOpts {
        let opts = self.measure_opts(
            placed.rect,
            placed.y_offset,
            placed.show_clef,
            placed.show_time_signature,
            placed.staff,
        );
        Self::voice_of(opts, voice, placed.voices.len())
    }

    /// `opts` for `voice` of a measure with `voices`: with more than one the
    /// line is shared, the first voice above it, the others below it with
    /// the stems down.
    fn voice_of(opts: LayoutOpts, voice: VoiceIdx, voices: usize) -> LayoutOpts {
        LayoutOpts { stem_down: voice > 0, shared_line: voices > 1, ..opts }
    }

    /// Build per-measure [`LayoutOpts`] with a measure-local rect, the staff
    /// line `y_offset` below its center, and clef/TS flags.
    fn measure_opts(
        &self,
        rect: Rect,
        y_offset: f32,
        show_clef: bool,
        show_ts: bool,
        staff: StaffKind,
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
            stem_down: false,
            shared_line: false,
            staff,
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
    /// How far the staff line (the kit's middle line) sits below the center
    /// of `rect`.
    pub y_offset: f32,
    /// The staff of its score.
    pub staff: StaffKind,
    /// The layout of each voice of the measure, the first one upper.
    pub voices: Vec<MeasureLayout>,
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
    /// How far every row reaches above and below its staff line.
    pub room: RowRoom,
}

impl PlacedMeasure {
    /// The layout of the first voice.
    pub fn layout(&self) -> &MeasureLayout {
        &self.voices[0]
    }
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

/// Minimum natural width (px) for a measure, `voices` of it played
/// together, given which header glyphs (clef / time signature) it will
/// draw. `notes_min` is the narrowest note area in which no glyph overlaps
/// the next one in its voice.
///
/// The body is sized for at least a measure full of sixteenths, so up to
/// that density the width usually depends only on the time signature:
/// editing notes or swapping measures (endless mode) rarely reflows the
/// staff. Denser measures (32nds, large tuplets) grow with their beat count,
/// and measures mixing very short and long notes grow until the short ones,
/// spaced by duration, no longer collide.
fn min_measure_width(
    voices: &[&VoiceBar],
    notes_min: f32,
    show_clef: bool,
    show_ts: bool,
    opts: &StaffOpts,
) -> f32 {
    let m = voices[0];
    let beats = voices
        .iter()
        .map(|v| v.beats().len() as f32)
        .fold(sixteenth_slots(&m.time_signature()), f32::max);
    let body = (opts.min_measure_width_em + beats * opts.note_width_em) * opts.em;
    // Short notes next to long ones (e.g. 32nds among eighths) get little
    // space when spaced by duration: widen until no glyph overlaps the next.
    let body = body.max(notes_min);
    let clef = if show_clef { clef_width(opts.em) } else { 0.0 };
    let ts = if show_ts { time_sig_width(&m.time_signature(), opts.em) } else { 0.0 };
    body + clef + ts
}

/// The beats of `voices` with the least advance of each, to be spaced
/// together.
fn together<'a>(voices: &'a [VoiceBar], advances: &'a [Vec<f32>]) -> Vec<(&'a [Beat], &'a [f32])> {
    voices.iter().zip(advances).map(|(v, a)| (&v.beats()[..], &a[..])).collect()
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
fn row_room(reaches: &[Vec<RowRoom>], opts: &StaffOpts) -> RowRoom {
    let past_stems =
        |reach: f32| (opts.stem_length_factor + STEM_MARGIN_EM).max(reach + INK_MARGIN_EM);
    let upper = reaches.iter().map(|r| r[0]).fold(RowRoom::NONE, RowRoom::max);
    let mut room_below = opts.room_below_em.max(upper.below + INK_MARGIN_EM);
    if let Some(lower) = reaches.iter().flat_map(|r| r.iter().skip(1).copied()).reduce(RowRoom::max)
    {
        room_below = room_below.max(past_stems(lower.below) + opts.lower_label_room_em);
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
    assert!(!score.is_empty(), "Score must have at least one measure");

    // 1. show-flags: clef only on first. The time signature only shows when
    // the score mixes meters, then on the first measure + every change, or
    // on the first when asked for.
    // A staff may start every row with its clef: the kit's tells its five
    // lines from a pitched staff's.
    let staff = Staff::of(score.staff);
    let show_clef: Vec<bool> = (0..score.len())
        .map(|i| staff.clef_every_row || (i == 0 && opts.layout_clef_first))
        .collect();
    let meter_changes_at =
        |i: usize| score.measures()[i].time_signature() != score.measures()[i - 1].time_signature();
    let mixed_meters = (1..score.len()).any(meter_changes_at);
    let show_ts: Vec<bool> = (0..score.len())
        .map(|i| {
            (i == 0 && opts.time_signature_first)
                || (mixed_meters && (i == 0 || meter_changes_at(i)))
        })
        .collect();

    // 2. natural widths at the baseline em. Each voice is planned (beams,
    // tuplets) once, for its width and its layout. With several voices they
    // share the staff and no bar shows as one whole rest. Sharing one line,
    // every beamed rest is then a stemlet, clear of the line; on the kit's
    // five lines rests have their own places, so they stay rests, as in
    // drum notation.
    let voices = |i: usize| score.measures()[i].voices();
    let plans: Vec<Vec<RenderPlan>> = (0..score.len())
        .map(|i| {
            let shared = voices(i).len() > 1;
            voices(i)
                .iter()
                .map(|v| RenderPlan {
                    bar_rest: opts.bar_rests && !shared && rests_only(v),
                    ..plan_measure_with(v, shared && staff.one_line())
                })
                .collect()
        })
        .collect();
    let voice_opts = |i: usize, opts: LayoutOpts| -> Vec<LayoutOpts> {
        let n = voices(i).len();
        (0..n).map(|v| StaffOpts::voice_of(opts.clone(), v, n)).collect()
    };
    // Several voices are spaced together: beats at the same time line up,
    // and what one voice needs at a time the others get too.
    let advances = |i: usize, opts: &[LayoutOpts]| -> Vec<Vec<f32>> {
        voices(i)
            .iter()
            .zip(&plans[i])
            .zip(opts)
            .map(|((v, p), o)| note_advances(v, p, o))
            .collect()
    };
    let widths_min: Vec<f32> = (0..score.len())
        .map(|i| {
            let per_voice =
                voice_opts(i, opts.measure_opts(opts.rect, 0.0, false, false, score.staff));
            let notes_min = if voices(i).len() > 1 {
                note_lead(opts.em)
                    + shared_min_width(&together(voices(i), &advances(i, &per_voice)))
            } else {
                min_note_area_width(&voices(i)[0], &plans[i][0], &per_voice[0])
            };
            let measures: Vec<&VoiceBar> = voices(i).iter().collect();
            min_measure_width(&measures, notes_min, show_clef[i], show_ts[i], opts)
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
    let lay_out = |i: usize, rect: Rect, y_offset: f32| -> Vec<(MeasureLayout, LayoutOpts)> {
        let per_opts =
            effective_opts.measure_opts(rect, y_offset, show_clef[i], show_ts[i], score.staff);
        let per_voice = voice_opts(i, per_opts);
        if voices(i).len() == 1 {
            let layout = build_measure_layout_with_plan(&voices(i)[0], &plans[i][0], &per_voice[0]);
            return vec![(layout, per_voice[0].clone())];
        }
        let advances = advances(i, &per_voice);
        let area =
            rect.width() - note_area_offset(&score.measures()[i].time_signature(), &per_voice[0]);
        let xs = calculate_shared_x_centers(
            &together(voices(i), &advances),
            area,
            note_lead(effective_opts.em),
        );
        voices(i)
            .iter()
            .zip(&plans[i])
            .zip(per_voice)
            .zip(&xs)
            .map(|(((v, plan), o), xs)| (build_measure_layout_at(v, plan, xs, &o), o))
            .collect()
    };
    let reaches: Vec<Vec<RowRoom>> = (0..score.len())
        .map(|i| {
            let rect = Rect::from_min_size(pos2(left, 0.0), vec2(widths[i], 0.0));
            lay_out(i, rect, 0.0).iter().map(|(layout, o)| ink_reach_em(layout, o)).collect()
        })
        .collect();
    let mut room = row_room(&reaches, opts);
    // Lines past the middle one need their room: the kit's reach two staff
    // spaces either way. A staff position is half a staff space, an eighth
    // of an em.
    let line_room = |pos: i8| match pos {
        0 => 0.0,
        p => f32::from(p.unsigned_abs()) * 0.125 + INK_MARGIN_EM,
    };
    room = RowRoom {
        above: room.above.max(line_room(staff.top())),
        below: room.below.max(line_room(staff.bottom())),
    };
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
        let voices = lay_out(i, rect, y_offset).into_iter().map(|(layout, _)| layout).collect();
        max_row_width = max_row_width.max(width);
        systems.push(SystemLayout {
            y_baseline: rect.center().y + y_offset,
            rect,
            measures: vec![PlacedMeasure {
                measure_idx: i,
                rect,
                y_offset,
                voices,
                staff: score.staff,
                show_clef: show_clef[i],
                show_time_signature: show_ts[i],
            }],
        });
        y_acc += row_height + spacing;
    }

    let total_height = (y_acc - spacing - opts.rect.top()).max(row_height);
    StaffLayout { total_size: vec2(max_row_width, total_height), systems, scale: em_scale, room }
}

/// Find `(measure_idx, beat_idx)` of the beat closest to `pos`. `pos` is in
/// the same coordinate space as `staff` (i.e. the inner space of the
/// ScrollArea content). The row — and with it the measure, one per row — is
/// picked by `pos.y` (clamped to the first/last row), then the beat by
/// `pos.x`. Returns `None` if the score has no notes anywhere.
pub fn hit_test_staff(staff: &StaffLayout, pos: Pos2) -> Option<(MeasureIdx, BeatIdx)> {
    hit_test_voice(staff, pos).map(|(_, m, b)| (m, b))
}

/// Like [`hit_test_staff`], with the voice on `pos`'s side of the line: on a
/// staff with lower voices, below the line picks the beat nearest `pos.x` in
/// each and, of those, the voice whose note is nearest `pos`.
pub fn hit_test_voice(staff: &StaffLayout, pos: Pos2) -> Option<(VoiceIdx, MeasureIdx, BeatIdx)> {
    let system =
        staff.systems.iter().find(|s| pos.y < s.rect.bottom()).or_else(|| staff.systems.last())?;
    let target = system.measures.first()?;
    if pos.y > system.y_baseline
        && let Some((voice, beat)) = target
            .voices
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(_, layout)| !layout.notes.is_empty())
            .map(|(v, layout)| {
                let beat = nearest_note(layout, pos.x);
                (v, beat, layout.notes[beat].center.distance_sq(pos))
            })
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .map(|(v, beat, _)| (v, beat))
    {
        return Some((voice, target.measure_idx, beat));
    }

    if target.layout().notes.is_empty() {
        // Try any measure, in any row, with notes.
        let with_notes = staff
            .systems
            .iter()
            .flat_map(|s| s.measures.iter())
            .find(|p| !p.layout().notes.is_empty())?;
        return Some((0, with_notes.measure_idx, 0));
    }
    Some((0, target.measure_idx, nearest_note(target.layout(), pos.x)))
}

/// What a tap points at: a beat of a voice, and the instrument whose head
/// there is meant (see [`hit_test_head`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeadHit {
    pub measure_idx: MeasureIdx,
    pub voice: VoiceIdx,
    pub beat_idx: BeatIdx,
    pub instrument: Instrument,
}

/// Like [`hit_test_voice`], with the instrument whose place on the staff is
/// nearest `pos` ([`Staff::instrument_near`]). On a staff of one line that
/// is its rhythm, and the voice is the one on `pos`'s side of the line. On
/// the kit's staff the voice is the one whose note nearest `pos.x` has a
/// head of that instrument, else the instrument's default voice.
pub fn hit_test_head(staff: &StaffLayout, pos: Pos2) -> Option<HeadHit> {
    let (voice, measure_idx, beat_idx) = hit_test_voice(staff, pos)?;
    let system =
        staff.systems.iter().find(|s| pos.y < s.rect.bottom()).or_else(|| staff.systems.last())?;
    let placed = system.measures.iter().find(|p| p.measure_idx == measure_idx)?;
    let model = Staff::of(placed.staff);
    let hit = HeadHit { measure_idx, voice, beat_idx, instrument: model.instruments[0] };
    let Some(em) = placed.voices.iter().flat_map(|l| l.notes.first()).map(|n| n.head.size).next()
    else {
        return Some(hit);
    };
    if model.one_line() {
        return Some(hit);
    }
    // Staff positions are half staff spaces, an eighth of an em.
    let half_ss = em * 0.125;
    let instrument = model.instrument_near((system.y_baseline - pos.y) / half_ss);
    let place = model.place_of(instrument);
    let at_place =
        |g: &PlacedGlyph| ((system.y_baseline - g.origin.y) / half_ss).round() == f32::from(place);
    let holding = placed.voices.iter().enumerate().find_map(|(v, layout)| {
        let beat = nearest_note(layout, pos.x);
        let note = layout.notes.get(beat)?;
        let heads = std::iter::once(&note.head).chain(&note.chord);
        (note.kind == BeatKind::Note && heads.into_iter().any(at_place)).then_some((v, beat))
    });
    let (voice, beat_idx) = holding.unwrap_or_else(|| {
        let v = instrument.default_voice().min(placed.voices.len() - 1);
        (v, nearest_note(&placed.voices[v], pos.x))
    });
    Some(HeadHit { measure_idx, voice, beat_idx, instrument })
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
            min_measure_width_em: 6.0,
            note_width_em: 0.6,
            system_spacing_em: 0.5,
            layout_clef_first: true,
            time_signature_first: false,
            room_below_em: ROOM_BELOW_EM,
            lower_label_room_em: 0.0,
            bar_rests: false,
        }
    }

    /// `upper` with the measures of `lower` as its second voice.
    fn with_lower(upper: &Score, lower: &Score) -> Score {
        let mut score = upper.clone();
        for (m, l) in score.measures_mut().iter_mut().zip(lower.measures()) {
            m.push_voices(l).unwrap();
        }
        score
    }

    fn measure_with_quarters(ts: TimeSignature) -> Measure {
        let mut m = Measure::new(ts);
        for i in 0..(ts.beats as usize) {
            m.first_voice_mut().set_beat(i, Beat::note(q())).unwrap();
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
            m.first_voice(),
            &staff_opts.measure_opts(rect, 0.0, true, false, StaffKind::Line),
        );

        assert_eq!(placed.layout().notes.len(), direct.notes.len());
        for (a, b) in placed.layout().notes.iter().zip(direct.notes.iter()) {
            assert!(
                (a.center.x - b.center.x).abs() < 0.01,
                "x mismatch: {} vs {}",
                a.center.x,
                b.center.x
            );
        }
    }

    #[test]
    fn lower_voice_hangs_below_the_line() {
        use grooph_measure::notation::parse_score;
        let em = 20.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 100.0));
        let upper = parse_score("4/4 >q q q q").unwrap();
        let lower = parse_score("4/4 t4 r:t4 (t4) r:t4 (t4) r:t4").unwrap();
        let staff_opts = opts(em, rect);
        let staff = build_staff_layout(&with_lower(&upper, &lower), &staff_opts);
        let system = &staff.systems[0];
        let placed = &system.measures[0];
        let below = &placed.voices[1];
        // Both voices start their beats at the same x: beat 1 and beat 3.
        assert!((placed.layout().notes[0].center.x - below.notes[0].center.x).abs() < 0.5);
        assert!((placed.layout().notes[2].center.x - below.notes[3].center.x).abs() < 0.5);
        // The lower voice's stems end inside the row.
        let stem_end = below.notes[0].stem.unwrap().p2.y;
        assert!(stem_end > system.y_baseline && stem_end < placed.rect.bottom());
        assert!(
            build_staff_layout(&upper, &opts(em, rect)).systems[0].measures[0].voices.len() == 1
        );
    }

    #[test]
    fn hit_test_picks_the_nearer_voice() {
        use grooph_measure::notation::parse_score;
        let em = 20.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 100.0));
        let upper = parse_score("4/4 >q q q q").unwrap();
        let lower = parse_score("4/4 t4 r:t4 (t4) r:t4 (t4) r:t4").unwrap();
        let staff_opts = opts(em, rect);
        let staff = build_staff_layout(&with_lower(&upper, &lower), &staff_opts);
        let system = &staff.systems[0];
        let below = &system.measures[0].voices[1];
        let x = below.notes[3].center.x;
        let lower_y = system.y_baseline + 5.0;
        assert_eq!(hit_test_voice(&staff, Pos2::new(x, lower_y)), Some((1, 0, 3)));
        assert_eq!(hit_test_voice(&staff, Pos2::new(x, system.y_baseline)), Some((0, 0, 2)));
        // Without a lower voice everything hits the upper one.
        let single = build_staff_layout(&upper, &opts(em, rect));
        let y = single.systems[0].rect.bottom() - 1.0;
        assert_eq!(hit_test_voice(&single, Pos2::new(x, y)).map(|h| h.0), Some(0));
    }

    /// The kit staff's layout of `text` (one measure), its middle line's y
    /// and staff space.
    fn kit(text: &str) -> (PlacedMeasure, f32, f32) {
        let score = grooph_measure::notation::parse_score(text).unwrap();
        assert_eq!(score.staff, StaffKind::Kit);
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(500.0, 200.0));
        let o = opts(20.0, rect);
        let staff = build_staff_layout(&score, &o);
        let placed = staff.systems[0].measures[0].clone();
        let line = staff.systems[0].y_baseline;
        (placed, line, o.rescaled(staff.scale).staff_space())
    }

    #[test]
    fn a_tap_on_the_kit_finds_the_head_by_its_place() {
        use grooph_measure::Instrument::{Crash, HiHat, Kick, Rhythm, Snare};
        let score =
            grooph_measure::notation::parse_score("4/4 S-H:q H:q H:q H:q ; K:h r:h").unwrap();
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(500.0, 200.0));
        let o = opts(20.0, rect);
        let staff = build_staff_layout(&score, &o);
        let system = &staff.systems[0];
        let half_ss = o.rescaled(staff.scale).staff_space() * 0.5;
        let notes = &system.measures[0].voices[0].notes;
        let at = |beat: usize, pos: f32| {
            let tap = Pos2::new(notes[beat].center.x, system.y_baseline - pos * half_ss);
            let hit = hit_test_head(&staff, tap).unwrap();
            (hit.voice, hit.beat_idx, hit.instrument)
        };
        // The snare under the hi-hat, both in the upper voice.
        assert_eq!(at(0, 1.2), (0, 0, Snare));
        assert_eq!(at(0, 4.6), (0, 0, HiHat));
        // The kick in the lower voice, by its place.
        assert_eq!(at(0, -3.0), (1, 0, Kick));
        // No crash there: its default voice, the upper, at that beat.
        assert_eq!(at(2, 6.4), (0, 2, Crash));
        // No kick on the third beat's lower voice: a rest there, its voice.
        assert_eq!(at(2, -3.0), (1, 1, Kick));

        // On the one line, its rhythm.
        let line = grooph_measure::notation::parse_score("4/4 q q q q").unwrap();
        let staff = build_staff_layout(&line, &o);
        let tap = Pos2::new(staff.systems[0].measures[0].layout().notes[1].center.x, 10.0);
        let hit = hit_test_head(&staff, tap).unwrap();
        assert_eq!((hit.voice, hit.beat_idx, hit.instrument), (0, 1, Rhythm));
    }

    #[test]
    fn kit_heads_sit_at_their_instruments_places_on_one_stem() {
        let (placed, line, ss) = kit("4/4 S-H:q K-H:q T1:q CR:q");
        let notes = &placed.layout().notes;
        // Snare and hi-hat share a stem from the snare to past the hi-hat.
        let chord = &notes[0];
        assert_eq!(chord.staff_pos, 5);
        assert_eq!(chord.chord.len(), 1);
        let stem = chord.stem.unwrap();
        let snare_y = line - 0.5 * ss;
        let hihat_y = line - 2.5 * ss;
        assert!((chord.chord[0].origin.y - snare_y).abs() < 0.01);
        assert!((chord.head.origin.y - hihat_y).abs() < 0.01);
        assert!(stem.p1.y > snare_y - ss && stem.p2.y < hihat_y - 2.0 * ss, "{stem:?}");
        // Both left of the up stem, an x head for the hi-hat.
        for head in std::iter::once(&chord.head).chain(&chord.chord) {
            assert!(head.ink().right() <= stem.p1.x + 0.5 * ss, "{head:?} {stem:?}");
        }
        assert_eq!(chord.head.glyph, crate::glyphs::NOTEHEAD_X_BLACK);
        assert_eq!(notes[1].chord[0].origin.y, line + 1.5 * ss, "kick in the first space");
        assert_eq!(notes[2].staff_pos, 3);
        // The crash, above the staff, on a ledger line.
        let crash = &notes[3];
        assert_eq!(crash.staff_pos, 6);
        assert_eq!(crash.ledgers.len(), 1);
        assert!((crash.ledgers[0].p1.y - (line - 3.0 * ss)).abs() < 0.01);
        assert!(notes[..3].iter().all(|n| n.ledgers.is_empty()));
    }

    #[test]
    fn kit_rests_and_beams_in_compound_and_odd_meters() {
        // 6/8: a rest among the snares leaves the beam where the snares
        // alone put it, and stays a rest, as in drum notation (no stemlet).
        let tips = |text: &str| -> Vec<Option<f32>> {
            let (placed, _, _) = kit(text);
            placed.layout().notes.iter().map(|n| n.stem.map(|s| s.p2.y)).collect()
        };
        let with_rest = tips("6/8 S:e r:e S:e S:e S:e S:e ; K:q. r:q.");
        let without = tips("6/8 S:e S:e S:e S:e S:e S:e ; K:q. r:q.");
        assert_eq!(with_rest[1], None);
        let same = |a: &Option<f32>, b: &Option<f32>| (a.unwrap() - b.unwrap()).abs() < 0.01;
        let notes = [0, 2, 3, 4, 5];
        assert!(notes.iter().all(|&i| same(&with_rest[i], &without[i])), "{with_rest:?}");
        // The rest under the beam stays a rest: in 6/8, 7/8 (2+2+3) and 4/4,
        // in either voice.
        for (text, voice, idx) in [
            ("6/8 S:e r:e S:e S:e S:e S:e ; K:q. r:q.", 0, 1),
            ("7/8 H:e H:e H:e H:e H:e r:e H:e ; K:e r:e r:e r:h", 0, 5),
            ("4/4 H:e H:e H:e H:e H:h ; K:s r:s K:s K:s r:q r:h", 1, 1),
        ] {
            let (placed, _, _) = kit(text);
            let notes = &placed.voices[voice].notes;
            let rest = &notes[idx];
            assert!(rest.kind == grooph_measure::BeatKind::Rest, "{text}");
            assert!(!rest.stemlet && rest.stem.is_none(), "{text}: {rest:?}");
            // Under the beam of the notes around it.
            let tip = |i: usize| notes[i].stem.unwrap().p2.y;
            assert!((tip(idx - 1) - tip(idx + 1)).abs() < 0.01, "{text}");
            assert!(notes[idx - 1].flag.is_none(), "{text}: beamed, not flagged");
        }
        let (placed, _, _) = kit("6/8 S:e r:e S:e S:e S:e S:e ; K:q. r:q.");
        let feet = &placed.voices[1].notes;
        assert_eq!(feet[1].staff_pos, -4, "the feet's rest below the middle line");
        assert!(feet[1].ledgers.is_empty());
        // Half rests of the lower voice sit on the bottom line, with no
        // ledger line through the staff: in 7/8 and in 4/4.
        for text in ["7/8 H:e H:e H:e H:e H:e H:e H:e ; K:e r:e r:e r:h", "4/4 S:h S:h ; r:h r:h"] {
            let (placed, _, _) = kit(text);
            for rest in
                placed.voices[1].notes.iter().filter(|n| n.kind == grooph_measure::BeatKind::Rest)
            {
                assert!(rest.ledgers.is_empty(), "{text}: {rest:?}");
                if rest.duration.base_note() == grooph_measure::duration::NoteValue::Half {
                    assert_eq!(rest.staff_pos, -4, "{text}");
                }
            }
        }
    }

    #[test]
    fn an_empty_kit_bar_hangs_its_whole_rest_from_the_fourth_line() {
        // An empty bar shown as one whole rest hangs from the fourth line,
        // as a whole rest of one voice does.
        let score = grooph_measure::notation::parse_score("4/4 S:q S:q S:q S:q | r:h r:h").unwrap();
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(500.0, 400.0));
        let o = StaffOpts { bar_rests: true, ..opts(20.0, rect) };
        let staff = build_staff_layout(&score, &o);
        let empty = &staff.systems[1];
        let rest = &empty.measures[0].layout().notes[0];
        assert_eq!(rest.head.glyph, crate::glyphs::REST_WHOLE);
        let fourth = empty.y_baseline - o.rescaled(staff.scale).staff_space();
        assert!((rest.head.origin.y - fourth).abs() < 0.01, "{rest:?}");
    }

    #[test]
    fn a_ghost_notes_parentheses_center_on_the_note() {
        // On one line as on the kit's staff, unless a head is flipped.
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(500.0, 200.0));
        let score = grooph_measure::notation::parse_score("4/4 q (q) q q").unwrap();
        let staff = build_staff_layout(&score, &opts(20.0, rect));
        let (line, _, _) = kit("4/4 S:q (S):q S:q S:q");
        for note in [&staff.systems[0].measures[0].layout().notes[1], &line.layout().notes[1]] {
            let [left, right] = [note.parens[0].ink(), note.parens[1].ink()];
            assert!(((left.right() + right.left()) / 2.0 - note.center.x).abs() < 0.01);
        }
    }

    #[test]
    fn kit_heads_a_second_apart_take_both_sides_of_the_stem() {
        // Ride (4) and hi-hat (5), mid (2) and high tom (3): a second each.
        let (placed, _, ss) = kit("2/4 RD-H:q T2-T1:q");
        for note in &placed.layout().notes {
            let stem = note.stem.unwrap();
            let lower = note.chord[0].ink();
            let upper = note.head.ink();
            assert!(lower.right() <= stem.p1.x + 0.5 * ss, "the lower head left of the stem");
            assert!(upper.left() >= stem.p1.x - 0.5 * ss, "the upper one right of it");
            assert!(lower.right() <= upper.left() + 0.5 * ss);
        }
        // With the stem down the other way round: from the top, the head a
        // second below goes left of the stem.
        let (placed, _, ss) = kit("2/4 CR:h ; T2-T1:h");
        let toms = &placed.voices[1].notes[0];
        let stem = toms.stem.unwrap();
        assert!(toms.chord[0].ink().left() >= stem.p1.x - 0.5 * ss, "high tom right of the stem");
        assert!(toms.head.ink().right() <= stem.p1.x + 0.5 * ss, "mid tom left of it");
    }

    #[test]
    fn kit_signs_stack_past_the_accent_and_rests_keep_to_their_voice() {
        let (placed, line, _) = kit("4/4 >Ho:q H:q Hc:q (S)-H:q ; K:q r:q K:q r:q");
        let hands = &placed.layout().notes;
        let accent = hands[0].accent.unwrap().ink();
        let open = hands[0].signs[0];
        assert_eq!(open.glyph, crate::glyphs::SIGN_OPEN);
        assert!(open.ink().bottom() <= accent.top(), "the o above the accent");
        assert!(accent.bottom() <= hands[0].stem.unwrap().p2.y + 0.01, "the accent above the stem");
        assert!(hands[1].signs.is_empty());
        assert_eq!(hands[2].signs[0].glyph, crate::glyphs::SIGN_CLOSED);
        assert_eq!(hands[3].parens.len(), 2, "the ghost snare's parentheses");
        let feet = &placed.voices[1].notes;
        assert!(feet[1].staff_pos < 0 && feet[1].head.ink().top() > line, "kick rests below");
        assert!(feet[0].stem.unwrap().p2.y > line, "stems down");
        // One voice alone keeps its rests in the middle.
        let (placed, _, _) = kit("2/4 S:q r:q");
        assert_eq!(placed.layout().notes[1].staff_pos, 0);
        let (placed, _, _) = kit("2/4 S:q r:q ; K:h");
        assert!(placed.layout().notes[1].staff_pos > 0, "above with a lower voice");
        // Each head with a technique gets its sign, stacked.
        let (placed, _, _) = kit("2/4 Ho-CRc:q H:q");
        let signs = &placed.layout().notes[0].signs;
        let glyphs: Vec<_> = signs.iter().map(|g| g.glyph).collect();
        assert_eq!(glyphs, [crate::glyphs::SIGN_OPEN, crate::glyphs::SIGN_CHOKE]);
        assert!(signs[1].ink().bottom() <= signs[0].ink().top(), "the choke above the o");
    }

    #[test]
    fn kit_beams_reach_every_stem_and_the_row_holds_five_lines() {
        let (placed, line, ss) = kit("2/4 H:e S:e K-H:e S:e");
        let notes = &placed.layout().notes;
        let tips: Vec<f32> = notes.iter().map(|n| n.stem.unwrap().p2.y).collect();
        assert!(tips.windows(2).all(|w| (w[0] - w[1]).abs() < 0.01), "{tips:?}");
        assert!(placed.layout().beams.iter().all(|b| b.rect.bottom() <= tips[0] + 0.5 * ss));
        assert!(placed.rect.top() < line - 2.0 * ss && placed.rect.bottom() > line + 2.0 * ss);
        assert!(placed.show_clef, "every kit row starts with its clef");
    }

    #[test]
    fn hit_test_reaches_every_lower_voice() {
        use grooph_measure::notation::parse_score;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 100.0));
        let upper = parse_score("4/4 q q q q").unwrap();
        let mut score = with_lower(&upper, &parse_score("4/4 q q q q").unwrap());
        let third = parse_score("4/4 r:e e r:e e r:h").unwrap();
        score.measures_mut()[0].push_voices(&third.measures()[0]).unwrap();
        let staff = build_staff_layout(&score, &opts(20.0, rect));
        let system = &staff.systems[0];
        let voices = &system.measures[0].voices;
        // On an offbeat the third voice's note is the nearest below the line;
        // on a beat the second voice's.
        let offbeat = voices[2].notes[3].center;
        assert_eq!(hit_test_voice(&staff, offbeat), Some((2, 0, 3)));
        let beat = voices[1].notes[2].center;
        assert_eq!(hit_test_voice(&staff, beat), Some((1, 0, 2)));
    }

    #[test]
    fn the_voices_keep_to_either_side_of_the_line() {
        use grooph_measure::notation::parse_score;
        let em = 20.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 100.0));
        let upper = parse_score("4/4 >q e r:e q q").unwrap();
        let lower = parse_score("4/4 t4 r:t4 (t4) r:t4 (t4) r:t4").unwrap();
        let staff_opts = opts(em, rect);
        let staff = build_staff_layout(&with_lower(&upper, &lower), &staff_opts);
        let system = &staff.systems[0];
        let y = system.y_baseline;
        let placed = &system.measures[0];
        let below = &placed.voices[1];
        // Every head and rest keeps to its voice's side of the line.
        for n in &placed.layout().notes {
            let ink = n.stem.map_or(n.head.ink(), |s| Rect::from_two_pos(s.p1, s.p2));
            assert!(ink.bottom() <= y + 0.01 && n.staff_pos > 0, "{n:?}");
        }
        for n in &below.notes {
            assert!(n.head.ink().top() >= y - 0.01 && n.staff_pos < 0, "{n:?}");
        }
        // The upper voice's beamed rest is a stemlet, like the lower one's.
        assert!(placed.layout().notes[2].stemlet);
        // Beats at the same time still line up.
        assert!((placed.layout().notes[0].center.x - below.notes[0].center.x).abs() < 0.5);
        // Above the line hits the upper voice, below it the lower one.
        let x = below.notes[3].center.x;
        assert_eq!(hit_test_voice(&staff, Pos2::new(x, y + 2.0)), Some((1, 0, 3)));
        assert_eq!(hit_test_voice(&staff, Pos2::new(x, y - 2.0)), Some((0, 0, 3)));
        // Without a lower voice the line is the upper voice's alone.
        let single = build_staff_layout(&upper, &staff_opts);
        assert!(single.systems[0].measures[0].layout().notes.iter().all(|n| n.staff_pos == 0));
    }

    #[test]
    fn crowded_voices_keep_their_beats_together() {
        use grooph_measure::notation::parse_score;
        let em = 20.0;
        // Narrow, with no room beyond what the notes need: the 32nds are
        // crowded and take more than their share.
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(150.0, 100.0));
        let upper = parse_score("4/4 th th th th th th th th th th th th th th th th h").unwrap();
        let lower = parse_score("4/4 q q h").unwrap();
        let staff_opts =
            StaffOpts { min_measure_width_em: 0.0, note_width_em: 0.0, ..opts(em, rect) };
        let staff = build_staff_layout(&with_lower(&upper, &lower), &staff_opts);
        let placed = &staff.systems[0].measures[0];
        let below = &placed.voices[1];
        let x = |l: &MeasureLayout, i: usize| l.notes[i].center.x;
        // Beats at the same time line up, though only the upper voice
        // is crowded.
        assert!((x(placed.layout(), 0) - x(below, 0)).abs() < 0.01);
        assert!((x(placed.layout(), 8) - x(below, 1)).abs() < 0.01);
        assert!((x(placed.layout(), 16) - x(below, 2)).abs() < 0.01);
        // And the 32nds still get their room: their heads don't overlap,
        // but for the pixel each one's stem snaps by.
        for w in placed.layout().notes[..16].windows(2) {
            assert!(w[0].head.ink().right() <= w[1].head.ink().left() + 1.0);
        }
    }

    #[test]
    fn staff_layout_show_time_signature_only_on_change() {
        let em = 20.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(4000.0, 100.0));
        let staff_opts = opts(em, rect);

        let score = Score::new(vec![
            measure_with_quarters(TimeSignature::FOUR_FOUR),
            measure_with_quarters(TimeSignature::FOUR_FOUR),
            measure_with_quarters(TimeSignature::THREE_FOUR),
            measure_with_quarters(TimeSignature::THREE_FOUR),
        ]);

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

        let score = Score::new(vec![
            measure_with_quarters(TimeSignature::THREE_FOUR),
            measure_with_quarters(TimeSignature::THREE_FOUR),
        ]);

        let staff = build_staff_layout(&score, &staff_opts);
        let placed: Vec<_> = staff.systems.iter().flat_map(|s| &s.measures).collect();
        assert!(
            placed.iter().all(|m| !m.show_time_signature && m.layout().time_signature.is_none())
        );
    }

    #[test]
    fn staff_layout_time_signature_first_for_single_meter() {
        let em = 20.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(4000.0, 100.0));
        let staff_opts = StaffOpts { time_signature_first: true, ..opts(em, rect) };

        let score = Score::new(vec![
            measure_with_quarters(TimeSignature::THREE_FOUR),
            measure_with_quarters(TimeSignature::THREE_FOUR),
        ]);

        let staff = build_staff_layout(&score, &staff_opts);
        let flags: Vec<bool> =
            staff.systems.iter().flat_map(|s| &s.measures).map(|m| m.show_time_signature).collect();
        assert_eq!(flags, vec![true, false]);
        let first = &staff.systems[0].measures[0];
        assert!(first.layout().time_signature.is_some());

        // A compound and an odd meter alone, as in a library preview.
        for ts in [TimeSignature::new(6, 8), TimeSignature::new(7, 8)] {
            let alone = Score::single(Measure::new(ts));
            let staff = build_staff_layout(&alone, &staff_opts);
            assert!(staff.systems[0].measures[0].layout().time_signature.is_some(), "{ts}");
        }
    }

    #[test]
    fn staff_layout_show_clef_only_first() {
        let em = 20.0;
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(4000.0, 100.0));
        let staff_opts = opts(em, rect);

        let score = Score::new(vec![
            measure_with_quarters(TimeSignature::FOUR_FOUR),
            measure_with_quarters(TimeSignature::FOUR_FOUR),
            measure_with_quarters(TimeSignature::FOUR_FOUR),
        ]);

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

        let score = Score::new(vec![
            measure_with_quarters(TimeSignature::FOUR_FOUR),
            measure_with_quarters(TimeSignature::FOUR_FOUR),
            measure_with_quarters(TimeSignature::FOUR_FOUR),
        ]);

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

        let score = Score::new(vec![
            measure_with_quarters(TimeSignature::FOUR_FOUR),
            measure_with_quarters(TimeSignature::FOUR_FOUR),
            measure_with_quarters(TimeSignature::FOUR_FOUR),
        ]);

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

        let score = Score::new(vec![
            measure_with_quarters(TimeSignature::FOUR_FOUR),
            measure_with_quarters(TimeSignature::FOUR_FOUR),
            measure_with_quarters(TimeSignature::FOUR_FOUR),
        ]);

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
        while idx < m.first_voice().beats().len() {
            m.first_voice_mut().set_beat(idx, Beat::note(s())).unwrap();
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
        let sparse = Score::new(vec![
            measure_with_quarters(TimeSignature::FOUR_FOUR),
            measure_with_quarters(TimeSignature::FOUR_FOUR),
        ]);
        let dense = Score::new(vec![
            measure_with_quarters(TimeSignature::FOUR_FOUR),
            measure_with_sixteenths(TimeSignature::FOUR_FOUR),
        ]);
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
        while idx < thirty_seconds.first_voice().beats().len() {
            thirty_seconds.first_voice_mut().set_beat(idx, Beat::note(th())).unwrap();
            idx += 1;
        }
        let score = Score::new(vec![
            measure_with_sixteenths(TimeSignature::FOUR_FOUR),
            measure_with_sixteenths(TimeSignature::FOUR_FOUR),
            thirty_seconds,
        ]);
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

        let score = Score::new(vec![
            measure_with_quarters(TimeSignature::FOUR_FOUR),
            measure_with_quarters(TimeSignature::FOUR_FOUR),
            measure_with_quarters(TimeSignature::FOUR_FOUR),
        ]);
        let staff = build_staff_layout(&score, &staff_opts);
        let y = staff.systems[0].rect.center().y;

        // far left -> first beat of the row's measure
        assert_eq!(hit_test_staff(&staff, Pos2::new(-100.0, y)), Some((0, 0)));
        // far right -> last beat of the row's measure
        let m0 = &staff.systems[0].measures[0];
        assert_eq!(
            hit_test_staff(&staff, Pos2::new(1e6, y)),
            Some((0, m0.layout().notes.len() - 1))
        );
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

        let score = Score::new(vec![
            measure_with_quarters(TimeSignature::FOUR_FOUR),
            measure_with_quarters(TimeSignature::FOUR_FOUR),
            measure_with_quarters(TimeSignature::FOUR_FOUR),
        ]);
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
        let score = Score::new(vec![
            measure_with_quarters(TimeSignature::FOUR_FOUR),
            measure_with_quarters(TimeSignature::FOUR_FOUR),
        ]);
        for below in [ROOM_BELOW_EM, ROOM_BELOW_WITH_LABELS_EM] {
            let staff_opts = StaffOpts { room_below_em: below, ..opts(em, rect) };
            let staff = build_staff_layout(&score, &staff_opts);
            for system in &staff.systems {
                let placed = &system.measures[0];
                let note_y = placed.layout().notes[0].center.y;
                assert!((note_y - system.y_baseline).abs() < 0.01);
                let below_px = system.rect.bottom() - system.y_baseline;
                assert!((below_px - below * em).abs() < 0.01);
                // The renderer rebuilds the opts from the placed rect.
                let render_opts = staff_opts.voice_opts_for(placed, 0);
                assert!((render_opts.y_center() - system.y_baseline).abs() < 0.01);
            }
        }
    }

    #[test]
    fn staff_layout_counts_a_grouping_change_as_a_meter_change() {
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(4000.0, 100.0));
        let grouped = TimeSignature::SEVEN_EIGHT.with_grouping(1);
        let score = Score::new(vec![
            Measure::new(TimeSignature::SEVEN_EIGHT),
            Measure::new(grouped),
            Measure::new(grouped),
        ]);

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
        let room =
            |text: &str| build_staff_layout(&parse_score(text).unwrap(), &opts(em, rect)).room;
        // Full-length stems and the room below, whatever the notes.
        let plain = room("4/4 q q q q");
        assert!((plain.above - (3.5 + STEM_MARGIN_EM)).abs() < 1e-4);
        assert_eq!(room("4/4 s s s s s s s s s s s s s s s s"), plain);
        assert!((plain.below - ROOM_BELOW_EM).abs() < 1e-4);
        // Accents above, and more for a tuplet over them.
        let accented = room("4/4 >e e e e e e e e");
        assert!(accented.above > plain.above);
        let tuplet = room("4/4 >t8 r:t8 t8 q q q");
        assert!(tuplet.above > accented.above);
        // Each measured to its ink.
        let staff =
            build_staff_layout(&parse_score("4/4 >t8 r:t8 t8 q q q").unwrap(), &opts(em, rect));
        let system = &staff.systems[0];
        let ink = system.measures[0].layout().notes_ink().unwrap();
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
        let without = build_staff_layout(&with_lower(&upper, &lower), &opts(em, rect)).room;
        assert!(without.below >= 3.5 + STEM_MARGIN_EM);
        let labels = StaffOpts { lower_label_room_em: LOWER_VOICE_LABEL_ROOM_EM, ..opts(em, rect) };
        let with = build_staff_layout(&with_lower(&upper, &lower), &labels).room;
        assert!((with.below - without.below - LOWER_VOICE_LABEL_ROOM_EM).abs() < 1e-4);
    }

    #[test]
    fn an_empty_bar_shows_one_whole_rest_in_its_middle() {
        use crate::glyphs::REST_WHOLE;
        use grooph_measure::notation::parse_score;
        let em = 20.0;
        let rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1200.0, 400.0));
        let on = StaffOpts { bar_rests: true, ..opts(em, rect) };
        for meter in [
            "4/4 r:h r:h | q r:q h",
            "3/4 r:h r:q | q r:h",
            "6/8 r:q. r:q. | e r:e r:e r:q.",
            "2/4 r:h | q q",
            "7/8 r:q r:q. r:q | e r:e r:q. r:q",
        ] {
            let score = parse_score(meter).unwrap();
            let staff = build_staff_layout(&score, &on);
            let placed = staff.placed(0).unwrap();
            let notes = &placed.layout().notes;
            // Every beat keeps its layout, the first one draws the rest.
            assert_eq!(notes.len(), score.measures()[0].first_voice().beats().len(), "{meter}");
            assert_eq!(notes[0].head.glyph, REST_WHOLE, "{meter}");
            assert!(notes.iter().all(|n| n.dots.is_empty()), "{meter}");
            assert!(notes[1..].iter().all(|n| n.hidden && n.glyphs().next().is_none()), "{meter}");
            let area = (placed.layout().notes_left_edge + placed.rect.right()) / 2.0;
            assert!((notes[0].head.ink().center().x - area).abs() < 1.0, "{meter} centred");
            // It hangs from the line.
            let ink = notes[0].head.ink();
            let line =
                on.measure_opts(placed.rect, placed.y_offset, false, false, placed.staff).y_of(0);
            assert!(ink.top() <= line && ink.bottom() > line, "{meter} hangs from the line");
            // A measure with a note keeps its rests.
            let other = &staff.placed(1).unwrap().layout().notes;
            assert!(other.iter().all(|n| !n.hidden));
        }
        // Off (while editing) and in triplets, the rests stay as written.
        let score = parse_score("4/4 r:h r:h").unwrap();
        let off = build_staff_layout(&score, &opts(em, rect));
        assert!(off.placed(0).unwrap().layout().notes.iter().all(|n| !n.hidden));
        let triplets = parse_score("2/4 r:t4 r:t4 r:t4").unwrap();
        let staff = build_staff_layout(&triplets, &on);
        assert!(staff.placed(0).unwrap().layout().notes.iter().all(|n| !n.hidden));
    }
}
