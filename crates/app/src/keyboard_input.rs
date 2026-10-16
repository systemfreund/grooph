use crate::Grooph;
use crate::Mode;
use crate::tools::matching_tool;
use eframe::egui;
use eframe::egui::Key;

impl Grooph {
    pub(super) fn handle_keyboard_input(&mut self, ui: &mut egui::Ui) {
        ui.input(|i| {
            // Modal dialogs swallow global shortcuts; their own UI provides Cancel/Confirm.
            if matches!(self.ui.mode, Mode::TimeSignature { .. }) {
                return;
            }
            // Calibration swaps in its own click score: no editing, but Space
            // still stops (= cancels) it.
            if self.playback_ctl.play_along.calibration.is_some() {
                if i.key_pressed(Key::Space) {
                    self.stop_transport();
                }
                return;
            }
            // App-global shortcuts run regardless of edit mode; the tool registry handles the rest.
            if i.key_pressed(Key::Escape) {
                self.toggle_mode(Mode::Edit);
            }
            if i.key_pressed(Key::Space) {
                self.toggle_playback();
            }

            if let Some(tool) = matching_tool(i) {
                self.apply_tool(tool);
            }
        });
    }
}
