use crate::Grooph;
use crate::Mode;
use eframe::egui;
use eframe::egui::SliderOrientation;
use eframe::egui::scroll_area::{ScrollBarVisibility, ScrollSource};
use eframe::egui::style::HandleShape;

/// One vertical volume slider shown in percent.
fn volume_slider(ui: &mut egui::Ui, value: &mut f32, label: &str) {
    ui.add(
        egui::Slider::new(value, 0.0..=1.0)
            .orientation(SliderOrientation::Vertical)
            .handle_shape(HandleShape::Rect { aspect_ratio: 1.5 })
            .custom_formatter(|value, _| format!("{:.0}%", value * 100.0))
            .text(label),
    );
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
    /// Volumes of the two voices: the metronome (downbeat, primary beats,
    /// count-in) and the rhythm (notes, accents, ghost notes).
    pub(super) fn mixer_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("mixer").show_animated_inside(ui, self.ui.mode == Mode::Mixer, |ui| {
            egui::ScrollArea::horizontal()
                .scroll_source(ScrollSource::ALL)
                .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    let settings = &mut self.playback_ctl.audio_cfg.settings;
                    ui.horizontal_top(|ui| {
                        voice_group(
                            ui,
                            "Metronome",
                            &mut settings.metronome_muted,
                            "Silence downbeat and beats; the count-in still sounds",
                            |ui| {
                                volume_slider(ui, &mut settings.downbeat, "Downbeat");
                                volume_slider(ui, &mut settings.primary, "Beats");
                            },
                            |ui| volume_slider(ui, &mut settings.count_in, "Count-in"),
                        );
                        ui.separator();
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
                    });
                });
        });
    }
}
