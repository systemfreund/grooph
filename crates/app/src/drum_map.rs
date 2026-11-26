//! Which instrument of the kit a MIDI note plays: the General MIDI drum map
//! plus the common notes of e-drum kits (Roland's layout).
//!
//! A device can have a custom drum map instead ([`CustomDrumMap`], learned
//! in the MIDI input picker), which then replaces the table whole. It lives
//! in the app, not in `grooph-midi`, which knows nothing of the music model.
//! The channel plays no part: not every e-kit sends on channel 10.

use grooph_measure::{Instrument, Technique};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One entry of a drum map: a MIDI note and what it plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
#[cfg(test)]
pub(crate) fn gm_drum(note: u8) -> Option<(Instrument, Technique)> {
    lookup(GM_DRUM_MAP, note)
}

/// What a note plays: an instrument and its technique. One row of the drum
/// map list.
pub(crate) type Combo = (Instrument, Technique);

/// The instruments in the drum map list's order: the feet, the toms from
/// low to high, the snare, the cymbals.
const LIST_ORDER: [Instrument; 9] = {
    use Instrument::*;
    [HiHatFoot, Kick, FloorTom, MidTom, HighTom, Snare, Ride, HiHat, Crash]
};

/// The combinations a note can be learned for, in the list's order: every
/// technique the register allows each instrument, except a choked crash
/// (aftertouch, not a note).
pub(crate) fn offered_combos() -> Vec<Combo> {
    LIST_ORDER
        .iter()
        .flat_map(|&i| i.techniques().map(move |t| (i, t)))
        .filter(|&(_, t)| t != Technique::Choke)
        .collect()
}

/// A combination's name in the list: the register's label, and the
/// technique after a dot unless it is the usual stroke.
pub(crate) fn combo_label((instrument, technique): Combo) -> String {
    let technique = match technique {
        Technique::Normal => return instrument.label().to_owned(),
        Technique::Open => "Open",
        Technique::HalfOpen => "Half-open",
        Technique::Closed => "Closed",
        Technique::CrossStick => "Cross-stick",
        Technique::Rimshot => "Rimshot",
        Technique::Bell => "Bell",
        Technique::Choke => "Choke",
    };
    format!("{} · {technique}", instrument.label())
}

/// A device's own drum map, learned in the drum map list: each note at
/// most once, each with a technique its instrument allows. A note it does
/// not list plays nothing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Vec<DrumMapEntry>", into = "Vec<DrumMapEntry>")]
pub(crate) struct CustomDrumMap {
    entries: Vec<DrumMapEntry>,
}

impl Default for CustomDrumMap {
    /// A copy of the built-in table, which the first change starts from.
    fn default() -> Self {
        Self { entries: GM_DRUM_MAP.to_vec() }
    }
}

impl From<Vec<DrumMapEntry>> for CustomDrumMap {
    /// Keeps the first entry of each note, and only techniques their
    /// instrument allows (a saved map edited by hand, or an older register).
    fn from(list: Vec<DrumMapEntry>) -> Self {
        let mut entries: Vec<DrumMapEntry> = Vec::with_capacity(list.len());
        for e in list {
            if e.instrument.allows(e.technique) && entries.iter().all(|k| k.note != e.note) {
                entries.push(e);
            }
        }
        Self { entries }
    }
}

impl From<CustomDrumMap> for Vec<DrumMapEntry> {
    fn from(map: CustomDrumMap) -> Self {
        map.entries
    }
}

impl CustomDrumMap {
    pub(crate) fn entries(&self) -> &[DrumMapEntry] {
        &self.entries
    }

    /// Makes `note` play `combo`, taking it from the combination it played
    /// before. A technique the instrument does not allow changes nothing.
    pub(crate) fn assign(&mut self, note: u8, (instrument, technique): Combo) {
        if !instrument.allows(technique) {
            return;
        }
        self.remove(note);
        self.entries.push(DrumMapEntry { note, instrument, technique });
    }

    /// Makes `note` play nothing.
    pub(crate) fn remove(&mut self, note: u8) {
        self.entries.retain(|e| e.note != note);
    }
}

/// The notes `combo` is played by in `map`, from low to high.
pub(crate) fn notes_of(map: &[DrumMapEntry], combo: Combo) -> Vec<u8> {
    let mut notes: Vec<u8> =
        map.iter().filter(|e| (e.instrument, e.technique) == combo).map(|e| e.note).collect();
    notes.sort_unstable();
    notes
}

/// The key a device's drum map is saved under. Under Linux (ALSA) its port
/// name without the ` client:port` numbers at the end, which change when it
/// is plugged in again; elsewhere (the web, macOS, Windows) the name as
/// reported, where such numbers may be part of it. Two identical devices
/// share a map.
pub(crate) fn device_key(port_name: &str) -> &str {
    if cfg!(all(target_os = "linux", not(target_arch = "wasm32"))) {
        without_alsa_numbers(port_name)
    } else {
        port_name
    }
}

/// `port_name` without ALSA's ` client:port` numbers at its end
/// (`"TD-17:TD-17 MIDI 1 24:0"` → `"TD-17:TD-17 MIDI 1"`).
fn without_alsa_numbers(port_name: &str) -> &str {
    let Some((rest, numbers)) = port_name.rsplit_once(' ') else {
        return port_name;
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    match numbers.split_once(':') {
        Some((client, port)) if digits(client) && digits(port) => rest,
        _ => port_name,
    }
}

/// The custom drum maps by device key ([`device_key`]). Saved with the
/// settings.
pub(crate) type DrumMaps = BTreeMap<String, CustomDrumMap>;

/// The map the device named `port_name` plays by: its custom map if it
/// has one, otherwise the built-in table.
pub(crate) fn map_for<'a>(maps: &'a DrumMaps, port_name: Option<&str>) -> &'a [DrumMapEntry] {
    port_name
        .and_then(|name| maps.get(device_key(name)))
        .map_or(GM_DRUM_MAP, CustomDrumMap::entries)
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

    #[test]
    fn a_fresh_custom_map_is_the_built_in_table() {
        assert_eq!(CustomDrumMap::default().entries(), GM_DRUM_MAP);
    }

    #[test]
    fn assigning_a_mapped_note_moves_it() {
        let mut map = CustomDrumMap::default();
        map.assign(38, (Ride, Bell));
        assert_eq!(map.entries().iter().filter(|e| e.note == 38).count(), 1);
        assert_eq!(lookup(map.entries(), 38), Some((Ride, Bell)));
        assert_eq!(notes_of(map.entries(), (Snare, Normal)), Vec::<u8>::new());
        assert_eq!(notes_of(map.entries(), (Ride, Bell)), [38, 53]);
        // A new note is added.
        map.assign(60, (Kick, Normal));
        assert_eq!(lookup(map.entries(), 60), Some((Kick, Normal)));
        assert_eq!(map.entries().len(), GM_DRUM_MAP.len() + 1);
        // A technique the instrument does not allow changes nothing.
        let before = map.clone();
        map.assign(61, (Kick, Rimshot));
        assert_eq!(map, before);
    }

    #[test]
    fn a_removed_note_plays_nothing() {
        let mut map = CustomDrumMap::default();
        map.remove(36);
        assert_eq!(lookup(map.entries(), 36), None);
        assert_eq!(lookup(map.entries(), 35), Some((Kick, Normal)));
    }

    #[test]
    fn every_entry_has_a_technique_its_instrument_allows() {
        let mut map = CustomDrumMap::default();
        for (i, combo) in offered_combos().into_iter().enumerate() {
            map.assign(60 + i as u8, combo);
        }
        for e in map.entries() {
            assert!(e.instrument.allows(e.technique), "{e:?}");
        }
        // A saved map that breaks the rules loses the broken entries.
        let saved = vec![
            entry(60, Kick, Normal),
            entry(60, Snare, Normal),
            entry(61, Kick, Bell),
            entry(62, HiHat, HalfOpen),
        ];
        let map = CustomDrumMap::from(saved);
        assert_eq!(map.entries(), [entry(60, Kick, Normal), entry(62, HiHat, HalfOpen)]);
    }

    #[test]
    fn a_custom_map_survives_a_save_and_load() {
        let mut map = CustomDrumMap::default();
        map.assign(60, (HiHat, HalfOpen));
        map.remove(36);
        let text = ron::to_string(&map).unwrap();
        assert_eq!(ron::from_str::<CustomDrumMap>(&text).unwrap(), map);
    }

    #[test]
    fn the_list_offers_every_combination_but_a_choked_crash() {
        let labels: Vec<String> = offered_combos().into_iter().map(combo_label).collect();
        assert_eq!(
            labels,
            [
                "Hi-hat foot",
                "Kick",
                "Floor tom",
                "Floor tom · Rimshot",
                "Tom 2",
                "Tom 2 · Rimshot",
                "Tom 1",
                "Tom 1 · Rimshot",
                "Snare",
                "Snare · Cross-stick",
                "Snare · Rimshot",
                "Ride",
                "Ride · Bell",
                "Hi-hat",
                "Hi-hat · Open",
                "Hi-hat · Half-open",
                "Hi-hat · Closed",
                "Crash",
            ]
        );
    }

    #[test]
    fn a_device_without_a_custom_map_plays_the_built_in_table() {
        let mut maps = DrumMaps::new();
        assert_eq!(map_for(&maps, None), GM_DRUM_MAP);
        assert_eq!(map_for(&maps, Some("TD-17:TD-17 MIDI 1 24:0")), GM_DRUM_MAP);
        let mut custom = CustomDrumMap::default();
        custom.assign(60, (Kick, Normal));
        custom.remove(36);
        maps.insert(device_key("TD-17:TD-17 MIDI 1 24:0").into(), custom);
        let map = map_for(&maps, Some("TD-17:TD-17 MIDI 1 24:0"));
        assert_eq!(lookup(map, 60), Some((Kick, Normal)));
        assert_eq!(lookup(map, 36), None);
        // After replugging, under other ALSA numbers.
        if cfg!(all(target_os = "linux", not(target_arch = "wasm32"))) {
            assert_eq!(map_for(&maps, Some("TD-17:TD-17 MIDI 1 28:0")), map);
        }
        assert_eq!(map_for(&maps, Some("Other pad")), GM_DRUM_MAP);
    }

    #[test]
    fn the_device_key_drops_the_alsa_numbers() {
        let key = without_alsa_numbers("TD-17:TD-17 MIDI 1 24:0");
        assert_eq!(key, "TD-17:TD-17 MIDI 1");
        assert_eq!(without_alsa_numbers("TD-17:TD-17 MIDI 1 28:0"), key);
        for name in ["TD-17", "Roland TD-17", "Pad 2:1x", "USB MIDI 1", ""] {
            assert_eq!(without_alsa_numbers(name), name);
        }
        // Only ALSA adds them: elsewhere they belong to the name.
        let names = ["TD-17:TD-17 MIDI 1 24:0", "TD-17:TD-17 MIDI 1 28:0"];
        let linux = cfg!(all(target_os = "linux", not(target_arch = "wasm32")));
        assert_eq!(device_key(names[0]) == device_key(names[1]), linux);
        assert_eq!(device_key("Studio Kit 1:0") == device_key("Studio Kit 2:0"), linux);
    }
}
