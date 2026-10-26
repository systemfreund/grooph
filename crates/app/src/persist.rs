//! Restoring the saved app state field by field.
//!
//! The state is stored as one RON struct. Read in one piece, a single field
//! that no longer deserializes (a renamed enum variant, a changed type) or a
//! version bump dropped all of it, and the next save wrote the defaults over
//! the library and presets. Here each field is read on its own, so a field
//! that does not read falls back to its default alone, older versions are
//! migrated, and whenever anything was dropped the stored text is kept under
//! a backup key that later saves leave alone.

use crate::{PERSISTED_STATE_VERSION, PersistedState};
use grooph_measure::{BeatIdx, Cursor, Measure, Score};
use log::{info, warn};
use ron::value::RawValue;
use serde::de::{Deserialize, Deserializer, MapAccess, Visitor};
use std::fmt;

/// Prefix of the keys the stored text is backed up under, followed by the
/// version it was saved with (`grooph_state_backup_v2`, then `_v2_2`, ...).
const BACKUP_KEY_PREFIX: &str = "grooph_state_backup_v";

/// Stored text that did not restore completely, to be written under its own
/// key at the next save.
pub(crate) struct StateBackup {
    version: u8,
    ron: String,
}

impl StateBackup {
    /// Writes the backup under the first backup key of its version that is
    /// still free, so an earlier backup is never overwritten.
    pub(crate) fn write(self, storage: &mut dyn eframe::Storage) {
        let base = format!("{BACKUP_KEY_PREFIX}{}", self.version);
        let key = std::iter::once(base.clone())
            .chain((2..).map(|n| format!("{base}_{n}")))
            .find(|key| storage.get_string(key).is_none())
            .unwrap_or(base);
        info!("Backing up the saved state under `{key}` ({} bytes)", self.ron.len());
        storage.set_string(&key, self.ron);
    }
}

/// The state restored from `saved` (the stored RON text, `None` on the first
/// start), and the backup to keep when not all of it could be restored.
pub(crate) fn restore(saved: Option<&str>) -> (PersistedState, Option<StateBackup>) {
    let Some(saved) = saved else {
        return (PersistedState::default(), None);
    };
    let backup = |version| Some(StateBackup { version, ron: saved.to_owned() });
    let mut fields = match ron::from_str::<Fields>(saved) {
        Ok(Fields(fields)) => fields,
        Err(err) => {
            warn!("Saved state does not read, starting from defaults: {err}");
            return (PersistedState::default(), backup(PERSISTED_STATE_VERSION));
        }
    };

    // A version that does not read counts as dropped: the text is backed up.
    let stored_version = field(&fields, "version").map(|raw| raw.into_rust::<u8>().ok());
    let version_read = stored_version.is_none_or(|v| v.is_some());
    if !version_read {
        warn!("Saved state: `version` does not read, assuming the current one");
    }
    let version = stored_version.flatten().unwrap_or(PERSISTED_STATE_VERSION);
    if version < PERSISTED_STATE_VERSION {
        migrate(version, &mut fields);
    }
    fields.retain(|(name, _)| name != "version");

    // Serde ignores unknown fields and reads each field independently of the
    // others, so the fields that read alone, each name once, also read
    // together. Of a name stored twice the last one counts.
    let (kept, dropped): (Vec<_>, Vec<_>) =
        fields.iter().enumerate().partition(|(i, (name, raw))| {
            !fields[i + 1..].iter().any(|(later, _)| later == name)
                && ron::from_str::<PersistedState>(&struct_ron([(name, raw)])).is_ok()
        });
    for (_, (name, _)) in &dropped {
        warn!("Saved state: `{name}` does not read, using its default");
    }
    let state = ron::from_str(&struct_ron(kept.iter().map(|(_, (name, raw))| (name, raw))));
    let read_together = state.is_ok();
    let state = state.unwrap_or_else(|err| {
        warn!("Saved state does not read, starting from defaults: {err}");
        PersistedState::default()
    });

    let complete =
        dropped.is_empty() && read_together && version_read && version == PERSISTED_STATE_VERSION;
    (state, if complete { None } else { backup(version) })
}

/// Brings the fields of a state saved with an older `version` up to the
/// current one.
fn migrate(version: u8, fields: &mut Vec<(String, Box<RawValue>)>) {
    if version < 2 {
        // Version 1 held one measure and the beat index in it; version 2 a
        // score and a cursor into it.
        let measure = take(fields, "measure").and_then(|raw| raw.into_rust::<Measure>().ok());
        if let Some(score) = measure.and_then(|m| RawValue::from_rust(&Score::single(m)).ok()) {
            fields.push(("score".into(), score));
        }
        let beat_idx = take(fields, "cursor_idx").and_then(|raw| raw.into_rust::<BeatIdx>().ok());
        let cursor = beat_idx.map(|beat_idx| Cursor { measure_idx: 0, beat_idx });
        if let Some(cursor) = cursor.and_then(|c| RawValue::from_rust(&c).ok()) {
            fields.push(("cursor".into(), cursor));
        }
    }
}

fn field<'a>(fields: &'a [(String, Box<RawValue>)], name: &str) -> Option<&'a RawValue> {
    fields.iter().find(|(n, _)| n == name).map(|(_, raw)| &**raw)
}

fn take(fields: &mut Vec<(String, Box<RawValue>)>, name: &str) -> Option<Box<RawValue>> {
    let idx = fields.iter().position(|(n, _)| n == name)?;
    Some(fields.remove(idx).1)
}

/// The RON text of a struct with the given fields.
fn struct_ron<'a>(fields: impl IntoIterator<Item = (&'a String, &'a Box<RawValue>)>) -> String {
    let fields: Vec<_> =
        fields.into_iter().map(|(name, raw)| format!("{name}:{}", raw.get_ron())).collect();
    format!("({})", fields.join(","))
}

/// The fields of a stored struct, each kept as unparsed RON.
struct Fields(Vec<(String, Box<RawValue>)>);

impl<'de> Deserialize<'de> for Fields {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct FieldsVisitor;

        impl<'de> Visitor<'de> for FieldsVisitor {
            type Value = Fields;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a struct")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Fields, A::Error> {
                let mut fields = Vec::new();
                while let Some(name) = map.next_key::<String>()? {
                    fields.push((name, map.next_value()?));
                }
                Ok(Fields(fields))
            }
        }

        deserializer.deserialize_struct("PersistedState", &[], FieldsVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::{BACKUP_KEY_PREFIX, StateBackup, restore};
    use crate::presets::builtin_presets;
    use crate::shell::Tab;
    use crate::{PERSISTED_STATE_VERSION, PersistedState};
    use grooph_measure::duration::e;
    use grooph_measure::{Beat, Cursor, Measure, Score, TimeSignature};
    use std::collections::HashMap;

    /// A saved state with a pattern in the library, a user preset and a
    /// non-default tempo and tab.
    fn saved_state() -> PersistedState {
        let mut s = PersistedState { bpm: 93, tab: Tab::Edit, ..Default::default() };
        let mut m = Measure::new(TimeSignature::THREE_FOUR);
        m.set_beat(0, Beat::note(e())).unwrap();
        s.library.add("Groove".into(), Score::single(m), 93);
        s.presets.add("Mine", builtin_presets()[0].1);
        s
    }

    fn saved_ron() -> String {
        ron::to_string(&saved_state()).unwrap()
    }

    fn assert_library_and_presets_kept(s: &PersistedState) {
        assert_eq!(s.library.patterns.len(), 1);
        assert_eq!(s.library.patterns[0].name, "Groove");
        assert_eq!(s.presets.presets.len(), 1);
    }

    #[test]
    fn a_complete_state_restores_without_backup() {
        let (s, backup) = restore(Some(&saved_ron()));
        assert_library_and_presets_kept(&s);
        assert_eq!(s.bpm, 93);
        assert_eq!(s.tab, Tab::Edit);
        assert!(backup.is_none());
        assert!(restore(None).1.is_none());
    }

    #[test]
    fn a_field_that_does_not_read_falls_back_alone() {
        // A tab variant this version does not know, e.g. renamed.
        let saved = saved_ron().replace("tab:Edit", "tab:Practise");
        assert_ne!(saved, saved_ron());
        let (s, backup) = restore(Some(&saved));
        assert_eq!(s.tab, PersistedState::default().tab);
        assert_library_and_presets_kept(&s);
        assert_eq!(s.bpm, 93);
        let backup = backup.expect("the stored text is backed up");
        assert_eq!(backup.ron, saved);
        assert_eq!(backup.version, PERSISTED_STATE_VERSION);
    }

    #[test]
    fn a_broken_score_falls_back_alone() {
        let mut state = saved_state();
        state.score = Score::single(Measure::new(TimeSignature::SEVEN_EIGHT));
        let saved = ron::to_string(&state).unwrap();
        // Seven eighths in 6/8: an over-full measure.
        assert_eq!(saved.matches("beats:7,").count(), 1, "{saved}");
        let saved = saved.replace("beats:7,", "beats:6,");
        let (s, backup) = restore(Some(&saved));
        assert!(s.score.same_beats(&PersistedState::default().score));
        assert_library_and_presets_kept(&s);
        assert_eq!(s.bpm, 93);
        assert_eq!(backup.unwrap().ron, saved);
    }

    #[test]
    fn an_unknown_field_is_ignored() {
        let saved = saved_ron().replacen('(', "(removed_setting:Some(3),", 1);
        let (s, backup) = restore(Some(&saved));
        assert_library_and_presets_kept(&s);
        assert!(backup.is_none());
    }

    #[test]
    fn mixer_settings_with_the_old_counter_mute_still_load() {
        let mut state = saved_state();
        state.audio_settings.counter = 0.4;
        state.audio_settings.rhythm_muted = true;
        let saved = ron::to_string(&state).unwrap();
        // Saved before the counter rhythm joined the metronome group.
        let saved = saved.replacen("rhythm_muted:", "counter_muted:true,rhythm_muted:", 1);
        assert!(saved.contains("counter_muted:true"), "{saved}");
        let (s, backup) = restore(Some(&saved));
        assert_eq!(s.audio_settings, state.audio_settings);
        assert!(backup.is_none());
    }

    #[test]
    fn a_field_stored_twice_keeps_the_last_and_is_backed_up() {
        let saved = saved_ron().replacen('(', "(bpm:60,", 1);
        let (s, backup) = restore(Some(&saved));
        assert_eq!(s.bpm, 93);
        assert_library_and_presets_kept(&s);
        assert_eq!(backup.unwrap().ron, saved);
    }

    #[test]
    fn text_that_does_not_read_is_backed_up() {
        let (s, backup) = restore(Some("(bpm: 93, library: ("));
        assert_eq!(s.bpm, PersistedState::default().bpm);
        assert_eq!(backup.unwrap().ron, "(bpm: 93, library: (");
    }

    #[test]
    fn a_version_that_does_not_read_is_backed_up() {
        let saved = saved_ron().replacen(
            &format!("version:{PERSISTED_STATE_VERSION}"),
            "version:\"two\"",
            1,
        );
        let (s, backup) = restore(Some(&saved));
        assert_library_and_presets_kept(&s);
        assert_eq!(backup.unwrap().ron, saved);
    }

    #[test]
    fn a_version_1_state_is_migrated() {
        let mut m = Measure::new(TimeSignature::THREE_FOUR);
        m.set_beat(0, Beat::note(e())).unwrap();
        let saved =
            format!("(version:1,measure:{},cursor_idx:2,bpm:87)", ron::to_string(&m).unwrap());
        let (s, backup) = restore(Some(&saved));
        assert_eq!(ron::to_string(&s.score), ron::to_string(&Score::single(m)));
        assert_eq!(s.cursor, Cursor { measure_idx: 0, beat_idx: 2 });
        assert_eq!(s.bpm, 87);
        assert_eq!(s.version, PERSISTED_STATE_VERSION);
        assert_eq!(backup.unwrap().version, 1);
    }

    #[test]
    fn a_newer_version_keeps_what_reads() {
        let saved = saved_ron().replacen(
            &format!("version:{PERSISTED_STATE_VERSION}"),
            &format!("version:{}", PERSISTED_STATE_VERSION + 1),
            1,
        );
        let (s, backup) = restore(Some(&saved));
        assert_library_and_presets_kept(&s);
        assert_eq!(backup.unwrap().version, PERSISTED_STATE_VERSION + 1);
    }

    #[derive(Default)]
    struct MemoryStorage(HashMap<String, String>);

    impl eframe::Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }

    #[test]
    fn a_backup_never_overwrites_an_earlier_one() {
        let mut storage = MemoryStorage::default();
        StateBackup { version: 2, ron: "first".into() }.write(&mut storage);
        StateBackup { version: 2, ron: "second".into() }.write(&mut storage);
        StateBackup { version: 1, ron: "old".into() }.write(&mut storage);
        let get = |key: &str| storage.0.get(&format!("{BACKUP_KEY_PREFIX}{key}")).cloned();
        assert_eq!(get("2").as_deref(), Some("first"));
        assert_eq!(get("2_2").as_deref(), Some("second"));
        assert_eq!(get("1").as_deref(), Some("old"));
    }
}
