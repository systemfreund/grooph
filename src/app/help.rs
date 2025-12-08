use crate::Grooph;
use crate::measure::BeatKind::{Note, Rest};
use crate::measure::duration::human_readable;
use eframe::egui;
use eframe::egui::{Label, RichText};

impl Grooph {
    pub(super) fn help_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("info").show_animated(ctx, self.show_info, |ui| {
            ui.collapsing("Keybindings", |ui| {
                let text = RichText::new(
                    "   Arrow keys: Move cursor
 Del/Backspace: Remove note
         Space: Toggle between note and rest
             A: Set/unset accent
           1-4: Set duration (1=1/4, 2=1/8, 3=1/16, 4=1/32)
        Period: Toggle dotted
             T: Cycle tuplet",
                )
                    .monospace()
                    .size(16.0);
                ui.label(text);
            });

            // Label showing absolute beat position at the cursor and human-readable duration/kind
            let mut beat_text = String::from("-");
            let idx = self.cursor_idx;
            let positions = self.measure.beat_positions();
            if idx < positions.len() {
                let v = positions[idx];
                let mut s = format!("{:.3}", v);
                // Trim trailing zeros and optional dot for a cleaner look
                while s.ends_with('0') {
                    s.pop();
                }
                if s.ends_with('.') {
                    s.pop();
                }
                beat_text = s;
            }
            let mut label = format!("Beat: {}", beat_text);
            if idx < self.measure.beats().len() {
                let b = self.measure.beats()[idx];
                let desc = human_readable(&b.duration);
                let kind = match b.kind {
                    Note => "note",
                    Rest => "rest",
                };
                label = format!("Beat: {}, {} {}", beat_text, desc, kind);
            }
            ui.add(Label::new(label));
        });
    }
}
