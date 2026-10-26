//! Picker dialogs for single settings (meter, notes, level, bars, tempo,
//! swing, guide, counting, preset): a grid of big buttons that works by touch, opened from the
//! settings card and the options sheet. Also the grouping picker of the level
//! dialog.

use crate::guide::Guide;
use crate::metronome_view::{FillLook, click_fills, ratio_label, same_bar};
use crate::presets::{self, PresetId};
use crate::shell::paint_grouped_meter;
use crate::tool_palette::{TOOL_PALETTE_BUTTON_SIZE, none_button, notation_button};
use crate::{CountingBase, Grooph};
use eframe::egui;
use eframe::egui::{Button, Color32, FontFamily, FontId, Id, Modal, Response, RichText, Ui, vec2};
use grooph_audio::{GapClick, GapMode, GapMute, Sound};
use grooph_measure::TimeSignature;
use grooph_measure::counter::{Ratio, own_ratios, step_clicks, step_span};
use grooph_measure::counting::CountingSystem;
use grooph_measure::generator::{
    BeatSpan, FigureSet, MAX_BARS, MAX_COMPLEXITY, Subdivision, complexity_figures,
    effective_subdivision, figures, offered_subdivisions, supports_time_signature,
};
use grooph_measure::swing::{MAX_SWING_PERCENT, MIN_SWING_PERCENT, SwingUnit};
use grooph_measure::tap_tempo::{TapPulse, tap_pulse};
use grooph_measure::tempo::{MAX_BPM, MIN_BPM};
use std::ops::RangeInclusive;

/// Height of a touch-friendly row or button.
pub(crate) const ROW_HEIGHT: f32 = 44.0;

/// Chances offered for a silent bar in random mode; the slider sets any other.
const RANDOM_SILENT_PERCENTS: [u8; 3] = [10, 30, 50];

/// Height of a subdivision tile in the metronome: small enough that all of
/// them fit a phone screen with the picker.
const FILL_TILE_SIZE: f32 = 48.0;
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
    /// What the staff shows while playing.
    Guide,
    /// The count under the staff and the words it is spoken in.
    Counting,
    Preset,
    /// The metronome tab's meter.
    MetronomeMeter,
    /// A ready-made click bar for the metronome meter.
    ClickFill,
    /// The metronome's counter rhythm and its sound.
    Polyrhythm,
    /// Bars the click drops out of, to practise inner time.
    SilentBars,
    /// The sounds of the rhythm, the metronome and the counter rhythm.
    Sounds,
    /// The MIDI device the play-along listens to.
    MidiInput,
}

pub(crate) fn counting_system_name(system: CountingSystem) -> &'static str {
    match system {
        CountingSystem::Numbers => "Numbers",
        CountingSystem::Takadimi => "Takadimi",
        CountingSystem::Gordon => "Gordon",
    }
}

/// Which bars go silent, in a few words, for the options row. Also while
/// off, so the row shows what switching on brings back.
pub(crate) fn silent_bars_label(gap: &GapClick) -> String {
    match gap.mode {
        GapMode::Pattern => format!("{} on · {} off", gap.on_bars, gap.off_bars),
        GapMode::Random => format!("Random {}%", gap.random_percent),
    }
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
    match shown_groups(ts) {
        Some(groups) => format!("{meter} ({groups})"),
        None => meter,
    }
}

/// The beat groups of an x/8 or x/16 meter with more than one, e.g. "2+3+2".
pub(crate) fn shown_groups(ts: TimeSignature) -> Option<String> {
    (ts.beat_unit != 4 && ts.groups().len() >= 2).then(|| ts.groups_label())
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
pub(crate) fn choice_grid<T: Copy + PartialEq>(
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
                // Long labels shrink to fit the cell.
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

/// Height of a tile in the time signature pickers.
const METER_TILE_HEIGHT: f32 = 72.0;
/// Music font size of a meter on its tile.
const METER_TILE_EM: f32 = 42.0;

/// Height of the scrolling time signature grid: the tall tiles get as much
/// of the screen as leaves room for "Done".
fn meter_grid_height(ui: &Ui) -> f32 {
    (ui.ctx().content_rect().height() - 240.0).clamp(360.0, 560.0)
}

/// Grid of time signatures, each drawn as in the settings card; returns the
/// one that was tapped.
fn meter_grid(
    ui: &mut Ui,
    meters: &[TimeSignature],
    current: TimeSignature,
    columns: usize,
    family: &FontFamily,
) -> Option<TimeSignature> {
    let mut picked = None;
    let spacing = ui.spacing().item_spacing.x;
    let width = (ui.available_width() - spacing * (columns as f32 - 1.0)) / columns as f32;
    for row in meters.chunks(columns) {
        ui.horizontal(|ui| {
            for &ts in row {
                let selected = ts == current;
                let button =
                    Button::new("").selected(selected).min_size(vec2(width, METER_TILE_HEIGHT));
                let label = time_signature_label(ts);
                let resp = ui.add(button);
                // The meter is painted, so the button gets its name here.
                resp.widget_info(|| {
                    egui::WidgetInfo::selected(egui::Role::Button, true, selected, &label)
                });
                let resp = resp.on_hover_text(label);
                let color = ui.style().interact_selectable(&resp, selected).text_color();
                let room = width - 2.0 * ui.spacing().button_padding.x;
                let center = resp.rect.center();
                let groups_color = color.gamma_multiply(0.7);
                let painter = ui.painter();
                paint_grouped_meter(
                    painter,
                    ts,
                    center,
                    METER_TILE_EM,
                    room,
                    family,
                    color,
                    groups_color,
                );
                if resp.clicked() {
                    picked = Some(ts);
                }
            }
        });
    }
    picked
}

/// Full-width button with a centered label.
/// Two steppers for `ratio`, its clicks and its beats, each skipping to the
/// next value `ts` has a counter bar for. The numbers are lit while `active`
/// (the counter rhythm plays `ratio`). Returns the ratio stepped to.
fn ratio_stepper(ui: &mut Ui, ts: TimeSignature, ratio: Ratio, active: bool) -> Option<Ratio> {
    let mut stepped = None;
    let own = own_ratios(ts);
    let spacing = ui.spacing().item_spacing.x;
    // Four buttons, two numbers and the colon in one row; on a narrow
    // picker the buttons give way first.
    let room = ui.available_width() - 6.0 * spacing;
    let button = ((room - 3.0 * 24.0) / 4.0).clamp(28.0, ROW_HEIGHT);
    let number = ((room - 4.0 * button) / 3.0).max(16.0);
    let color = if active { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
    let mut step = |ui: &mut Ui, label: &str, to: Option<Ratio>| {
        let button =
            Button::new(RichText::new(label).size(22.0)).min_size(vec2(button, ROW_HEIGHT));
        if ui.add_enabled(to.is_some(), button).clicked() {
            stepped = to;
        }
    };
    let value = |ui: &mut Ui, text: String| {
        ui.add_sized(
            vec2(number, ROW_HEIGHT),
            egui::Label::new(RichText::new(text).size(24.0).color(color)),
        );
    };
    ui.horizontal(|ui| {
        step(ui, "−", step_clicks(&own, ratio, false));
        value(ui, ratio.n.to_string());
        step(ui, "+", step_clicks(&own, ratio, true));
        value(ui, ":".to_owned());
        step(ui, "−", step_span(&own, ratio, false));
        value(ui, ratio.span.to_string());
        step(ui, "+", step_span(&own, ratio, true));
    });
    stepped
}

/// A label with a − / + stepper for `value` on the right.
fn count_stepper(ui: &mut Ui, label: &str, value: &mut u32, range: RangeInclusive<u32>) {
    let spacing = ui.spacing().item_spacing.x;
    let number = 48.0;
    ui.horizontal(|ui| {
        let label_width = ui.available_width() - 2.0 * (ROW_HEIGHT + spacing) - number - spacing;
        let (rect, _) = ui.allocate_exact_size(vec2(label_width, ROW_HEIGHT), egui::Sense::hover());
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            FontId::proportional(18.0),
            ui.visuals().text_color(),
        );
        let current = *value;
        let mut stepped = None;
        let mut step = |ui: &mut Ui, text: &str, to: u32| {
            let button =
                Button::new(RichText::new(text).size(22.0)).min_size(vec2(ROW_HEIGHT, ROW_HEIGHT));
            if ui.add_enabled(range.contains(&to) && to != current, button).clicked() {
                stepped = Some(to);
            }
        };
        step(ui, "−", current.saturating_sub(1));
        ui.add_sized(
            vec2(number, ROW_HEIGHT),
            egui::Label::new(RichText::new(current.to_string()).size(24.0).strong()),
        );
        step(ui, "+", current + 1);
        if let Some(to) = stepped {
            *value = to;
        }
    });
}

/// Every [`Sound`] in two columns; returns the one that was tapped.
fn sound_grid(ui: &mut Ui, current: Sound) -> Option<Sound> {
    let sounds: Vec<_> = Sound::ALL.iter().map(|&s| (s, s.label().to_owned())).collect();
    choice_grid(ui, &sounds, current, 2)
}

pub(crate) fn wide_button(ui: &mut Ui, label: &str, size: f32, height: f32) -> Response {
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

        let modal = Modal::new(Id::unique("settings_picker")).show(ui.ctx(), |ui| {
            ui.set_width(width);
            let title = match picker {
                Picker::TimeSignature => "Time signature",
                Picker::Subdivision => "Notes",
                Picker::Complexity => "Level",
                Picker::Bars => "Bars",
                Picker::Bpm => "Tempo",
                Picker::Swing => "Swing",
                Picker::Guide => "Guide",
                Picker::Counting => "Count labels",
                Picker::Preset => "Preset",
                Picker::MetronomeMeter => "Time signature",
                Picker::ClickFill => "Subdivision",
                Picker::Polyrhythm => "Polyrhythm",
                Picker::SilentBars => "Silent bars",
                Picker::Sounds => "Sounds",
                Picker::MidiInput => "MIDI input",
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
                Picker::Guide => self.guide_choices(ui),
                Picker::Counting => self.counting_choices(ui),
                Picker::Preset => {
                    preset = self.preset_choices(ui);
                    preset.is_some()
                }
                Picker::MetronomeMeter => self.metronome_meter_choices(ui),
                Picker::ClickFill => self.click_fill_choices(ui),
                Picker::Polyrhythm => self.polyrhythm_choices(ui),
                Picker::SilentBars => {
                    self.silent_bars_choices(ui);
                    false
                }
                Picker::Sounds => {
                    self.sound_choices(ui);
                    false
                }
                Picker::MidiInput => self.midi_input_choices(ui),
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
                        .id(Id::unique("rename_preset"))
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
        let family = self.ui.music_font_id.family.clone();
        let settings = &mut self.editor.generator.settings;
        let picked = egui::ScrollArea::vertical()
            .max_height(meter_grid_height(ui))
            .show(ui, |ui| meter_grid(ui, &time_signatures(), settings.time_signature, 2, &family))
            .inner;
        if let Some(ts) = picked {
            settings.time_signature = ts;
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
            .id_salt("card_groupings")
            .show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    self.figures_picker(ui);
                });
            });
        picked.is_some()
    }

    /// A bar count, or endless: a running score that never loops.
    fn bars_choices(&mut self, ui: &mut Ui) -> bool {
        let generator = &mut self.editor.generator;
        let mut options: Vec<_> = (1..=MAX_BARS).map(|b| (Some(b), b.to_string())).collect();
        options.push((None, "Endless".to_owned()));
        let current = (!generator.endless).then_some(generator.settings.bars.max(1));
        let picked = choice_grid(ui, &options, current, 4);
        match picked {
            Some(Some(bars)) => {
                generator.settings.bars = bars;
                generator.endless = false;
            }
            Some(None) => generator.endless = true,
            None => {}
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
        let family = self.ui.music_font_id.family.clone();
        let current = self.metronome_meter();
        let picked = egui::ScrollArea::vertical()
            .max_height(meter_grid_height(ui))
            .show(ui, |ui| meter_grid(ui, &all_time_signatures(), current, 2, &family))
            .inner;
        if let Some(ts) = picked {
            self.set_metronome_meter(ts);
        }
        picked.is_some()
    }

    /// Ready-made click bars of the metronome meter. Returns whether one was
    /// picked.
    /// Each shown as its notes, like the tool palette (see
    /// [`crate::metronome_view::ClickFill::tile`]).
    fn click_fill_choices(&mut self, ui: &mut Ui) -> bool {
        let fills = click_fills(self.metronome_meter());
        let bar = &self.editor.score.measures[0];
        let current = fills.iter().position(|f| same_bar(&f.bar, bar));
        let family = self.ui.music_font_id.family.clone();
        let mut picked = None;
        // Nine tiles wrap to many rows on a narrow phone: they scroll where
        // they would push "Done" off the screen.
        let room = (ui.ctx().content_rect().height() - 300.0).max(200.0);
        egui::ScrollArea::vertical().max_height(room).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (i, fill) in fills.iter().enumerate() {
                    let selected = current == Some(i);
                    let id = egui::IdSalt::new(("click_fill", i));
                    let tile = match &fill.look() {
                        Some(FillLook::Notes(tile)) => {
                            notation_button(ui, id, tile, family.clone(), selected, FILL_TILE_SIZE)
                        }
                        Some(FillLook::NoSubdivision) => {
                            none_button(ui, id, selected, FILL_TILE_SIZE)
                        }
                        None => ui.add(
                            Button::new(RichText::new(&fill.name).size(18.0))
                                .selected(selected)
                                .min_size(vec2(0.0, ROW_HEIGHT)),
                        ),
                    };
                    if tile.on_hover_text(&fill.name).clicked() {
                        picked = Some(i);
                    }
                }
            });
        });
        // The name of the one picked, as the tiles show only notes.
        ui.add_space(4.0);
        let name = current.map_or("Custom", |i| fills[i].name.as_str());
        ui.label(RichText::new(name).size(18.0).weak());
        if let Some(i) = picked {
            self.replace_click_bar(fills[i].bar.clone());
        }
        picked.is_some()
    }

    /// Counter rhythms of the metronome meter, "N:M" for N clicks in the
    /// time of M beats, then one of one's own, then the sound they play
    /// with.
    fn polyrhythm_choices(&mut self, ui: &mut Ui) -> bool {
        let counter = self.counter_ratio();
        let current = counter.map(|(r, _)| r);
        let edited = counter.filter(|&(_, edited)| edited).map(|(r, _)| r);
        let mut options = vec![(None, "Off".to_owned())];
        options.extend(
            self.counter_choices()
                .into_iter()
                .map(|r| (Some(r), ratio_label((r, edited == Some(r))))),
        );
        // A custom counter bar is none of the choices: highlight none.
        let shown = if self.counter_bar().is_some() && current.is_none() {
            Some(Ratio { n: 0, span: 0 })
        } else {
            current
        };
        let mut picked = choice_grid(ui, &options, shown, 3);
        // Picking the ratio again starts its clicks over; so does this, also
        // for a ratio of one's own, which has no tile.
        if let Some(ratio) = edited {
            ui.add_space(4.0);
            ui.label("Clicks edited.");
            if wide_button(ui, &format!("Reset {ratio} clicks"), 18.0, ROW_HEIGHT).clicked() {
                picked = Some(Some(ratio));
            }
        }
        if let Some(ratio) = picked {
            self.set_counter_ratio(ratio);
        }

        // Any N and M the meter has a counter bar for, stepped through.
        let ts = self.metronome_meter();
        let own = current
            .or_else(|| self.counter_choices().first().copied())
            .or_else(|| own_ratios(ts).first().copied());
        if let Some(own) = own {
            ui.add_space(8.0);
            ui.label("Your own (clicks : beats)");
            let active = current == Some(own);
            if let Some(ratio) = ratio_stepper(ui, ts, own, active) {
                self.set_counter_ratio(Some(ratio));
            }
        }

        ui.add_space(8.0);
        ui.label("Sound");
        let settings = &mut self.playback_ctl.audio_cfg.settings;
        if let Some(sound) = sound_grid(ui, settings.counter_sound) {
            settings.counter_sound = sound;
        }
        // Picking a sound keeps the picker open to compare them.
        false
    }

    /// A sound for each voice, in the mixer's order. Picking one keeps the
    /// picker open to compare them.
    fn sound_choices(&mut self, ui: &mut Ui) {
        let settings = &mut self.playback_ctl.audio_cfg.settings;
        let voices = [
            ("Rhythm", &mut settings.rhythm_sound),
            ("Metronome", &mut settings.metronome_sound),
            ("Counter rhythm", &mut settings.counter_sound),
        ];
        for (i, (label, sound)) in voices.into_iter().enumerate() {
            if i > 0 {
                ui.add_space(8.0);
            }
            ui.label(label);
            if let Some(picked) = sound_grid(ui, *sound) {
                *sound = picked;
            }
        }
    }

    /// From the most support (the line) to none; steps the meter has no
    /// use for are left out.
    fn guide_choices(&mut self, ui: &mut Ui) -> bool {
        let ts = self.guide_meter();
        let options: Vec<_> = Guide::ALL
            .into_iter()
            .filter(|g| g.offered_in(&ts))
            .map(|g| (g, g.label().to_owned()))
            .collect();
        let current = self.ui.guide.in_meter(&ts);
        let picked = choice_grid(ui, &options, current, 1);
        if let Some(guide) = picked {
            self.ui.guide = guide;
        }
        picked.is_some()
    }

    /// The words to count in, then the count itself; picking a count closes
    /// the dialog. Counts too fine for every measure of the shown score are
    /// left out, unless one is already chosen.
    fn counting_choices(&mut self, ui: &mut Ui) -> bool {
        let metronome = self.metronome_mode().then(|| self.metronome_score());
        let score = metronome.as_ref().unwrap_or(&self.editor.score);
        let counting = &mut self.ui.counting;
        let systems: Vec<_> =
            CountingSystem::ALL.map(|s| (s, counting_system_name(s).to_owned())).into();
        if let Some(system) = choice_grid(ui, &systems, counting.system, 3) {
            counting.system = system;
        }
        ui.add_space(8.0);
        let bases: Vec<_> = CountingBase::ALL
            .into_iter()
            .filter(|&b| b == counting.base || b.fits(score))
            .map(|b| (b, b.label(counting.system, score)))
            .collect();
        let picked = choice_grid(ui, &bases, counting.base, 1);
        if let Some(base) = picked {
            counting.base = base;
        }
        picked.is_some()
    }

    /// Off, a pattern of sounding and silent bars, or silent bars at random;
    /// and whether the notes go quiet too.
    fn silent_bars_choices(&mut self, ui: &mut Ui) {
        let gap = &mut self.playback_ctl.audio_cfg.gap;
        ui.weak("The click drops out for some bars, so you keep the time yourself.");
        ui.add_space(8.0);
        let modes = [
            (None, "Off".to_owned()),
            (Some(GapMode::Pattern), "Pattern".to_owned()),
            (Some(GapMode::Random), "Random".to_owned()),
        ];
        let current = gap.enabled.then_some(gap.mode);
        if let Some(picked) = choice_grid(ui, &modes, current, 3) {
            // Off keeps the mode, so switching back on brings it back.
            gap.enabled = picked.is_some();
            gap.mode = picked.unwrap_or(gap.mode);
        }
        if !gap.enabled {
            return;
        }
        ui.add_space(8.0);
        match gap.mode {
            GapMode::Pattern => {
                count_stepper(ui, "Sounding bars", &mut gap.on_bars, 1..=16);
                count_stepper(ui, "Silent bars", &mut gap.off_bars, 1..=16);
            }
            GapMode::Random => {
                ui.label("Chance per bar");
                let chances: Vec<_> =
                    RANDOM_SILENT_PERCENTS.iter().map(|&p| (p, format!("{p}%"))).collect();
                if let Some(percent) = choice_grid(ui, &chances, gap.random_percent, 3) {
                    gap.random_percent = percent;
                }
                ui.add_space(4.0);
                ui.spacing_mut().slider_width = ui.available_width();
                ui.add(egui::Slider::new(&mut gap.random_percent, 5..=90).show_value(false));
            }
        }
        ui.add_space(8.0);
        ui.label("Mute");
        let mutes = [
            (GapMute::Metronome, "Click only".to_owned()),
            (GapMute::Everything, "Click and notes".to_owned()),
        ];
        if let Some(mute) = choice_grid(ui, &mutes, gap.mute, 2) {
            gap.mute = mute;
        }
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
            if enabled && settings.custom_patterns.is_empty() {
                settings.custom_patterns = preset;
            }
            settings.custom_figures_enabled = enabled;
        }

        let active: FigureSet =
            if settings.custom_figures_enabled { settings.custom_patterns } else { preset };
        let available = figures(settings.subdivision, ts);
        if settings.custom_figures_enabled
            && !available.iter().any(|g| settings.custom_patterns.contains(g.id))
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
            // One row per level: a span can have up to 64 figures.
            for level in 1..=MAX_COMPLEXITY {
                let of_level: Vec<_> = of_span.iter().filter(|g| g.level == level).collect();
                if of_level.is_empty() {
                    continue;
                }
                ui.weak(format!("From level {level}"));
                ui.horizontal_wrapped(|ui| {
                    for figure in of_level {
                        let measure = figure.measure();
                        let id = egui::IdSalt::new(("figure", figure.id.0));
                        let tile = notation_button(
                            ui,
                            id,
                            &measure,
                            family.clone(),
                            active.contains(figure.id),
                            TOOL_PALETTE_BUTTON_SIZE,
                        );
                        if tile.clicked() {
                            if !settings.custom_figures_enabled {
                                settings.custom_patterns = preset;
                                settings.custom_figures_enabled = true;
                            }
                            settings.custom_patterns.toggle(figure.id);
                        }
                    }
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_are_shown_for_x8_and_x16_meters_with_several() {
        assert_eq!(shown_groups(TimeSignature::FOUR_FOUR), None);
        assert_eq!(shown_groups(TimeSignature::new(3, 8)), None);
        assert_eq!(shown_groups(TimeSignature::new(4, 8)).as_deref(), Some("2+2"));
        assert_eq!(shown_groups(TimeSignature::new(6, 8)).as_deref(), Some("3+3"));
        assert_eq!(
            shown_groups(TimeSignature::new(7, 16).with_grouping(1)).as_deref(),
            Some("2+2+3")
        );
    }
}
