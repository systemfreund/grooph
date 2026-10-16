//! Picker dialogs for single settings (meter, notes, level, bars, tempo,
//! swing, preset): a grid of big buttons that works by touch. The compact
//! layout opens them from its settings card and options sheet, the desktop
//! layout for the time signature.

use crate::Grooph;
use crate::generator::ENDLESS_MIN_BARS;
use crate::generator_panel::{subdivision_label, time_signature_label, time_signatures};
use crate::presets::{self, PresetId};
use eframe::egui;
use eframe::egui::{Button, Color32, FontId, Id, Modal, Response, RichText, Ui, vec2};
use grooph_measure::generator::{
    MAX_BARS, MAX_COMPLEXITY, effective_subdivision, offered_subdivisions,
};
use grooph_measure::swing::{MAX_SWING_PERCENT, MIN_SWING_PERCENT, SwingUnit};
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
}

pub(crate) fn swing_label(percent: u8) -> String {
    if percent <= MIN_SWING_PERCENT { "Off".to_owned() } else { format!("{percent}%") }
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
    /// Dialog for the setting in [`crate::state::UiShell::picker`]: opened
    /// from the compact layout's card and options sheet, and from the
    /// desktop generator panel for the time signature.
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
            };
            ui.add_space(12.0);
            picked || wide_button(ui, "Done", 19.0, ROW_HEIGHT).clicked()
        });
        if modal.inner || modal.should_close() {
            self.ui.picker = None;
            self.ui.tap_tempo.reset();
        }

        self.finish_generator_change(before, endless_before, false);
        if let Some(id) = preset {
            self.apply_preset(id);
        }
    }

    /// Presets to load, and saving the current settings as a new one.
    fn preset_choices(&mut self, ui: &mut Ui) -> Option<PresetId> {
        let options: Vec<_> =
            presets::all(&self.editor.presets).into_iter().map(|(id, n)| (Some(id), n)).collect();
        let current = self.editor.active_preset;
        let picked = egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| choice_grid(ui, &options, current, 1))
            .inner
            .flatten();
        ui.add_space(8.0);
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
        let current = if settings.custom_groupings_enabled { 0 } else { settings.complexity };
        let options: Vec<_> = (1..=MAX_COMPLEXITY).map(|l| (l, l.to_string())).collect();
        let picked = choice_grid(ui, &options, current, MAX_COMPLEXITY as usize);
        if let Some(level) = picked {
            settings.complexity = level;
            settings.custom_groupings_enabled = false;
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
                    self.groupings_picker(ui);
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

        let tap = wide_button(ui, "Tap", 22.0, 72.0);
        if tap.clicked()
            && let Some(tapped) = self.ui.tap_tempo.tap(ui.input(|i| i.time))
        {
            bpm = tapped;
        }

        if bpm != self.playback_ctl.bpm {
            self.playback_ctl.bpm = bpm;
            self.editor.dirty = true;
            self.handle_bpm_change();
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
}
