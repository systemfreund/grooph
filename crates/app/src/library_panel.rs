use crate::Grooph;
use crate::library::SavedPattern;
use crate::pickers::ROW_HEIGHT;
use crate::pickers::dialog_scroll_height;
use crate::state::PendingLoad;
use eframe::egui;
use eframe::egui::{
    Align, Button, FontFamily, FontId, Frame, Id, Label, Margin, RichText, Sense, Stroke,
    UiBuilder, vec2,
};
use grooph_layout::staff_layout::{ROOM_BELOW_EM, StaffOpts, Voice, build_staff_layout};
use grooph_measure::Score;
use grooph_render::staff::{PlaybackMarks, draw_staff};

/// Glyph size of a pattern's preview.
const PREVIEW_EM: f32 = 28.0;
/// Height of a preview: the staff with room for stems and beams above and
/// rests and accents below.
const PREVIEW_HEIGHT_EM: f32 = 2.0;

/// Width of a button showing `text`, at least [`ROW_HEIGHT`]: the style's
/// padding makes a button wider than its `min_size`, and a row planned with
/// that would push the card, and the cards after it, wider.
fn button_width(ui: &egui::Ui, text: &str) -> f32 {
    let galley = egui::WidgetText::from(text).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    (galley.size().x + 2.0 * ui.spacing().button_padding.x).max(ROW_HEIGHT)
}

/// How a preview draws the notes, taken from the main staff.
struct PreviewLook {
    family: FontFamily,
    stem_length_factor: f32,
}

/// The first bar of `score` as notation in a row `width` wide.
fn pattern_preview(ui: &mut egui::Ui, score: &Score, width: f32, look: &PreviewLook) {
    let (rect, _) =
        ui.allocate_exact_size(vec2(width, PREVIEW_HEIGHT_EM * PREVIEW_EM), Sense::hover());
    // Off screen in the list: no layout to build.
    if !ui.is_rect_visible(rect) {
        return;
    }
    let opts = StaffOpts {
        rect,
        font_id: FontId::new(PREVIEW_EM, look.family.clone()),
        pixels_per_point: ui.ctx().pixels_per_point(),
        em: PREVIEW_EM,
        y_offset: 0.0,
        stem_length_factor: look.stem_length_factor,
        stem_thickness_factor: 0.04,
        accent_displacement: 0.07,
        min_measure_width_em: 2.0,
        note_width_em: 0.36,
        system_spacing_em: 0.2,
        layout_clef_first: false,
        // The bar stands on its own: say its meter.
        time_signature_first: true,
        room_below_em: ROOM_BELOW_EM,
        lower_label_room_em: 0.0,
        bar_rests: true,
    };
    // The first bar: two side by side would each need about 170 pt even
    // for quarters, with the meter, more than a phone's dialog has.
    let first = Score::single(score.measures[0].clone());
    let staff = build_staff_layout(&first, &opts);
    let mut clipped = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    clipped.set_clip_rect(rect.intersect(ui.clip_rect()));
    draw_staff(
        &mut clipped,
        &first,
        &staff,
        None,
        Voice::Upper,
        None,
        PlaybackMarks::Anchor(None),
        None,
        &opts.rescaled(staff.scale),
    );
}

/// What a tap on a library card asks for.
enum CardAction {
    Load,
    Delete,
}

/// Text size of a card's name, and of its bars and tempo below.
const NAME_SIZE: f32 = 18.0;
const META_SIZE: f32 = 14.0;

/// How long a card says "✓ Saved".
const SAVED_NOTICE_S: f64 = 1.5;

/// What a card shows besides its pattern.
struct CardState {
    /// The pattern on the staff: an accent border.
    active: bool,
    /// The rhythm on the staff has changes since: a dot after the name.
    unsaved: bool,
    /// "✓ Saved" instead of bars and tempo.
    just_saved: bool,
    /// Scroll the card into view.
    scroll_to: bool,
}

/// One saved pattern: name, bars and tempo, ✏, and the first bar below. A
/// tap anywhere else on the card loads it; ✏ opens renaming and deleting
/// under it.
fn pattern_card(
    ui: &mut egui::Ui,
    pattern: &mut SavedPattern,
    state: CardState,
    renaming: &mut Option<u64>,
    look: &PreviewLook,
) -> Option<CardAction> {
    let CardState { active, unsaved, just_saved, scroll_to } = state;
    let mut action = None;
    let editing = *renaming == Some(pattern.id);
    // Outlined, not filled: buttons share the card fill and would vanish.
    let stroke = if active {
        Stroke::new(2.0, ui.visuals().selection.stroke.color)
    } else {
        ui.visuals().widgets.noninteractive.bg_stroke
    };
    // The card senses before its contents, so ✏ and the name field keep
    // their own taps.
    let card = ui.scope_builder(UiBuilder::new().sense(Sense::click()), |ui| {
        Frame::new().stroke(stroke).corner_radius(8).inner_margin(Margin::symmetric(10, 6)).show(
            ui,
            |ui| {
                ui.set_width(ui.available_width());
                // Name over bars and tempo, ✏ after them: added last, it is
                // centered on both lines.
                ui.horizontal(|ui| {
                    let edit_size = vec2(button_width(ui, "✏"), ROW_HEIGHT);
                    let text_width =
                        ui.available_width() - edit_size.x - ui.spacing().item_spacing.x;
                    ui.vertical(|ui| {
                        ui.set_width(text_width);
                        let name = if unsaved {
                            format!("{} •", pattern.name)
                        } else {
                            pattern.name.clone()
                        };
                        ui.add(
                            Label::new(RichText::new(name).size(NAME_SIZE))
                                .truncate()
                                .selectable(false),
                        );
                        let meta = if just_saved {
                            let accent = ui.visuals().selection.stroke.color;
                            RichText::new("✓ Saved").color(accent)
                        } else {
                            let bars = pattern.score.len();
                            let bars =
                                if bars == 1 { "1 bar".to_owned() } else { format!("{bars} bars") };
                            RichText::new(format!("{bars} · {} BPM", pattern.bpm)).weak()
                        };
                        ui.add(Label::new(meta.size(META_SIZE)).selectable(false));
                    });
                    let edit = Button::new("✏").selected(editing).min_size(edit_size);
                    if ui.add(edit).on_hover_text("Rename or delete").clicked() {
                        *renaming = if editing { None } else { Some(pattern.id) };
                    }
                });
                let width = ui.available_width();
                pattern_preview(ui, &pattern.score, width, look);
                match editing.then(|| rename_row(ui, pattern)).flatten() {
                    Some(Renamed::Done) => *renaming = None,
                    Some(Renamed::Delete) => action = Some(CardAction::Delete),
                    None => {}
                }
            },
        );
    });
    if scroll_to {
        card.response.scroll_to_me(Some(Align::Center));
    }
    if card.response.clicked() && action.is_none() {
        action = Some(CardAction::Load);
    }
    card.response.on_hover_cursor(egui::CursorIcon::PointingHand);
    action
}

/// How renaming a card ended.
enum Renamed {
    Done,
    Delete,
}

/// Name field, delete and done under a card, as for presets.
fn rename_row(ui: &mut egui::Ui, pattern: &mut SavedPattern) -> Option<Renamed> {
    // The field first, the buttons at its height: a row centers what it
    // holds on the height it had so far, so a field added after them would
    // sit lower.
    let gap = ui.spacing().item_spacing.x;
    let valid = !pattern.name.trim().is_empty();
    let (field, ok, delete) = ui
        .horizontal(|ui| {
            let delete_width = button_width(ui, "🗑");
            let ok_width = button_width(ui, "✔");
            let width = ui.available_width() - delete_width - ok_width - 2.0 * gap;
            // `desired_width` would leave out the field's margin and push
            // the dialog wider: the size is the field's whole.
            let field = ui.add_sized(
                vec2(width, ui.spacing().interact_size.y),
                egui::TextEdit::singleline(&mut pattern.name).id(Id::unique("rename_pattern")),
            );
            let height = field.rect.height();
            let delete = ui
                .add(Button::new("🗑").min_size(vec2(delete_width, height)))
                .on_hover_text("Delete");
            let ok = ui.add_enabled(valid, Button::new("✔").min_size(vec2(ok_width, height)));
            (field, ok.clicked(), delete.clicked())
        })
        .inner;
    // Typing goes to the name while nothing else has the focus.
    if ui.memory(|m| m.focused().is_none()) {
        field.request_focus();
    }
    let submit = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let done = ok || (submit && valid);
    if delete {
        Some(Renamed::Delete)
    } else if done {
        pattern.name = pattern.name.trim().to_owned();
        Some(Renamed::Done)
    } else {
        None
    }
}

impl Grooph {
    /// Saved rhythms as cards: a tap loads one, ✏ renames or deletes it.
    pub(super) fn library_panel(&mut self, ui: &mut egui::Ui) {
        let active_id = self.editor.active_pattern_id;
        // The rhythm's, also while the metronome's click bar is edited.
        let dirty = self.metronome_stash().map_or(self.editor.dirty, |stash| stash.dirty);
        let active_name =
            active_id.and_then(|id| self.editor.library.get(id)).map(|p| p.name.clone());

        // "Save" names the pattern it overwrites, and is off while that has
        // no changes; without one it saves a new one. "Save as new" always
        // adds one, under a name to type at once.
        let save_label =
            active_name.as_ref().map_or("💾 Save".to_owned(), |name| format!("💾 Save {name}"));
        let mut saved = None;
        ui.columns(2, |cols| {
            let size = vec2(cols[0].available_width(), ROW_HEIGHT);
            let save = Button::new(save_label).truncate().min_size(size);
            let can_save = active_name.is_none() || dirty;
            if cols[0].add_enabled(can_save, save).clicked() {
                let id = self.save_active_pattern();
                if active_name.is_none() {
                    self.ui.renaming_pattern = Some(id);
                }
                saved = Some(id);
            }
            if cols[1].add(Button::new("➕ Save as new").min_size(size)).clicked() {
                let id = self.save_pattern_as();
                self.ui.renaming_pattern = Some(id);
                saved = Some(id);
            }
        });
        if let Some(id) = saved {
            self.ui.pattern_saved = Some((id, ui.input(|i| i.time)));
            self.ui.scroll_to_pattern = Some(id);
        }
        if active_id.is_none() {
            ui.weak("The rhythm on the staff is not saved.");
        }
        ui.add_space(8.0);

        if self.editor.library.patterns.is_empty() {
            ui.weak("No saved rhythms yet.");
            return;
        }

        // Apply after the loop, which borrows the patterns.
        let mut to_delete = None;
        let mut to_load = None;
        let look = PreviewLook {
            family: self.ui.music_font_id.family.clone(),
            stem_length_factor: self.ui.layout.stem_length_factor,
        };
        let now = ui.input(|i| i.time);
        let just_saved =
            self.ui.pattern_saved.filter(|(_, at)| now - at < SAVED_NOTICE_S).map(|(id, _)| id);
        if just_saved.is_some() {
            ui.ctx().request_repaint_after_secs(0.5);
        }
        let scroll_to = self.ui.scroll_to_pattern.take();
        let renaming = &mut self.ui.renaming_pattern;
        egui::ScrollArea::vertical().max_height(dialog_scroll_height(ui)).show(ui, |ui| {
            for pattern in &mut self.editor.library.patterns {
                let active = active_id == Some(pattern.id);
                let state = CardState {
                    active,
                    unsaved: active && dirty,
                    just_saved: just_saved == Some(pattern.id),
                    scroll_to: scroll_to == Some(pattern.id),
                };
                let card = pattern_card(ui, pattern, state, renaming, &look);
                match card {
                    Some(CardAction::Load) => to_load = Some(pattern.id),
                    Some(CardAction::Delete) => to_delete = Some(pattern.id),
                    None => {}
                }
                ui.add_space(6.0);
            }
        });

        if let Some(id) = to_load {
            self.ui.renaming_pattern = None;
            self.request_load_pattern(id);
        }
        if let Some(id) = to_delete {
            self.ui.renaming_pattern = None;
            self.delete_pattern(id);
        }
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

        // A modal, so that it comes up above the library dialog it is
        // asked from.
        egui::Modal::new(egui::Id::unique("unsaved_changes")).show(ui.ctx(), |ui| {
            ui.heading("Unsaved changes");
            ui.add_space(8.0);
            ui.label(message);
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                if ui.button(format!("Save & {action}")).clicked() {
                    self.save_active_pattern();
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
}
