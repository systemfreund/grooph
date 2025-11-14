use crate::measure::{Beat, Measure, TimeSignature};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NoteValue {
    Whole,
    Half,
    Quarter,
    Eighth,
    Sixteenth,
    ThirtySecond,
}

impl NoteValue {
    pub const fn denominator(self) -> u32 {
        match self {
            NoteValue::Whole => 1,
            NoteValue::Half => 2,
            NoteValue::Quarter => 4,
            NoteValue::Eighth => 8,
            NoteValue::Sixteenth => 16,
            NoteValue::ThirtySecond => 32,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            NoteValue::Whole => "Whole",
            NoteValue::Half => "Half",
            NoteValue::Quarter => "Quarter",
            NoteValue::Eighth => "Eighth",
            NoteValue::Sixteenth => "Sixteenth",
            NoteValue::ThirtySecond => "Thirty-second",
        }
    }

    pub const fn fraction(self) -> &'static str {
        match self {
            NoteValue::Whole => "1/1",
            NoteValue::Half => "1/2",
            NoteValue::Quarter => "1/4",
            NoteValue::Eighth => "1/8",
            NoteValue::Sixteenth => "1/16",
            NoteValue::ThirtySecond => "1/32",
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Duration {
    Simple(NoteValue),
    Dotted { base: NoteValue, dots: u8 },
    // Tuplet note: n in the time of m of the base note
    Tuplet { n: u8, m: u8, base: NoteValue },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Frac {
    num: u32,
    den: u32,
}

const fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

const fn lcm(a: u32, b: u32) -> u32 { (a / gcd(a, b)) * b }

const fn reduce(f: Frac) -> Frac {
    let g = gcd(f.num, f.den);
    Frac { num: f.num / g, den: f.den / g }
}

impl Duration {
    const fn as_fraction(&self) -> Frac {
        match *self {
            Duration::Simple(base) => Frac { num: 1, den: base.denominator() },
            Duration::Dotted { base, dots } => {
                let base_den = base.denominator();
                let k = dots as u32;
                if k == 0 {
                    return Frac { num: 1, den: base_den };
                }
                // Dotted: sum of geometric series -> (2^{k+1}-1)/2^k of the base note
                let two_pow_k = 1 << k; // 2^k
                let num = (two_pow_k << 1) - 1; // 2^{k+1} - 1
                let den = two_pow_k; // 2^k
                reduce(Frac { num, den: den * base_den })
            }
            Duration::Tuplet { n, m, base } => {
                let base_den = base.denominator();
                reduce(Frac { num: m as u32, den: (n as u32) * base_den })
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
            Duration::Tuplet { base, .. } => base,
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
        Duration::Tuplet { n, m, base } => {
            let base_lower = match base {
                NoteValue::Whole => "whole",
                NoteValue::Half => "half",
                NoteValue::Quarter => "quarter",
                NoteValue::Eighth => "eighth",
                NoteValue::Sixteenth => "sixteenth",
                NoteValue::ThirtySecond => "thirty-second",
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

/// A tick grid provider. Build dynamically from the set of supported durations.
#[derive(Clone, Copy, Debug)]
pub struct Grid {
    pub ticks_per_whole: u32,
}

impl Grid {
    /// Build a dynamic grid as the LCM of the denominators of the given durations.
    pub const fn from_durations(durs: &[Duration]) -> Grid {
        let mut l = 1u32;
        let mut i = 0usize;
        while i < durs.len() {
            let f = durs[i].as_fraction();
            l = lcm(l, f.den);
            i += 1;
        }
        Grid { ticks_per_whole: l }
    }

    pub fn ticks_from_fraction(&self, num: u32, den: u32) -> Option<u32> {
        if den == 0 {
            return None;
        }
        if self.ticks_per_whole % den != 0 {
            return None;
        }
        Some((self.ticks_per_whole / den) * num)
    }

    pub fn ticks_of(&self, d: &Duration) -> Option<u32> {
        let f = d.as_fraction();
        self.ticks_from_fraction(f.num, f.den)
    }

    pub fn ticks_per_beat(&self, time_signature: &TimeSignature) -> u32 {
        self.ticks_per_whole / (time_signature.beat_unit as u32)
    }

    /// Returns a measure's total duration in integer ticks
    pub fn ticks_per_measure(&self, time_signature: &TimeSignature) -> u32 {
        (time_signature.beats as u32) * self.ticks_per_beat(time_signature)
    }
}

pub const COMMON_DURATIONS: [Duration; 15] = [
    Duration::Simple(NoteValue::Quarter),
    Duration::Simple(NoteValue::Eighth),
    Duration::Simple(NoteValue::Sixteenth),
    Duration::Simple(NoteValue::ThirtySecond),
    Duration::Dotted { base: NoteValue::Quarter, dots: 1 }, // dotted 1/4
    Duration::Dotted { base: NoteValue::Eighth, dots: 1 }, // dotted 1/8
    Duration::Dotted { base: NoteValue::Sixteenth, dots: 1 }, // dotted 1/16
    Duration::Dotted { base: NoteValue::ThirtySecond, dots: 1 }, // dotted 1/32
    Duration::Tuplet { n: 3, m: 2, base: NoteValue::Eighth }, // triplet 1/8
    Duration::Tuplet { n: 3, m: 2, base: NoteValue::Sixteenth }, // triplet 1/16
    Duration::Tuplet { n: 3, m: 2, base: NoteValue::ThirtySecond }, // triplet 1/32
    Duration::Tuplet { n: 5, m: 4, base: NoteValue::Sixteenth }, // quintuplet 1/16
    Duration::Tuplet { n: 6, m: 4, base: NoteValue::Sixteenth }, // sextuplet 1/16
    Duration::Tuplet { n: 7, m: 4, base: NoteValue::Sixteenth }, // septuplet 1/16
    Duration::Tuplet { n: 9, m: 8, base: NoteValue::ThirtySecond }, // nonuplet 1/32
];

#[derive(Clone, Copy, Debug)]
pub struct DurationSet {
    pub durations: &'static [Duration],
    pub grid: Grid,
}

impl DurationSet {
    pub fn compute_onset_ticks(&self, beats: &Vec<Beat>) -> Vec<u32> {
        let mut onsets: Vec<u32> = Vec::with_capacity(beats.len());
        let mut t = 0;
        for b in beats.iter() {
            onsets.push(t);
            if let Some(dt) = self.grid.ticks_of(&b.duration) {
                t += dt;
            }
        }
        onsets
    }
}

pub const fn default_duration_set() -> DurationSet {
    let durs: &'static [Duration] = &COMMON_DURATIONS;
    let grid = Grid::from_durations(durs);
    DurationSet { durations: durs, grid }
}

pub const fn default_grid() -> Grid { default_duration_set().grid }

#[cfg(test)]
mod tests {
    use crate::duration::NoteValue::{Eighth, Quarter};
    use super::{default_duration_set, default_grid, Duration};

    #[test]
    fn roundtrip_ticks_presence() {
        let set = default_duration_set();
        for d in set.durations.iter() {
            assert!(set.grid.ticks_of(d).is_some());
        }
    }

    #[test]
    fn dotted_eighth_ticks() {
        let e_ticks = default_grid().ticks_of(&Duration::Simple(Eighth));
        let e_dotted_ticks = default_grid().ticks_of(&Duration::Dotted { base: Eighth, dots: 1 });
        assert_ne!(e_ticks, e_dotted_ticks);
    }
}
