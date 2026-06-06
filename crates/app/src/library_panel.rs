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

                let active_id = self.editor.active_pattern_id;
                let active_name =
                    active_id.and_then(|id| self.editor.library.name_of(id).map(|s| s.to_string()));

                // Header: status of the working score + save controls. A trailing
                // "*" marks unsaved changes relative to the active/saved state.
                let dirty = self.editor.dirty;
                ui.horizontal(|ui| {
                    match &active_name {
                        Some(name) => {
                            ui.label("Aktiv:");
                            ui.strong(name);
                        }
                        None => {
                            ui.weak("Nicht gespeichert");
                        }
                    }
                    if dirty {
                        ui.label("•").on_hover_text("Ungespeicherte Änderungen");
                    }
                });

                ui.horizontal(|ui| {
                    // "Save" overwrites the active pattern in place, or creates a
                    // new one (using the name field) when nothing is active.
                    let save_label = if active_name.is_some() {
                        "💾 Speichern"
                    } else {
                        "💾 Speichern (neu)"
                    };
                    if ui.button(save_label).clicked() {
                        let name = std::mem::take(&mut self.ui.save_name_buffer);
                        self.save_active_pattern(name);
                    }

                    ui.separator();

                    // "Save as" always creates a new entry from the name field.
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.ui.save_name_buffer)
                            .hint_text("Neuer Name…")
                            .desired_width(160.0),
                    );
                    let submit = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.button("➕ Speichern als").clicked() || submit {
                        let name = std::mem::take(&mut self.ui.save_name_buffer);
                        self.save_pattern_as(name);
                    }
                });

                ui.separator();

                if self.editor.library.patterns.is_empty() {
                    ui.weak("Noch keine gespeicherten Takte.");
                    return;
                }

                // List: load / rename / delete saved patterns. Collect the id to
                // load/delete and apply it after iterating to avoid borrow conflicts.
                let mut to_delete: Option<u64> = None;
                let mut to_load: Option<u64> = None;

                let active_bg = ui.visuals().selection.bg_fill;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for pattern in &mut self.editor.library.patterns {
                        let is_active = active_id == Some(pattern.id);
                        let mut row = |ui: &mut egui::Ui| {
                            ui.horizontal(|ui| {
                                if ui.button("▶ Laden").clicked() {
                                    to_load = Some(pattern.id);
                                }
                                ui.add(
                                    egui::TextEdit::singleline(&mut pattern.name)
                                        .desired_width(160.0),
                                );
                                let measures = pattern.score.len();
                                ui.weak(format!("{} Takt(e) · {} BPM", measures, pattern.bpm));
                                if ui.button("🗑").on_hover_text("Löschen").clicked() {
                                    to_delete = Some(pattern.id);
                                }
                            });
                        };

                        if is_active {
                            egui::Frame::new()
                                .fill(active_bg)
                                .inner_margin(egui::Margin::symmetric(4, 2))
                                .corner_radius(4)
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    row(ui);
                                });
                        } else {
                            row(ui);
                        }
                    }
                });

                if let Some(id) = to_load {
                    self.request_load_pattern(id);
                }
                if let Some(id) = to_delete {
                    self.delete_pattern(id);
                }
            },
        );
    }

    /// Modal shown when loading a pattern would discard unsaved changes. Offers
    /// to save first, discard, or cancel. Reads/clears `ui.pending_load`.
    pub(super) fn load_confirm_dialog(&mut self, ui: &mut egui::Ui) {
        let Some(target_id) = self.ui.pending_load else {
            return;
        };

        egui::Window::new("Ungespeicherte Änderungen")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ui.ctx(), |ui| {
                ui.label(
                    "Der aktuelle Takt hat ungespeicherte Änderungen. Vor dem Laden speichern?",
                );
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    if ui.button("Speichern & Laden").clicked() {
                        let name = std::mem::take(&mut self.ui.save_name_buffer);
                        self.save_active_pattern(name);
                        self.ui.pending_load = None;
                        self.load_pattern(target_id);
                    }
                    if ui.button("Verwerfen & Laden").clicked() {
                        self.ui.pending_load = None;
                        self.load_pattern(target_id);
                    }
                    if ui.button("Abbrechen").clicked() {
                        self.ui.pending_load = None;
                    }
                });
            });
    }
}
