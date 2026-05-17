use crate::Grooph;
use crate::Mode;
use eframe::egui;
use eframe::egui::SliderOrientation;
use eframe::egui::scroll_area::{ScrollBarVisibility, ScrollSource};
use eframe::egui::style::HandleShape;

impl Grooph {
    pub(super) fn mixer_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("mixer").show_animated_inside(ui, self.mode == Mode::Mixer, |ui| {
            egui::ScrollArea::horizontal()
                .scroll_source(ScrollSource::ALL)
                .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        let formatter = |value, _| format!("{:.0}%", value * 100.0);
                        ui.add(
                            egui::Slider::new(&mut self.audio_cfg.settings.downbeat, 0.0..=1.0)
                                .handle_shape(HandleShape::Rect { aspect_ratio: 1.5 })
                                .orientation(SliderOrientation::Vertical)
                                .custom_formatter(formatter)
                                .text("Downbeat"),
                        );
                        ui.add(
                            egui::Slider::new(&mut self.audio_cfg.settings.primary, 0.0..=1.0)
                                .orientation(SliderOrientation::Vertical)
                                .handle_shape(HandleShape::Rect { aspect_ratio: 1.5 })
                                .custom_formatter(formatter)
                                .text("Primary Beat"),
                        );
                        ui.add(
                            egui::Slider::new(&mut self.audio_cfg.settings.accent, 0.0..=1.0)
                                .orientation(SliderOrientation::Vertical)
                                .handle_shape(HandleShape::Rect { aspect_ratio: 1.5 })
                                .custom_formatter(formatter)
                                .text("Accented Beat"),
                        );
                        ui.add(
                            egui::Slider::new(&mut self.audio_cfg.settings.beat, 0.0..=1.0)
                                .orientation(SliderOrientation::Vertical)
                                .handle_shape(HandleShape::Rect { aspect_ratio: 1.5 })
                                .custom_formatter(formatter)
                                .text("Beat"),
                        );
                    });
                });
        });
    }
}
