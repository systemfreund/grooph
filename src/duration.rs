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
                let two_pow_k = 1 << k; // 2^k
                let num = two_pow_k - 1; // 2^k - 1
                let den = two_pow_k >> 1; // 2^{k-1}
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

    /// Inverse of `ticks_of`: try to reconstruct a Duration that exactly matches the given ticks.
    /// Preference order:
    /// 1) Known common durations (including tuplets) from COMMON_DURATIONS for exact structural match
    /// 2) Simple note values (whole, half, quarter, eighth, sixteenth, thirty-second)
    /// 3) Dotted forms of those simple bases with up to 3 dots
    pub fn duration_from_ticks(&self, ticks: i32) -> Option<Duration> {
        if ticks <= 0 {
            return None;
        }
        let f = reduce(Frac { num: ticks, den: self.ticks_per_whole });

        // 1) Try exact match against known common durations to preserve tuplets as authored
        for d in COMMON_DURATIONS.iter() {
            println!("f={:?} <--> d={:?}", f, d.as_fraction());
            if d.as_fraction() == f {
                return Some(*d);
            }
        }

        // 2) Try simple notes
        const SIMPLE: [NoteValue; 6] = [
            NoteValue::Whole,
            NoteValue::Half,
            NoteValue::Quarter,
            NoteValue::Eighth,
            NoteValue::Sixteenth,
            NoteValue::ThirtySecond,
        ];
        for base in SIMPLE.iter() {
            let d = Duration::Simple(*base);
            if d.as_fraction() == f {
                return Some(d);
            }
        }

        // 3) Try dotted forms (up to three dots which covers most practical usage)
        let mut dots = 1u8;
        while dots <= 3 {
            for base in SIMPLE.iter() {
                let d = Duration::Dotted { base: *base, dots };
                if d.as_fraction() == f {
                    return Some(d);
                }
            }
            dots += 1;
        }

        None
    }
}

pub const COMMON_DURATIONS: [Duration; 9] = [
    Duration::Simple(NoteValue::Quarter),
    Duration::Simple(NoteValue::Eighth),
    Duration::Simple(NoteValue::Sixteenth),
    Duration::Simple(NoteValue::ThirtySecond),
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
    use super::default_duration_set;

    #[test]
    fn roundtrip_ticks_presence() {
        let set = default_duration_set();
        for d in set.durations.iter() {
            assert!(set.grid.ticks_of(d).is_some());
        }
    }
}
