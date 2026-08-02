use crate::render_plan::plan_measure;
use egui::{FontId, Pos2, Rect, Vec2};
use grooph_measure::duration::{Duration, NoteValue};
use grooph_measure::{BeatKind, Measure, TimeSignature};

mod beams;
mod notes;
mod tuplets;

/// Layout constants. Suffix indicates the multiplier: `_EM` is multiplied by
/// `opts.em`, `_SS` by `opts.staff_space()`. Unsuffixed constants are pure
/// ratios applied to another already-scaled quantity.
mod c {
    // Dot positioning
    pub const DOT_FIRST_DX_WITH_FLAG_EM: f32 = 0.50;
    pub const DOT_FIRST_DX_NO_FLAG_EM: f32 = 0.26;
    pub const DOT_STEP_DX_EM: f32 = 0.26;
    pub const DOT_Y_OFFSET_EM: f32 = 0.10;

    // Collision pipeline
    pub const COLLISION_MIN_GAP_EM: f32 = 0.00;

    // Tuplet bracket / number
    pub const TUPLET_BRACKET_GAP_SS: f32 = 1.80;
    pub const TUPLET_HOOK_LEN_SS: f32 = 0.80;
    pub const TUPLET_HOOK_DY_FACTOR: f32 = 0.85;
    pub const TUPLET_DIGIT_FONT_FACTOR: f32 = 0.75;
    pub const TUPLET_MARGIN_EM: f32 = 0.15;
    pub const TUPLET_NUM_WIDTH_EM: f32 = 0.60;
    pub const TUPLET_NUM_PAD_EM: f32 = 0.25;
    pub const TUPLET_ACCENT_RAISE_SS: f32 = 1.40;
    pub const TUPLET_ACCENT_LOWER_SS: f32 = -0.40;
    pub const TUPLET_NUMBER_CLOSE_SS: f32 = -1.00;
    pub const TUPLET_NUMBER_BASELINE_LIFT_SS: f32 = 0.50;
    pub const TUPLET_MIN_SEG_SS: f32 = 0.50;

    // Beam alignment
    pub const BEAM_BASELINE_OFFSET: f32 = 0.95; // × beam_thickness
}

#[derive(Debug, Clone, Copy)]
pub struct GlyphMetrics {
    pub head_size: Vec2,
    pub dot_size: Vec2,
    pub accent_size: Vec2,
    pub flag_8th_size: Vec2,
    pub flag_16th_size: Vec2,
    pub flag_32nd_size: Vec2,
    pub rest_sizes: [Vec2; 6],
}

impl GlyphMetrics {
    /// Size of the flag glyph for a note value. Falls back to the 8th flag
    /// for non-flagged note values (callers should already gate on
    /// `requires_flag`).
    pub fn flag_size_for(&self, nv: NoteValue) -> Vec2 {
        match nv {
            NoteValue::Sixteenth => self.flag_16th_size,
            NoteValue::ThirtySecond => self.flag_32nd_size,
            _ => self.flag_8th_size,
        }
    }

    pub fn debug(em: f32) -> Self {
        let default_size = Vec2::new(0.4 * em, 0.25 * em);
        Self {
            head_size: default_size,
            dot_size: Vec2::new(0.2 * em, 0.2 * em),
            accent_size: Vec2::new(0.4 * em, 0.2 * em),
            flag_8th_size: Vec2::new(0.3 * em, 1.0 * em),
            flag_16th_size: Vec2::new(0.35 * em, 1.2 * em),
            flag_32nd_size: Vec2::new(0.4 * em, 1.4 * em),
            rest_sizes: [default_size; 6],
        }
    }

    /// Scale every glyph metric by `factor`. Used when a layout shrinks or
    /// grows toward the legibility floor without a live `egui::Ui` to
    /// re-measure the font — vector-font metrics scale ~linearly with size.
    pub fn scaled(&self, factor: f32) -> Self {
        Self {
            head_size: self.head_size * factor,
            dot_size: self.dot_size * factor,
            accent_size: self.accent_size * factor,
            flag_8th_size: self.flag_8th_size * factor,
            flag_16th_size: self.flag_16th_size * factor,
            flag_32nd_size: self.flag_32nd_size * factor,
            rest_sizes: self.rest_sizes.map(|v| v * factor),
        }
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
    pub proportional_spacing: bool,
    pub debug_bbox: bool,
    pub metrics: GlyphMetrics,
}

impl LayoutOpts {
    pub(super) const fn staff_space(&self) -> f32 { self.em * 0.25 }

    pub(super) const fn stem_length(&self) -> f32 { self.em * self.stem_length_factor }

    pub const fn stem_thickness(&self) -> f32 {
        self.snap_thickness(self.em * self.stem_thickness_factor)
    }

    pub(super) const fn stem_offset(&self) -> f32 {
        self.metrics.head_size.x * 0.5 - self.stem_thickness() * 0.5
    }

    pub const fn beam_thickness(&self) -> f32 {
        // Bravura ~0.5 sp
        0.5 * self.staff_space()
    }

    pub(super) const fn beam_gap(&self) -> f32 { 0.25 * self.staff_space() }

    pub(super) const fn stub_length(&self) -> f32 { self.em * 0.20 }

    pub const fn bracket_thickness(&self) -> f32 { self.em * 0.02 }

    pub(super) fn y_center(&self) -> f32 { self.rect.center().y + self.y_offset }

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
    /// Ziffer (z. B. 3 für Triole)
    pub count: u8,
    /// Zentrum der Zahl in Pixelkoordinaten
    pub number_center: Pos2,
    /// Font für die Zahl (vom Layout vorgegeben)
    pub number_font: FontId,
    /// Klammersegmente inkl. Haken; leer bei number-only Fall
    pub bracket: Vec<Line>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NoteLayout {
    pub center: Pos2,
    pub duration: Duration,
    pub kind: BeatKind,
    pub dots: Vec<Pos2>,
    pub stem: Option<Line>,
    pub flag_pos: Option<Pos2>,
    pub accent_pos: Option<Pos2>,
    pub debug_bbox: Option<Rect>,
    pub accent_debug_bbox: Option<Rect>,
}

/// Pixel-level layout for a measure.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasureLayout {
    pub beams: Vec<BeamLayout>,
    pub notes: Vec<NoteLayout>,
    pub tuplets: Vec<TupletLayout>,
    pub clef_pos: Option<Pos2>,
    pub time_signature: Option<TimeSignatureLayout>,
    // Left boundary of the notes drawing area (excludes clef and time signature)
    pub notes_left_edge: f32,
}

/// Build the pixel layout (`MeasureLayout`) from a `Measure`.
pub fn build_measure_layout(measure: &Measure, opts: &LayoutOpts) -> MeasureLayout {
    let mut x_offset_acc = opts.rect.left();

    let clef_pos = if opts.layout_clef {
        let clef_w = opts.em * 1.1; // reserved width for percussion clef
        x_offset_acc += clef_w;
        Some(Pos2::new(opts.rect.left() + clef_w * 0.4, opts.y_center()))
    } else {
        None
    };

    // Time signature
    let time_signature_layout = if opts.layout_time_signature {
        let ts_layout = build_time_sig_layout(&measure.time_signature(), x_offset_acc, opts);
        x_offset_acc += ts_layout.width;
        Some(ts_layout)
    } else {
        None
    };

    // Notes
    let notes_left_edge = x_offset_acc;
    let note_rect =
        Rect::from_min_max(Pos2::new(notes_left_edge, opts.rect.top()), opts.rect.right_bottom());
    let render_plan = plan_measure(measure);
    let note_layout = notes::build_note_layout(measure, &render_plan, &note_rect, opts);
    let beam_layout = beams::build_beam_layout(&note_layout, &render_plan.beams, opts);
    let tuplet_layout =
        tuplets::build_tuplet_layout(measure.beats(), &note_layout, &render_plan.tuplets, opts);

    MeasureLayout {
        beams: beam_layout,
        notes: note_layout,
        tuplets: tuplet_layout,
        clef_pos,
        time_signature: time_signature_layout,
        notes_left_edge,
    }
}

pub(super) fn digit_count(mut n: u32) -> usize {
    let mut c = 0usize;
    while n > 0 {
        c += 1;
        n /= 10;
    }
    c
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimeSignatureLayout {
    pub beats: Vec<Pos2>,
    pub beat_unit: Vec<Pos2>,
    pub width: f32,
}

pub fn build_time_sig_layout(
    time_signature: &TimeSignature,
    x: f32,
    opts: &LayoutOpts,
) -> TimeSignatureLayout {
    let ts_digit_w = opts.em * 0.35; // per column
    let top_digits = digit_count(time_signature.beats as u32);
    let bot_digits = digit_count(time_signature.beat_unit as u32);
    let ts_cols = top_digits.max(bot_digits) as f32;

    // Compute centered columns for both rows
    let mut time_sig_top: Vec<Pos2> = Vec::with_capacity(top_digits);
    let mut time_sig_bottom: Vec<Pos2> = Vec::with_capacity(bot_digits);

    let x_offset = top_digits.max(bot_digits) as f32 * ts_digit_w / 2.0;

    if top_digits > 0 {
        let offset = (ts_cols - top_digits as f32) * 0.5;
        for i in 0..top_digits {
            let cx = x - x_offset + ((i as f32) + 0.5 + offset) * ts_digit_w;
            time_sig_top.push(Pos2::new(cx, opts.y_center() - opts.em * 0.25));
        }
    }
    if bot_digits > 0 {
        let offset = (ts_cols - bot_digits as f32) * 0.5;
        for i in 0..bot_digits {
            let cx = x - x_offset + ((i as f32) + 0.5 + offset) * ts_digit_w;
            time_sig_bottom.push(Pos2::new(cx, opts.y_center() + opts.em * 0.25));
        }
    }

    TimeSignatureLayout {
        beats: time_sig_top,
        beat_unit: time_sig_bottom,
        width: ts_cols * ts_digit_w,
    }
}

pub(super) fn requires_flag(d: Duration) -> bool {
    matches!(d.base_note(), NoteValue::Eighth | NoteValue::Sixteenth | NoteValue::ThirtySecond)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{FontFamily, FontId, Pos2, Rect, Vec2};
    use grooph_measure::{Beat, Measure, TimeSignature};

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
            proportional_spacing: true,
            debug_bbox: true,
            metrics: GlyphMetrics::debug(em),
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
    fn test_layout_uses_metrics_if_provided() {
        use grooph_measure::duration::q;
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        m.set_beat(0, Beat::note(q())).unwrap();

        let em = 20.0;
        let mut opts = LayoutOpts {
            rect: Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(100.0, 100.0)),
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset: 0.0,
            stem_length_factor: 1.0,
            stem_thickness_factor: 0.1,
            accent_displacement: 0.0,
            accent_below: false,
            proportional_spacing: true,
            debug_bbox: true,
            metrics: GlyphMetrics::debug(em),
        };

        // 1. Standard Metrics (Debug defaults)
        let layout_std = build_measure_layout(&m, &opts);
        let bbox_std = layout_std.notes[0].debug_bbox.unwrap();
        let width_std = bbox_std.width();

        // 2. Mit Metrics (Large Head)
        opts.metrics = GlyphMetrics {
            head_size: Vec2::new(5.0 * em, 1.0 * em), // Huge head
            ..GlyphMetrics::debug(em)
        };

        let layout_metrics = build_measure_layout(&m, &opts);
        let bbox_metrics = layout_metrics.notes[0].debug_bbox.unwrap();
        let width_metrics = bbox_metrics.width();

        assert!(
            width_metrics > width_std * 2.0,
            "Metrics-based width should be significantly larger"
        );
        // Expect width roughly 5.0 * em
        assert!((width_metrics - 5.0 * em).abs() < 1.0);
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
            proportional_spacing: true,
            debug_bbox: true,
            metrics: GlyphMetrics::debug(em),
        };

        // Case 1: Accent Above (default)
        let layout = build_measure_layout(&m, &opts);
        let note = &layout.notes[0];
        let accent_pos = note.accent_pos.expect("Accent should be present");
        let stem = note.stem.expect("Stem should be present");

        let stem_top = stem.p2.y.min(stem.p1.y);
        let expected_y = stem_top - (displacement * em);

        assert!(
            (accent_pos.y - expected_y).abs() < 0.001,
            "Accent (y={}) should be exactly at expected (y={})",
            accent_pos.y,
            expected_y
        );

        // Case 2: Accent Below
        let mut opts_below = opts.clone();
        opts_below.accent_below = true;
        let layout_below = build_measure_layout(&m, &opts_below);
        let note_below = &layout_below.notes[0];
        let accent_pos_below = note_below.accent_pos.expect("Accent should be present");

        let head_bottom = opts.y_center() + opts.metrics.head_size.y * 0.5;

        assert!(accent_pos_below.y > head_bottom, "Accent should be below note head");
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
            proportional_spacing: true,
            debug_bbox: true,
            metrics: GlyphMetrics::debug(em),
        };

        let layout = build_measure_layout(&m, &opts);
        let note = &layout.notes[0];

        assert!(note.debug_bbox.is_some(), "Main debug bbox should be present");
        assert!(note.accent_debug_bbox.is_some(), "Accent debug bbox should be present");

        let main_bbox = note.debug_bbox.unwrap();
        let accent_bbox = note.accent_debug_bbox.unwrap();

        let expected_size = opts.metrics.accent_size;
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
            proportional_spacing: true,
            debug_bbox: true,
            metrics: GlyphMetrics::debug(em),
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
            proportional_spacing: true,
            debug_bbox: true,
            metrics: GlyphMetrics::debug(em),
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
}
