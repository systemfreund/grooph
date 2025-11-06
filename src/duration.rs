pub type Ticks = i32;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Duration {
    Quarter,
    Eighth,
    TripletEighth,
    Sixteenth,
    QuintupletSixteenth,
    SextupletSixteenth,
    SeptupletSixteenth,
    ThirtySecond,
    NonupletThirtySecond,
}

// Compile-time utilities to compute GCD/LCM for integer constants
const fn gcd(mut a: Ticks, mut b: Ticks) -> Ticks {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    if a < 0 { -a } else { a }
}

const fn lcm(a: Ticks, b: Ticks) -> Ticks {
    (a / gcd(a, b)) * b
}

impl Duration {
    /// All supported durations.
    pub const DURATIONS: [Duration; 9] = [
        Duration::Quarter,
        Duration::Eighth,
        Duration::TripletEighth,
        Duration::Sixteenth,
        Duration::QuintupletSixteenth,
        Duration::SextupletSixteenth,
        Duration::SeptupletSixteenth,
        Duration::ThirtySecond,
        Duration::NonupletThirtySecond,
    ];

    /// Returns the denominator of this duration as a fraction of a whole note (e.g., Quarter -> 4).
    pub const fn denominator_of(d: Duration) -> i32 {
        match d {
            Duration::Quarter => 4,
            Duration::Eighth => 8,
            Duration::TripletEighth => 12,
            Duration::Sixteenth => 16,
            Duration::QuintupletSixteenth => 20,
            Duration::SextupletSixteenth => 24,
            Duration::SeptupletSixteenth => 28,
            Duration::ThirtySecond => 32,
            Duration::NonupletThirtySecond => 36,
        }
    }

    /// Compute LCM of denominators of the provided durations (const-evaluable)
    pub const fn lcm_durations(arr: &[Duration]) -> Ticks {
        let mut i = 0;
        let mut result = 1;
        while i < arr.len() {
            let d = Self::denominator_of(arr[i]);
            result = lcm(result, d);
            i += 1;
        }
        result
    }

    /// Ticks per whole note. Computed at compile time as the LCM of all denominators.
    pub const TICKS_PER_WHOLE: Ticks = Self::lcm_durations(&Self::DURATIONS);

    /// Returns the duration in integer ticks (exact)
    pub fn ticks(&self) -> Ticks {
        let denom = Self::denominator_of(*self);
        Self::TICKS_PER_WHOLE / denom
    }

    /// Returns the Duration that exactly corresponds to the given tick count, if any.
    /// This performs an exact match; if the ticks value doesn't match a supported
    /// duration, None is returned.
    pub fn from_ticks(ticks: Ticks) -> Option<Duration> {
        let mut i = 0;
        while i < Self::DURATIONS.len() {
            let d = Self::DURATIONS[i];
            if d.ticks() == ticks { return Some(d); }
            i += 1;
        }
        None
    }

    pub fn tuplet_cardinality(&self) -> Option<u8> {
        use Duration::*;
        match self {
            TripletEighth => Some(3),
            QuintupletSixteenth => Some(5),
            SextupletSixteenth => Some(6),
            SeptupletSixteenth => Some(7),
            NonupletThirtySecond => Some(9),
            _ => None,
        }
    }

    // The canonical frame these tuplets belong to.
    pub fn tuplet_group_frame(&self) -> Option<Duration> {
        use Duration::*;
        match self {
            TripletEighth | QuintupletSixteenth | SextupletSixteenth | SeptupletSixteenth | NonupletThirtySecond => {
                Some(Quarter) // quarter-note frame
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Duration;

    #[test]
    fn from_ticks_roundtrip() {
        for d in Duration::DURATIONS.iter() {
            assert_eq!(Duration::from_ticks(d.ticks()), Some(*d));
        }
    }
}
