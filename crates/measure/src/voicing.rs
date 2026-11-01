//! Which voice of a kit score each instrument is written in.
//!
//! A [`Voicing`] is the rule of a score: the set of instruments whose heads
//! go in the lower voice when they are entered; every other instrument goes
//! in the upper one. An empty set is a score of one voice. The presets are
//! named sets; a set that is none of them is a custom voicing, and it is
//! recognised by equality.
//!
//! The rule only says where a new head goes. Where a head already is, the
//! score itself says.

use crate::{Instrument, VoiceIdx};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The instruments of a kit score that are written in the lower voice.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Voicing {
    /// A bit per instrument, at its place in [`Instrument::ALL`].
    lower: u16,
}

impl Default for Voicing {
    fn default() -> Self {
        Self::HANDS_FEET
    }
}

const fn bit(instrument: Instrument) -> u16 {
    1 << instrument as u16
}

impl Voicing {
    /// The hands above, the feet below: the usual drumkit notation.
    pub const HANDS_FEET: Self = Self { lower: bit(Instrument::Kick) | bit(Instrument::HiHatFoot) };
    /// Everything in one voice.
    pub const ONE_VOICE: Self = Self { lower: 0 };
    /// The cymbals above, the drums and the feet below.
    pub const CYMBALS_ABOVE: Self = Self {
        lower: Self::HANDS_FEET.lower
            | bit(Instrument::Snare)
            | bit(Instrument::HighTom)
            | bit(Instrument::MidTom)
            | bit(Instrument::FloorTom),
    };

    /// The voicing with `lower` in the lower voice. The one line's rhythm is
    /// always in the upper.
    pub fn from_lower(lower: impl IntoIterator<Item = Instrument>) -> Self {
        let lower =
            lower.into_iter().filter(|&i| i != Instrument::Rhythm).fold(0, |set, i| set | bit(i));
        Self { lower }
    }

    /// Whether this is the voicing a score has unless told otherwise.
    pub fn is_default(&self) -> bool {
        *self == Self::HANDS_FEET
    }

    /// Whether `instrument` is written in the lower voice.
    pub fn is_lower(&self, instrument: Instrument) -> bool {
        self.lower & bit(instrument) != 0
    }

    /// The voice a new head of `instrument` goes to: 1 for the lower, else 0.
    pub fn voice_of(&self, instrument: Instrument) -> VoiceIdx {
        usize::from(self.is_lower(instrument))
    }

    /// How many voices a new bar of the score has: one if nothing goes in
    /// the lower voice, else two.
    pub fn voices(&self) -> usize {
        if self.lower == 0 { 1 } else { 2 }
    }

    /// The instruments in the lower voice, in the order of [`Instrument::ALL`].
    pub fn lower(&self) -> impl Iterator<Item = Instrument> + '_ {
        Instrument::ALL.into_iter().filter(|&i| self.is_lower(i))
    }
}

impl Serialize for Voicing {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.lower())
    }
}

impl<'de> Deserialize<'de> for Voicing {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::from_lower(Vec::<Instrument>::deserialize(deserializer)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Instrument::*;

    #[test]
    fn presets() {
        let voices = |v: Voicing| Instrument::ALL.map(|i| v.voice_of(i));
        assert_eq!(voices(Voicing::HANDS_FEET), [0, 1, 1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(voices(Voicing::ONE_VOICE), [0; 10]);
        assert_eq!(voices(Voicing::CYMBALS_ABOVE), [0, 1, 1, 1, 1, 1, 1, 0, 0, 0]);
        assert_eq!(Voicing::HANDS_FEET.voices(), 2);
        assert_eq!(Voicing::ONE_VOICE.voices(), 1);
        assert_eq!(Voicing::CYMBALS_ABOVE.voices(), 2);
        assert_eq!(Voicing::default(), Voicing::HANDS_FEET);
        assert!(Voicing::HANDS_FEET.is_default());
        assert!(!Voicing::ONE_VOICE.is_default());
    }

    #[test]
    fn rhythm_stays_above() {
        let v = Voicing::from_lower([Rhythm]);
        assert_eq!(v, Voicing::ONE_VOICE);
        assert_eq!(v.voice_of(Rhythm), 0);
    }

    #[test]
    fn custom_set_is_not_a_preset() {
        let v = Voicing::from_lower([Kick, HiHatFoot, FloorTom]);
        assert_eq!(v.voice_of(FloorTom), 1);
        assert_eq!(v.voice_of(Snare), 0);
        assert!(![Voicing::HANDS_FEET, Voicing::ONE_VOICE, Voicing::CYMBALS_ABOVE].contains(&v));
    }

    #[test]
    fn serde_roundtrip() {
        for v in [Voicing::HANDS_FEET, Voicing::ONE_VOICE, Voicing::CYMBALS_ABOVE] {
            let text = ron::to_string(&v).unwrap();
            assert_eq!(ron::from_str::<Voicing>(&text).unwrap(), v);
        }
        assert_eq!(ron::to_string(&Voicing::HANDS_FEET).unwrap(), "[HiHatFoot,Kick]");
    }
}
