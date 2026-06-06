use crate::Grooph;
use crate::Mode;
use eframe::egui;

impl Grooph {
    pub(super) fn library_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("library").resizable(true).show_animated_inside(
            ui,
            self.ui.mode == Mode::Library,
            |ui| {
                ui.set_min_height(220.0);

                // Header: save the current score + tempo under a name.
                ui.horizontal(|ui| {
                    ui.label("Name:");
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.ui.save_name_buffer)
                            .hint_text("Takt benennen…")
                            .desired_width(180.0),
                    );
                    let submit = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.button("💾 Speichern").clicked() || submit {
                        let name = std::mem::take(&mut self.ui.save_name_buffer);
                        self.save_current_pattern(name);
                    }
                });

                ui.separator();

                if self.editor.library.patterns.is_empty() {
                    ui.weak("Noch keine gespeicherten Takte.");
                    return;
                }

                // List: load / rename / delete saved patterns. Collect the id to
                // delete and apply it after iterating to avoid borrow conflicts.
                let mut to_delete: Option<u64> = None;
                let mut to_load: Option<u64> = None;

                egui::ScrollArea::vertical().show(ui, |ui| {
                    for pattern in &mut self.editor.library.patterns {
                        ui.horizontal(|ui| {
                            if ui.button("▶ Laden").clicked() {
                                to_load = Some(pattern.id);
                            }
                            ui.add(
                                egui::TextEdit::singleline(&mut pattern.name).desired_width(160.0),
                            );
                            let measures = pattern.score.len();
                            ui.weak(format!("{} Takt(e) · {} BPM", measures, pattern.bpm));
                            if ui.button("🗑").on_hover_text("Löschen").clicked() {
                                to_delete = Some(pattern.id);
                            }
                        });
                    }
                });

                if let Some(id) = to_load {
                    self.load_pattern(id);
                }
                if let Some(id) = to_delete {
                    self.delete_pattern(id);
                }
            },
        );
    }
}
