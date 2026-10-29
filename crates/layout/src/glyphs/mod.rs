//! The SMuFL glyphs of Bravura the notation is drawn with, and their metrics.

mod bravura;

pub use bravura::*;
use egui::{Rect, pos2};
use grooph_measure::duration::Duration;
use grooph_measure::duration::NoteValue::{Eighth, Half, Quarter, Sixteenth, ThirtySecond, Whole};
use grooph_measure::head::Notehead;
use grooph_measure::{Instrument, Technique};

/// A SMuFL glyph with its metrics in staff spaces (a quarter of the font
/// size), relative to its origin on the baseline, y up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    pub ch: char,
    pub advance: f32,
    /// South-west (bottom left) corner of the ink's bounding box.
    pub sw: [f32; 2],
    /// North-east (top right) corner of the ink's bounding box.
    pub ne: [f32; 2],
}

impl Glyph {
    /// The ink's bounding box in screen coordinates (y down) relative to the
    /// origin, for staff space `ss` px.
    pub fn ink(&self, ss: f32) -> Rect {
        Rect::from_min_max(
            pos2(self.sw[0] * ss, -self.ne[1] * ss),
            pos2(self.ne[0] * ss, -self.sw[1] * ss),
        )
    }
}

pub fn rest_glyph_for_duration(d: Duration) -> Glyph {
    match d.base_note() {
        Whole => REST_WHOLE,
        Half => REST_HALF,
        Quarter => REST_QUARTER,
        Eighth => REST_8TH,
        Sixteenth => REST_16TH,
        ThirtySecond => REST_32ND,
    }
}

/// Hollow for halves (and wholes), filled for the rest. Both heads have the
/// same size and stem anchors.
pub fn notehead_glyph_for_duration(d: Duration) -> Glyph {
    match d.base_note() {
        Half | Whole => NOTEHEAD_HALF,
        _ => NOTEHEAD_BLACK,
    }
}

/// A note head and the points its stems attach to, relative to its origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeadGlyph {
    pub glyph: Glyph,
    pub stem_up_se: [f32; 2],
    pub stem_down_nw: [f32; 2],
}

impl HeadGlyph {
    const fn of(glyph: Glyph, stem_up_se: [f32; 2], stem_down_nw: [f32; 2]) -> Self {
        HeadGlyph { glyph, stem_up_se, stem_down_nw }
    }
}

/// The head of `instrument` played with `technique` in a note of duration
/// `d`: the usual oval for drums and the one line, an x for cymbals; a
/// cross-stick is a circled x, a rimshot a slashed head, the ride's bell a
/// diamond (Weinberg).
pub fn head_glyph_for(instrument: Instrument, technique: Technique, d: Duration) -> HeadGlyph {
    let hollow = matches!(d.base_note(), Half | Whole);
    match (technique, instrument.notehead(), hollow) {
        (Technique::CrossStick, ..) => HeadGlyph::of(
            NOTEHEAD_CIRCLE_X,
            NOTEHEAD_CIRCLE_X_STEM_UP_SE,
            NOTEHEAD_CIRCLE_X_STEM_DOWN_NW,
        ),
        (Technique::Rimshot, ..) => HeadGlyph::of(
            NOTEHEAD_SLASHED_BLACK,
            NOTEHEAD_SLASHED_BLACK_STEM_UP_SE,
            NOTEHEAD_SLASHED_BLACK_STEM_DOWN_NW,
        ),
        (Technique::Bell, ..) => HeadGlyph::of(
            NOTEHEAD_DIAMOND_BLACK,
            NOTEHEAD_DIAMOND_BLACK_STEM_UP_SE,
            NOTEHEAD_DIAMOND_BLACK_STEM_DOWN_NW,
        ),
        (_, Notehead::Cross, false) => HeadGlyph::of(
            NOTEHEAD_X_BLACK,
            NOTEHEAD_X_BLACK_STEM_UP_SE,
            NOTEHEAD_X_BLACK_STEM_DOWN_NW,
        ),
        (_, Notehead::Cross, true) => {
            HeadGlyph::of(NOTEHEAD_X_HALF, NOTEHEAD_X_HALF_STEM_UP_SE, NOTEHEAD_X_HALF_STEM_DOWN_NW)
        }
        (_, Notehead::Normal, false) => {
            HeadGlyph::of(NOTEHEAD_BLACK, NOTEHEAD_BLACK_STEM_UP_SE, NOTEHEAD_BLACK_STEM_DOWN_NW)
        }
        (_, Notehead::Normal, true) => {
            HeadGlyph::of(NOTEHEAD_HALF, NOTEHEAD_HALF_STEM_UP_SE, NOTEHEAD_HALF_STEM_DOWN_NW)
        }
    }
}

/// The sign written by a note for `technique`, if it has one: o for an open
/// hi-hat, + for a closed one, the half-open sign between, and the choke's
/// mark.
pub fn technique_sign(technique: Technique) -> Option<Glyph> {
    match technique {
        Technique::Open => Some(SIGN_OPEN),
        Technique::HalfOpen => Some(SIGN_HALF_OPEN),
        Technique::Closed => Some(SIGN_CLOSED),
        Technique::Choke => Some(SIGN_CHOKE),
        Technique::Normal | Technique::CrossStick | Technique::Rimshot | Technique::Bell => None,
    }
}

/// The flag of a note of duration `d`, if it has one, and the point where it
/// meets the end of the stem, relative to the flag's origin.
pub fn flag_glyph_for_duration(d: Duration, stem_down: bool) -> Option<(Glyph, [f32; 2])> {
    match (d.base_note(), stem_down) {
        (Eighth, false) => Some((FLAG_8TH_UP, FLAG_8TH_UP_STEM_UP_NW)),
        (Sixteenth, false) => Some((FLAG_16TH_UP, FLAG_16TH_UP_STEM_UP_NW)),
        (ThirtySecond, false) => Some((FLAG_32ND_UP, FLAG_32ND_UP_STEM_UP_NW)),
        (Eighth, true) => Some((FLAG_8TH_DOWN, FLAG_8TH_DOWN_STEM_DOWN_SW)),
        (Sixteenth, true) => Some((FLAG_16TH_DOWN, FLAG_16TH_DOWN_STEM_DOWN_SW)),
        (ThirtySecond, true) => Some((FLAG_32ND_DOWN, FLAG_32ND_DOWN_STEM_DOWN_SW)),
        (Quarter | Half | Whole, _) => None,
    }
}

const TIME_SIG_DIGITS: [Glyph; 10] = [
    TIME_SIG_0, TIME_SIG_1, TIME_SIG_2, TIME_SIG_3, TIME_SIG_4, TIME_SIG_5, TIME_SIG_6, TIME_SIG_7,
    TIME_SIG_8, TIME_SIG_9,
];

const TUPLET_DIGITS: [Glyph; 10] = [
    TUPLET_0, TUPLET_1, TUPLET_2, TUPLET_3, TUPLET_4, TUPLET_5, TUPLET_6, TUPLET_7, TUPLET_8,
    TUPLET_9,
];

fn digits(n: u8, set: &[Glyph; 10]) -> Vec<Glyph> {
    n.to_string().chars().filter_map(|c| c.to_digit(10).map(|d| set[d as usize])).collect()
}

/// The digits of `n` for a time signature.
pub fn ts_glyphs(n: u8) -> Vec<Glyph> {
    digits(n, &TIME_SIG_DIGITS)
}

/// The digits of `n` for a tuplet number.
pub fn tuplet_glyphs(n: u8) -> Vec<Glyph> {
    digits(n, &TUPLET_DIGITS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_heads_share_their_metrics() {
        // The layout sizes and attaches stems by the black head's metrics.
        let (black, half) = (NOTEHEAD_BLACK, NOTEHEAD_HALF);
        assert_eq!((black.advance, black.sw, black.ne), (half.advance, half.sw, half.ne));
        assert_eq!(NOTEHEAD_BLACK_STEM_UP_SE, NOTEHEAD_HALF_STEM_UP_SE);
        assert_eq!(NOTEHEAD_BLACK_STEM_DOWN_NW, NOTEHEAD_HALF_STEM_DOWN_NW);
    }
}
