//! Restoring the saved app state field by field.
//!
//! The state is stored as one RON struct. Read in one piece, a single field
//! that no longer deserializes (a renamed enum variant, a changed type) or a
//! version bump dropped all of it, and the next save wrote the defaults over
//! the library and presets. Here each field is read on its own, so a field
//! that does not read falls back to its default alone, older versions are
//! migrated, and whenever anything was dropped the stored text is kept under
//! a backup key that later saves leave alone. The documents and the settings
//! groups ([`crate::settings`]) are restored field by field too.

use crate::metronome_bars::{MeterBars, SavedMetronome};
use crate::settings::{
    DeviceSettings, FeedbackSettings, GeneratorConfig, PlaybackSettings, ViewSettings,
};
use crate::{PERSISTED_STATE_VERSION, PersistedState, SavedDoc};
use grooph_measure::click::ClickBars;
use grooph_measure::counter::{Ratio, named_ratio_of, respelled};
use grooph_measure::{BeatIdx, Cursor, Measure, Score, TimeSignature};
use log::{info, warn};
use ron::value::RawValue;
use serde::de::{Deserialize, DeserializeOwned, Deserializer, MapAccess, Visitor};
use std::fmt;

/// The fields holding a rhythm document ([`SavedDoc`]), whose own fields
/// are restored one by one too.
const DOC_FIELDS: [&str; 2] = ["score_doc", "practice_doc"];

/// The settings groups of version 4, each with the flat fields of earlier
/// versions it is made of: (name before, name in the group).
const SETTINGS_GROUPS: [(&str, &[(&str, &str)]); 5] = [
    (
        "playback",
        &[
            ("bpm", "bpm"),
            ("count_in", "count_in"),
            ("swing", "swing"),
            ("gap_click", "gap_click"),
            ("tempo_ramp", "tempo_ramp"),
        ],
    ),
    (
        "generator",
        &[
            ("generator_settings", "settings"),
            ("endless", "endless"),
            ("ghost_notes", "ghost_notes"),
        ],
    ),
    (
        "devices",
        &[
            ("audio_latency_enabled", "latency_enabled"),
            ("audio_offset", "offset"),
            ("midi_selected_port_id", "midi_port_id"),
        ],
    ),
    (
        "feedback",
        &[
            ("accuracy_enabled", "accuracy_enabled"),
            ("flash_enabled", "flash_enabled"),
            ("play_along", "play_along"),
        ],
    ),
    (
        "view",
        &[
            ("counting", "counting"),
            ("sticking", "sticking"),
            ("guide", "guide"),
            ("show_score_tab", "show_score_tab"),
        ],
    ),
];

/// The groups of a preset's settings from version 4 on, and the flat fields
/// of a preset saved before.
const PRESET_GROUPS: [(&str, &[(&str, &str)]); 2] = [
    (
        "playback",
        &[
            ("bpm", "bpm"),
            ("count_in", "count_in"),
            ("swing", "swing"),
            ("gap_click", "gap_click"),
            ("tempo_ramp", "tempo_ramp"),
        ],
    ),
    (
        "generator",
        &[("generator", "settings"), ("endless", "endless"), ("ghost_notes", "ghost_notes")],
    ),
];

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
    // Within a document or a settings group too, a field that does not read
    // falls back alone.
    let mut nested_dropped = false;
    for (name, raw) in &mut fields {
        if let Some((repaired, dropped)) = repair_field(name, raw) {
            for field in &dropped {
                warn!("Saved state: `{name}.{field}` does not read, using its default");
            }
            nested_dropped |= !dropped.is_empty();
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
        && !nested_dropped
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
    if version < 4 {
        // Version 3 kept every setting in a flat field, the presets' too;
        // version 4 in one group per feature.
        carried &= group_fields(fields, &SETTINGS_GROUPS);
        if let Some(raw) = take(fields, "presets") {
            // Kept as it was when it is no library: the text is backed up.
            let (presets, regrouped) = regroup_presets(&raw).unwrap_or((raw, false));
            carried &= regrouped;
            fields.push(("presets".to_owned(), presets));
        }
    }
    if version < 5 {
        // Version 4 kept the metronome's bars of the meter it clicked in
        // its document and those of the others in the settings, saved as
        // four fields; version 5 as one.
        carried &= join_metronome(fields);
    }
    carried
}

/// Moves the metronome's four fields of version 4 into one
/// ([`SavedMetronome`]). Whether each was stored at most once and read.
fn join_metronome(fields: &mut Vec<Field>) -> bool {
    const OLD: [&str; 4] = ["click_bars", "counter_bars", "counter_names", "metronome_meter"];
    if !OLD.iter().any(|name| field(fields, name).is_some()) {
        return true;
    }
    let mut carried = true;
    // Each falls back alone, as it did as a field of its own.
    let clicks: Vec<Measure> = take_read(fields, OLD[0], &mut carried).unwrap_or_default();
    let counters: Vec<Measure> = take_read(fields, OLD[1], &mut carried).unwrap_or_default();
    let names: Vec<(TimeSignature, Ratio)> =
        take_read(fields, OLD[2], &mut carried).unwrap_or_default();
    let meter = take_read(fields, OLD[3], &mut carried).unwrap_or(TimeSignature::FOUR_FOUR);
    let mut click = ClickBars::default();
    let mut meters = vec![meter];
    for bar in clicks {
        meters.push(bar.time_signature());
        click.set(bar);
    }
    meters.extend(counters.iter().map(Measure::time_signature));
    let mut bars: Vec<MeterBars> = Vec::new();
    for ts in meters {
        if bars.iter().any(|b| b.meter() == ts) {
            continue;
        }
        // In today's spelling, and named only as a ratio its bar is.
        let counter = counters.iter().find(|m| m.time_signature() == ts).cloned().map(respelled);
        let name = names.iter().rev().find(|&&(t, _)| t == ts).map(|&(_, r)| r);
        let name = name
            .filter(|&r| counter.as_ref().is_some_and(|c| named_ratio_of(c, Some(r)) == Some(r)));
        bars.push(MeterBars::new(click.for_meter(ts), counter.as_ref(), name));
    }
    bars.retain(|b| *b != MeterBars::plain(b.meter()));
    let saved = SavedMetronome { meter, bars };
    match ron::to_string(&saved) {
        Ok(ron) => push_ron(fields, "metronome", ron) && carried,
        Err(_) => false,
    }
}

/// Moves the flat `fields` named in `groups` into their groups. Whether
/// each was stored at most once, and each group reads as RON.
fn group_fields(fields: &mut Vec<Field>, groups: &[(&str, &[(&str, &str)])]) -> bool {
    let mut carried = true;
    // All taken first: a group may have the name of a field it replaces.
    let grouped: Vec<(&str, Vec<Field>)> = groups
        .iter()
        .map(|(group, members)| {
            let members = members
                .iter()
                .filter_map(|(before, name)| {
                    let (raw, once) = take_last(fields, before);
                    carried &= once;
                    Some(((*name).to_owned(), raw?))
                })
                .collect();
            (*group, members)
        })
        .collect();
    for (group, members) in grouped {
        if !members.is_empty() {
            carried &= push_ron(fields, group, struct_ron(&members));
        }
    }
    carried
}

/// A preset library saved before version 4 with each preset's settings in
/// groups, and whether each field of them was stored once; `None` when it
/// is no library of presets.
fn regroup_presets(raw: &RawValue) -> Option<(Box<RawValue>, bool)> {
    let Fields(mut library) = raw.into_rust().ok()?;
    let mut regrouped = true;
    for (name, raw) in &mut library {
        if name != "presets" {
            continue;
        }
        let presets: Vec<Fields> = raw.into_rust().ok()?;
        let mut items = Vec::new();
        for Fields(mut preset) in presets {
            for (name, raw) in &mut preset {
                if name == "settings" {
                    let Fields(mut settings) = raw.into_rust().ok()?;
                    regrouped &= group_fields(&mut settings, &PRESET_GROUPS);
                    *raw = RawValue::from_boxed_ron(struct_ron(&settings).into_boxed_str()).ok()?;
                }
            }
            items.push(struct_ron(&preset));
        }
        let list = format!("[{}]", items.join(","));
        *raw = RawValue::from_boxed_ron(list.into_boxed_str()).ok()?;
    }
    let library = RawValue::from_boxed_ron(struct_ron(&library).into_boxed_str()).ok()?;
    Some((library, regrouped))
}

/// Adds the field `name` with the value `ron`; whether it reads as RON.
fn push_ron(fields: &mut Vec<(String, Box<RawValue>)>, name: &str, ron: String) -> bool {
    let raw = RawValue::from_boxed_ron(ron.into_boxed_str());
    raw.map(|raw| fields.push((name.to_owned(), raw))).is_ok()
}

/// The stored field `name` with only its own fields that read, and the names
/// of the ones dropped, for a document or a settings group; `None` for any
/// other field, which reads or drops as a whole.
fn repair_field(name: &str, raw: &RawValue) -> Option<(Box<RawValue>, Vec<String>)> {
    match name {
        _ if DOC_FIELDS.contains(&name) => repair::<SavedDoc>(raw),
        "playback" => repair::<PlaybackSettings>(raw),
        "generator" => repair::<GeneratorConfig>(raw),
        "devices" => repair::<DeviceSettings>(raw),
        "feedback" => repair::<FeedbackSettings>(raw),
        "view" => repair::<ViewSettings>(raw),
        "metronome" => repair::<SavedMetronome>(raw),
        _ => None,
    }
}

/// A stored struct `T` (`(…)` or `Some((…))`) with only its fields that read,
/// and the names of the ones dropped; `None` for `None` or for text that is
/// no struct, which then reads or drops as a whole.
fn repair<T: DeserializeOwned>(raw: &RawValue) -> Option<(Box<RawValue>, Vec<String>)> {
    let (Fields(fields), some) = match raw.into_rust::<Fields>() {
        Ok(fields) => (fields, false),
        Err(_) => (raw.into_rust::<Option<Fields>>().ok()??, true),
    };
    let (kept, dropped) = partition_fields::<T>(&fields);
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

/// Takes every field `name` and reads the last one as a `T`, `None` if
/// there is none; clears `carried` if it was stored twice or does not read.
fn take_read<T: DeserializeOwned>(
    fields: &mut Vec<Field>,
    name: &str,
    carried: &mut bool,
) -> Option<T> {
    let (raw, once) = take_last(fields, name);
    let value = raw.map(|raw| raw.into_rust::<T>().ok());
    *carried &= once && value.as_ref().is_none_or(Option::is_some);
    value.flatten()
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
    use super::{
        BACKUP_KEY_PREFIX, DOC_FIELDS, Fields, SETTINGS_GROUPS, StateBackup, field, repair_field,
        restore, struct_ron,
    };
    use crate::guide::Guide;
    use crate::metronome_bars::MeterBars;
    use crate::play_along::{PlayAlongSettings, TapKey};
    use crate::presets::{PresetSettings, builtin_presets};
    use crate::settings::{
        DeviceSettings, FeedbackSettings, GeneratorConfig, PlaybackSettings, ViewSettings,
    };
    use crate::shell::Tab;
    use crate::{
        CountingBase, CountingSettings, PERSISTED_STATE_VERSION, PersistedState, SavedDoc,
        StickingSettings,
    };
    use grooph_audio::{AudioSettings, GapClick, GapMode, Sound, TempoRamp};
    use grooph_measure::click::{ClickLevel, meter_click_bar};
    use grooph_measure::counter::{Ratio, counter_bar, counter_ratios};
    use grooph_measure::counting::CountingSystem;
    use grooph_measure::duration::e;
    use grooph_measure::generator::{GeneratorSettings, Subdivision};
    use grooph_measure::swing::{Swing, SwingUnit};
    use grooph_measure::{Beat, Cursor, Measure, Score, TimeSignature};
    use ron::value::RawValue;
    use std::collections::HashMap;

    /// A saved state with a pattern in the library, a user preset and a
    /// non-default tempo and tab.
    fn saved_state() -> PersistedState {
        let mut s = PersistedState { tab: Tab::Score, ..Default::default() };
        s.playback.bpm = 93;
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
        assert_eq!(s.playback.bpm, 93);
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
        assert_eq!(s.playback.bpm, 93);
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
            assert_eq!(s.playback.bpm, 93);
            assert_eq!(backup.unwrap().ron, saved);
        }
    }

    /// The fields of a version 2 state: the score tab's document flat, and
    /// Practice's rhythm as `practice_score` if `practice`.
    fn version_2_state(practice: bool) -> String {
        // The settings flat as in version 3, the score tab's document as
        // `saved_state`'s.
        let state = saved_state();
        let fields = STATE_V3;
        let saved_doc = format!("score_doc:{}", ron::to_string(&state.score_doc).unwrap());
        assert!(fields.contains(&saved_doc));
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
        fields.replacen("version:3", "version:2", 1)
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
            assert_eq!(s.playback.bpm, 93);
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
        let saved = saved_ron().replacen('(', "(playback:(bpm:60),", 1);
        let (s, backup) = restore(Some(&saved));
        assert_eq!(s.playback.bpm, 93);
        assert_library_and_presets_kept(&s);
        assert_eq!(backup.unwrap().ron, saved);
    }

    #[test]
    fn text_that_does_not_read_is_backed_up() {
        let (s, backup) = restore(Some("(bpm: 93, library: ("));
        assert_eq!(s.playback.bpm, PersistedState::default().playback.bpm);
        assert_eq!(backup.unwrap().ron, "(bpm: 93, library: (");
    }

    /// A state saved with version 3 (`testdata/state_v3.ron`, written by
    /// the app before the settings groups) in which every setting differs
    /// from its default, and a user preset's too; with a pattern in the
    /// library.
    const STATE_V3: &str = include_str!("testdata/state_v3.ron");

    /// The settings of [`STATE_V3`], in groups.
    fn every_setting_changed() -> PersistedState {
        let mut s = PersistedState {
            playback: PlaybackSettings {
                bpm: 93,
                count_in: true,
                swing: Swing { unit: SwingUnit::Sixteenths, percent: 61 },
                gap_click: GapClick {
                    enabled: true,
                    mode: GapMode::Random,
                    on_bars: 3,
                    off_bars: 1,
                    ..GapClick::default()
                },
                tempo_ramp: TempoRamp {
                    enabled: true,
                    target_bpm: 140,
                    step_bpm: 4,
                    every: 3,
                    keep_reached: true,
                    ..TempoRamp::default()
                },
            },
            generator: GeneratorConfig {
                settings: GeneratorSettings {
                    subdivision: Subdivision::Triplets,
                    complexity: 3,
                    bars: 4,
                    time_signature: TimeSignature::THREE_FOUR,
                    space: 0.6,
                    custom_figures_enabled: true,
                    ..GeneratorSettings::default()
                },
                endless: true,
                ghost_notes: true,
            },
            devices: DeviceSettings {
                latency_enabled: false,
                offset: 0.042,
                midi_port_id: Some("port-7".into()),
            },
            feedback: FeedbackSettings {
                accuracy_enabled: true,
                flash_enabled: true,
                play_along: PlayAlongSettings {
                    tap_enabled: false,
                    tap_key: TapKey::D,
                    mic_enabled: true,
                    midi_offset_ms: 12.5,
                    ..PlayAlongSettings::default()
                },
            },
            view: ViewSettings {
                counting: CountingSettings {
                    base: CountingBase::Triplet,
                    system: CountingSystem::Takadimi,
                    above: true,
                },
                sticking: StickingSettings { shown: false, above: true },
                guide: Guide::EverySecondBar,
                show_score_tab: true,
            },
            audio_settings: AudioSettings {
                downbeat: 0.31,
                metronome_sound: Sound::Claves,
                ..AudioSettings::default()
            },
            tab: Tab::Practice,
            editing: true,
            ..PersistedState::default()
        };
        let mut preset = builtin_presets()[1].1;
        preset.playback.bpm = 77;
        preset.playback.count_in = false;
        preset.playback.tempo_ramp.enabled = true;
        preset.playback.gap_click.enabled = true;
        preset.generator.endless = true;
        preset.metronome_sound = Sound::Xylophone;
        preset.rhythm_sound = Sound::Click;
        s.presets.add("Mine", preset);
        let mut m = Measure::new(TimeSignature::THREE_FOUR);
        m.first_voice_mut().set_beat(0, Beat::note(e())).unwrap();
        s.library.add("Groove".into(), Score::single(m), 93);
        s
    }

    /// The fields of a struct's RON that differ from those of `default`.
    fn changed_fields(value: &str, default: &str) -> Vec<String> {
        let Fields(value) = ron::from_str(value).unwrap();
        let Fields(default) = ron::from_str(default).unwrap();
        value
            .iter()
            .filter(|(name, raw)| {
                field(&default, name).is_none_or(|d| d.get_ron() != raw.get_ron())
            })
            .map(|(name, _)| name.clone())
            .collect()
    }

    #[test]
    fn every_settings_group_is_restored_field_by_field() {
        // A group without its arm in `repair_field` would read as a whole.
        let saved = ron::to_string(&every_setting_changed()).unwrap();
        let Fields(fields) = ron::from_str(&saved).unwrap();
        for (group, members) in SETTINGS_GROUPS {
            let raw = field(&fields, group).unwrap_or_else(|| panic!("{group} is saved"));
            let (_, broken) = members[0];
            let Fields(mut inner) = raw.into_rust().unwrap();
            let value = inner.iter_mut().find(|(name, _)| name == broken).unwrap();
            value.1 = RawValue::from_boxed_ron("Unknown".into()).unwrap();
            let raw = RawValue::from_boxed_ron(struct_ron(&inner).into_boxed_str()).unwrap();
            let (_, dropped) = repair_field(group, &raw).unwrap_or_else(|| panic!("{group}"));
            assert_eq!(dropped, [broken], "{group}");
        }
    }

    #[test]
    fn every_setting_of_the_test_state_differs_from_its_default() {
        let s = every_setting_changed();
        let d = PersistedState::default();
        let groups = [
            (ron::to_string(&s.playback), ron::to_string(&d.playback)),
            (ron::to_string(&s.generator), ron::to_string(&d.generator)),
            (ron::to_string(&s.devices), ron::to_string(&d.devices)),
            (ron::to_string(&s.feedback), ron::to_string(&d.feedback)),
            (ron::to_string(&s.view), ron::to_string(&d.view)),
        ];
        for ((value, default), (group, members)) in groups.into_iter().zip(SETTINGS_GROUPS) {
            let (value, default) = (value.unwrap(), default.unwrap());
            let all: Vec<_> = members.iter().map(|(_, name)| name.to_string()).collect();
            assert_eq!(changed_fields(&value, &default), all, "{group}");
        }
        // The user preset, in every group.
        let preset = ron::to_string(&s.presets.presets[0].settings).unwrap();
        let default = ron::to_string(&PresetSettings::default()).unwrap();
        let all = ["playback", "generator", "metronome_sound", "rhythm_sound"];
        assert_eq!(changed_fields(&preset, &default), all);
        assert_ne!(s.tab, d.tab);
        assert_ne!(s.editing, d.editing);
    }

    #[test]
    fn a_version_3_state_is_migrated_without_loss() {
        assert!(STATE_V3.starts_with("(version:3,"));
        let (s, backup) = restore(Some(STATE_V3));
        assert!(backup.is_none());
        assert_eq!(s.version, PERSISTED_STATE_VERSION);
        let want = ron::to_string(&every_setting_changed()).unwrap();
        assert_eq!(ron::to_string(&s).unwrap(), want);
    }

    /// The metronome fields of a version 4 state: 3/4 with an edited click
    /// bar, 4/4 with a counter rhythm picked as 6:4 (3:2's bar), and 7/8,
    /// clicked, with both. Also the bars each meter has then.
    fn version_4_metronome() -> (String, TimeSignature, Vec<MeterBars>) {
        let (three, four, seven) =
            (TimeSignature::THREE_FOUR, TimeSignature::FOUR_FOUR, TimeSignature::SEVEN_EIGHT);
        let mut click_3 = meter_click_bar(three);
        click_3.first_voice_mut().set_click_level(1, ClickLevel::Off);
        let mut click_7 = meter_click_bar(seven);
        click_7.first_voice_mut().set_click_level(0, ClickLevel::Off);
        let six_four = Ratio { n: 6, span: 4 };
        let counter_4 = counter_bar(four, six_four).unwrap();
        let ratio_7 = counter_ratios(seven)[0];
        let mut counter_7 = counter_bar(seven, ratio_7).unwrap();
        counter_7.first_voice_mut().set_click_level(0, ClickLevel::Strong);
        let fields = format!(
            "click_bars:{},counter_bars:{},counter_names:{},metronome_meter:{}",
            ron::to_string(&vec![&click_3, &click_7]).unwrap(),
            ron::to_string(&vec![&counter_4, &counter_7]).unwrap(),
            ron::to_string(&vec![(four, six_four)]).unwrap(),
            ron::to_string(&seven).unwrap(),
        );
        let bars = vec![
            MeterBars::new(click_3, None, None),
            MeterBars::new(meter_click_bar(four), Some(&counter_4), Some(six_four)),
            MeterBars::new(click_7, Some(&counter_7), None),
        ];
        (fields, seven, bars)
    }

    #[test]
    fn a_version_4_metronome_is_migrated_without_loss() {
        let (fields, meter, bars) = version_4_metronome();
        let (s, backup) = restore(Some(&format!("(version:4,{fields})")));
        assert!(backup.is_none());
        let doc = s.metronome.into_doc();
        assert_eq!(doc.clicked_bars().meter(), meter);
        for want in bars {
            assert_eq!(doc.meter_bars(want.meter()), want);
        }
        assert_eq!(
            doc.meter_bars(TimeSignature::TWO_FOUR),
            MeterBars::plain(TimeSignature::TWO_FOUR)
        );
    }

    #[test]
    fn a_version_4_metronome_field_that_does_not_read_falls_back_alone() {
        let (fields, meter, bars) = version_4_metronome();
        let names = fields.find("counter_names:").unwrap();
        let end = names + fields[names..].find(",metronome_meter").unwrap();
        let broken = format!("{}counter_names:Unknown{}", &fields[..names], &fields[end..]);
        let (s, backup) = restore(Some(&format!("(version:4,{broken})")));
        assert!(backup.is_some());
        let doc = s.metronome.into_doc();
        assert_eq!(doc.clicked_bars().meter(), meter);
        for want in bars {
            // Only the name is gone.
            let want = MeterBars { name: None, ..want };
            assert_eq!(doc.meter_bars(want.meter()), want);
        }
    }

    #[test]
    fn the_metronome_restores_field_by_field() {
        let (fields, _, _) = version_4_metronome();
        let (s, _) = restore(Some(&format!("(version:4,{fields})")));
        let saved = ron::to_string(&s).unwrap();
        assert_eq!(restore(Some(&saved)).0.metronome, s.metronome);
        let broken = saved.replacen("metronome:(meter:", "metronome:(meter:Unknown,old:", 1);
        let (restored, backup) = restore(Some(&broken));
        assert!(backup.is_some());
        // The meter falls back alone; the bars stay.
        assert_eq!(restored.metronome.meter, TimeSignature::FOUR_FOUR);
        assert_eq!(restored.metronome.bars, s.metronome.bars);
    }

    #[test]
    fn a_state_of_this_version_restores_every_setting() {
        let saved = ron::to_string(&every_setting_changed()).unwrap();
        let (s, backup) = restore(Some(&saved));
        assert!(backup.is_none());
        assert_eq!(ron::to_string(&s).unwrap(), saved);
    }

    #[test]
    fn a_setting_that_does_not_read_falls_back_alone() {
        // In a group, and in a flat field of version 3 that goes into one.
        let current = ron::to_string(&every_setting_changed()).unwrap();
        for (saved, broken) in [
            (current.as_str(), "guide:EverySecondBar"),
            (STATE_V3, "guide:EverySecondBar"),
            (current.as_str(), "swing:(unit:Sixteenths,percent:61)"),
            (STATE_V3, "swing:(unit:Sixteenths,percent:61)"),
        ] {
            assert_eq!(saved.matches(broken).count(), 1, "{broken}");
            let saved = saved.replace(broken, &broken.replace(':', ":Unknown"));
            let (s, backup) = restore(Some(&saved));
            let mut want = every_setting_changed();
            want.view.guide = Guide::default();
            if broken.starts_with("swing") {
                want.view.guide = Guide::EverySecondBar;
                want.playback.swing = Swing::default();
            }
            assert_eq!(ron::to_string(&s).unwrap(), ron::to_string(&want).unwrap(), "{broken}");
            assert_eq!(backup.unwrap().ron, saved);
        }
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
        assert_eq!(s.playback.bpm, 87);
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
