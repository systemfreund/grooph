use crate::duration::Duration;
use crate::duration::NoteValue::{Eighth, Half, Quarter, Sixteenth, ThirtySecond, Whole};

// SMuFL glyphs (Bravura)
// Notehead black: U+E0A4
pub(super) const GLYPH_NOTEHEAD_BLACK: char = '\u{E0A4}';
// Augmentation dot: U+E1E7
pub(super) const GLYPH_AUGMENTATION_DOT: char = '\u{E1E7}';
// Rests: quarter..32nd: U+E4E5..U+E4E8
const GLYPH_REST_QUARTER: char = '\u{E4E5}';
const GLYPH_REST_EIGHTH: char = '\u{E4E6}';
const GLYPH_REST_SIXTEENTH: char = '\u{E4E7}';
const GLYPH_REST_32ND: char = '\u{E4E8}';

// Up-stem flags (SMuFL): U+E240..U+E244
const GLYPH_FLAG_8TH_UP: char = '\u{E240}';
const GLYPH_FLAG_16TH_UP: char = '\u{E242}';
const GLYPH_FLAG_32ND_UP: char = '\u{E244}';

// Clef and time signature digits
pub(super) const GLYPH_CLEF_PERCUSSION: char = '\u{E069}';
const TS_DIGITS: [char; 10] = [
    '\u{E080}', // 0
    '\u{E081}', // 1
    '\u{E082}', // 2
    '\u{E083}', // 3
    '\u{E084}', // 4
    '\u{E085}', // 5
    '\u{E086}', // 6
    '\u{E087}', // 7
    '\u{E088}', // 8
    '\u{E089}', // 9
];

pub(crate) const GLYPH_ACCENT_ABOVE: char = '\u{E4A0}';

pub(super) fn ts_glyphs(n: u32) -> Vec<char> {
    n.to_string().chars().filter_map(|c| c.to_digit(10).map(|d| TS_DIGITS[d as usize])).collect()
}

pub(super) fn rest_glyph_for_duration(d: Duration) -> char {
    match d.base_note() {
        Quarter => GLYPH_REST_QUARTER,
        Eighth => GLYPH_REST_EIGHTH,
        Sixteenth => GLYPH_REST_SIXTEENTH,
        ThirtySecond => GLYPH_REST_32ND,
        Half | Whole => GLYPH_REST_QUARTER,
    }
}

pub(super) fn flag_glyph_for_duration(d: Duration) -> Option<char> {
    match d.base_note() {
        Quarter => None,
        Eighth => Some(GLYPH_FLAG_8TH_UP),
        Sixteenth => Some(GLYPH_FLAG_16TH_UP),
        ThirtySecond => Some(GLYPH_FLAG_32ND_UP),
        Half | Whole => None,
    }
}

// Tuplet numeral digits (SMuFL): U+E880..U+E889
const TUPLET_DIGITS: [char; 10] = [
    '\u{E880}', // 0
    '\u{E881}', // 1
    '\u{E882}', // 2
    '\u{E883}', // 3
    '\u{E884}', // 4
    '\u{E885}', // 5
    '\u{E886}', // 6
    '\u{E887}', // 7
    '\u{E888}', // 8
    '\u{E889}', // 9
];

pub(super) fn tuplet_glyphs(n: u8) -> String {
    n.to_string()
        .chars()
        .filter_map(|c| c.to_digit(10).map(|d| TUPLET_DIGITS[d as usize]))
        .collect()
}
