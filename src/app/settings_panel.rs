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
                global_theme_preference_buttons(ui);
            },
        );
    }
}
