//! Restoring the saved app state field by field.
//!
//! The state is stored as one RON struct. Read in one piece, a single field
//! that no longer deserializes (a renamed enum variant, a changed type) or a
//! version bump dropped all of it, and the next save wrote the defaults over
//! the library and presets. Here each field is read on its own, so a field
//! that does not read falls back to its default alone, older versions are
//! migrated, and whenever anything was dropped the stored text is kept under
//! a backup key that later saves leave alone.

use crate::{PERSISTED_STATE_VERSION, PersistedState, SavedDoc};
use grooph_measure::{BeatIdx, Cursor, Measure, Score};
use log::{info, warn};
use ron::value::RawValue;
use serde::de::{Deserialize, DeserializeOwned, Deserializer, MapAccess, Visitor};
use std::fmt;

/// The fields holding a rhythm document ([`SavedDoc`]), whose own fields
/// are restored one by one too.
const DOC_FIELDS: [&str; 2] = ["score_doc", "practice_doc"];

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
    let migrated = version < PERSISTED_STATE_VERSION && migrate(version, &mut fields);
    fields.retain(|(name, _)| name != "version");
    // Within a document too, a field that does not read falls back alone.
    let mut doc_dropped = false;
    for (name, raw) in &mut fields {
        if DOC_FIELDS.contains(&name.as_str())
            && let Some((repaired, dropped)) = repair_doc(raw)
        {
            for field in &dropped {
                warn!("Saved state: `{name}.{field}` does not read, using its default");
            }
            doc_dropped |= !dropped.is_empty();
            *raw = repaired;
        }
    }

    let (kept, dropped) = partition_fields::<PersistedState>(&fields);
    for (name, _) in &dropped {
        warn!("Saved state: `{name}` does not read, using its default");
    }
    let state = ron::from_str(&struct_ron(kept));
    let read_together = state.is_ok();
    let state = state.unwrap_or_else(|err| {
        warn!("Saved state does not read, starting from defaults: {err}");
        PersistedState::default()
    });

    let complete = dropped.is_empty()
        && !doc_dropped
        && read_together
        && version_read
        && (version == PERSISTED_STATE_VERSION || migrated);
    (state, if complete { None } else { backup(version) })
}

/// Brings the fields of a state saved with an older `version` up to the
/// current one. Returns whether all of it was carried over, so that the
/// stored text needs no backup; a version 1 state is always backed up.
fn migrate(version: u8, fields: &mut Vec<(String, Box<RawValue>)>) -> bool {
    let mut carried = version >= 2;
    if version < 2 {
        // Version 1 held one measure and the beat index in it; version 2 a
        // score and a cursor into it.
        let measure = take(fields, "measure").and_then(|raw| raw.into_rust::<Measure>().ok());
        if let Some(score) = measure.and_then(|m| RawValue::from_rust(&Score::single(m)).ok()) {
            fields.push(("score".into(), score));
        }
        let beat_idx = take(fields, "cursor_idx").and_then(|raw| raw.into_rust::<BeatIdx>().ok());
        let cursor = beat_idx.map(|beat_idx| Cursor::at(0, beat_idx));
        if let Some(cursor) = cursor.and_then(|c| RawValue::from_rust(&c).ok()) {
            fields.push(("cursor".into(), cursor));
        }
    }
    if version < 3 {
        // Version 2 kept the score tab's document in flat fields and
        // Practice's rhythm, if any, as `practice_score`; version 3 each
        // document as one entry of the same shape.
        // A field stored twice: the last counts and the text is backed up.
        let mut take = |fields: &mut Vec<Field>, name: &str| {
            let (raw, once) = take_last(fields, name);
            carried &= once;
            raw
        };
        let score_doc: Vec<Field> = ["score", "cursor", "dirty", "active_pattern_id"]
            .into_iter()
            .filter_map(|name| Some((name.to_owned(), take(fields, name)?)))
            .collect();
        let practice_score = take(fields, "practice_score");
        let practice_cursor = take(fields, "practice_cursor");
        if !score_doc.is_empty() {
            carried &= push_ron(fields, "score_doc", struct_ron(&score_doc));
        }
        // `None` (or no field): saved before Practice had its own rhythm.
        let score = practice_score.and_then(|raw| match raw.into_rust::<Option<Box<RawValue>>>() {
            Ok(score) => score,
            // Kept as it was: it drops alone, and the text is backed up.
            Err(_) => {
                carried = false;
                Some(raw)
            }
        });
        if let Some(score) = score {
            let doc = [("score".to_owned(), score)]
                .into_iter()
                .chain(practice_cursor.map(|raw| ("cursor".to_owned(), raw)))
                .collect::<Vec<_>>();
            let doc = struct_ron(&doc);
            carried &= push_ron(fields, "practice_doc", format!("Some({doc})"));
        }
    }
    carried
}

/// Adds the field `name` with the value `ron`; whether it reads as RON.
fn push_ron(fields: &mut Vec<(String, Box<RawValue>)>, name: &str, ron: String) -> bool {
    let raw = RawValue::from_boxed_ron(ron.into_boxed_str());
    raw.map(|raw| fields.push((name.to_owned(), raw))).is_ok()
}

/// A stored document (`(…)` or `Some((…))`) with only its fields that read,
/// and the names of the ones dropped; `None` for `None` or for text that is
/// no struct, which then reads or drops as a whole.
fn repair_doc(raw: &RawValue) -> Option<(Box<RawValue>, Vec<String>)> {
    let (Fields(fields), some) = match raw.into_rust::<Fields>() {
        Ok(fields) => (fields, false),
        Err(_) => (raw.into_rust::<Option<Fields>>().ok()??, true),
    };
    let (kept, dropped) = partition_fields::<SavedDoc>(&fields);
    let doc = struct_ron(kept);
    let doc = if some { format!("Some({doc})") } else { doc };
    let dropped = dropped.into_iter().map(|(name, _)| name.clone()).collect();
    Some((RawValue::from_boxed_ron(doc.into_boxed_str()).ok()?, dropped))
}

/// A stored field: its name and its unparsed RON.
type Field = (String, Box<RawValue>);

/// `fields` split into those kept and those dropped as fields of `T`. Serde
/// ignores unknown fields and reads each field independently of the others,
/// so the fields that read alone, each name once, also read together. Of a
/// name stored twice the last one counts, and the earlier one is dropped.
fn partition_fields<T: DeserializeOwned>(fields: &[Field]) -> (Vec<&Field>, Vec<&Field>) {
    let reads = |i: usize, field: &Field| {
        !fields[i + 1..].iter().any(|(later, _)| *later == field.0)
            && ron::from_str::<T>(&struct_ron([field])).is_ok()
    };
    let (mut kept, mut dropped) = (Vec::new(), Vec::new());
    for (i, field) in fields.iter().enumerate() {
        if reads(i, field) { kept.push(field) } else { dropped.push(field) }
    }
    (kept, dropped)
}

fn field<'a>(fields: &'a [(String, Box<RawValue>)], name: &str) -> Option<&'a RawValue> {
    fields.iter().find(|(n, _)| n == name).map(|(_, raw)| &**raw)
}

fn take(fields: &mut Vec<(String, Box<RawValue>)>, name: &str) -> Option<Box<RawValue>> {
    let idx = fields.iter().position(|(n, _)| n == name)?;
    Some(fields.remove(idx).1)
}

/// Takes every field `name`: the last one counts, as in [`restore`]. Also
/// whether it was stored at most once.
fn take_last(fields: &mut Vec<Field>, name: &str) -> (Option<Box<RawValue>>, bool) {
    let mut taken = Vec::new();
    while let Some(raw) = take(fields, name) {
        taken.push(raw);
    }
    let once = taken.len() <= 1;
    (taken.pop(), once)
}

/// The RON text of a struct with the given fields.
fn struct_ron<'a>(fields: impl IntoIterator<Item = &'a Field>) -> String {
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
    use super::{BACKUP_KEY_PREFIX, DOC_FIELDS, Fields, StateBackup, field, restore};
    use crate::presets::builtin_presets;
    use crate::shell::Tab;
    use crate::{PERSISTED_STATE_VERSION, PersistedState, SavedDoc};
    use grooph_measure::duration::e;
    use grooph_measure::{Beat, Cursor, Measure, Score, TimeSignature};
    use ron::value::RawValue;
    use std::collections::HashMap;

    /// A saved state with a pattern in the library, a user preset and a
    /// non-default tempo and tab.
    fn saved_state() -> PersistedState {
        let mut s = PersistedState { bpm: 93, tab: Tab::Score, ..Default::default() };
        let mut m = Measure::new(TimeSignature::THREE_FOUR);
        m.first_voice_mut().set_beat(0, Beat::note(e())).unwrap();
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
        assert_eq!(s.tab, Tab::Score);
        assert!(backup.is_none());
        assert!(restore(None).1.is_none());
    }

    #[test]
    fn a_field_that_does_not_read_falls_back_alone() {
        // A tab variant this version does not know, e.g. renamed.
        let saved = saved_ron().replace("tab:Score", "tab:Practise");
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
        for practice in [false, true] {
            let mut state = saved_state();
            // The broken score in the score tab's document, or in Practice's.
            let mut doc = SavedDoc {
                score: Score::single(Measure::new(TimeSignature::SEVEN_EIGHT)),
                cursor: Cursor::at(0, 1),
                dirty: true,
                ..Default::default()
            };
            let mut plain = SavedDoc { dirty: true, ..Default::default() };
            if practice {
                std::mem::swap(&mut doc, &mut plain);
            }
            state.score_doc = doc;
            state.practice_doc = Some(plain);
            let saved = ron::to_string(&state).unwrap();
            // Seven eighths in 6/8: an over-full measure.
            assert_eq!(saved.matches("beats:7,").count(), 1, "{saved}");
            let saved = saved.replace("beats:7,", "beats:6,");
            let (s, backup) = restore(Some(&saved));
            // Only the score falls back, the rest of its document stays.
            let doc = if practice { s.practice_doc.as_ref().unwrap() } else { &s.score_doc };
            assert!(doc.score.same_beats(&SavedDoc::default().score), "{practice}");
            assert_eq!(doc.cursor, Cursor::at(0, 1));
            assert!(doc.dirty);
            assert!(s.score_doc.dirty && s.practice_doc.as_ref().unwrap().dirty);
            assert_library_and_presets_kept(&s);
            assert_eq!(s.bpm, 93);
            assert_eq!(backup.unwrap().ron, saved);
        }
    }

    /// The fields of a version 2 state: the score tab's document flat, and
    /// Practice's rhythm as `practice_score` if `practice`.
    fn version_2_state(practice: bool) -> String {
        let state = saved_state();
        let fields = ron::to_string(&state).unwrap();
        let (doc, practice_doc) = (&state.score_doc, state.practice_doc.as_ref());
        assert!(practice_doc.is_none());
        let start = fields.find("score_doc:").unwrap();
        let end = start + "score_doc:".len() + ron::to_string(doc).unwrap().len();
        let flat = format!(
            "score:{},cursor:{},dirty:true,active_pattern_id:Some({})",
            ron::to_string(&doc.score).unwrap(),
            ron::to_string(&Cursor::at(0, 1)).unwrap(),
            state.library.patterns[0].id,
        );
        let practice = if practice {
            format!(
                ",practice_score:Some({}),practice_cursor:{}",
                ron::to_string(&practice_score()).unwrap(),
                ron::to_string(&Cursor::at(0, 2)).unwrap(),
            )
        } else {
            String::new()
        };
        let fields = format!("{}{flat}{practice}{}", &fields[..start], &fields[end..]);
        let fields = fields.replace(",practice_doc:None", "");
        fields.replacen(&format!("version:{PERSISTED_STATE_VERSION}"), "version:2", 1)
    }

    fn practice_score() -> Score {
        grooph_measure::notation::parse_score("4/4 q e e q q").unwrap()
    }

    #[test]
    fn a_version_2_state_is_migrated_without_loss() {
        for practice in [true, false] {
            let saved = version_2_state(practice);
            assert!(saved.contains("version:2,") && !saved.contains("score_doc"), "{saved}");
            assert_eq!(saved.contains("practice_score"), practice, "{saved}");
            let (s, backup) = restore(Some(&saved));
            assert!(backup.is_none(), "{saved}");
            assert_eq!(s.version, PERSISTED_STATE_VERSION);
            let doc = &s.score_doc;
            assert!(doc.score.same_beats(&saved_state().score_doc.score));
            assert_eq!(doc.cursor, Cursor::at(0, 1));
            assert!(doc.dirty);
            // Its pattern is in the library, so the app keeps it.
            let pattern = s.library.patterns[0].id;
            assert_eq!(doc.active_pattern_id, Some(pattern));
            // Saved before Practice had its own: none, so it gets a new one.
            match &s.practice_doc {
                Some(doc) => {
                    assert!(practice);
                    assert!(doc.score.same_beats(&practice_score()));
                    assert_eq!(doc.cursor, Cursor::at(0, 2));
                    assert!(!doc.dirty && doc.active_pattern_id.is_none());
                }
                None => assert!(!practice),
            }
            assert_library_and_presets_kept(&s);
            assert_eq!(s.bpm, 93);
            let doc = s.score_doc.into_doc(&s.library);
            assert_eq!(doc.active_pattern_id, Some(pattern));
            assert!(doc.dirty);
        }
    }

    #[test]
    fn a_version_2_field_stored_twice_keeps_the_last_and_is_backed_up() {
        let saved = version_2_state(true).replacen("dirty:true", "dirty:false,dirty:true", 1);
        assert!(saved.contains("dirty:false,dirty:true"), "{saved}");
        let (s, backup) = restore(Some(&saved));
        assert!(s.score_doc.dirty);
        assert_eq!(backup.unwrap().version, 2);
    }

    #[test]
    fn the_doc_fields_are_the_states_documents() {
        // The fields whose value is a document (a struct with a score and a
        // cursor), as saved with both documents.
        let state = PersistedState { practice_doc: Some(SavedDoc::default()), ..saved_state() };
        let Fields(fields) = ron::from_str(&ron::to_string(&state).unwrap()).unwrap();
        let is_doc = |raw: &RawValue| {
            let fields = raw.into_rust::<Fields>().or_else(|_| {
                raw.into_rust::<Option<Fields>>().map_err(|_| ()).and_then(|f| f.ok_or(()))
            });
            fields.is_ok_and(|Fields(f)| ["score", "cursor"].iter().all(|n| field(&f, n).is_some()))
        };
        let docs: Vec<_> =
            fields.iter().filter(|(_, raw)| is_doc(raw)).map(|(name, _)| name.as_str()).collect();
        assert_eq!(docs, DOC_FIELDS);
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
        assert_library_and_presets_kept(&s);
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
        m.first_voice_mut().set_beat(0, Beat::note(e())).unwrap();
        let saved =
            format!("(version:1,measure:{},cursor_idx:2,bpm:87)", ron::to_string(&m).unwrap());
        let (s, backup) = restore(Some(&saved));
        assert_eq!(ron::to_string(&s.score_doc.score), ron::to_string(&Score::single(m)));
        assert_eq!(s.score_doc.cursor, Cursor::at(0, 2));
        assert_eq!(s.bpm, 87);
        assert_eq!(s.version, PERSISTED_STATE_VERSION);
        assert_eq!(backup.unwrap().version, 1);
    }

    #[test]
    fn a_score_saved_before_heads_reads_unchanged() {
        // A score as the app saved it before beats had heads (#251): an
        // accented note, a ghost note and a rest.
        let old = r#"(measures:[{"beats":[(duration:Simple(Eighth),kind:Note,accented:true,soft:false,tuplet_group_id:None),(duration:Simple(Eighth),kind:Note,accented:false,soft:true,tuplet_group_id:None),(duration:Simple(Quarter),kind:Rest,accented:false,soft:false,tuplet_group_id:None)],"time_signature":(beats:2,beat_unit:4),"tuplet_anchors":{},"next_tuplet_id":1}])"#;
        let saved = format!("(version:{PERSISTED_STATE_VERSION},score_doc:(score:{old}))");
        let (s, backup) = restore(Some(&saved));
        assert!(backup.is_none(), "the old score is read, nothing is dropped");
        let want = grooph_measure::notation::parse_score("2/4 >e (e) r:q").unwrap();
        assert_eq!(ron::to_string(&s.score_doc.score), ron::to_string(&want));
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
