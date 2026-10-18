use crate::glyphs::{self, Glyph};
use crate::render_plan::{RenderPlan, plan_measure};
use egui::{FontId, Pos2, Rect};
use grooph_measure::duration::{Duration, NoteValue};
use grooph_measure::{BeatKind, Measure, TimeSignature};

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
    pub const TUPLET_BRACKET_GAP_SS: f32 = 2.30;
    pub const TUPLET_HOOK_LEN_SS: f32 = 0.80;
    pub const TUPLET_HOOK_DY_FACTOR: f32 = 0.85;
    pub const TUPLET_DIGIT_FONT_FACTOR: f32 = 0.75;
    pub const TUPLET_MARGIN_EM: f32 = 0.15;
    /// Space between the number's ink and the bracket on either side.
    pub const TUPLET_NUMBER_CLEARANCE_SS: f32 = 1.75;
    pub const TUPLET_ACCENT_RAISE_SS: f32 = 1.40;
    pub const TUPLET_ACCENT_LOWER_SS: f32 = -0.40;
    pub const TUPLET_NUMBER_CLOSE_SS: f32 = -1.00;
    pub const TUPLET_MIN_SEG_SS: f32 = 0.50;

    /// Space between a soft note's head and the ink of its parentheses.
    pub const SOFT_PAREN_GAP_SS: f32 = 0.016;

    // Beam alignment
    pub const BEAM_BASELINE_OFFSET: f32 = 0.95; // × beam_thickness

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

/// `glyphs` set one after another, the run's advance centered on `x` and its
/// ink centered on `y`.
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
        shift_y(&mut run, y - ink.center().y);
    }
    run
}

/// Bounding box of the ink of `glyphs`, if any.
fn ink_of(glyphs: &[PlacedGlyph]) -> Option<Rect> {
    glyphs.iter().map(PlacedGlyph::ink).reduce(|a, b| a.union(b))
}

fn shift_y(glyphs: &mut [PlacedGlyph], dy: f32) {
    for g in glyphs {
        g.origin.y += dy;
    }
}

/// Move `glyphs` as a whole to where their ink, mirrored about `cy`, is.
fn mirror_ink(glyphs: &mut [PlacedGlyph], cy: f32) {
    if let Some(ink) = ink_of(glyphs) {
        shift_y(glyphs, (2.0 * cy - ink.bottom()) - ink.top());
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
    pub accent_below: bool,
    /// Stems point down, with beams, flags, tuplets and accents below the
    /// notes: the lower voice of a staff with two.
    pub stem_down: bool,
    pub proportional_spacing: bool,
    pub debug_bbox: bool,
}

impl LayoutOpts {
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
        self.em * 0.02
    }

    /// Y of the staff line (note heads sit on it).
    pub fn y_center(&self) -> f32 {
        self.rect.center().y + self.y_offset
    }

    const fn snap_thickness(&self, t: f32) -> f32 {
        let ppp = self.pixels_per_point;
        (t * ppp).round().max(1.0) / ppp
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
    /// Middle of the head or rest, on the staff line.
    pub center: Pos2,
    pub duration: Duration,
    pub kind: BeatKind,
    /// The note head, or the rest.
    pub head: PlacedGlyph,
    /// The parentheses of a soft (ghost) note's head.
    pub parens: Vec<PlacedGlyph>,
    pub dots: Vec<PlacedGlyph>,
    pub stem: Option<Line>,
    pub flag: Option<PlacedGlyph>,
    pub accent: Option<PlacedGlyph>,
    pub debug_bbox: Option<Rect>,
    pub accent_debug_bbox: Option<Rect>,
}

impl NoteLayout {
    /// Every glyph of the note.
    pub fn glyphs(&self) -> impl Iterator<Item = &PlacedGlyph> {
        std::iter::once(&self.head)
            .chain(&self.parens)
            .chain(&self.dots)
            .chain(&self.flag)
            .chain(&self.accent)
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
            .filter_map(|n| n.stem)
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
pub fn min_note_area_width(measure: &Measure, plan: &RenderPlan, opts: &LayoutOpts) -> f32 {
    notes::min_note_area_width(measure, plan, opts)
}

/// Build the pixel layout (`MeasureLayout`) from a `Measure`.
pub fn build_measure_layout(measure: &Measure, opts: &LayoutOpts) -> MeasureLayout {
    build_measure_layout_with_plan(measure, &plan_measure(measure), opts)
}

/// Width reserved for the percussion clef at the start of a row.
pub(crate) fn clef_width(em: f32) -> f32 {
    c::CLEF_WIDTH_EM * em
}

/// [`build_measure_layout`] with the measure's [`plan_measure`] already made.
pub fn build_measure_layout_with_plan(
    measure: &Measure,
    render_plan: &RenderPlan,
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
    let note_layout = notes::build_note_layout(measure, render_plan, &note_rect, opts);
    let beam_layout = beams::build_beam_layout(&note_layout, &render_plan.beams, opts);
    let tuplet_layout =
        tuplets::build_tuplet_layout(measure.beats(), &note_layout, &render_plan.tuplets, opts);

    let mut layout = MeasureLayout {
        beams: beam_layout,
        notes: note_layout,
        tuplets: tuplet_layout,
        clef,
        time_signature: time_signature_layout,
        notes_left_edge,
    };
    if opts.stem_down {
        flip_down(&mut layout, opts);
    }
    layout
}

/// Mirror what hangs off the stems (stems, flags, beams, tuplets, accents)
/// to below the staff line, and lower the rests a staff space, clear of an
/// upper voice's. Heads and dots stay where they are.
fn flip_down(layout: &mut MeasureLayout, opts: &LayoutOpts) {
    let cy = opts.y_center();
    let y = |y: f32| 2.0 * cy - y;
    let pos = |p: Pos2| Pos2::new(p.x, y(p.y));
    let rect = |r: Rect| Rect::from_two_pos(pos(r.min), pos(r.max));
    for note in &mut layout.notes {
        if note.kind == BeatKind::Rest {
            note.center.y += opts.staff_space();
            note.head.origin.y += opts.staff_space();
        }
        if let Some(stem) = &mut note.stem {
            *stem = Line { p1: pos(stem.p1), p2: pos(stem.p2) };
            if let Some(flag) = &mut note.flag {
                *flag = notes::flag_at(note.duration, stem, true, opts).unwrap_or(*flag);
            }
        }
        if let Some(accent) = &mut note.accent {
            mirror_ink(std::slice::from_mut(accent), cy);
        }
        note.debug_bbox = note.debug_bbox.map(rect);
        note.accent_debug_bbox = note.accent_debug_bbox.map(rect);
    }
    for beam in &mut layout.beams {
        beam.rect = rect(beam.rect);
    }
    for tuplet in &mut layout.tuplets {
        // The number moves as a whole, its ink on the mirrored bracket line.
        mirror_ink(&mut tuplet.number, cy);
        for line in &mut tuplet.bracket {
            *line = Line { p1: pos(line.p1), p2: pos(line.p2) };
        }
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
            accent_below: false,
            stem_down: false,
            proportional_spacing: true,
            debug_bbox: false,
        };
        let layout = build_measure_layout(&Measure::new(TimeSignature::THREE_FOUR), &opts);
        let ts = layout.time_signature.expect("time signature laid out");
        for digit in ts.beats.iter().chain(ts.beat_unit.iter()) {
            let ink = digit.ink();
            assert!(ink.left() >= opts.rect.left() - 0.01, "digit at {} clipped", ink.left());
            assert!(ink.right() <= layout.notes_left_edge + 0.01);
        }
    }

    #[test]
    fn test_debug_bbox_includes_stem_length() {
        use grooph_measure::duration::e;
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Add one eighth note (has flag -> stem_len_factor = 1.0)
        m.set_beat(0, Beat::note(e())).unwrap();

        let em = 20.0;
        let opts = LayoutOpts {
            rect: Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(100.0, 100.0)),
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset: 0.0,
            stem_length_factor: 2.0, // Long stem: 2.0 * 20.0 = 40.0
            stem_thickness_factor: 0.1,
            accent_displacement: 0.0,
            accent_below: false,
            stem_down: false,
            proportional_spacing: true,
            debug_bbox: true,
        };

        let layout = build_measure_layout(&m, &opts);
        let note = &layout.notes[0];
        let bbox = note.debug_bbox.expect("Debug bbox should be present");

        let cy = opts.rect.center().y; // 50.0
        let stem_len = em * opts.stem_length_factor; // 40.0 (factor 1.0 due to flag)

        // Assert bbox top covers the stem
        // Allow small margin for snapping
        assert!(
            bbox.min.y <= cy - stem_len + 1.0,
            "BBox top ({}) should cover stem top ({})",
            bbox.min.y,
            cy - stem_len
        );
    }

    #[test]
    fn test_accent_positioning_relative_to_bbox() {
        use grooph_measure::duration::q;
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Quarter note with accent
        let b = Beat::note(q());
        m.set_beat(0, b).unwrap();
        m.toggle_accent(0);

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
            accent_below: false,
            stem_down: false,
            proportional_spacing: true,
            debug_bbox: true,
        };

        // Case 1: Accent Above (default)
        let layout = build_measure_layout(&m, &opts);
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

        // Case 2: Accent Below
        let mut opts_below = opts.clone();
        opts_below.accent_below = true;
        let layout_below = build_measure_layout(&m, &opts_below);
        let note_below = &layout_below.notes[0];
        let accent_below = note_below.accent.expect("Accent should be present").ink();

        let head_bottom = note_below.head.ink().bottom();

        assert!(accent_below.top() > head_bottom, "Accent should be below note head");
    }

    #[test]
    fn test_accent_debug_bbox_separate() {
        use grooph_measure::duration::q;
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        let b = Beat::note(q());
        m.set_beat(0, b).unwrap();
        m.toggle_accent(0);

        let em = 20.0;
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
            accent_displacement: 0.5,
            accent_below: false,
            stem_down: false,
            proportional_spacing: true,
            debug_bbox: true,
        };

        let layout = build_measure_layout(&m, &opts);
        let note = &layout.notes[0];

        assert!(note.debug_bbox.is_some(), "Main debug bbox should be present");
        assert!(note.accent_debug_bbox.is_some(), "Accent debug bbox should be present");

        let main_bbox = note.debug_bbox.unwrap();
        let accent_bbox = note.accent_debug_bbox.unwrap();

        let expected_size = note.accent.unwrap().ink().size();
        assert!((accent_bbox.width() - expected_size.x).abs() < 0.001);
        assert!((accent_bbox.height() - expected_size.y).abs() < 0.001);

        assert!(!main_bbox.intersects(accent_bbox), "BBoxes should be separate");
    }

    #[test]
    fn test_beam_group_spacing_preservation() {
        use grooph_measure::duration::{t8, th};
        let mut m = Measure::new(TimeSignature::TWO_FOUR);

        // Group 1: 7x 32nd notes
        for i in 0..7 {
            m.set_beat(i, Beat::note(th())).unwrap();
        }

        // Group 2: 3x triplet 8th (Indices 7, 8, 9)
        for i in 0..3 {
            m.set_beat(7 + i, Beat::note(t8())).unwrap();
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
            accent_below: false,
            stem_down: false,
            proportional_spacing: true,
            debug_bbox: true,
        };

        let layout = build_measure_layout(&m, &opts);

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
            m.set_beat(i, Beat::note(s)).unwrap();
        }

        // 3x Quarter Triplets
        let tq = Duration::Tuplet(TupletSpec { n: 3, m: 2, base: NoteValue::Quarter });
        m.set_beat(4, Beat::note(tq)).unwrap();
        m.set_beat(5, Beat::note(tq)).unwrap();
        m.set_beat(6, Beat::note(tq)).unwrap();

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
            accent_below: false,
            stem_down: false,
            proportional_spacing: true,
            debug_bbox: true,
        };

        let layout = build_measure_layout(&m, &opts);

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
    fn down_stems_mirror_what_hangs_off_them() {
        use grooph_measure::notation::parse_score;
        let m = parse_score("4/4 >t8 t8 t8 e e q q").unwrap().measures.remove(0);
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
            accent_below: false,
            stem_down: false,
            proportional_spacing: true,
            debug_bbox: false,
        };
        let down = LayoutOpts { stem_down: true, ..up.clone() };
        let (a, b) = (build_measure_layout(&m, &up), build_measure_layout(&m, &down));
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
        assert!(b.beams.iter().all(|r| r.rect.min.y > cy));
        // The number's ink is mirrored as a whole: centered on its bracket.
        let ink = |l: &MeasureLayout| ink_of(&l.tuplets[0].number).unwrap();
        assert!(ink(&b).top() > cy);
        assert!((ink(&b).center().y - cy - (cy - ink(&a).center().y)).abs() < 0.01);
    }

    #[test]
    fn dots_sit_right_of_parentheses_and_clear_of_down_flags() {
        use grooph_measure::notation::parse_score;
        let m = parse_score("3/4 e. (e.) (e.) e.").unwrap().measures.remove(0);
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
            accent_below: false,
            stem_down: false,
            proportional_spacing: true,
            debug_bbox: false,
        };
        let down = LayoutOpts { stem_down: true, ..up.clone() };
        let dx =
            |l: &MeasureLayout, i: usize| l.notes[i].dots[0].ink().center().x - l.notes[i].center.x;
        let (u, d) = (build_measure_layout(&m, &up), build_measure_layout(&m, &down));
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
            .measures
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
            accent_below: false,
            stem_down: false,
            proportional_spacing: true,
            debug_bbox: false,
        };
        let plan = crate::render_plan::plan_measure(&m);
        let width = min_note_area_width(&m, &plan, &opts);
        opts.rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(width, 100.0));
        let layout = build_measure_layout(&m, &opts);
        let xs: Vec<f32> = layout.notes.iter().map(|n| n.center.x).collect();
        // Equal triplets, equally spaced, even where the measure is tight.
        let steps: Vec<f32> = xs.windows(2).map(|p| p[1] - p[0]).collect();
        for step in &steps {
            assert!((step - steps[0]).abs() < 0.5, "{xs:?}");
        }
    }
}
