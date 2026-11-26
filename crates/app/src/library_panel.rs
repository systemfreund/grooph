use crate::Grooph;
use crate::documents::Document;
use crate::library::SavedPattern;
use crate::pickers::ROW_HEIGHT;
use crate::pickers::{dialog_scroll_height, note_focused_field};
use crate::state::PendingLoad;
use eframe::egui;
use eframe::egui::{
    Align, Button, FontFamily, FontId, Frame, Id, Label, Margin, RichText, Sense, Stroke,
    UiBuilder, vec2,
};
use grooph_layout::staff_layout::{ROOM_BELOW_EM, StaffOpts, build_staff_layout};
use grooph_measure::{Score, StaffKind};
use grooph_render::staff::{PlaybackMarks, RenderContext, draw_staff};

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
pub(crate) struct PreviewLook {
    pub(crate) family: FontFamily,
    pub(crate) stem_length_factor: f32,
}

/// The first bar of `score` as notation in a row `width` wide.
pub(crate) fn pattern_preview(ui: &mut egui::Ui, score: &Score, width: f32, look: &PreviewLook) {
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
        text_rows: Default::default(),
        bar_rests: true,
        veil_hidden_rests: false,
    };
    // The first bar: two side by side would each need about 170 pt even
    // for quarters, with the meter, more than a phone's dialog has.
    let first = Score::single(score.measures()[0].clone());
    let staff = build_staff_layout(&first, &opts);
    let mut clipped = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    clipped.set_clip_rect(rect.intersect(ui.clip_rect()));
    let opts = opts.rescaled(staff.scale);
    let ctx = RenderContext {
        marks: PlaybackMarks::Anchor(None),
        ..RenderContext::new(&first, &staff, &opts)
    };
    draw_staff(&mut clipped, &ctx);
}

/// What a tap on a library card asks for.
enum CardAction {
    Load,
    /// ✏: open or close renaming.
    Edit,
    /// The new name is in.
    Renamed,
    Delete,
}

/// Text size of a card's name, and of the line below it.
const NAME_SIZE: f32 = 18.0;
const META_SIZE: f32 = 14.0;

/// How long a card says "✓ Saved".
const SAVED_NOTICE_S: f64 = 1.5;

/// What a card shows besides its name.
struct CardState {
    /// The score tab's pattern: an accent border.
    active: bool,
    /// That has changes since: a dot after the name.
    unsaved: bool,
    /// "✓ Saved" instead of the line below the name.
    just_saved: bool,
    /// Scroll the card into view.
    scroll_to: bool,
    /// Renaming is open.
    editing: bool,
}

/// One saved rhythm: name, `meta` below it, ✏, and the first bar of
/// `score`. A tap anywhere else on the card loads it; ✏ opens
/// renaming and deleting under it. The new name is typed into `typed` and
/// only becomes the name on ✔ or Enter, so closing renaming any other way
/// leaves the old one.
fn library_card(
    ui: &mut egui::Ui,
    name: &mut String,
    typed: &mut String,
    meta: String,
    score: &Score,
    look: &PreviewLook,
    state: CardState,
) -> Option<CardAction> {
    let CardState { active, unsaved, just_saved, scroll_to, editing } = state;
    let mut action = None;
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
                // Name over the line below it, ✏ after them: added last, it
                // is centered on both lines.
                ui.horizontal(|ui| {
                    let edit_size = vec2(button_width(ui, "✏"), ROW_HEIGHT);
                    let edit_room = edit_size.x + ui.spacing().item_spacing.x;
                    let text_width = ui.available_width() - edit_room;
                    ui.vertical(|ui| {
                        ui.set_width(text_width);
                        let shown = if unsaved { format!("{name} •") } else { name.clone() };
                        ui.add(
                            Label::new(RichText::new(shown).size(NAME_SIZE))
                                .truncate()
                                .selectable(false),
                        );
                        let meta = if just_saved {
                            let accent = ui.visuals().selection.stroke.color;
                            RichText::new("✓ Saved").color(accent)
                        } else {
                            RichText::new(meta).weak()
                        };
                        ui.add(Label::new(meta.size(META_SIZE)).truncate().selectable(false));
                    });
                    let edit = Button::new("✏").selected(editing).min_size(edit_size);
                    if ui.add(edit).on_hover_text("Rename or delete").clicked() {
                        action = Some(CardAction::Edit);
                    }
                });
                let width = ui.available_width();
                pattern_preview(ui, score, width, look);
                if editing {
                    match rename_row(ui, typed) {
                        Some(Renamed::Done) => {
                            *name = typed.trim().to_owned();
                            action = Some(CardAction::Renamed);
                        }
                        Some(Renamed::Delete) => action = Some(CardAction::Delete),
                        None => {}
                    }
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

/// Name field, delete and done under a card.
fn rename_row(ui: &mut egui::Ui, name: &mut String) -> Option<Renamed> {
    // The field first, the buttons at its height: a row centers what it
    // holds on the height it had so far, so a field added after them would
    // sit lower.
    let gap = ui.spacing().item_spacing.x;
    let valid = !name.trim().is_empty();
    let (field, ok, delete) = ui
        .horizontal(|ui| {
            let delete_width = button_width(ui, "🗑");
            let ok_width = button_width(ui, "✔");
            let width = ui.available_width() - delete_width - ok_width - 2.0 * gap;
            // `desired_width` would leave out the field's margin and push
            // the dialog wider: the size is the field's whole.
            let field = ui.add_sized(
                vec2(width, ui.spacing().interact_size.y),
                egui::TextEdit::singleline(name).id(Id::unique("rename_library_item")),
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
    // The dialog moves it above an on-screen keyboard.
    if field.has_focus() {
        note_focused_field(ui.ctx(), field.rect);
    }
    let submit = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let done = ok || (submit && valid);
    if delete {
        Some(Renamed::Delete)
    } else if done {
        Some(Renamed::Done)
    } else {
        None
    }
}

/// What "Save" does in a half of the library: overwrite the active entry
/// (named in its label) or save a new one, and whether it is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SaveTarget {
    pub(crate) overwrites: Option<String>,
    pub(crate) enabled: bool,
}

/// "Save" as `target` says, and "Save as new". Returns which was tapped.
fn save_buttons(ui: &mut egui::Ui, target: &SaveTarget) -> Option<SaveAs> {
    let label =
        target.overwrites.as_ref().map_or("💾 Save".to_owned(), |name| format!("💾 Save {name}"));
    let mut tapped = None;
    ui.columns(2, |cols| {
        let size = vec2(cols[0].available_width(), ROW_HEIGHT);
        let save = Button::new(label).truncate().min_size(size);
        if cols[0].add_enabled(target.enabled, save).clicked() {
            let update = target.overwrites.is_some();
            tapped = Some(if update { SaveAs::Update } else { SaveAs::New });
        }
        if cols[1].add(Button::new("➕ Save as new").min_size(size)).clicked() {
            tapped = Some(SaveAs::New);
        }
    });
    tapped
}

/// The rhythms' save buttons, by the page the library was opened from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RhythmSaving {
    /// The score tab: "Save" and "Save as new" keep its score.
    Score,
    /// Practice: "Save as new" only, which saves its rhythm and opens it in
    /// the score tab (see `Grooph::request_save_practice_rhythm`).
    Practice,
    /// The metronome: none, its click bar is no pattern.
    Off,
}

/// What a save button asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SaveAs {
    /// Overwrite the active one.
    Update,
    /// A new one, under a name to type at once.
    New,
}

/// Bars and tempo, the line under a rhythm's name.
fn pattern_summary(pattern: &SavedPattern) -> String {
    let bars = pattern.score.len();
    let bars = if bars == 1 { "1 bar".to_owned() } else { format!("{bars} bars") };
    format!("{bars} · {} BPM", pattern.bpm)
}

impl Grooph {
    /// Saved rhythms, as cards: a tap opens one in the score tab, ✏
    /// renames or deletes it.
    pub(super) fn library_panel(&mut self, ui: &mut egui::Ui) {
        self.rhythms(ui);
    }

    /// The card that says "✓ Saved", and the one to scroll into view once.
    fn card_notices(&mut self, ui: &egui::Ui) -> (Option<u64>, Option<u64>) {
        let now = ui.input(|i| i.time);
        let just_saved =
            self.ui.library.saved.filter(|(_, at)| now - at < SAVED_NOTICE_S).map(|(id, _)| id);
        if just_saved.is_some() {
            ui.ctx().request_repaint_after_secs(0.5);
        }
        (just_saved, self.ui.library.scroll_to.take())
    }

    /// The card of pattern `id` says "✓ Saved" from UI time `now` on and
    /// scrolls into view.
    pub(crate) fn saved(&mut self, id: u64, now: f64) {
        self.ui.library.saved = Some((id, now));
        self.ui.library.scroll_to = Some(id);
    }

    /// What a save button of the rhythms asked for; a new pattern opens its
    /// name. Returns the pattern's id.
    pub(crate) fn save_rhythm(&mut self, save: SaveAs, now: f64) -> u64 {
        let id = match save {
            SaveAs::Update => self.save_active_pattern(),
            SaveAs::New => {
                let id = self.save_pattern_as();
                self.ui.library.renaming_pattern = Some(id);
                self.ui.library.rename_typed =
                    self.editor.library.get(id).map(|p| p.name.clone()).unwrap_or_default();
                id
            }
        };
        self.saved(id, now);
        id
    }

    /// "Save" for the rhythms, which keeps the score tab's score: its
    /// pattern while it has changes, a new one without a pattern.
    pub(crate) fn rhythm_save_target(&self) -> SaveTarget {
        let active =
            self.editor.docs.score.active_pattern_id.and_then(|id| self.editor.library.get(id));
        SaveTarget {
            overwrites: active.map(|p| p.name.clone()),
            enabled: active.is_none() || self.editor.docs.score.dirty,
        }
    }

    /// Which save buttons the rhythms have: those of the page the library
    /// was opened from (the page shown, or behind the More screen).
    pub(crate) fn rhythm_saving(&self) -> RhythmSaving {
        match self.document() {
            Document::Score => RhythmSaving::Score,
            Document::Practice => RhythmSaving::Practice,
            Document::Metronome => RhythmSaving::Off,
        }
    }

    /// Saved rhythms: a tap opens one in the score tab.
    fn rhythms(&mut self, ui: &mut egui::Ui) {
        let active_id = self.editor.docs.score.active_pattern_id;
        let dirty = self.editor.docs.score.dirty;
        let now = ui.input(|i| i.time);
        match self.rhythm_saving() {
            RhythmSaving::Score => {
                let target = self.rhythm_save_target();
                if let Some(save) = save_buttons(ui, &target) {
                    self.save_rhythm(save, now);
                }
                if target.overwrites.is_none() {
                    ui.weak("The score is not saved.");
                }
                ui.add_space(4.0);
            }
            RhythmSaving::Practice => {
                let size = vec2(ui.available_width(), ROW_HEIGHT);
                if ui.add(Button::new("➕ Save as new").min_size(size)).clicked() {
                    self.request_save_practice_rhythm(now);
                }
                ui.weak("Saves the practice rhythm, to edit in the score tab.");
                ui.add_space(4.0);
            }
            RhythmSaving::Off => {}
        }
        // A new score of the score tab, on the kit's staff or the one
        // line: offered while that tab is shown.
        if self.ui.view.show_score_tab {
            ui.horizontal(|ui| {
                if ui.button("🎼 New drum groove").clicked() {
                    self.request_new_score(StaffKind::Kit);
                }
                if ui.button("🎵 New rhythm").clicked() {
                    self.request_new_score(StaffKind::Line);
                }
            });
        }
        ui.add_space(8.0);
        if self.editor.library.patterns.is_empty() {
            ui.weak("No saved rhythms yet.");
            return;
        }

        // Apply after the loop, which borrows the patterns.
        let mut picked = None;
        let (just_saved, scroll_to) = self.card_notices(ui);
        let look = &PreviewLook {
            family: self.ui.fonts.music_font_id.family.clone(),
            stem_length_factor: self.ui.layout.stem_length_factor,
        };
        let renaming = &mut self.ui.library.renaming_pattern;
        let typed = &mut self.ui.library.rename_typed;
        egui::ScrollArea::vertical().max_height(dialog_scroll_height(ui)).show(ui, |ui| {
            for pattern in &mut self.editor.library.patterns {
                let id = pattern.id;
                let active = active_id == Some(id);
                let state = CardState {
                    active,
                    unsaved: active && dirty,
                    just_saved: just_saved == Some(id),
                    scroll_to: scroll_to == Some(id),
                    editing: *renaming == Some(id),
                };
                let meta = pattern_summary(pattern);
                let action =
                    library_card(ui, &mut pattern.name, typed, meta, &pattern.score, look, state);
                match action {
                    Some(CardAction::Edit) => {
                        *renaming = if *renaming == Some(id) { None } else { Some(id) };
                        typed.clone_from(&pattern.name);
                    }
                    Some(CardAction::Renamed) => *renaming = None,
                    Some(action) => picked = Some((id, action)),
                    None => {}
                }
                ui.add_space(6.0);
            }
        });
        match picked {
            Some((id, CardAction::Load)) => {
                self.ui.library.renaming_pattern = None;
                self.request_load_pattern(id);
            }
            Some((id, CardAction::Delete)) => {
                self.ui.library.renaming_pattern = None;
                self.delete_pattern(id);
            }
            _ => {}
        }
    }

    /// Modal shown when loading a pattern or opening a shared link would
    /// discard unsaved changes. Offers to save first, discard, or cancel.
    /// Reads/clears `ui.dialogs.pending_load`.
    pub(super) fn load_confirm_dialog(&mut self, ui: &mut egui::Ui) {
        let (message, action) = match &self.ui.dialogs.pending_load {
            None => return,
            Some(PendingLoad::Pattern(_)) => {
                ("The current measure has unsaved changes. Save before loading?", "Load")
            }
            Some(PendingLoad::Link(_)) => (
                "The link replaces the current measure, which has unsaved changes. Save first?",
                "Open link",
            ),
            Some(PendingLoad::NewScore(StaffKind::Kit)) => (
                "A new drum groove replaces the current score, which has unsaved changes. Save first?",
                "Start",
            ),
            Some(PendingLoad::NewScore(StaffKind::Line)) => (
                "A new rhythm replaces the current score, which has unsaved changes. Save first?",
                "Start",
            ),
            Some(PendingLoad::PracticeRhythm) => (
                "The saved rhythm opens in the score tab and replaces its score, which has unsaved changes. Save first?",
                "Continue",
            ),
        };

        let now = ui.input(|i| i.time);
        // A modal, so that it comes up above the library dialog it is
        // asked from.
        let save = format!("Save & {action}");
        let discard = format!("Discard & {action}");
        let width = dialog_width(ui.ctx());
        egui::Modal::new(egui::Id::unique("unsaved_changes")).show(ui.ctx(), |ui| {
            ui.set_max_width(width);
            ui.heading("Unsaved changes");
            ui.add_space(8.0);
            ui.label(message);
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            match choice_buttons(ui, width, &[&save, &discard, "Cancel"]) {
                Some(0) => {
                    self.save_active_pattern();
                    if let Some(pending) = self.ui.dialogs.pending_load.take() {
                        self.finish_pending_load(pending, now);
                    }
                }
                Some(1) => {
                    if let Some(pending) = self.ui.dialogs.pending_load.take() {
                        self.finish_pending_load(pending, now);
                    }
                }
                Some(_) => self.ui.dialogs.pending_load = None,
                None => {}
            }
        });
    }

    /// Shown when the app was opened with a link it cannot read.
    pub(super) fn link_error_dialog(&mut self, ui: &mut egui::Ui) {
        let Some(err) = &self.ui.dialogs.link_error else {
            return;
        };
        let mut close = false;
        egui::Window::new("Link could not be opened")
            .max_width(dialog_width(ui.ctx()))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ui.ctx(), |ui| {
                ui.label(err);
                ui.add_space(8.0);
                close = ui.button("OK").clicked();
            });
        if close {
            self.ui.dialogs.link_error = None;
        }
    }
}

/// Widest a dialog gets: 380 points, less on a phone so that it keeps a
/// margin to the screen edges.
pub(crate) fn dialog_width(ctx: &egui::Context) -> f32 {
    (ctx.content_rect().width() - 48.0).clamp(200.0, 380.0)
}

/// A dialog's buttons side by side, or stacked at full width when the row
/// would be wider than `width`. Returns the index of the clicked one.
pub(crate) fn choice_buttons(ui: &mut egui::Ui, width: f32, labels: &[&str]) -> Option<usize> {
    let pad = ui.spacing().button_padding.x * 2.0;
    let gaps = ui.spacing().item_spacing.x * labels.len().saturating_sub(1) as f32;
    let row: f32 = labels
        .iter()
        .map(|label| {
            let text = egui::WidgetText::from(*label);
            let galley = text.into_galley(
                ui,
                Some(egui::TextWrapMode::Extend),
                f32::INFINITY,
                egui::TextStyle::Button,
            );
            galley.size().x + pad
        })
        .sum::<f32>()
        + gaps;

    let mut clicked = None;
    if row <= width {
        ui.horizontal(|ui| {
            for (i, label) in labels.iter().enumerate() {
                if ui.button(*label).clicked() {
                    clicked = Some(i);
                }
            }
        });
    } else {
        ui.set_width(width);
        ui.vertical_centered_justified(|ui| {
            for (i, label) in labels.iter().enumerate() {
                if ui.button(*label).clicked() {
                    clicked = Some(i);
                }
            }
        });
    }
    clicked
}
