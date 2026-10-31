use crate::glyphs::{self, Glyph};
use crate::render_plan::{RenderPlan, plan_measure};
use crate::staff::{Staff, Voicing};
use egui::{FontId, Pos2, Rect};
use grooph_measure::duration::{Duration, NoteValue};
use grooph_measure::onsets::Onset;
use grooph_measure::{BeatKind, StaffKind, TimeSignature, VoiceBar};

mod beams;
mod notes;
mod tuplets;

/// Layout constants. Suffix indicates the multiplier: `_EM` is multiplied by
/// `opts.em`, `_SS` by `opts.staff_space()`. Unsuffixed constants are pure
/// ratios applied to another already-scaled quantity.
mod c {
    // Dot positioning: centers, from the head's center
    pub const DOT_FIRST_DX_WITH_FLAG_EM: f32 = 0.50;
    pub const DOT_FIRST_DX_NO_FLAG_EM: f32 = 0.26;
    pub const DOT_STEP_DX_EM: f32 = 0.26;
    pub const DOT_Y_OFFSET_EM: f32 = 0.10;

    // Collision pipeline
    pub const COLLISION_MIN_GAP_EM: f32 = 0.00;

    /// Space from the start of the note area to the first note head.
    pub const NOTE_LEAD_EM: f32 = 0.45;

    // Tuplet bracket / number
    /// From the stem tips to the bracket line, the number's middle.
    pub const TUPLET_BRACKET_GAP_SS: f32 = 1.90;
    /// From the stem tips to the middle of a number without a bracket.
    pub const TUPLET_NUMBER_GAP_SS: f32 = 1.30;
    /// Least space between the notes' ink (accents, flags, beams) and a
    /// tuplet's number, bracket line or hooks, across and along.
    pub const TUPLET_INK_GAP_SS: f32 = 0.60;
    pub const TUPLET_HOOK_LEN_SS: f32 = 0.80;
    pub const TUPLET_HOOK_DY_FACTOR: f32 = 0.85;
    pub const TUPLET_DIGIT_FONT_FACTOR: f32 = 0.75;
    pub const TUPLET_MARGIN_EM: f32 = 0.05;
    /// Space between the number's ink and the bracket on either side.
    pub const TUPLET_NUMBER_CLEARANCE_SS: f32 = 0.60;
    pub const TUPLET_MIN_SEG_SS: f32 = 0.50;

    /// Space between a soft note's head and the ink of its parentheses.
    pub const SOFT_PAREN_GAP_SS: f32 = 0.016;

    // Beam alignment
    pub const BEAM_BASELINE_OFFSET: f32 = 0.95; // × beam_thickness
    /// A stemlet's share of a stem's length, from the beam toward the line.
    pub const STEMLET_LENGTH_FACTOR: f32 = 0.7;

    /// Width reserved for the percussion clef.
    pub const CLEF_WIDTH_EM: f32 = 1.1;
}

/// A glyph with its origin at `origin`, drawn at font size `size` px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedGlyph {
    pub glyph: Glyph,
    pub origin: Pos2,
    pub size: f32,
}

impl PlacedGlyph {
    /// `glyph` at font size `size`, its advance centered on `x`, its origin
    /// at `y`.
    pub fn centered(glyph: Glyph, x: f32, y: f32, size: f32) -> Self {
        Self { glyph, origin: Pos2::new(x - glyph.advance * size * 0.125, y), size }
    }

    /// Bounding box of the drawn ink.
    pub fn ink(&self) -> Rect {
        self.glyph.ink(self.size * 0.25).translate(self.origin.to_vec2())
    }
}

/// `glyphs` set one after another, their ink centered on `(x, y)`: an
/// italic digit's ink sits off the middle of its advance.
fn centered_run(glyphs: &[Glyph], x: f32, y: f32, size: f32) -> Vec<PlacedGlyph> {
    let ss = size * 0.25;
    let advance: f32 = glyphs.iter().map(|g| g.advance * ss).sum();
    let mut pen = x - advance * 0.5;
    let mut run: Vec<PlacedGlyph> = glyphs
        .iter()
        .map(|&glyph| {
            let placed = PlacedGlyph { glyph, origin: Pos2::new(pen, y), size };
            pen += glyph.advance * ss;
            placed
        })
        .collect();
    if let Some(ink) = ink_of(&run) {
        shift(&mut run, Pos2::new(x, y) - ink.center());
    }
    run
}

/// Bounding box of the ink of `glyphs`, if any.
fn ink_of(glyphs: &[PlacedGlyph]) -> Option<Rect> {
    glyphs.iter().map(PlacedGlyph::ink).reduce(|a, b| a.union(b))
}

fn shift(glyphs: &mut [PlacedGlyph], d: egui::Vec2) {
    for g in glyphs {
        g.origin += d;
    }
}

pub fn compute_em(rect: &Rect, width_cap_factor: f32, ui: &egui::Ui) -> f32 {
    // Derive font size mainly from the available height, modulated by width caps
    let min_size = 12.0 * ui.ctx().pixels_per_point();
    let width_cap = (rect.width() * width_cap_factor).max(min_size);
    let max_size = rect.height().max(min_size);
    min_size.max(max_size.min(width_cap))
}

#[derive(Clone)]
pub struct LayoutOpts {
    pub rect: Rect,
    /// The music font, at size `em`.
    pub font_id: FontId,
    pub pixels_per_point: f32,
    pub em: f32,
    pub layout_clef: bool,
    pub layout_time_signature: bool,

    pub y_offset: f32,
    pub stem_length_factor: f32,
    pub stem_thickness_factor: f32,

    pub accent_displacement: f32,
    /// Stems point down, with beams, flags, tuplets and accents below the
    /// notes: the lower voice of a staff with two.
    pub stem_down: bool,
    /// The staff line is shared with another voice: this one keeps to its
    /// side of it, heads just above the line (just below with
    /// `stem_down`) and rests clear of it. On the kit's staff: the measure
    /// has another voice, and the rests keep to this one's half.
    pub shared_line: bool,
    /// The staff: one line, or the kit's five, where each head sits at its
    /// instrument's place.
    pub staff: StaffKind,
}

impl LayoutOpts {
    /// The staff's description: its lines and where heads and rests sit.
    pub fn staff_model(&self) -> &'static Staff {
        Staff::of(self.staff)
    }

    /// How this voice shares its measure.
    pub const fn voicing(&self) -> Voicing {
        Voicing { shared: self.shared_line, stem_down: self.stem_down }
    }

    pub const fn staff_space(&self) -> f32 {
        self.em * 0.25
    }

    pub(super) const fn stem_length(&self) -> f32 {
        self.em * self.stem_length_factor
    }

    pub const fn stem_thickness(&self) -> f32 {
        self.snap_thickness(self.em * self.stem_thickness_factor)
    }

    /// Half the note head's width.
    pub(super) const fn head_half_width(&self) -> f32 {
        glyphs::NOTEHEAD_BLACK.advance * self.staff_space() * 0.5
    }

    pub(super) const fn stem_offset(&self) -> f32 {
        self.head_half_width() - self.stem_thickness() * 0.5
    }

    pub const fn beam_thickness(&self) -> f32 {
        glyphs::BEAM_THICKNESS * self.staff_space()
    }

    pub(super) const fn beam_gap(&self) -> f32 {
        glyphs::BEAM_SPACING * self.staff_space()
    }

    pub(super) const fn stub_length(&self) -> f32 {
        self.em * 0.20
    }

    pub const fn bracket_thickness(&self) -> f32 {
        self.snap_thickness(self.em * 0.02)
    }

    /// Y of the staff line (note heads sit on it).
    pub fn y_center(&self) -> f32 {
        self.rect.center().y + self.y_offset
    }

    /// Y of staff position `pos`, counted in half staff spaces up from the
    /// staff line: 0 on the line, 1 just above it, -2 a staff space below.
    pub fn y_of(&self, pos: i8) -> f32 {
        self.y_center() - f32::from(pos) * self.staff_space() * 0.5
    }

    /// Which way the stems point on screen: -1 up, 1 down.
    pub(super) const fn stem_dir(&self) -> f32 {
        if self.stem_down { 1.0 } else { -1.0 }
    }

    /// The outermost of `positions` along the stems: the highest for up
    /// stems, the lowest for down stems.
    pub(super) fn outermost(&self, positions: impl Iterator<Item = i8>) -> Option<i8> {
        if self.stem_down { positions.min() } else { positions.max() }
    }

    /// Y of the stem tips of notes reaching from staff position `pos`.
    pub(super) fn stem_tip_y(&self, pos: i8) -> f32 {
        self.y_of(pos) + self.stem_dir() * self.stem_length()
    }

    /// Whether accents go below the notes: they go by the stems, below
    /// with `stem_down`.
    pub(super) const fn accents_below(&self) -> bool {
        self.stem_down
    }

    /// Thickness of the staff line, and of ledger lines.
    pub const fn staff_line_thickness(&self) -> f32 {
        self.em * 0.02
    }

    const fn snap_thickness(&self, t: f32) -> f32 {
        let ppp = self.pixels_per_point;
        (t * ppp).round().max(1.0) / ppp
    }

    /// `p` moved to where a line `thickness` wide through it covers whole
    /// pixels, on both axes.
    pub const fn snap_pos(&self, p: Pos2, thickness: f32) -> Pos2 {
        Pos2::new(self.snap_x(p.x, thickness), self.snap_x(p.y, thickness))
    }

    pub(crate) const fn snap_x(&self, x: f32, thickness: f32) -> f32 {
        let ppp = self.pixels_per_point;
        let px_thickness = (thickness * ppp).round() as i32;
        if px_thickness % 2 != 0 {
            // Odd width: center on half-pixel
            ((x * ppp).round() + 0.5) / ppp
        } else {
            // Even width: center on integer pixel
            (x * ppp).round() / ppp
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BeamLayout {
    pub rect: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line {
    pub p1: Pos2,
    pub p2: Pos2,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TupletLayout {
    /// The number, e.g. 3 for a triplet
    pub count: u8,
    /// The digits of the number
    pub number: Vec<PlacedGlyph>,
    /// Bracket segments including hooks; empty when only the number shows
    pub bracket: Vec<Line>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NoteLayout {
    /// Middle of the head or rest, at `staff_pos`.
    pub center: Pos2,
    /// Where the head or rest sits, in half staff spaces up from the staff
    /// line (see [`LayoutOpts::y_of`]).
    pub staff_pos: i8,
    pub duration: Duration,
    pub kind: BeatKind,
    /// The note head at `staff_pos` (the one nearest the stem's tip), or
    /// the rest. A stemlet's rest is not drawn.
    pub head: PlacedGlyph,
    /// The chord's other heads, on the same stem.
    pub chord: Vec<PlacedGlyph>,
    /// A beamed rest, drawn as a stem from the beam toward the line and no
    /// head (see [`crate::render_plan::plan_measure_with`]).
    pub stemlet: bool,
    /// A rest of an empty bar, drawn as the bar's whole rest by its first
    /// beat (see [`RenderPlan::bar_rest`]): the others are not drawn.
    pub hidden: bool,
    /// The parentheses of each soft (ghost) head.
    pub parens: Vec<PlacedGlyph>,
    /// Ledger lines of the heads or the half or whole rest off the staff
    /// (see [`crate::staff::Staff::ledgers`]).
    pub ledgers: Vec<Line>,
    pub dots: Vec<PlacedGlyph>,
    pub stem: Option<Line>,
    pub flag: Option<PlacedGlyph>,
    pub accent: Option<PlacedGlyph>,
    /// The signs of the heads' techniques (an open or closed hi-hat, a
    /// choke), past the accent.
    pub signs: Vec<PlacedGlyph>,
}

impl NoteLayout {
    /// Every glyph of the note.
    pub fn glyphs(&self) -> impl Iterator<Item = &PlacedGlyph> {
        std::iter::once(&self.head)
            .filter(|_| !self.stemlet && !self.hidden)
            .chain(&self.chord)
            .chain(&self.parens)
            .chain(&self.dots)
            .chain(&self.flag)
            .chain(&self.accent)
            .chain(&self.signs)
    }
}

/// Pixel-level layout for a measure.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasureLayout {
    pub beams: Vec<BeamLayout>,
    pub notes: Vec<NoteLayout>,
    pub tuplets: Vec<TupletLayout>,
    pub clef: Option<PlacedGlyph>,
    pub time_signature: Option<TimeSignatureLayout>,
    // Left boundary of the notes drawing area (excludes clef and time signature)
    pub notes_left_edge: f32,
}

/// The head standing for `onset` in the layouts of its measure's `voices`
/// (see [`Onset::voice`]): where the onset is drawn. `None` where that
/// voice's layout has no such note, i.e. a layout of another measure (a bar
/// drawn as one whole rest keeps a hidden layout per beat).
pub fn onset_head<'a>(voices: &'a [MeasureLayout], onset: &Onset) -> Option<&'a NoteLayout> {
    voices.get(onset.voice)?.notes.get(onset.beat_idx)
}

impl MeasureLayout {
    /// Bounding box of everything drawn for the notes: glyphs, stems, beams
    /// and tuplets.
    pub fn notes_ink(&self) -> Option<Rect> {
        let glyphs = self
            .notes
            .iter()
            .flat_map(NoteLayout::glyphs)
            .chain(self.tuplets.iter().flat_map(|t| &t.number))
            .map(PlacedGlyph::ink);
        let lines = self
            .notes
            .iter()
            .flat_map(|n| n.stem.into_iter().chain(n.ledgers.iter().copied()))
            .chain(self.tuplets.iter().flat_map(|t| t.bracket.iter().copied()))
            .map(|l| Rect::from_two_pos(l.p1, l.p2));
        let beams = self.beams.iter().map(|b| b.rect);
        glyphs.chain(lines).chain(beams).reduce(|a, b| a.union(b))
    }
}

/// Narrowest note area (px, excluding clef and time signature) in which
/// `measure`'s notes, placed by duration, don't overlap each other or the
/// barline.
///
/// `plan` is [`plan_measure`] of `measure`, shared with
/// [`build_measure_layout_with_plan`] so a staff plans each measure once.
pub fn min_note_area_width(measure: &VoiceBar, plan: &RenderPlan, opts: &LayoutOpts) -> f32 {
    notes::min_note_area_width(measure, plan, opts)
}

/// Least distance (px) each beat of `measure` needs to the next note head,
/// or to the barline for the last one, so their glyphs don't overlap: the
/// `min_advance` of [`crate::calculate_x_centers`]. `plan` is
/// [`plan_measure`] of `measure`.
pub fn note_advances(measure: &VoiceBar, plan: &RenderPlan, opts: &LayoutOpts) -> Vec<f32> {
    notes::note_advances(measure, plan, opts)
}

/// Space from the start of the note area to the first note head.
pub fn note_lead(em: f32) -> f32 {
    c::NOTE_LEAD_EM * em
}

/// Width of the clef and time signature `opts` lay out before the notes of
/// a measure in `time_signature`.
pub fn note_area_offset(time_signature: &TimeSignature, opts: &LayoutOpts) -> f32 {
    let clef = if opts.layout_clef { clef_width(opts.em) } else { 0.0 };
    let ts = if opts.layout_time_signature { time_sig_width(time_signature, opts.em) } else { 0.0 };
    clef + ts
}

/// Build the pixel layout (`MeasureLayout`) from a `Measure`.
pub fn build_measure_layout(measure: &VoiceBar, opts: &LayoutOpts) -> MeasureLayout {
    build_measure_layout_with_plan(measure, &plan_measure(measure), opts)
}

/// Width reserved for the percussion clef at the start of a row.
pub(crate) fn clef_width(em: f32) -> f32 {
    c::CLEF_WIDTH_EM * em
}

/// [`build_measure_layout`] with the measure's [`plan_measure`] already made.
pub fn build_measure_layout_with_plan(
    measure: &VoiceBar,
    render_plan: &RenderPlan,
    opts: &LayoutOpts,
) -> MeasureLayout {
    lay_out_measure(measure, render_plan, None, opts)
}

/// [`build_measure_layout_with_plan`] with the notes at `x_centers`, from
/// the left edge of the note area (see [`note_area_offset`]): one voice of
/// several spaced together ([`crate::calculate_shared_x_centers`]).
pub fn build_measure_layout_at(
    measure: &VoiceBar,
    render_plan: &RenderPlan,
    x_centers: &[f32],
    opts: &LayoutOpts,
) -> MeasureLayout {
    lay_out_measure(measure, render_plan, Some(x_centers), opts)
}

fn lay_out_measure(
    measure: &VoiceBar,
    render_plan: &RenderPlan,
    x_centers: Option<&[f32]>,
    opts: &LayoutOpts,
) -> MeasureLayout {
    let mut x_offset_acc = opts.rect.left();

    let clef = if opts.layout_clef {
        let clef_w = clef_width(opts.em);
        x_offset_acc += clef_w;
        let x = opts.rect.left() + clef_w * 0.4;
        Some(PlacedGlyph::centered(glyphs::CLEF_PERCUSSION, x, opts.y_center(), opts.em))
    } else {
        None
    };

    // Time signature
    let time_signature_layout = if opts.layout_time_signature {
        // `build_time_sig_layout` centers on `x`; center it in its reserved
        // slot so it doesn't hang over the measure's left edge (clipped at the
        // start of a row when no clef precedes it).
        let ts = measure.time_signature();
        let ts_width = time_sig_width(&ts, opts.em);
        let ts_layout = build_time_sig_layout(&ts, x_offset_acc + ts_width / 2.0, opts);
        x_offset_acc += ts_width;
        Some(ts_layout)
    } else {
        None
    };

    // Notes
    let notes_left_edge = x_offset_acc;
    let note_rect =
        Rect::from_min_max(Pos2::new(notes_left_edge, opts.rect.top()), opts.rect.right_bottom());
    let mut note_layout =
        notes::build_note_layout(measure, render_plan, &note_rect, x_centers, opts);
    beams::reach_beams(&mut note_layout, &render_plan.beams, opts);
    notes::place_marks(&mut note_layout, measure.beats(), opts);
    let beam_layout = beams::build_beam_layout(&note_layout, &render_plan.beams, opts);
    let tuplet_layout =
        tuplets::build_tuplet_layout(&note_layout, &beam_layout, &render_plan.tuplets, opts);

    MeasureLayout {
        beams: beam_layout,
        notes: note_layout,
        tuplets: tuplet_layout,
        clef,
        time_signature: time_signature_layout,
        notes_left_edge,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimeSignatureLayout {
    pub beats: Vec<PlacedGlyph>,
    pub beat_unit: Vec<PlacedGlyph>,
    pub width: f32,
}

/// Horizontal space reserved for `time_signature` at `em`: the wider of its
/// two numbers.
pub(crate) fn time_sig_width(time_signature: &TimeSignature, em: f32) -> f32 {
    let advance = |n: u8| glyphs::ts_glyphs(n).iter().map(|g| g.advance).sum::<f32>();
    advance(time_signature.beats).max(advance(time_signature.beat_unit)) * em * 0.25
}

/// Lay out time-signature digits horizontally centered on `x`, the numbers
/// centered in the upper and lower half of a five-line staff.
pub fn build_time_sig_layout(
    time_signature: &TimeSignature,
    x: f32,
    opts: &LayoutOpts,
) -> TimeSignatureLayout {
    let row = |n: u8, y: f32| {
        let glyphs = glyphs::ts_glyphs(n);
        let advance: f32 = glyphs.iter().map(|g| g.advance * opts.staff_space()).sum();
        let mut pen = x - advance * 0.5;
        glyphs
            .into_iter()
            .map(|glyph| {
                let placed = PlacedGlyph { glyph, origin: Pos2::new(pen, y), size: opts.em };
                pen += glyph.advance * opts.staff_space();
                placed
            })
            .collect()
    };
    let cy = opts.y_center();
    TimeSignatureLayout {
        beats: row(time_signature.beats, cy - opts.staff_space()),
        beat_unit: row(time_signature.beat_unit, cy + opts.staff_space()),
        width: time_sig_width(time_signature, opts.em),
    }
}

pub(super) fn requires_flag(d: Duration) -> bool {
    matches!(d.base_note(), NoteValue::Eighth | NoteValue::Sixteenth | NoteValue::ThirtySecond)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{FontFamily, FontId, Pos2, Rect};
    use grooph_measure::{Beat, Measure, TimeSignature};

    #[test]
    fn time_signature_without_clef_stays_inside_measure() {
        // A measure starting a row with a time signature but no clef (e.g. a
        // meter change): the digits sit inside its reserved slot, between
        // the measure's left edge and the notes, instead of straddling the
        // left edge.
        let em = 20.0;
        let opts = LayoutOpts {
            rect: Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(200.0, 100.0)),
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: true,
            y_offset: 0.0,
            stem_length_factor: 2.0,
            stem_thickness_factor: 0.1,
            accent_displacement: 0.0,
            stem_down: false,
            shared_line: false,
            staff: StaffKind::Line,
        };
        let layout =
            build_measure_layout(Measure::new(TimeSignature::THREE_FOUR).first_voice(), &opts);
        let ts = layout.time_signature.expect("time signature laid out");
        for digit in ts.beats.iter().chain(ts.beat_unit.iter()) {
            let ink = digit.ink();
            assert!(ink.left() >= opts.rect.left() - 0.01, "digit at {} clipped", ink.left());
            assert!(ink.right() <= layout.notes_left_edge + 0.01);
        }
    }

    #[test]
    fn test_accent_positioning_relative_to_bbox() {
        use grooph_measure::duration::q;
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Quarter note with accent
        let b = Beat::note(q());
        m.first_voice_mut().set_beat(0, b).unwrap();
        m.first_voice_mut().toggle_accent(0, grooph_measure::Instrument::Rhythm);

        let em = 20.0;
        let displacement = 0.5; // 0.5 em gap
        let opts = LayoutOpts {
            rect: Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(100.0, 100.0)),
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset: 0.0,
            stem_length_factor: 3.5,
            stem_thickness_factor: 0.1,
            accent_displacement: displacement,
            stem_down: false,
            shared_line: false,
            staff: StaffKind::Line,
        };

        // Case 1: stems up, accent above
        let layout = build_measure_layout(m.first_voice(), &opts);
        let note = &layout.notes[0];
        let accent = note.accent.expect("Accent should be present").ink();
        let stem = note.stem.expect("Stem should be present");

        let stem_top = stem.p2.y.min(stem.p1.y);
        let expected_y = stem_top - (displacement * em);

        assert!(
            (accent.bottom() - expected_y).abs() < 0.001,
            "Accent (bottom={}) should be exactly at expected (y={})",
            accent.bottom(),
            expected_y
        );

        // Case 2: stems down, accent below
        let opts_below = LayoutOpts { stem_down: true, ..opts.clone() };
        let layout_below = build_measure_layout(m.first_voice(), &opts_below);
        let note_below = &layout_below.notes[0];
        let accent_below = note_below.accent.expect("Accent should be present").ink();

        let head_bottom = note_below.head.ink().bottom();

        assert!(accent_below.top() > head_bottom, "Accent should be below note head");
    }

    #[test]
    fn lower_voice_accent_clears_the_final_flag() {
        use grooph_measure::duration::e;
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        m.first_voice_mut().set_beat(0, Beat::note(e())).unwrap();
        m.first_voice_mut().set_beat(1, Beat::rest(e())).unwrap();
        m.first_voice_mut().toggle_accent(0, grooph_measure::Instrument::Rhythm);

        let em = 20.0;
        let opts = LayoutOpts {
            rect: Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(200.0, 100.0)),
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset: 0.0,
            stem_length_factor: 3.5,
            stem_thickness_factor: 0.1,
            accent_displacement: 0.1,
            stem_down: true,
            shared_line: false,
            staff: StaffKind::Line,
        };
        let layout = build_measure_layout(m.first_voice(), &opts);
        let note = &layout.notes[0];
        let flag = note.flag.expect("an unbeamed eighth has a flag").ink();
        let stem = note.stem.expect("a note has a stem");
        let content_bottom = flag.bottom().max(stem.p1.y.max(stem.p2.y));
        let accent = note.accent.expect("accented").ink();
        assert!(
            (accent.top() - (content_bottom + 0.1 * em)).abs() < 0.001,
            "accent top {} should sit 0.1em under the stem and flag ({})",
            accent.top(),
            content_bottom
        );
    }

    #[test]
    fn test_beam_group_spacing_preservation() {
        use grooph_measure::duration::{t8, th};
        let mut m = Measure::new(TimeSignature::TWO_FOUR);

        // Group 1: 7x 32nd notes
        for i in 0..7 {
            m.first_voice_mut().set_beat(i, Beat::note(th())).unwrap();
        }

        // Group 2: 3x triplet 8th (Indices 7, 8, 9)
        for i in 0..3 {
            m.first_voice_mut().set_beat(7 + i, Beat::note(t8())).unwrap();
        }

        let em = 10.0;
        let rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(50.0, 100.0));

        let opts = LayoutOpts {
            rect,
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset: 0.0,
            stem_length_factor: 3.5,
            stem_thickness_factor: 0.1,
            accent_displacement: 0.0,
            stem_down: false,
            shared_line: false,
            staff: StaffKind::Line,
        };

        let layout = build_measure_layout(m.first_voice(), &opts);

        let n7 = &layout.notes[7];
        let n8 = &layout.notes[8];
        let n9 = &layout.notes[9];

        let d1 = n8.center.x - n7.center.x;
        let d2 = n9.center.x - n8.center.x;

        println!("Triplet spacing: {:.2} vs {:.2}", d1, d2);

        assert!(
            (d1 - d2).abs() < 1.0,
            "Spacing in triplet group should be consistent. Got {:.2} vs {:.2}",
            d1,
            d2
        );
    }

    #[test]
    fn test_primary_group_spacing_preservation() {
        use grooph_measure::duration::{Duration, NoteValue, TupletSpec};

        let em = 10.0;
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);

        // 4x 16th
        let s = Duration::Simple(NoteValue::Sixteenth);
        for i in 0..4 {
            m.first_voice_mut().set_beat(i, Beat::note(s)).unwrap();
        }

        // 3x Quarter Triplets
        let tq = Duration::Tuplet(TupletSpec { n: 3, m: 2, base: NoteValue::Quarter });
        m.first_voice_mut().set_beat(4, Beat::note(tq)).unwrap();
        m.first_voice_mut().set_beat(5, Beat::note(tq)).unwrap();
        m.first_voice_mut().set_beat(6, Beat::note(tq)).unwrap();

        let opts = LayoutOpts {
            rect: Rect::from_min_max(Pos2::ZERO, Pos2::new(50.0, 100.0)),
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset: 0.0,
            stem_length_factor: 3.5,
            stem_thickness_factor: 0.1,
            accent_displacement: 0.0,
            stem_down: false,
            shared_line: false,
            staff: StaffKind::Line,
        };

        let layout = build_measure_layout(m.first_voice(), &opts);

        let n4 = &layout.notes[4];
        let n5 = &layout.notes[5];
        let n6 = &layout.notes[6];

        let d1 = n5.center.x - n4.center.x;
        let d2 = n6.center.x - n5.center.x;

        assert!(
            (d1 - d2).abs() < 0.1,
            "Spacing in primary/tuplet group should be consistent. Got {:.2} vs {:.2}",
            d1,
            d2
        );
    }

    #[test]
    fn down_stems_hang_below_what_up_stems_hang_above() {
        use grooph_measure::notation::parse_score;
        let m = parse_score("4/4 >t8 t8 t8 e e q q").unwrap().into_measures().remove(0);
        let em = 20.0;
        let up = LayoutOpts {
            rect: Rect::from_min_max(Pos2::ZERO, Pos2::new(300.0, 200.0)),
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset: 0.0,
            stem_length_factor: 3.5,
            stem_thickness_factor: 0.1,
            accent_displacement: 0.1,
            stem_down: false,
            shared_line: false,
            staff: StaffKind::Line,
        };
        let down = LayoutOpts { stem_down: true, ..up.clone() };
        let (a, b) = (
            build_measure_layout(m.first_voice(), &up),
            build_measure_layout(m.first_voice(), &down),
        );
        let cy = up.y_center();
        for (u, d) in a.notes.iter().zip(&b.notes) {
            // Same heads, the stem on the other side and below the line.
            assert_eq!(u.center, d.center);
            if let (Some(us), Some(ds)) = (u.stem, d.stem) {
                assert!(us.p1.x > u.center.x && ds.p1.x < d.center.x);
                assert!((ds.p2.y - cy - (cy - us.p2.y)).abs() < 0.01);
            }
        }
        assert!(b.notes[0].accent.unwrap().ink().top() > cy);
        // Beams and brackets are the up stems' mirrored about the line.
        let mirrored = |y: f32| 2.0 * cy - y;
        assert_eq!(a.beams.len(), b.beams.len());
        for (u, d) in a.beams.iter().zip(&b.beams) {
            assert!((d.rect.min.y - mirrored(u.rect.max.y)).abs() < 0.01);
            assert!((d.rect.max.y - mirrored(u.rect.min.y)).abs() < 0.01);
        }
        // A rest at the end of the triplet gives it a bracket.
        let r = parse_score("4/4 t8 t8 r:t8 q h").unwrap().into_measures().remove(0);
        let (ra, rb) = (
            build_measure_layout(r.first_voice(), &up),
            build_measure_layout(r.first_voice(), &down),
        );
        let (ub, db) = (&ra.tuplets[0].bracket, &rb.tuplets[0].bracket);
        assert!(!ub.is_empty());
        assert_eq!(ub.len(), db.len());
        for (u, d) in ub.iter().zip(db) {
            assert!((d.p1.y - mirrored(u.p1.y)).abs() < 0.01);
            assert!((d.p2.y - mirrored(u.p2.y)).abs() < 0.01);
        }
        // The number's ink is centered on its bracket.
        let ink = |l: &MeasureLayout| ink_of(&l.tuplets[0].number).unwrap();
        assert!(ink(&b).top() > cy);
        assert!((ink(&b).center().y - cy - (cy - ink(&a).center().y)).abs() < 0.01);
    }

    #[test]
    fn a_beamed_tuplets_number_centers_on_its_middle_stem() {
        use grooph_measure::notation::parse_score;
        let em = 20.0;
        let up = LayoutOpts {
            rect: Rect::from_min_max(Pos2::ZERO, Pos2::new(300.0, 200.0)),
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset: 0.0,
            stem_length_factor: 0.9,
            stem_thickness_factor: 0.1,
            accent_displacement: 0.07,
            stem_down: false,
            shared_line: false,
            staff: StaffKind::Line,
        };
        let down = LayoutOpts { stem_down: true, ..up.clone() };
        let m = parse_score("4/4 t8 t8 t8 t16 t16 t16 t16 t16 t16 h")
            .unwrap()
            .into_measures()
            .remove(0);
        for opts in [&up, &down] {
            let l = build_measure_layout(m.first_voice(), opts);
            assert_eq!(l.tuplets.len(), 3);
            for (t, mid) in l.tuplets.iter().zip([1, 4, 7]) {
                let stem = l.notes[mid].stem.unwrap().p1.x;
                let center = ink_of(&t.number).unwrap().center().x;
                // Up to the half pixel the stems snap by.
                assert!((center - stem).abs() <= 0.51, "{center} vs {stem}");
            }
        }
    }

    #[test]
    fn a_bracket_leaves_its_clearance_either_side_of_the_number() {
        use grooph_measure::notation::parse_score;
        let em = 20.0;
        let opts = LayoutOpts {
            rect: Rect::from_min_max(Pos2::ZERO, Pos2::new(600.0, 200.0)),
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset: 0.0,
            stem_length_factor: 0.9,
            stem_thickness_factor: 0.1,
            accent_displacement: 0.07,
            stem_down: false,
            shared_line: false,
            staff: StaffKind::Line,
        };
        let clearance = c::TUPLET_NUMBER_CLEARANCE_SS * opts.staff_space();
        // One digit and two, each with a rest for a bracket.
        let ten = "4/4 10:8e 10:8e 10:8e 10:8e 10:8e 10:8e 10:8e 10:8e 10:8e r:10:8e";
        for s in ["4/4 t4 r:t4 t4 h", ten] {
            let l = build_measure_layout(
                parse_score(s).unwrap().into_measures().remove(0).first_voice(),
                &opts,
            );
            let t = &l.tuplets[0];
            let number = ink_of(&t.number).unwrap();
            let (left, right) = (t.bracket[0].p2.x, t.bracket[1].p1.x);
            assert!((number.left() - left - clearance).abs() < 0.01, "{s}");
            assert!((right - number.right() - clearance).abs() < 0.01, "{s}");
        }
    }

    #[test]
    fn tuplets_clear_the_accents_under_them_by_their_ink() {
        use grooph_measure::notation::parse_score;
        let em = 20.0;
        let up = LayoutOpts {
            rect: Rect::from_min_max(Pos2::ZERO, Pos2::new(300.0, 200.0)),
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset: 0.0,
            stem_length_factor: 0.9,
            stem_thickness_factor: 0.1,
            accent_displacement: 0.07,
            stem_down: false,
            shared_line: false,
            staff: StaffKind::Line,
        };
        let ss = up.staff_space();
        let gap = c::TUPLET_INK_GAP_SS * ss;
        let tip = up.stem_tip_y(0);
        let lay_out = |s: &str, opts: &LayoutOpts| {
            build_measure_layout(
                parse_score(s).unwrap().into_measures().remove(0).first_voice(),
                opts,
            )
        };
        let line_of = |l: &MeasureLayout| l.tuplets[0].bracket[0].p1.y;
        let number_of = |l: &MeasureLayout| ink_of(&l.tuplets[0].number).unwrap();

        // Without accents the bracket and the number keep their distance
        // from the stem tips.
        let plain = lay_out("4/4 t8 t8 r:t8 q h", &up);
        assert!((line_of(&plain) - (tip - c::TUPLET_BRACKET_GAP_SS * ss)).abs() < 0.01);
        let beamed = lay_out("4/4 t8 t8 t8 q h", &up);
        assert!(
            (number_of(&beamed).center().y - (tip - c::TUPLET_NUMBER_GAP_SS * ss)).abs() < 0.01
        );

        // An accent under the number lifts it just clear of the accent.
        let l = lay_out("4/4 t8 >t8 t8 q h", &up);
        let accent = l.notes[1].accent.unwrap().ink();
        assert!((number_of(&l).bottom() - (accent.top() - gap)).abs() < 0.01);
        // An accent under a hook lifts the bracket so the hook clears it.
        let l = lay_out("4/4 >t8 t8 r:t8 q h", &up);
        let accent = l.notes[0].accent.unwrap().ink();
        // The hook as painted, half its thickness past its end.
        let hook_end = l.tuplets[0].bracket.iter().map(|b| b.p2.y).fold(f32::MIN, f32::max)
            + 0.5 * up.bracket_thickness();
        assert!((hook_end - (accent.top() - gap)).abs() < 0.01);
        assert!(number_of(&l).bottom() < accent.top() - gap);
        // A quarter's accent sits low enough to leave the bracket nearly
        // where it is without one.
        let l = lay_out("4/4 >t4 t4 t4 h", &up);
        assert!(line_of(&l) > tip - 2.1 * ss);

        // Down stems hang the same below the notes.
        let down = LayoutOpts { stem_down: true, ..up.clone() };
        let (a, b) = (lay_out("4/4 t8 >t8 t8 q h", &up), lay_out("4/4 t8 >t8 t8 q h", &down));
        let cy = up.y_center();
        assert!((number_of(&b).center().y - cy - (cy - number_of(&a).center().y)).abs() < 0.01);
    }

    #[test]
    fn lower_rests_drop_but_half_rests_keep_their_line() {
        use grooph_measure::notation::parse_score;
        let m = parse_score("4/4 h r:q r:q").unwrap().into_measures().remove(0);
        let h = parse_score("4/4 h r:h").unwrap().into_measures().remove(0);
        let em = 20.0;
        let down = LayoutOpts {
            rect: Rect::from_min_max(Pos2::ZERO, Pos2::new(300.0, 200.0)),
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset: 0.0,
            stem_length_factor: 3.5,
            stem_thickness_factor: 0.1,
            accent_displacement: 0.1,
            stem_down: true,
            shared_line: false,
            staff: StaffKind::Line,
        };
        let cy = down.y_center();
        let quarter_rest = &build_measure_layout(m.first_voice(), &down).notes[1];
        assert_eq!(quarter_rest.staff_pos, -2);
        assert!((quarter_rest.center.y - cy - down.staff_space()).abs() < 0.01);
        assert_eq!(quarter_rest.center.y, down.y_of(-2));
        assert_eq!(build_measure_layout(h.first_voice(), &down).notes[1].center.y, cy);

        // A dotted rest's dot drops with it. Off the line, with no line to
        // keep clear of, it stays level with the rest's middle.
        let dotted = parse_score("4/4 h r:q. e").unwrap().into_measures().remove(0);
        let up_rest = &build_measure_layout(
            dotted.first_voice(),
            &LayoutOpts { stem_down: false, ..down.clone() },
        )
        .notes[1];
        let down_rest = &build_measure_layout(dotted.first_voice(), &down).notes[1];
        let dot_dy = |n: &NoteLayout| n.dots[0].origin.y - n.center.y;
        assert_eq!(down_rest.dots.len(), 1);
        assert!(down_rest.dots[0].origin.y > up_rest.dots[0].origin.y);
        assert!(dot_dy(down_rest).abs() < 0.01 && dot_dy(up_rest) < 0.0);
    }

    #[test]
    fn a_shared_lines_voices_keep_their_rests_and_accents_on_their_side() {
        use grooph_measure::notation::parse_score;
        let em = 20.0;
        let up = LayoutOpts {
            rect: Rect::from_min_max(Pos2::ZERO, Pos2::new(400.0, 200.0)),
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset: 0.0,
            stem_length_factor: 0.9,
            stem_thickness_factor: 0.1,
            accent_displacement: 0.07,
            stem_down: false,
            shared_line: true,
            staff: StaffKind::Line,
        };
        let down = LayoutOpts { stem_down: true, ..up.clone() };
        let cy = up.y_center();
        let rests =
            parse_score("4/4 >q r:q r:e r:s r:th r:th r:e. s").unwrap().into_measures().remove(0);
        let (a, b) = (
            build_measure_layout(rests.first_voice(), &up),
            build_measure_layout(rests.first_voice(), &down),
        );
        // Heads sit on the line or hang from it.
        assert_eq!((a.notes[0].staff_pos, b.notes[0].staff_pos), (1, -1));
        assert!((a.notes[0].head.ink().bottom() - cy).abs() < 0.01);
        assert!((b.notes[0].head.ink().top() - cy).abs() < 0.01);
        // Rests clear the line by at least a quarter staff space.
        let clear = 0.25 * up.staff_space() - 0.01;
        for (u, d) in a.notes.iter().zip(&b.notes).filter(|(u, _)| u.kind == BeatKind::Rest) {
            assert!(u.head.ink().bottom() <= cy - clear, "{u:?}");
            assert!(d.head.ink().top() >= cy + clear, "{d:?}");
            assert!(u.ledgers.is_empty() && d.ledgers.is_empty());
        }
        // Accents go by the stems.
        assert!(a.notes[0].accent.unwrap().ink().bottom() < a.notes[0].stem.unwrap().p2.y);
        assert!(b.notes[0].accent.unwrap().ink().top() > b.notes[0].stem.unwrap().p2.y);

        // A half rest sits on the line, or on a ledger line below it in the
        // lower voice.
        let half = parse_score("4/4 r:h q q").unwrap().into_measures().remove(0);
        let (upper_half, lower_half) = (
            &build_measure_layout(half.first_voice(), &up).notes[0],
            &build_measure_layout(half.first_voice(), &down).notes[0],
        );
        assert!(upper_half.ledgers.is_empty());
        assert!((upper_half.head.ink().bottom() - cy).abs() < 0.1);
        let ledger = *lower_half.ledgers.first().expect("a ledger line below");
        assert_eq!(ledger.p1.y, down.y_of(-2));
        let ink = lower_half.head.ink();
        assert!(ink.top() > cy && (ink.bottom() - ledger.p1.y).abs() < 0.1);
        assert!(ledger.p1.x < ink.left() && ledger.p2.x > ink.right());

        // Dots and a soft note's parentheses stay off the line too.
        let dotted = parse_score("3/4 (q.) (q.)").unwrap().into_measures().remove(0);
        let (a, b) = (
            build_measure_layout(dotted.first_voice(), &up),
            build_measure_layout(dotted.first_voice(), &down),
        );
        for (u, d) in a.notes.iter().zip(&b.notes) {
            let ink =
                |n: &NoteLayout| ink_of(&[n.dots.clone(), n.parens.clone()].concat()).unwrap();
            assert!(ink(u).bottom() <= cy + 0.01, "{u:?}");
            assert!(ink(d).top() >= cy - 0.01, "{d:?}");
            // The parentheses still reach past the head on its far side.
            assert!(ink_of(&u.parens).unwrap().top() < u.head.ink().top());
            assert!(ink_of(&d.parens).unwrap().bottom() > d.head.ink().bottom());
        }

        // Without a shared line nothing moves.
        let alone = LayoutOpts { shared_line: false, ..up.clone() };
        assert!(
            build_measure_layout(rests.first_voice(), &alone)
                .notes
                .iter()
                .all(|n| n.staff_pos == 0)
        );
    }

    #[test]
    fn dots_sit_right_of_parentheses_and_clear_of_down_flags() {
        use grooph_measure::notation::parse_score;
        let m = parse_score("3/4 e. (e.) (e.) e.").unwrap().into_measures().remove(0);
        let em = 20.0;
        let up = LayoutOpts {
            rect: Rect::from_min_max(Pos2::ZERO, Pos2::new(400.0, 200.0)),
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset: 0.0,
            stem_length_factor: 0.9,
            stem_thickness_factor: 0.1,
            accent_displacement: 0.0,
            stem_down: false,
            shared_line: false,
            staff: StaffKind::Line,
        };
        let down = LayoutOpts { stem_down: true, ..up.clone() };
        let dx =
            |l: &MeasureLayout, i: usize| l.notes[i].dots[0].ink().center().x - l.notes[i].center.x;
        let (u, d) = (
            build_measure_layout(m.first_voice(), &up),
            build_measure_layout(m.first_voice(), &down),
        );
        let plain = c::DOT_FIRST_DX_NO_FLAG_EM * em;
        // Beamed: right after the head, or after the parentheses.
        assert!((dx(&u, 0) - plain).abs() < 0.01);
        let paren_right = u.notes[1].parens[1].ink().right() - u.notes[1].center.x;
        assert!(dx(&u, 1) > paren_right);
        // A down-stem flag hangs below the dots: they stay at the head.
        assert!((dx(&d, 3) - plain).abs() < 0.01);
        assert!(dx(&d, 2) > paren_right && dx(&d, 2) < dx(&u, 2) + 0.01);
    }

    #[test]
    fn soft_notes_dont_squeeze_plain_ones_of_the_same_length() {
        use grooph_measure::notation::parse_score;
        let m = parse_score("4/4 >t8 t8 (t8) t8 (t8) (t8) t8 (t8) (t8) t8 (t8) (t8)")
            .unwrap()
            .into_measures()
            .remove(0);
        let em = 20.0;
        let mut opts = LayoutOpts {
            rect: Rect::from_min_max(Pos2::ZERO, Pos2::new(100.0, 100.0)),
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset: 0.0,
            stem_length_factor: 3.5,
            stem_thickness_factor: 0.1,
            accent_displacement: 0.0,
            stem_down: false,
            shared_line: false,
            staff: StaffKind::Line,
        };
        let plan = crate::render_plan::plan_measure(m.first_voice());
        let width = min_note_area_width(m.first_voice(), &plan, &opts);
        opts.rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(width, 100.0));
        let layout = build_measure_layout(m.first_voice(), &opts);
        let xs: Vec<f32> = layout.notes.iter().map(|n| n.center.x).collect();
        // Equal triplets, equally spaced, even where the measure is tight.
        let steps: Vec<f32> = xs.windows(2).map(|p| p[1] - p[0]).collect();
        for step in &steps {
            assert!((step - steps[0]).abs() < 0.5, "{xs:?}");
        }
    }

    #[test]
    fn beamed_rests_of_a_lower_voice_are_stemlets_under_its_beams() {
        use grooph_measure::notation::parse_score;
        let m = parse_score("1/4 s r:s r:s s").unwrap().into_measures().remove(0);
        let em = 20.0;
        let opts = LayoutOpts {
            rect: Rect::from_min_max(Pos2::ZERO, Pos2::new(100.0, 100.0)),
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset: 0.0,
            stem_length_factor: 3.5,
            stem_thickness_factor: 0.1,
            accent_displacement: 0.0,
            stem_down: true,
            shared_line: false,
            staff: StaffKind::Line,
        };
        let plan = crate::render_plan::plan_measure_with(m.first_voice(), true);
        let layout = build_measure_layout_with_plan(m.first_voice(), &plan, &opts);
        let [note, rest, _, _] = &layout.notes[..] else { panic!("four beats") };
        assert!(rest.stemlet && !note.stemlet);
        // No rest glyph, and a stem from the beam, shorter than the note's.
        assert_eq!(rest.glyphs().count(), 0);
        assert_eq!(rest.staff_pos, 0);
        let (n, r) = (note.stem.unwrap(), rest.stem.unwrap());
        assert_eq!(r.p2.y, n.p2.y);
        assert!((r.p2.y - r.p1.y).abs() < (n.p2.y - n.p1.y).abs());
        // Two beam levels between each two of the four stems.
        assert_eq!(layout.beams.len(), 6);
    }
}
