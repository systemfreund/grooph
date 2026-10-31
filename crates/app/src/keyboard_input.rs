use crate::Grooph;
use crate::Mode;
use crate::shell::Tab;
use crate::tools::matching_tool;
use eframe::egui;
use eframe::egui::Key;

impl Grooph {
    pub(super) fn handle_keyboard_input(&mut self, ui: &mut egui::Ui) {
        // Keys typed into a text field (preset or pattern name) are text.
        if ui.ctx().text_edit_focused() {
            return;
        }
        ui.input(|i| {
            // Modal dialogs swallow global shortcuts; their own UI provides Cancel/Confirm.
            if matches!(self.ui.mode, Mode::TimeSignature(_)) {
                return;
            }
            // During a calibration: no editing, but Space still stops
            // (= cancels) it.
            if self.playback_ctl.play_along.calibration.is_some() {
                if i.key_pressed(Key::Space) {
                    self.stop_transport();
                }
                return;
            }
            // Embedded there is only the metronome: no edit mode, no tools.
            if self.embedded() {
                if i.key_pressed(Key::Space) {
                    self.toggle_playback();
                }
                return;
            }
            // App-global shortcuts run regardless of edit mode; the tool registry handles the rest.
            // On the More screen Esc closes what is open there: no edit mode
            // to toggle, and with the metronome behind it, its click bar
            // would open.
            if i.key_pressed(Key::Escape) {
                if self.tab() == Tab::More {
                    self.back();
                } else {
                    self.toggle_edit();
                }
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
