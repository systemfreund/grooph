use crate::Grooph;
use grooph_measure::BeatKind::{Note, Rest};
use grooph_measure::duration::human_readable;
use eframe::egui;
use eframe::egui::{Label, RichText};
use crate::Mode;

impl Grooph {
    pub(super) fn help_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("info").show_animated(ctx, self.mode == Mode::Help, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    "This app is in early development. Please report any bugs or feature requests.",
                );
                ui.hyperlink_to("Email: hello@grooph.app", "mailto:hello@grooph.app");
            });
            ui.separator();
            ui.collapsing("Keybindings", |ui| {
                let text = RichText::new(
                    "         Space: Play/pause
    Arrow keys: Move cursor
        Escape: Toggle between edit mode and playback mode
 Del/Backspace: Remove note
         Enter: Toggle between note and rest
             A: Set/unset accent
           1-4: Set duration (1=1/4, 2=1/8, 3=1/16, 4=1/32)
        Period: Toggle dotted
             T: Cycle tuplet",
                )
                .monospace()
                .size(16.0);
                ui.label(text);
            });

            ui.collapsing("Mouse/Finger controls", |ui| {
                let text = RichText::new(
                    "       Click/Tap: Move cursor
            Drag: Move cursor
Double-click/Tap: Toggle Note/Rest",
                )
                .monospace()
                .size(16.0);
                ui.label(text);
            });

        });
    }
}
