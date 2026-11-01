use eframe::egui;
use eframe::egui::{Color32, Pos2, Stroke};
use grooph_layout::pixel_layout::{LayoutOpts, NoteLayout};
use grooph_measure::duration::NoteValue;

use crate::glyph::draw_glyph;

pub(super) fn draw_beat(
    painter: &egui::Painter,
    note: &NoteLayout,
    opts: &LayoutOpts,
    color: Color32,
) {
    if let Some(stem) = drawn_stem(note, opts) {
        painter.line_segment(stem, Stroke::new(opts.stem_thickness(), color));
    }
    let stroke = Stroke::new(opts.staff_line_thickness(), color);
    for ledger in &note.ledgers {
        painter.line_segment([ledger.p1, ledger.p2], stroke);
    }
    for glyph in note.glyphs() {
        draw_glyph(painter, glyph, &opts.font_id.family, color);
    }
}

/// How far a stem stops short of its tip under a flag, in stem thicknesses.
const FLAG_STEM_INSET: f32 = 1.5;

/// The stem as drawn, from the head to the tip. Where a flag or a beam
/// covers the tip, the stem stops short of it, inside the flag or beam:
/// egui rounds a line's ends to pixels and adds a pixel of anti-aliasing
/// past them, while beams and glyphs round on their own, so a stem reaching
/// the tip showed above the beam or flag at some zooms and positions. Under
/// a flag also because the flag's top is drawn for the font's thin stem and
/// slopes away from it, so a thicker stem shows its corner above it.
fn drawn_stem(note: &NoteLayout, opts: &LayoutOpts) -> Option<[Pos2; 2]> {
    let stem = note.stem?;
    let inset = if note.flag.is_some() {
        FLAG_STEM_INSET * opts.stem_thickness()
    } else if beamed(note) {
        // Into the middle of the outermost beam.
        0.5 * opts.beam_thickness()
    } else {
        0.0
    };
    let mut tip = stem.p2;
    tip.y -= (stem.p2.y - stem.p1.y).signum() * inset;
    Some([stem.p1, tip])
}

/// Whether the note's stem ends in a beam: a stemlet, or a note shorter
/// than a quarter without a flag.
fn beamed(note: &NoteLayout) -> bool {
    let short = matches!(
        note.duration.base_note(),
        NoteValue::Eighth | NoteValue::Sixteenth | NoteValue::ThirtySecond
    );
    note.stemlet || (short && note.flag.is_none())
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{FontFamily, FontId, Rect};
    use grooph_layout::pixel_layout::build_measure_layout;
    use grooph_measure::duration::{e, q};
    use grooph_measure::{Beat, Measure, TimeSignature};

    fn opts(stem_down: bool) -> LayoutOpts {
        let em = 40.0;
        LayoutOpts {
            rect: Rect::from_min_size(Pos2::ZERO, egui::vec2(200.0, 200.0)),
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset: 0.0,
            stem_length_factor: 0.8,
            stem_thickness_factor: 0.06,
            accent_displacement: 0.0,
            stem_down,
            shared_line: false,
            staff: grooph_measure::StaffKind::Line,
        }
    }

    #[test]
    fn a_stem_stops_short_of_its_flag_and_reaches_no_further() {
        for stem_down in [false, true] {
            let opts = opts(stem_down);
            let mut measure = Measure::new(TimeSignature::new(3, 8));
            measure.first_voice_mut().set_beat(0, Beat::note(q())).unwrap();
            measure.first_voice_mut().set_beat(1, Beat::note(e())).unwrap();
            let layout = build_measure_layout(measure.first_voice(), &opts);
            let [quarter, eighth] = &layout.notes[..] else { panic!("{layout:?}") };
            // Without a flag the stem is drawn as laid out.
            let stem = quarter.stem.unwrap();
            assert_eq!(drawn_stem(quarter, &opts), Some([stem.p1, stem.p2]));
            // Under the flag it ends inside it, on the head's side of the tip.
            let stem = eighth.stem.unwrap();
            let [p1, p2] = drawn_stem(eighth, &opts).unwrap();
            assert_eq!(p1, stem.p1);
            let short = (stem.p2.y - p2.y) * (stem.p2.y - stem.p1.y).signum();
            assert!(short > opts.stem_thickness(), "stem_down {stem_down}: {short}");
            let flag = eighth.flag.unwrap().ink();
            assert!(flag.y_range().contains(p2.y), "stem_down {stem_down}: {p2:?} {flag:?}");
        }
    }

    #[test]
    fn a_beamed_stem_ends_in_the_middle_of_its_beam() {
        for stem_down in [false, true] {
            let opts = opts(stem_down);
            let mut measure = Measure::new(TimeSignature::new(2, 8));
            measure.first_voice_mut().set_beat(0, Beat::note(e())).unwrap();
            measure.first_voice_mut().set_beat(1, Beat::note(e())).unwrap();
            let layout = build_measure_layout(measure.first_voice(), &opts);
            let beam = layout.beams[0].rect;
            for note in &layout.notes {
                assert!(note.flag.is_none());
                let stem = note.stem.unwrap();
                let [p1, p2] = drawn_stem(note, &opts).unwrap();
                assert_eq!(p1, stem.p1);
                let mid = beam.center().y;
                assert!(
                    (p2.y - mid).abs() < 0.1 * beam.height(),
                    "stem_down {stem_down}: {p2:?} {beam:?}"
                );
            }
        }
    }
}
