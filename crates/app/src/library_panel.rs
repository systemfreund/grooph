use crate::state::PendingLoad;
use crate::{Grooph, Mode, platform};
use eframe::egui;
use log::warn;

/// How long "Link copied" stays visible.
const LINK_COPIED_NOTICE_S: f64 = 2.0;

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
                            ui.label("Active:");
                            ui.strong(name);
                        }
                        None => {
                            ui.weak("Not saved");
                        }
                    }
                    if dirty {
                        ui.label("•").on_hover_text("Unsaved changes");
                    }
                });

                ui.horizontal(|ui| {
                    // "Save" overwrites the active pattern in place, or creates a
                    // new one (using the name field) when nothing is active.
                    let save_label =
                        if active_name.is_some() { "💾 Save" } else { "💾 Save (new)" };
                    if ui.button(save_label).clicked() {
                        let name = std::mem::take(&mut self.ui.save_name_buffer);
                        self.save_active_pattern(name);
                    }

                    ui.separator();

                    // "Save as" always creates a new entry from the name field.
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.ui.save_name_buffer)
                            .hint_text("New name…")
                            .desired_width(160.0),
                    );
                    let submit = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.button("➕ Save as").clicked() || submit {
                        let name = std::mem::take(&mut self.ui.save_name_buffer);
                        self.save_pattern_as(name);
                    }
                });

                ui.horizontal(|ui| {
                    if ui
                        .button("Copy link")
                        .on_hover_text("Copy a link that opens this rhythm at the current tempo")
                        .clicked()
                    {
                        self.copy_share_link(ui);
                    }
                    let now = ui.input(|i| i.time);
                    if let Some(at) = self.ui.link_copied_at {
                        if now - at < LINK_COPIED_NOTICE_S {
                            ui.weak("Link copied");
                            ui.ctx().request_repaint_after_secs(0.5);
                        } else {
                            self.ui.link_copied_at = None;
                        }
                    }
                });

                ui.separator();

                if self.editor.library.patterns.is_empty() {
                    ui.weak("No saved measures yet.");
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
                                if ui.button("▶ Load").clicked() {
                                    to_load = Some(pattern.id);
                                }
                                ui.add(
                                    egui::TextEdit::singleline(&mut pattern.name)
                                        .desired_width(160.0),
                                );
                                let measures = pattern.score.len();
                                ui.weak(format!("{} measure(s) · {} BPM", measures, pattern.bpm));
                                if ui.button("🗑").on_hover_text("Delete").clicked() {
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

    /// Modal shown when loading a pattern or opening a shared link would
    /// discard unsaved changes. Offers to save first, discard, or cancel.
    /// Reads/clears `ui.pending_load`.
    pub(super) fn load_confirm_dialog(&mut self, ui: &mut egui::Ui) {
        let (message, action) = match &self.ui.pending_load {
            None => return,
            Some(PendingLoad::Pattern(_)) => {
                ("The current measure has unsaved changes. Save before loading?", "Load")
            }
            Some(PendingLoad::Link(_)) => (
                "The link replaces the current measure, which has unsaved changes. Save first?",
                "Open link",
            ),
        };

        egui::Window::new("Unsaved changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ui.ctx(), |ui| {
                ui.label(message);
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    if ui.button(format!("Save & {action}")).clicked() {
                        let name = std::mem::take(&mut self.ui.save_name_buffer);
                        self.save_active_pattern(name);
                        if let Some(pending) = self.ui.pending_load.take() {
                            self.finish_pending_load(pending);
                        }
                    }
                    if ui.button(format!("Discard & {action}")).clicked()
                        && let Some(pending) = self.ui.pending_load.take()
                    {
                        self.finish_pending_load(pending);
                    }
                    if ui.button("Cancel").clicked() {
                        self.ui.pending_load = None;
                    }
                });
            });
    }

    /// Shown when the app was opened with a link it cannot read.
    pub(super) fn link_error_dialog(&mut self, ui: &mut egui::Ui) {
        let Some(err) = &self.ui.link_error else {
            return;
        };
        let mut close = false;
        egui::Window::new("Link could not be opened")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ui.ctx(), |ui| {
                ui.label(err);
                ui.add_space(8.0);
                close = ui.button("OK").clicked();
            });
        if close {
            self.ui.link_error = None;
        }
    }

    /// Copy a link to the working score and tempo to the clipboard.
    fn copy_share_link(&mut self, ui: &egui::Ui) {
        let base = platform::share_base_url();
        match grooph_link::share_url(&base, &self.editor.score, self.playback_ctl.bpm) {
            Some(url) => {
                ui.ctx().copy_text(url);
                self.ui.link_copied_at = Some(ui.input(|i| i.time));
            }
            None => warn!("The score cannot be written as a link"),
        }
    }
}
