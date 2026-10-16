//! Generator presets in the app: take a snapshot of the current settings,
//! apply a preset, and the preset controls of the generator panel.

use crate::Grooph;
use crate::presets::{self, PresetId, PresetSettings};
use eframe::egui;

impl Grooph {
    /// The current settings as a preset.
    pub(crate) fn preset_snapshot(&self) -> PresetSettings {
        let generator = &self.editor.generator;
        let audio = &self.playback_ctl.audio_cfg;
        PresetSettings {
            generator: generator.settings,
            bpm: self.playback_ctl.bpm,
            swing: audio.swing,
            ghost_notes: generator.ghost_notes,
            count_in: audio.count_in,
            endless: generator.endless,
            gap_click: audio.gap,
            tempo_ramp: audio.ramp,
            metronome_sound: audio.settings.metronome_sound,
            rhythm_sound: audio.settings.rhythm_sound,
        }
    }

    /// Restore every setting of preset `id` and generate a new rhythm with it.
    pub(crate) fn apply_preset(&mut self, id: PresetId) {
        let Some((_, preset)) = presets::lookup(&self.editor.presets, id) else {
            return;
        };
        let generator = &mut self.editor.generator;
        generator.settings = preset.generator;
        generator.ghost_notes = preset.ghost_notes;
        generator.endless = preset.endless;
        let audio = &mut self.playback_ctl.audio_cfg;
        audio.swing = preset.swing;
        audio.count_in = preset.count_in;
        audio.gap = preset.gap_click;
        audio.ramp = preset.tempo_ramp;
        audio.settings.metronome_sound = preset.metronome_sound;
        audio.settings.rhythm_sound = preset.rhythm_sound;
        if self.playback_ctl.bpm != preset.bpm {
            self.playback_ctl.bpm = preset.bpm;
            self.handle_bpm_change();
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
    pub(crate) fn save_preset_as(&mut self, name: &str) {
        let id = self.editor.presets.add(name, self.preset_snapshot());
        self.editor.active_preset = Some(PresetId::User(id));
    }

    /// Overwrite the active user preset, or save a new one when a built-in
    /// preset or none is active.
    pub(crate) fn save_preset(&mut self, name: &str) {
        let snapshot = self.preset_snapshot();
        match self.editor.active_preset {
            Some(PresetId::User(id)) if self.editor.presets.update(id, snapshot) => {}
            _ => self.save_preset_as(name),
        }
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

    /// Preset dropdown; returns the preset picked from it.
    pub(crate) fn preset_combo(&self, ui: &mut egui::Ui) -> Option<PresetId> {
        let mut picked = None;
        egui::ComboBox::from_id_salt("gen_preset")
            .selected_text(self.active_preset_label())
            .show_ui(ui, |ui| {
                for (id, name) in presets::all(&self.editor.presets) {
                    let active = self.editor.active_preset == Some(id);
                    if ui.selectable_label(active, name).clicked() {
                        picked = Some(id);
                    }
                }
            });
        picked
    }

    /// Save, rename and delete presets. Returns a preset to load.
    pub(crate) fn preset_manager(&mut self, ui: &mut egui::Ui) -> Option<PresetId> {
        let mut to_load = None;
        let mut to_delete = None;

        ui.horizontal(|ui| {
            let save_label = match self.editor.active_preset {
                Some(PresetId::User(_)) => "💾 Save",
                _ => "💾 Save (new)",
            };
            if ui
                .button(save_label)
                .on_hover_text("Store the current generator and playback settings")
                .clicked()
            {
                let name = std::mem::take(&mut self.ui.preset_name_buffer);
                self.save_preset(&name);
            }
            ui.separator();
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.ui.preset_name_buffer)
                    .hint_text("New name…")
                    .desired_width(160.0),
            );
            let submit = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("➕ Save as").clicked() || submit {
                let name = std::mem::take(&mut self.ui.preset_name_buffer);
                self.save_preset_as(&name);
            }
        });

        let active = self.editor.active_preset;
        egui::ScrollArea::vertical().id_salt("gen_preset_list").max_height(240.0).show(ui, |ui| {
            for (i, (name, _)) in presets::builtin_presets().into_iter().enumerate() {
                let id = PresetId::BuiltIn(i);
                ui.horizontal(|ui| {
                    if ui.selectable_label(active == Some(id), "▶ Load").clicked() {
                        to_load = Some(id);
                    }
                    ui.label(name);
                    ui.weak("built-in");
                });
            }
            for preset in &mut self.editor.presets.presets {
                let id = PresetId::User(preset.id);
                ui.horizontal(|ui| {
                    if ui.selectable_label(active == Some(id), "▶ Load").clicked() {
                        to_load = Some(id);
                    }
                    ui.add(egui::TextEdit::singleline(&mut preset.name).desired_width(160.0));
                    if ui.button("🗑").on_hover_text("Delete").clicked() {
                        to_delete = Some(preset.id);
                    }
                });
            }
        });

        if let Some(id) = to_delete {
            self.delete_preset(id);
        }
        to_load
    }
}
