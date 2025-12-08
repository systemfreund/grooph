use crate::Grooph;
use eframe::egui;
use eframe::egui::SliderOrientation;
use eframe::egui::style::HandleShape;

impl Grooph {
    pub(super) fn mixer_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("mixer").show_animated(ctx, self.show_mixer, |ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let formatter = |value, _| format!("{:.0}%", value * 100.0);
                ui.add(egui::Slider::new(&mut self.mixer_vol_downbeat, 0.0..=1.0)
                    .handle_shape(HandleShape::Rect { aspect_ratio: 1.5 })
                    .orientation(SliderOrientation::Vertical)
                    .custom_formatter(formatter)
                    .text("Downbeat"));
                ui.add(egui::Slider::new(&mut self.mixer_vol_primary, 0.0..=1.0)
                    .orientation(SliderOrientation::Vertical)
                    .handle_shape(HandleShape::Rect { aspect_ratio: 1.5 })
                    .custom_formatter(formatter)
                    .text("Primary Beat"));
                ui.add(egui::Slider::new(&mut self.mixer_vol_accent, 0.0..=1.0)
                    .orientation(SliderOrientation::Vertical)
                    .handle_shape(HandleShape::Rect { aspect_ratio: 1.5 })
                    .custom_formatter(formatter)
                    .text("Accented Beat"));
            });
        });
    }
}
