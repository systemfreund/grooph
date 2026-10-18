//! Picker dialogs for single settings (meter, notes, level, bars, tempo,
//! swing, preset): a grid of big buttons that works by touch, opened from the
//! settings card and the options sheet. Also the grouping picker of the level
//! dialog.

use crate::Grooph;
use crate::generator::ENDLESS_MIN_BARS;
use crate::metronome_view::{click_fills, same_bar};
use crate::presets::{self, PresetId};
use crate::tool_palette::notation_button;
use eframe::egui;
use eframe::egui::{Button, Color32, FontId, Id, Modal, Response, RichText, Ui, vec2};
use grooph_audio::CounterSound;
use grooph_measure::TimeSignature;
use grooph_measure::counter::{Ratio, ratio_of};
use grooph_measure::generator::{
    BeatSpan, FigureSet, MAX_BARS, MAX_COMPLEXITY, Subdivision, complexity_figures,
    effective_subdivision, figures, offered_subdivisions, supports_time_signature,
};
use grooph_measure::swing::{MAX_SWING_PERCENT, MIN_SWING_PERCENT, SwingUnit};
use grooph_measure::tap_tempo::{TapPulse, tap_pulse};
use grooph_measure::tempo::{MAX_BPM, MIN_BPM};

/// Height of a touch-friendly row or button.
pub(crate) const ROW_HEIGHT: f32 = 44.0;
/// Swing amounts offered in the swing picker (percent of a pair).
const SWING_PRESETS: [u8; 6] = [MIN_SWING_PERCENT, 55, 60, 66, 70, MAX_SWING_PERCENT];

/// Setting edited in the picker dialog ([`Grooph::settings_picker`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Picker {
    TimeSignature,
    Subdivision,
    Complexity,
    Bars,
    Bpm,
    Swing,
    Preset,
    /// The metronome tab's meter.
    MetronomeMeter,
    /// A ready-made click bar for the metronome meter.
    ClickFill,
    /// The metronome's counter rhythm and its sound.
    Polyrhythm,
}

pub(crate) fn swing_label(percent: u8) -> String {
    if percent <= MIN_SWING_PERCENT { "Off".to_owned() } else { format!("{percent}%") }
}

/// Time signatures in the picker: every meter the editor offers in each of
/// its groupings (x/8 and x/16 are written in beat groups, e.g. 6/8 = 3+3,
/// 7/16 = 2+3+2 or 2+2+3).
pub(crate) fn time_signatures() -> Vec<TimeSignature> {
    all_time_signatures().into_iter().filter(|ts| supports_time_signature(*ts)).collect()
}

/// Every meter the editor offers, in each of its groupings.
pub(crate) fn all_time_signatures() -> Vec<TimeSignature> {
    TimeSignature::BEAT_UNITS
        .into_iter()
        .flat_map(|beat_unit| {
            TimeSignature::beat_range(beat_unit)
                .into_iter()
                .flatten()
                .map(move |beats| TimeSignature::new(beats, beat_unit))
        })
        .flat_map(|ts| (0..ts.groupings().len() as u8).map(move |g| ts.with_grouping(g)))
        .collect()
}

/// Beat groups shown next to an x/8 or x/16 meter, e.g. "7/8 (2+3+2)".
pub(crate) fn time_signature_label(ts: TimeSignature) -> String {
    let meter = format!("{}/{}", ts.beats, ts.beat_unit);
    if ts.beat_unit == 4 || ts.groups().len() < 2 {
        return meter;
    }
    format!("{meter} ({})", ts.groups_label())
}

pub(crate) fn subdivision_label(s: Subdivision) -> &'static str {
    match s {
        Subdivision::Eighths => "8th Notes",
        Subdivision::Sixteenths => "16th Notes",
        Subdivision::Triplets => "Triplets",
        Subdivision::Mixed => "Mixed",
    }
}

/// Grid of option buttons; returns the one that was tapped.
fn choice_grid<T: Copy + PartialEq>(
    ui: &mut Ui,
    options: &[(T, String)],
    current: T,
    columns: usize,
) -> Option<T> {
    let mut picked = None;
    let spacing = ui.spacing().item_spacing.x;
    let width = (ui.available_width() - spacing * (columns as f32 - 1.0)) / columns as f32;
    for row in options.chunks(columns) {
        ui.horizontal(|ui| {
            for (value, label) in row {
                // Long labels (e.g. "13/16 (3+3+3+2+2)") shrink to fit the cell.
                let text_width = ui
                    .painter()
                    .layout_no_wrap(label.clone(), FontId::proportional(18.0), Color32::WHITE)
                    .size()
                    .x;
                let room = width - 2.0 * ui.spacing().button_padding.x;
                let size = if text_width > room { 18.0 * room / text_width } else { 18.0 };
                let button = Button::new(RichText::new(label).size(size))
                    .selected(*value == current)
                    .min_size(vec2(width, ROW_HEIGHT));
                if ui.add(button).clicked() {
                    picked = Some(*value);
                }
            }
        });
    }
    picked
}

/// Full-width button with a centered label.
fn wide_button(ui: &mut Ui, label: &str, size: f32, height: f32) -> Response {
    let width = ui.available_width();
    ui.horizontal(|ui| {
        ui.add(Button::new(RichText::new(label).size(size)).min_size(vec2(width, height)))
    })
    .inner
}

impl Grooph {
    /// Dialog for the setting in [`crate::state::UiShell::picker`], opened
    /// from the settings card and the options sheet.
    pub(crate) fn settings_picker(&mut self, ui: &mut Ui) {
        let Some(picker) = self.ui.picker else {
            return;
        };
        let before = self.editor.generator.settings;
        let endless_before = self.editor.generator.endless;
        let width = (ui.ctx().content_rect().width() - 48.0).clamp(200.0, 380.0);
        let mut preset = None;

        let modal = Modal::new(Id::new("settings_picker")).show(ui.ctx(), |ui| {
            ui.set_width(width);
            let title = match picker {
                Picker::TimeSignature => "Time signature",
                Picker::Subdivision => "Notes",
                Picker::Complexity => "Level",
                Picker::Bars => "Bars",
                Picker::Bpm => "Tempo",
                Picker::Swing => "Swing",
                Picker::Preset => "Preset",
                Picker::MetronomeMeter => "Time signature",
                Picker::ClickFill => "Clicks",
                Picker::Polyrhythm => "Polyrhythm",
            };
            ui.heading(title);
            ui.add_space(8.0);
            let picked = match picker {
                Picker::TimeSignature => self.time_signature_choices(ui),
                Picker::Subdivision => self.subdivision_choices(ui),
                Picker::Complexity => self.complexity_choices(ui),
                Picker::Bars => self.bars_choices(ui),
                Picker::Bpm => {
                    self.bpm_editor(ui);
                    false
                }
                Picker::Swing => {
                    self.swing_choices(ui);
                    false
                }
                Picker::Preset => {
                    preset = self.preset_choices(ui);
                    preset.is_some()
                }
                Picker::MetronomeMeter => self.metronome_meter_choices(ui),
                Picker::ClickFill => self.click_fill_choices(ui),
                Picker::Polyrhythm => self.polyrhythm_choices(ui),
            };
            ui.add_space(12.0);
            picked || wide_button(ui, "Done", 19.0, ROW_HEIGHT).clicked()
        });
        if modal.inner || modal.should_close() {
            self.ui.picker = None;
            self.ui.tap_tempo.reset();
            self.ui.renaming_preset = None;
        }

        self.finish_generator_change(before, endless_before, false);
        if let Some(id) = preset {
            self.apply_preset(id);
        }
    }

    /// Presets to load; saving, updating, renaming and deleting the user's own.
    fn preset_choices(&mut self, ui: &mut Ui) -> Option<PresetId> {
        let current = self.editor.active_preset;
        let builtin: Vec<_> = presets::builtin_presets()
            .into_iter()
            .enumerate()
            .map(|(i, (name, _))| (Some(PresetId::BuiltIn(i)), name.to_owned()))
            .collect();
        let mut picked = None;
        egui::ScrollArea::vertical().id_salt("preset_list").max_height(320.0).show(ui, |ui| {
            // The user's own presets first: they are the ones picked most.
            for preset in &self.editor.presets.presets {
                let id = PresetId::User(preset.id);
                ui.horizontal(|ui| {
                    let name_width =
                        ui.available_width() - ROW_HEIGHT - ui.spacing().item_spacing.x - 4.0;
                    let name = Button::new(RichText::new(&preset.name).size(18.0))
                        .selected(current == Some(id))
                        .truncate();
                    if ui.add_sized(vec2(name_width, ROW_HEIGHT), name).clicked() {
                        picked = Some(id);
                    }
                    let edit = Button::new(RichText::new("✏").size(18.0))
                        .selected(self.ui.renaming_preset == Some(preset.id))
                        .min_size(vec2(ROW_HEIGHT, ROW_HEIGHT));
                    if ui.add(edit).on_hover_text("Rename or delete").clicked() {
                        self.ui.renaming_preset = Some(preset.id);
                    }
                });
            }
            if let Some(id) = choice_grid(ui, &builtin, current, 1).flatten() {
                picked = Some(id);
            }
        });
        ui.add_space(8.0);

        let renaming = self.ui.renaming_preset;
        if let Some(preset) =
            renaming.and_then(|id| self.editor.presets.presets.iter_mut().find(|p| p.id == id))
        {
            let mut delete = false;
            let mut done = false;
            ui.horizontal(|ui| {
                let field_width =
                    ui.available_width() - 2.0 * (ROW_HEIGHT + ui.spacing().item_spacing.x) - 16.0;
                let field = ui.add(
                    egui::TextEdit::singleline(&mut preset.name)
                        .id(Id::new("rename_preset"))
                        .desired_width(field_width),
                );
                // Typing goes to the name while nothing else has the focus.
                if ui.memory(|m| m.focused().is_none()) {
                    field.request_focus();
                }
                let submit = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let size = vec2(ROW_HEIGHT, 32.0);
                delete = ui.add(Button::new("🗑").min_size(size)).on_hover_text("Delete").clicked();
                let valid = !preset.name.trim().is_empty();
                done = ui.add_enabled(valid, Button::new("✔").min_size(size)).clicked()
                    || (submit && valid);
            });
            if done {
                preset.name = preset.name.trim().to_owned();
                self.ui.renaming_preset = None;
            }
            if delete {
                let id = preset.id;
                self.delete_preset(id);
                self.ui.renaming_preset = None;
            }
            return picked;
        }

        if let Some(PresetId::User(id)) = current
            && let Some(preset) = self.editor.presets.get(id)
            && preset.settings != self.preset_snapshot()
        {
            let label = format!("Update \u{201c}{}\u{201d}", preset.name);
            if wide_button(ui, &label, 18.0, ROW_HEIGHT)
                .on_hover_text("Store the current settings in this preset")
                .clicked()
            {
                self.editor.presets.update(id, self.preset_snapshot());
            }
        }
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.ui.preset_name_buffer)
                    .hint_text("Name")
                    .desired_width(ui.available_width() - 80.0),
            );
            if ui.add(Button::new("Save").min_size(vec2(72.0, 32.0))).clicked() {
                let name = std::mem::take(&mut self.ui.preset_name_buffer);
                self.save_preset_as(&name);
            }
        });
        picked
    }

    fn time_signature_choices(&mut self, ui: &mut Ui) -> bool {
        let settings = &mut self.editor.generator.settings;
        let options: Vec<_> =
            time_signatures().into_iter().map(|ts| (ts, time_signature_label(ts))).collect();
        let picked = egui::ScrollArea::vertical()
            .max_height(360.0)
            .show(ui, |ui| choice_grid(ui, &options, settings.time_signature, 2))
            .inner;
        if let Some(ts) = picked {
            settings.time_signature = ts;
            self.follow_meter(ts);
        }
        picked.is_some()
    }

    fn subdivision_choices(&mut self, ui: &mut Ui) -> bool {
        let settings = &mut self.editor.generator.settings;
        let shown = effective_subdivision(settings.subdivision, settings.time_signature);
        let options: Vec<_> = offered_subdivisions(settings.time_signature)
            .iter()
            .map(|&s| (s, subdivision_label(s).to_owned()))
            .collect();
        let picked = choice_grid(ui, &options, shown, 2);
        if let Some(s) = picked {
            settings.subdivision = s;
        }
        picked.is_some()
    }

    /// Complexity levels; picking one turns custom groupings off. The grouping
    /// picker below lets the user hand-pick instead.
    fn complexity_choices(&mut self, ui: &mut Ui) -> bool {
        let settings = &mut self.editor.generator.settings;
        let current = if settings.custom_figures_enabled { 0 } else { settings.complexity };
        let options: Vec<_> = (1..=MAX_COMPLEXITY).map(|l| (l, l.to_string())).collect();
        let picked = choice_grid(ui, &options, current, MAX_COMPLEXITY as usize);
        if let Some(level) = picked {
            settings.complexity = level;
            settings.custom_figures_enabled = false;
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Space").size(18.0));
            ui.label(RichText::new(format!("{:.0}%", settings.space * 100.0)).size(18.0).weak());
        });
        ui.spacing_mut().slider_width = ui.available_width();
        ui.add(egui::Slider::new(&mut settings.space, 0.0..=1.0).show_value(false))
            .on_hover_text("More rests in the rhythm");
        ui.add_space(8.0);
        egui::CollapsingHeader::new(RichText::new("Groupings").size(18.0))
            .id_salt("compact_groupings")
            .show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    self.figures_picker(ui);
                });
            });
        picked.is_some()
    }

    fn bars_choices(&mut self, ui: &mut Ui) -> bool {
        let generator = &mut self.editor.generator;
        let min_bars = if generator.endless { ENDLESS_MIN_BARS } else { 1 };
        let options: Vec<_> = (min_bars..=MAX_BARS).map(|b| (b, b.to_string())).collect();
        let picked = choice_grid(ui, &options, generator.effective_bars(), 4);
        if let Some(bars) = picked {
            generator.settings.bars = bars;
        }
        picked.is_some()
    }

    /// Big tempo display with steppers, a slider and tap tempo.
    fn bpm_editor(&mut self, ui: &mut Ui) {
        let mut bpm = self.playback_ctl.bpm;
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(bpm.to_string()).size(56.0).strong());
        });

        let steps: [i32; 4] = [-5, -1, 1, 5];
        let spacing = ui.spacing().item_spacing.x;
        let width = (ui.available_width() - spacing * 3.0) / 4.0;
        ui.horizontal(|ui| {
            for step in steps {
                let label = if step > 0 { format!("+{step}") } else { format!("−{}", -step) };
                let button =
                    Button::new(RichText::new(label).size(20.0)).min_size(vec2(width, ROW_HEIGHT));
                if ui.add(button).clicked() {
                    bpm = (bpm as i32 + step).clamp(MIN_BPM as i32, MAX_BPM as i32) as u32;
                }
            }
        });
        ui.add_space(4.0);
        ui.spacing_mut().slider_width = ui.available_width();
        ui.add(egui::Slider::new(&mut bpm, MIN_BPM..=MAX_BPM).show_value(false));
        ui.add_space(4.0);

        let pulse = tap_pulse(self.current_measure().time_signature());
        let tap = wide_button(ui, "Tap", 22.0, 72.0);
        if pulse != TapPulse::Quarter {
            ui.vertical_centered(|ui| ui.weak(format!("Tap in {}", pulse.name())));
        }
        if tap.clicked()
            && let Some(tapped) = self.ui.tap_tempo.tap(ui.input(|i| i.time), pulse)
        {
            bpm = tapped;
        }

        if bpm != self.playback_ctl.bpm {
            self.playback_ctl.bpm = bpm;
            self.editor.dirty = true;
            self.handle_bpm_change();
        }
    }

    /// Meter of the metronome tab: any the editor offers. Returns whether
    /// one was picked.
    fn metronome_meter_choices(&mut self, ui: &mut Ui) -> bool {
        let options: Vec<_> =
            all_time_signatures().into_iter().map(|ts| (ts, time_signature_label(ts))).collect();
        let current = self.metronome_meter();
        let picked = egui::ScrollArea::vertical()
            .max_height(360.0)
            .show(ui, |ui| choice_grid(ui, &options, current, 2))
            .inner;
        if let Some(ts) = picked {
            self.set_metronome_meter(ts);
        }
        picked.is_some()
    }

    /// Ready-made click bars of the metronome meter. Returns whether one was
    /// picked.
    fn click_fill_choices(&mut self, ui: &mut Ui) -> bool {
        let fills = click_fills(self.metronome_meter());
        let bar = &self.editor.score.measures[0];
        let current = fills.iter().position(|(m, _)| same_bar(m, bar));
        let options: Vec<_> =
            fills.iter().enumerate().map(|(i, (_, name))| (Some(i), name.clone())).collect();
        let picked = choice_grid(ui, &options, current, 2).flatten();
        if let Some(i) = picked {
            self.replace_click_bar(fills[i].0.clone());
        }
        picked.is_some()
    }

    /// Counter rhythms of the metronome meter, "N:M" for N clicks in the
    /// time of M beats, then the sound they play with.
    fn polyrhythm_choices(&mut self, ui: &mut Ui) -> bool {
        let current = self.counter_bar().and_then(ratio_of);
        let mut options = vec![(None, "Off".to_owned())];
        options.extend(self.counter_choices().into_iter().map(|r| (Some(r), r.to_string())));
        // A custom counter bar is none of the choices: highlight none.
        let shown = if self.counter_bar().is_some() && current.is_none() {
            Some(Ratio { n: 0, span: 0 })
        } else {
            current
        };
        let picked = choice_grid(ui, &options, shown, 3);
        if let Some(ratio) = picked {
            self.set_counter_ratio(ratio);
        }
        ui.add_space(8.0);
        ui.label("Sound");
        let settings = &mut self.playback_ctl.audio_cfg.settings;
        let sounds: Vec<_> = CounterSound::ALL.iter().map(|&s| (s, s.label().to_owned())).collect();
        if let Some(sound) = choice_grid(ui, &sounds, settings.counter_sound, 3) {
            settings.counter_sound = sound;
        }
        // Picking a sound keeps the picker open to compare them.
        false
    }

    fn swing_choices(&mut self, ui: &mut Ui) {
        let swing = &mut self.playback_ctl.audio_cfg.swing;
        let options: Vec<_> = SWING_PRESETS.iter().map(|&p| (p, swing_label(p))).collect();
        if let Some(percent) = choice_grid(ui, &options, swing.percent, 3) {
            swing.percent = percent;
        }
        ui.add_space(4.0);
        ui.spacing_mut().slider_width = ui.available_width();
        ui.add(
            egui::Slider::new(&mut swing.percent, MIN_SWING_PERCENT..=MAX_SWING_PERCENT)
                .show_value(false),
        );
        if !swing.is_straight() {
            ui.add_space(8.0);
            ui.label("Swung note value");
            let units: Vec<_> = SwingUnit::ALL.iter().map(|&u| (u, u.label().to_owned())).collect();
            if let Some(unit) = choice_grid(ui, &units, swing.unit, 2) {
                swing.unit = unit;
            }
        }
    }

    /// Figure picker, labelled "Groupings" like the reference app's "Custom
    /// Groupings". While custom figures are off, the tiles show which figures
    /// the complexity level unlocks; picking a tile switches to a custom selection seeded from it.
    pub(crate) fn figures_picker(&mut self, ui: &mut egui::Ui) {
        let family = self.ui.music_font_id.family.clone();
        let settings = &mut self.editor.generator.settings;
        let ts = settings.time_signature;
        let preset = complexity_figures(settings.subdivision, ts, settings.complexity);

        let mut enabled = settings.custom_figures_enabled;
        if ui.checkbox(&mut enabled, "Custom groupings").changed() {
            if enabled && settings.custom_figures.is_empty() {
                settings.custom_figures = preset;
            }
            settings.custom_figures_enabled = enabled;
        }

        let active: FigureSet =
            if settings.custom_figures_enabled { settings.custom_figures } else { preset };
        let available = figures(settings.subdivision, ts);
        if settings.custom_figures_enabled
            && !available.iter().any(|g| settings.custom_figures.contains(g.id))
        {
            ui.weak("No grouping selected for this subdivision, using the complexity level.");
        } else if !settings.custom_figures_enabled {
            ui.weak(format!(
                "Level {} uses the highlighted groupings. Pick one to customise.",
                settings.complexity
            ));
        }

        // Meters with both group sizes (5/8, 7/8) get one row per size.
        let mixed_spans = available.iter().any(|g| g.span != available[0].span);
        for span in BeatSpan::ALL {
            let of_span: Vec<_> = available.iter().filter(|g| g.span == span).collect();
            if of_span.is_empty() {
                continue;
            }
            if mixed_spans {
                ui.weak(match span {
                    BeatSpan::Quarter => "Groups of two eighths",
                    BeatSpan::DottedQuarter => "Groups of three eighths",
                    BeatSpan::Eighth => "Groups of two sixteenths",
                    BeatSpan::DottedEighth => "Groups of three sixteenths",
                });
            }
            ui.horizontal_wrapped(|ui| {
                for figure in of_span {
                    let measure = figure.measure();
                    let id = egui::Id::new(("figure", figure.id.0));
                    let tile = notation_button(
                        ui,
                        id,
                        &measure,
                        family.clone(),
                        active.contains(figure.id),
                    )
                    .on_hover_text(format!("Unlocked at level {}", figure.level));
                    if tile.clicked() {
                        if !settings.custom_figures_enabled {
                            settings.custom_figures = preset;
                            settings.custom_figures_enabled = true;
                        }
                        settings.custom_figures.toggle(figure.id);
                    }
                }
            });
        }
    }
}
