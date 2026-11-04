//! Generator presets in the app: take a snapshot of the current settings,
//! apply a preset, save and delete one.

use crate::Grooph;
use crate::presets::{self, PresetId, PresetSettings};
use crate::score_change::ScoreChange;

impl Grooph {
    /// The current settings as a preset.
    pub(crate) fn preset_snapshot(&self) -> PresetSettings {
        let sounds = &self.playback_ctl.audio_cfg.settings;
        PresetSettings {
            playback: self.playback_ctl.settings,
            generator: self.editor.generator.config,
            metronome_sound: sounds.metronome_sound,
            rhythm_sound: sounds.rhythm_sound,
        }
    }

    /// Restore every setting of preset `id` and generate a new rhythm with it.
    pub(crate) fn apply_preset(&mut self, id: PresetId) {
        let Some((_, preset)) = presets::lookup(&self.editor.presets, id) else {
            return;
        };
        self.editor.generator.config = preset.generator;
        let sounds = &mut self.playback_ctl.audio_cfg.settings;
        sounds.metronome_sound = preset.metronome_sound;
        sounds.rhythm_sound = preset.rhythm_sound;
        let tempo_changed = self.playback_ctl.settings.bpm != preset.playback.bpm;
        self.playback_ctl.settings = preset.playback;
        if tempo_changed {
            self.score_changed(ScoreChange::Tempo);
        }
        self.editor.active_preset = Some(id);
        self.generate_new_score();
    }

    /// First preset whose settings equal the current ones.
    pub(crate) fn matching_preset(&self) -> Option<PresetId> {
        let snapshot = self.preset_snapshot();
        presets::all(&self.editor.presets).into_iter().map(|(id, _)| id).find(|&id| {
            presets::lookup(&self.editor.presets, id).is_some_and(|(_, s)| s == snapshot)
        })
    }

    /// Store the current settings as a new preset and make it the active one.
    pub(crate) fn save_preset_as(&mut self, name: &str) -> u64 {
        let id = self.editor.presets.add(name, self.preset_snapshot());
        self.editor.active_preset = Some(PresetId::User(id));
        id
    }

    pub(crate) fn delete_preset(&mut self, id: u64) {
        self.editor.presets.remove(id);
        if self.editor.active_preset == Some(PresetId::User(id)) {
            self.editor.active_preset = None;
        }
    }

    /// Name of the active preset, with "•" when the settings differ from it.
    pub(crate) fn active_preset_label(&self) -> String {
        let active =
            self.editor.active_preset.and_then(|id| presets::lookup(&self.editor.presets, id));
        match active {
            Some((name, settings)) if settings == self.preset_snapshot() => name,
            Some((name, _)) => format!("{name} •"),
            None => "None".to_string(),
        }
    }
}
