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
    pub const fn denominator(self) -> i32 {
        match self {
            NoteValue::Whole => 1,
            NoteValue::Half => 2,
            NoteValue::Quarter => 4,
            NoteValue::Eighth => 8,
            NoteValue::Sixteenth => 16,
            NoteValue::ThirtySecond => 32,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Duration {
    Simple(NoteValue),
    // Dotted notes not currently used, but leave room for easy extension later.
    Dotted { base: NoteValue, dots: u8 },
    // Tuplet note: n in the time of m of the base note
    Tuplet { n: u8, m: u8, base: NoteValue },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Frac {
    num: i32,
    den: i32,
}

const fn gcd(mut a: i32, mut b: i32) -> i32 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    if a < 0 { -a } else { a }
}

const fn lcm(a: i32, b: i32) -> i32 { (a / gcd(a, b)) * b }

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
                let k = dots as i32;
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
                reduce(Frac { num: m as i32, den: (n as i32) * base_den })
            }
        }
    }

    /// Public helper for weight/grids: denominator of the reduced fraction relative to whole note.
    pub const fn denominator(&self) -> i32 { self.as_fraction().den }

    /// Convenience to get a base for glyph decisions (flags/rest shapes). Tuplets/dotted return their base.
    pub const fn base_note(&self) -> NoteValue {
        match *self {
            Duration::Simple(base) => base,
            Duration::Dotted { base, .. } => base,
            Duration::Tuplet { base, .. } => base,
        }
    }

    /// Mainly for debug/Display output
    pub(crate) const fn to_glyph(&self) -> (&'static str, &'static str) {
        // Glyphs are determined by base note value only; tuplets/rests share the same shapes
        match self.base_note() {
            NoteValue::Quarter => ("𝅘𝅥", "𝄽"),
            NoteValue::Eighth => ("𝅘𝅥𝅮", "𝄾"),
            NoteValue::Sixteenth => ("𝅘𝅥𝅯", "𝄿"),
            NoteValue::ThirtySecond => ("𝅘𝅥𝅰", "𝅀"),
            NoteValue::Half => ("𝅗𝅥", "𝄼"),
            NoteValue::Whole => ("𝅝", "𝄻"),
        }
    }
}

/// A tick grid provider. Build dynamically from the set of supported durations.
#[derive(Clone, Copy, Debug)]
pub struct Grid {
    pub ticks_per_whole: i32,
}

impl Grid {
    /// Build a dynamic grid as the LCM of the denominators of the given durations.
    pub const fn from_durations(durs: &[Duration]) -> Grid {
        let mut l = 1i32;
        let mut i = 0usize;
        while i < durs.len() {
            let f = durs[i].as_fraction();
            l = lcm(l, f.den);
            i += 1;
        }
        Grid { ticks_per_whole: l }
    }

    pub fn ticks_from_fraction(&self, num: i32, den: i32) -> Option<i32> {
        if den == 0 {
            return None;
        }
        if self.ticks_per_whole % den != 0 {
            return None;
        }
        Some((self.ticks_per_whole / den) * num)
    }

    pub fn ticks_of(&self, d: &Duration) -> Option<i32> {
        let f = d.as_fraction();
        self.ticks_from_fraction(f.num, f.den)
    }
}

pub const COMMON_DURATIONS: [Duration; 13] = [
    Duration::Simple(NoteValue::Quarter),
    Duration::Simple(NoteValue::Eighth),
    Duration::Simple(NoteValue::Sixteenth),
    Duration::Simple(NoteValue::ThirtySecond),
    Duration::Dotted { base: NoteValue::Quarter, dots: 1 }, // dotted quarter
    Duration::Dotted { base: NoteValue::Eighth, dots: 1 }, // dotted eighth
    Duration::Dotted { base: NoteValue::Sixteenth, dots: 1 }, // dotted 16th
    Duration::Dotted { base: NoteValue::ThirtySecond, dots: 1 }, // dotted 32nd
    Duration::Tuplet { n: 3, m: 2, base: NoteValue::Eighth }, // triplet eighth
    Duration::Tuplet { n: 5, m: 4, base: NoteValue::Sixteenth }, // quintuplet 16th
    Duration::Tuplet { n: 6, m: 4, base: NoteValue::Sixteenth }, // sextuplet 16th
    Duration::Tuplet { n: 7, m: 4, base: NoteValue::Sixteenth }, // septuplet 16th
    Duration::Tuplet { n: 9, m: 8, base: NoteValue::ThirtySecond }, // nonuplet 32nd
];

#[derive(Clone, Copy, Debug)]
pub struct DurationSet {
    pub durations: &'static [Duration],
    pub grid: Grid,
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
