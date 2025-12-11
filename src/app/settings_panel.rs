use crate::Grooph;
use crate::app::Mode;
use eframe::egui;
use eframe::egui::global_theme_preference_buttons;

impl Grooph {
    pub(super) fn settings_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("settings").show_animated(
            ctx,
            self.mode == Mode::Settings,
            |ui| {
                ui.horizontal(|ui| {
                    global_theme_preference_buttons(ui);
                });
                ui.separator();

                egui::CollapsingHeader::new("Click")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Base Frequency (PrimaryBeat):");
                            ui.add(egui::Slider::new(&mut self.mixer.base_frequency, 220.0..=880.0).text("Hz"));
                        });
                        ui.horizontal(|ui| {
                            ui.label("Click Decay:");
                            ui.add(egui::Slider::new(&mut self.mixer.decay, 0.01..=0.5).text("s"));
                        });
                    });

                egui::CollapsingHeader::new("Developer settings")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Size:");
                            ui.add(egui::Slider::new(&mut self.layout_width_cap_factor, 0.05..=0.5));
                        });
                        ui.horizontal(|ui| {
                            ui.label("Stem Length Factor:");
                            ui.add(egui::Slider::new(&mut self.layout_stem_length_factor, 0.1..=2.0));
                        });
                        ui.horizontal(|ui| {
                            ui.label("Accents Position:");
                            ui.radio_value(&mut self.layout_accent_below, true, "Below");
                            ui.radio_value(&mut self.layout_accent_below, false, "Above");
                        });
                    });
            },
        );
    }
}
