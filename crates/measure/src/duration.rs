use crate::math::{Frac, reduce};
use NoteValue::{Eighth, Sixteenth, ThirtySecond};
use NoteValue::{Half, Quarter, Whole};
use serde::{Deserialize, Serialize};
use std::fmt::{Debug, Formatter, Pointer};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteValue {
    Whole,
    Half,
    Quarter,
    Eighth,
    Sixteenth,
    ThirtySecond,
}

impl NoteValue {
    pub const fn denominator(self) -> u8 {
        match self {
            Whole => 1,
            Half => 2,
            Quarter => 4,
            Eighth => 8,
            Sixteenth => 16,
            ThirtySecond => 32,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Whole => "Whole",
            Half => "Half",
            Quarter => "Quarter",
            Eighth => "Eighth",
            Sixteenth => "Sixteenth",
            ThirtySecond => "Thirty-second",
        }
    }

    pub const fn fraction(self) -> &'static str {
        match self {
            Whole => "1/1",
            Half => "1/2",
            Quarter => "1/4",
            Eighth => "1/8",
            Sixteenth => "1/16",
            ThirtySecond => "1/32",
        }
    }

    /// Stable index into a 6-slot per-note-value array
    /// (e.g. `GlyphMetrics::rest_sizes`). Order matches `denominator()` powers.
    pub const fn rest_index(self) -> usize {
        match self {
            Whole => 0,
            Half => 1,
            Quarter => 2,
            Eighth => 3,
            Sixteenth => 4,
            ThirtySecond => 5,
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Duration {
    Simple(NoteValue),
    Dotted { base: NoteValue, dots: u8 },
    // Tuplet note: n in the time of m of the base note
    Tuplet(TupletSpec),
}

#[derive(Debug, PartialEq, Clone, Copy, Eq, Serialize, Deserialize)]
pub struct TupletSpec {
    pub n: u8,
    pub m: u8,
    pub base: NoteValue,
}

impl Duration {
    pub(super) const fn as_fraction(&self) -> Frac {
        match *self {
            Duration::Simple(base) => Frac { num: 1, den: base.denominator() as u32 },
            Duration::Dotted { base, dots } => {
                let base_den = base.denominator();
                let k = dots as u32;
                if k == 0 {
                    return Frac { num: 1, den: base_den as u32 };
                }
                // Dotted: sum of geometric series -> (2^{k+1}-1)/2^k of the base note
                let two_pow_k = 1 << k; // 2^k
                let num = (two_pow_k << 1) - 1; // 2^{k+1} - 1
                let den = two_pow_k; // 2^k
                reduce(Frac { num, den: den * base_den as u32 })
            }
            Duration::Tuplet(TupletSpec { n, m, base }) => {
                let base_den = base.denominator();
                reduce(Frac { num: m as u32, den: (n as u32) * base_den as u32 })
            }
        }
    }

    /// Public helper for weight/grids: denominator of the reduced fraction relative to whole note.
    pub const fn denominator(&self) -> u32 { self.as_fraction().den }

    /// Convenience to get a base for glyph decisions (flags/rest shapes). Tuplets/dotted return their base.
    pub const fn base_note(&self) -> NoteValue {
        match *self {
            Duration::Simple(base) => base,
            Duration::Dotted { base, .. } => base,
            Duration::Tuplet(TupletSpec { base, .. }) => base,
        }
    }

    /// Halve a simple duration (Whole→Half→Quarter→Eighth→Sixteenth→ThirtySecond).
    /// Returns None for ThirtySecond or non-simple durations (dotted/tuplet).
    pub fn halve_simple(self) -> Option<Duration> {
        use NoteValue::*;
        match self {
            Duration::Simple(Quarter) => Some(Duration::Simple(Eighth)),
            Duration::Simple(Eighth) => Some(Duration::Simple(Sixteenth)),
            Duration::Simple(Sixteenth) => Some(Duration::Simple(ThirtySecond)),
            Duration::Simple(ThirtySecond) => None,
            _ => None,
        }
    }

    /// Double a simple duration (ThirtySecond→Sixteenth→Eighth→Quarter→Half→Whole).
    /// Returns None for Whole or non-simple durations (dotted/tuplet).
    pub fn double_simple(self) -> Option<Duration> {
        use NoteValue::*;
        match self {
            Duration::Simple(ThirtySecond) => Some(Duration::Simple(Sixteenth)),
            Duration::Simple(Sixteenth) => Some(Duration::Simple(Eighth)),
            Duration::Simple(Eighth) => Some(Duration::Simple(Quarter)),
            _ => None,
        }
    }
}

impl Debug for Duration {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(duration_to_debug_str(self).as_str())
    }
}

pub(crate) fn duration_to_debug_str(duration: &Duration) -> String {
    let duration_fr = duration.base_note().fraction();
    match duration {
        Duration::Simple(_) => duration_fr.to_string(),
        Duration::Dotted { base: _base, dots } => {
            format!("{}{}", duration_fr, ".".repeat(*dots as usize))
        }
        Duration::Tuplet(TupletSpec { n, m, .. }) => format!("{}[{}:{}]", duration_fr, n, m),
    }
}

/// Human-readable description of a duration (without note/rest kind)
/// Examples:
/// - Simple(Eighth) -> "Eighth"
/// - Dotted { base: Quarter, dots: 1 } -> "Dotted Quarter"
/// - Tuplet { n: 3, m: 2, base: Eighth } -> "Triplet eighth"
/// - Tuplet { n: 7, m: 4, base: Sixteenth } -> "Septuplet sixteenth"
/// - Unknown tuplet n -> "Tuplet n:m <base-lower>"
pub fn human_readable(d: &Duration) -> String {
    match *d {
        Duration::Simple(nv) => nv.name().to_string(),
        Duration::Dotted { base, dots } => {
            let prefix = match dots {
                1 => "Dotted",
                2 => "Double-dotted",
                3 => "Triple-dotted",
                _ => "Dotted",
            };
            format!("{} {}", prefix, base.name())
        }
        Duration::Tuplet(TupletSpec { n, m, base }) => {
            let base_lower = match base {
                Whole => "whole",
                Half => "half",
                Quarter => "quarter",
                Eighth => "eighth",
                Sixteenth => "sixteenth",
                ThirtySecond => "thirty-second",
            };
            let name = match n {
                3 => Some("Triplet"),
                5 => Some("Quintuplet"),
                6 => Some("Sextuplet"),
                7 => Some("Septuplet"),
                9 => Some("Nonuplet"),
                _ => None,
            };
            if let Some(label) = name {
                format!("{} {}", label, base_lower)
            } else {
                format!("Tuplet {}:{} {}", n, m, base_lower)
            }
        }
    }
}

pub const COMMON_DURATIONS: [Duration; 16] = [
    q(),
    e(),
    s(),
    th(),
    Duration::Dotted { base: Quarter, dots: 1 }, // dotted 1/4
    Duration::Dotted { base: Eighth, dots: 1 },  // dotted 1/8
    Duration::Dotted { base: Sixteenth, dots: 1 }, // dotted 1/16
    Duration::Tuplet(TupletSpec { n: 3, m: 2, base: Quarter }),
    t8(),    // triplet 1/8
    t16(),   // triplet 1/16
    t32(),   // triplet 1/32
    qt16(),  // quintuplet 1/16
    st8(),   // sextuplet 1/8
    st16(),  // sextuplet 1/16
    spt16(), // septuplet 1/16
    nt16(),  // nonuplet 1/16
];

pub const fn q() -> Duration { Duration::Simple(Quarter) }
pub const fn e() -> Duration { Duration::Simple(Eighth) }
pub const fn s() -> Duration { Duration::Simple(Sixteenth) }
pub const fn th() -> Duration { Duration::Simple(ThirtySecond) }
pub const fn t8() -> Duration { Duration::Tuplet(TupletSpec { n: 3, m: 2, base: Eighth }) }
pub const fn t16() -> Duration { Duration::Tuplet(TupletSpec { n: 3, m: 2, base: Sixteenth }) }
pub const fn t32() -> Duration { Duration::Tuplet(TupletSpec { n: 3, m: 2, base: ThirtySecond }) }
pub const fn qt16() -> Duration { Duration::Tuplet(TupletSpec { n: 5, m: 4, base: Sixteenth }) }
pub const fn st8() -> Duration { Duration::Tuplet(TupletSpec { n: 6, m: 4, base: Eighth }) }
pub const fn st16() -> Duration { Duration::Tuplet(TupletSpec { n: 6, m: 4, base: Sixteenth }) }
pub const fn spt16() -> Duration { Duration::Tuplet(TupletSpec { n: 7, m: 4, base: Sixteenth }) }
pub const fn nt16() -> Duration { Duration::Tuplet(TupletSpec { n: 9, m: 8, base: Sixteenth }) }

#[cfg(test)]
mod tests {
    use super::Duration;
    use crate::duration::NoteValue::Eighth;
    use crate::grid::DEFAULT_GRID;

    #[test]
    fn roundtrip_ticks_presence() {
        let grid = DEFAULT_GRID;
        for d in grid.durations.iter() {
            assert!(grid.ticks_of(d).is_some());
        }
    }

    #[test]
    fn dotted_eighth_ticks() {
        let e_ticks = DEFAULT_GRID.ticks_of(&Duration::Simple(Eighth));
        let e_dotted_ticks = DEFAULT_GRID.ticks_of(&Duration::Dotted { base: Eighth, dots: 1 });
        assert_ne!(e_ticks, e_dotted_ticks);
    }
}
