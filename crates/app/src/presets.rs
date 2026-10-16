//! Generator presets: named snapshots of the generator settings and the
//! playback extras practised with them (tempo, swing, ghost notes, count-in,
//! endless mode, gap click, tempo ramp and the sound of both voices).
//!
//! A few built-in presets ship with the app; they are not stored, so they
//! cannot be deleted or renamed and always come back unchanged. User presets
//! live in [`PresetLibrary`] and are persisted with the app state.

use grooph_audio::{GapClick, GapMode, GapMute, MetronomeSound, RhythmSound, TempoRamp};
use grooph_measure::TimeSignature;
use grooph_measure::generator::{GeneratorSettings, Subdivision};
use grooph_measure::swing::{Swing, SwingUnit};
use serde::{Deserialize, Serialize};

/// Everything a preset restores.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PresetSettings {
    pub(crate) generator: GeneratorSettings,
    pub(crate) bpm: u32,
    pub(crate) swing: Swing,
    pub(crate) ghost_notes: bool,
    pub(crate) count_in: bool,
    pub(crate) endless: bool,
    pub(crate) gap_click: GapClick,
    pub(crate) tempo_ramp: TempoRamp,
    pub(crate) metronome_sound: MetronomeSound,
    pub(crate) rhythm_sound: RhythmSound,
}

impl Default for PresetSettings {
    fn default() -> Self {
        Self {
            generator: GeneratorSettings::default(),
            bpm: 120,
            swing: Swing::default(),
            ghost_notes: false,
            count_in: false,
            endless: false,
            gap_click: GapClick::default(),
            tempo_ramp: TempoRamp::default(),
            metronome_sound: MetronomeSound::default(),
            rhythm_sound: RhythmSound::default(),
        }
    }
}

/// A built-in preset (index into [`builtin_presets`]) or a user preset (its id).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PresetId {
    BuiltIn(usize),
    User(u64),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct UserPreset {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) settings: PresetSettings,
}

/// The user's presets. `next_id` only grows, so ids stay stable.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct PresetLibrary {
    pub(crate) presets: Vec<UserPreset>,
    pub(crate) next_id: u64,
}

impl PresetLibrary {
    /// Add a preset; an empty name becomes "Preset N".
    pub(crate) fn add(&mut self, name: &str, settings: PresetSettings) -> u64 {
        let name = match name.trim() {
            "" => format!("Preset {}", self.presets.len() + 1),
            name => name.to_string(),
        };
        let id = self.next_id;
        self.next_id += 1;
        self.presets.push(UserPreset { id, name, settings });
        id
    }

    /// Overwrite the settings of preset `id`, keeping its name.
    pub(crate) fn update(&mut self, id: u64, settings: PresetSettings) -> bool {
        match self.presets.iter_mut().find(|p| p.id == id) {
            Some(preset) => {
                preset.settings = settings;
                true
            }
            None => false,
        }
    }

    pub(crate) fn remove(&mut self, id: u64) {
        self.presets.retain(|p| p.id != id);
    }

    pub(crate) fn get(&self, id: u64) -> Option<&UserPreset> {
        self.presets.iter().find(|p| p.id == id)
    }
}

/// Name and settings of a preset, built-in or user, if it exists.
pub(crate) fn lookup(library: &PresetLibrary, id: PresetId) -> Option<(String, PresetSettings)> {
    match id {
        PresetId::BuiltIn(i) => builtin_presets().get(i).map(|(n, s)| (n.to_string(), *s)),
        PresetId::User(id) => library.get(id).map(|p| (p.name.clone(), p.settings)),
    }
}

/// Every preset in display order: built-in first, then the user's.
pub(crate) fn all(library: &PresetLibrary) -> Vec<(PresetId, String)> {
    let builtin = builtin_presets()
        .into_iter()
        .enumerate()
        .map(|(i, (name, _))| (PresetId::BuiltIn(i), name.to_string()));
    let user = library.presets.iter().map(|p| (PresetId::User(p.id), p.name.clone()));
    builtin.chain(user).collect()
}

fn generator(
    subdivision: Subdivision,
    complexity: u8,
    bars: usize,
    time_signature: TimeSignature,
    space: f32,
) -> GeneratorSettings {
    GeneratorSettings {
        subdivision,
        complexity,
        bars,
        time_signature,
        space,
        ..GeneratorSettings::default()
    }
}

/// The presets shipped with the app.
pub(crate) fn builtin_presets() -> Vec<(&'static str, PresetSettings)> {
    let four_four = TimeSignature::FOUR_FOUR;
    let base = PresetSettings { count_in: true, ..PresetSettings::default() };
    vec![
        (
            "8ths warm-up",
            PresetSettings {
                generator: generator(Subdivision::Eighths, 1, 4, four_four, 0.2),
                bpm: 80,
                ..base
            },
        ),
        (
            "16th funk",
            PresetSettings {
                generator: generator(Subdivision::Sixteenths, 3, 2, four_four, 0.3),
                bpm: 90,
                swing: Swing { unit: SwingUnit::Sixteenths, percent: 56 },
                ghost_notes: true,
                ..base
            },
        ),
        (
            "Triplet shuffle",
            PresetSettings {
                generator: generator(Subdivision::Triplets, 2, 4, four_four, 0.2),
                bpm: 90,
                ..base
            },
        ),
        (
            "Sparse reading",
            PresetSettings {
                generator: generator(Subdivision::Sixteenths, 2, 4, four_four, 0.7),
                bpm: 70,
                endless: true,
                ..base
            },
        ),
        (
            "7/8 groove",
            PresetSettings {
                generator: generator(
                    Subdivision::Sixteenths,
                    2,
                    4,
                    TimeSignature { beats: 7, beat_unit: 8 },
                    0.2,
                ),
                bpm: 100,
                ..base
            },
        ),
        (
            "Inner time (gap click)",
            PresetSettings {
                generator: generator(Subdivision::Eighths, 2, 4, four_four, 0.2),
                bpm: 90,
                gap_click: GapClick {
                    enabled: true,
                    mode: GapMode::Pattern,
                    on_bars: 2,
                    off_bars: 2,
                    mute: GapMute::Metronome,
                    ..GapClick::default()
                },
                ..base
            },
        ),
        (
            "Speed builder",
            PresetSettings {
                generator: generator(Subdivision::Sixteenths, 1, 2, four_four, 0.1),
                bpm: 70,
                tempo_ramp: TempoRamp {
                    enabled: true,
                    target_bpm: 110,
                    step_bpm: 5,
                    every: 4,
                    ..TempoRamp::default()
                },
                ..base
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use grooph_measure::generator::supports_time_signature;
    use grooph_measure::tempo::{MAX_BPM, MIN_BPM};

    #[test]
    fn add_names_and_numbers_presets() {
        let mut lib = PresetLibrary::default();
        let a = lib.add("  ", PresetSettings::default());
        let b = lib.add("Funk", PresetSettings::default());
        assert_eq!((a, b), (0, 1));
        assert_eq!(lib.get(a).unwrap().name, "Preset 1");
        assert_eq!(lib.get(b).unwrap().name, "Funk");
    }

    #[test]
    fn update_and_remove_touch_only_their_preset() {
        let mut lib = PresetLibrary::default();
        let a = lib.add("a", PresetSettings::default());
        let b = lib.add("b", PresetSettings::default());
        let faster = PresetSettings { bpm: 150, ..PresetSettings::default() };
        assert!(lib.update(b, faster));
        assert!(!lib.update(42, faster));
        assert_eq!(lib.get(a).unwrap().settings.bpm, 120);
        assert_eq!(lib.get(b).unwrap().settings.bpm, 150);
        lib.remove(a);
        assert!(lib.get(a).is_none());
        // Ids are not reused after a delete.
        assert_eq!(lib.add("c", PresetSettings::default()), 2);
    }

    #[test]
    fn presets_round_trip_through_serde() {
        let mut lib = PresetLibrary::default();
        for (name, settings) in builtin_presets() {
            lib.add(name, settings);
        }
        let json = ron::to_string(&lib).unwrap();
        let back: PresetLibrary = ron::from_str(&json).unwrap();
        assert_eq!(back, lib);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let settings: PresetSettings = ron::from_str("(bpm: 95)").unwrap();
        assert_eq!(settings, PresetSettings { bpm: 95, ..PresetSettings::default() });
    }

    #[test]
    fn builtin_presets_are_playable() {
        for (name, settings) in builtin_presets() {
            assert!(supports_time_signature(settings.generator.time_signature), "{name}");
            assert!((MIN_BPM..=MAX_BPM).contains(&settings.bpm), "{name}");
            let mut rng = grooph_measure::generator::Rng::new(1);
            grooph_measure::generator::generate_score(&settings.generator, &mut rng)
                .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        }
    }

    #[test]
    fn all_lists_builtin_then_user_presets() {
        let mut lib = PresetLibrary::default();
        let id = lib.add("Mine", PresetSettings::default());
        let all = all(&lib);
        assert_eq!(all[0], (PresetId::BuiltIn(0), "8ths warm-up".to_string()));
        assert_eq!(all.last().unwrap(), &(PresetId::User(id), "Mine".to_string()));
        assert_eq!(lookup(&lib, PresetId::User(id)).unwrap().0, "Mine");
        assert!(lookup(&lib, PresetId::User(99)).is_none());
    }
}
