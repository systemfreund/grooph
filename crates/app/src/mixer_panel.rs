use crate::Grooph;
use crate::pickers::dialog_scroll_height;
use eframe::egui;
use eframe::egui::scroll_area::{ScrollBarVisibility, ScrollSource};
use eframe::egui::style::HandleShape;
use eframe::egui::{Align, DragValue, Layout, SliderOrientation, TextStyle, TextWrapMode, vec2};

/// Width of `text` in the body font.
fn text_width(ui: &egui::Ui, text: &str) -> f32 {
    egui::WidgetText::from(text)
        .into_galley(ui, Some(TextWrapMode::Extend), f32::INFINITY, TextStyle::Body)
        .size()
        .x
}

/// One vertical volume slider in percent: rail, value and label centered in
/// a column. egui's own slider puts them flush left, so the three are laid
/// out here.
fn volume_slider(ui: &mut egui::Ui, value: &mut f32, label: &str) -> egui::Response {
    let spacing = ui.spacing();
    let value_width =
        (text_width(ui, "100%") + 2.0 * spacing.button_padding.x).max(spacing.interact_size.x);
    // Twice egui's thickness: an easier target for a finger.
    let thickness = 2.0 * ui.text_style_height(&TextStyle::Body).max(spacing.interact_size.y);
    let rail = 2.0 * spacing.slider_rail_height;
    let length = spacing.slider_width;
    let width = text_width(ui, label).max(value_width).max(thickness);
    let layout = Layout::top_down(Align::Center);
    ui.allocate_ui_with_layout(vec2(width, ui.available_height()), layout, |ui| {
        ui.set_width(width);
        let slider = ui
            .scope(|ui| {
                // The slider is as thick as the room it asks for.
                ui.spacing_mut().interact_size.y = thickness;
                ui.spacing_mut().slider_rail_height = rail;
                ui.add_sized(
                    vec2(thickness, length),
                    egui::Slider::new(value, 0.0..=1.0)
                        .orientation(SliderOrientation::Vertical)
                        // Half the ratio of before: twice as wide, as high.
                        .handle_shape(HandleShape::Rect { aspect_ratio: 0.75 })
                        .show_value(false),
                )
            })
            .inner;
        let number = ui.add(
            DragValue::new(value)
                .range(0.0..=1.0)
                .speed(0.005)
                .custom_formatter(|value, _| format!("{:.0}%", value * 100.0))
                .custom_parser(|text| {
                    let number = text.trim().trim_end_matches('%').trim();
                    number.parse::<f64>().ok().map(|percent| percent / 100.0)
                }),
        );
        ui.label(label);
        slider | number
    })
    .inner
}

/// A group of sliders under a title with a mute toggle. `muted_sliders` are
/// greyed out while the group is muted, `other_sliders` stay active.
fn voice_group(
    ui: &mut egui::Ui,
    title: &str,
    muted: &mut bool,
    mute_hint: &str,
    muted_sliders: impl FnOnce(&mut egui::Ui),
    other_sliders: impl FnOnce(&mut egui::Ui),
) {
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.strong(title);
            ui.checkbox(muted, "Mute").on_hover_text(mute_hint);
        });
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!*muted, muted_sliders);
            other_sliders(ui);
        });
    });
}

impl Grooph {
    /// Volumes of the voices: the rhythm (notes, accents, ghost notes) and
    /// the metronome (downbeat, primary beats, soft clicks, counter rhythm,
    /// count-in).
    pub(super) fn mixer_panel(&mut self, ui: &mut egui::Ui) {
        // The groups one above the other; a screen too narrow for the
        // dialog to fit them scrolls the sliders of a group sideways, one too
        // short scrolls the groups.
        let out = egui::ScrollArea::both()
            .max_height(dialog_scroll_height(ui))
            .scroll_source(ScrollSource::ALL)
            .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                let settings = &mut self.playback_ctl.audio_cfg.settings;
                voice_group(
                    ui,
                    "Rhythm",
                    &mut settings.rhythm_muted,
                    "Silence notes, accents and ghost notes",
                    |ui| {
                        volume_slider(ui, &mut settings.beat, "Notes");
                        volume_slider(ui, &mut settings.accent, "Accents");
                        volume_slider(ui, &mut settings.ghost, "Ghost notes");
                    },
                    |_| {},
                );
                ui.separator();
                voice_group(
                    ui,
                    "Metronome",
                    &mut settings.metronome_muted,
                    "Silence the metronome clicks and its counter rhythm; the count-in still sounds",
                    |ui| {
                        volume_slider(ui, &mut settings.downbeat, "Downbeat")
                            .on_hover_text("Accented clicks of the click bar");
                        volume_slider(ui, &mut settings.primary, "Medium")
                            .on_hover_text("Plain clicks of the click bar");
                        volume_slider(ui, &mut settings.soft, "Soft")
                            .on_hover_text("Soft clicks of the click bar (notes in parentheses)");
                        volume_slider(ui, &mut settings.counter, "Polyrhythm")
                            .on_hover_text("Clicks of the polyrhythm in the metronome tab");
                    },
                    |ui| {
                        volume_slider(ui, &mut settings.count_in, "Count-in");
                    },
                );
            });
        // The dialog takes this width from the next frame on.
        let width = out.content_size.x.ceil();
        if mixer_width(ui) != Some(width) {
            ui.data_mut(|d| d.insert_temp(mixer_width_id(), width));
            ui.ctx().request_repaint();
        }
    }
}

fn mixer_width_id() -> egui::Id {
    egui::Id::unique("mixer_width")
}

/// Width the mixer's sliders took when last shown.
pub(crate) fn mixer_width(ui: &egui::Ui) -> Option<f32> {
    ui.data(|d| d.get_temp(mixer_width_id()))
}
