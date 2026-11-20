//! Which instrument of the kit a MIDI note plays: the General MIDI drum map
//! plus the common notes of e-drum kits (Roland's layout).
//!
//! The map is a plain table so that a custom mapping (per device, learned)
//! can replace it later. It lives in the app, not in `grooph-midi`, which
//! knows nothing of the music model. The channel plays no part: not every
//! e-kit sends on channel 10.

use grooph_measure::{Instrument, Technique};

/// One entry of a drum map: a MIDI note and what it plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DrumMapEntry {
    pub(crate) note: u8,
    pub(crate) instrument: Instrument,
    pub(crate) technique: Technique,
}

const fn entry(note: u8, instrument: Instrument, technique: Technique) -> DrumMapEntry {
    DrumMapEntry { note, instrument, technique }
}

/// The General MIDI drum map and the e-kit notes 22, 26 (hi-hat edge,
/// closed and open) and 58 (tom 3 rim). The other e-kit rims fall on the GM
/// notes of their instrument. Every note not listed plays no instrument.
pub(crate) const GM_DRUM_MAP: &[DrumMapEntry] = {
    use Instrument::*;
    use Technique::*;
    &[
        entry(35, Kick, Normal),
        entry(36, Kick, Normal),
        entry(38, Snare, Normal),
        entry(40, Snare, Rimshot),
        entry(37, Snare, CrossStick),
        entry(42, HiHat, Closed),
        entry(22, HiHat, Closed),
        entry(46, HiHat, Open),
        entry(26, HiHat, Open),
        entry(44, HiHatFoot, Normal),
        entry(41, FloorTom, Normal),
        entry(43, FloorTom, Normal),
        entry(58, FloorTom, Normal),
        entry(45, MidTom, Normal),
        entry(47, MidTom, Normal),
        entry(48, HighTom, Normal),
        entry(50, HighTom, Normal),
        entry(51, Ride, Normal),
        entry(59, Ride, Normal),
        entry(53, Ride, Bell),
        entry(49, Crash, Normal),
        entry(57, Crash, Normal),
        // China and splash: the kit has one crash for all of them.
        entry(52, Crash, Normal),
        entry(55, Crash, Normal),
    ]
};

/// The instrument and technique `note` plays in `map`; `None` for a note
/// the map does not list.
pub(crate) fn lookup(map: &[DrumMapEntry], note: u8) -> Option<(Instrument, Technique)> {
    map.iter().find(|e| e.note == note).map(|e| (e.instrument, e.technique))
}

/// The instrument and technique `note` plays in the General MIDI drum map.
pub(crate) fn gm_drum(note: u8) -> Option<(Instrument, Technique)> {
    lookup(GM_DRUM_MAP, note)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Instrument::*;
    use Technique::*;

    #[test]
    fn every_note_of_the_table_plays_its_instrument() {
        let table: &[(&[u8], Instrument, Technique)] = &[
            (&[35, 36], Kick, Normal),
            (&[38], Snare, Normal),
            (&[40], Snare, Rimshot),
            (&[37], Snare, CrossStick),
            (&[42, 22], HiHat, Closed),
            (&[46, 26], HiHat, Open),
            (&[44], HiHatFoot, Normal),
            (&[41, 43, 58], FloorTom, Normal),
            (&[45, 47], MidTom, Normal),
            (&[48, 50], HighTom, Normal),
            (&[51, 59], Ride, Normal),
            (&[53], Ride, Bell),
            (&[49, 57, 52, 55], Crash, Normal),
        ];
        for &(notes, instrument, technique) in table {
            for &note in notes {
                assert_eq!(gm_drum(note), Some((instrument, technique)), "note {note}");
            }
        }
        let listed: usize = table.iter().map(|(notes, ..)| notes.len()).sum();
        assert_eq!(listed, GM_DRUM_MAP.len());
    }

    #[test]
    fn other_notes_play_nothing() {
        for note in [60, 56, 54, 39, 0, 127] {
            assert_eq!(gm_drum(note), None, "note {note}");
        }
    }

    #[test]
    fn each_technique_is_one_its_instrument_allows() {
        for e in GM_DRUM_MAP {
            assert!(e.instrument.allows(e.technique), "{e:?}");
        }
    }

    #[test]
    fn each_note_is_listed_once() {
        let mut notes: Vec<u8> = GM_DRUM_MAP.iter().map(|e| e.note).collect();
        notes.sort_unstable();
        notes.dedup();
        assert_eq!(notes.len(), GM_DRUM_MAP.len());
    }
}
