use crate::Grooph;
use crate::library::SavedPattern;
use crate::pickers::ROW_HEIGHT;
use crate::pickers::{dialog_scroll_height, note_focused_field};
use crate::presets::{self, PresetId, PresetSettings};
use crate::shell::{Tab, subdivision_short_label};
use crate::state::PendingLoad;
use eframe::egui;
use eframe::egui::{
    Align, Button, FontFamily, FontId, Frame, Id, Label, Margin, RichText, Sense, Stroke,
    UiBuilder, vec2,
};
use grooph_layout::staff_layout::{ROOM_BELOW_EM, StaffOpts, Voice, build_staff_layout};
use grooph_measure::Score;
use grooph_measure::generator::effective_subdivision;
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

/// A saved rhythm or preset, for the "✓ Saved" notice and scrolling to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LibraryItem {
    Pattern(u64),
    Preset(u64),
}

/// The two halves of the library.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum LibraryTab {
    #[default]
    Rhythms,
    Presets,
}

/// What a card shows besides its name.
struct CardState {
    /// What is on the staff or in the practice settings: an accent border.
    active: bool,
    /// That has changes since: a dot after the name.
    unsaved: bool,
    /// "✓ Saved" instead of the line below the name.
    just_saved: bool,
    /// Scroll the card into view.
    scroll_to: bool,
    /// Renaming is open. `None`: a built-in, without ✏.
    editing: Option<bool>,
}

/// One saved rhythm or preset: name, `meta` below it, ✏, and the first bar
/// of `preview`'s score. A tap anywhere else on the card loads it; ✏ opens
/// renaming and deleting under it. The new name is typed into `typed` and
/// only becomes the name on ✔ or Enter, so closing renaming any other way
/// leaves the old one.
fn library_card(
    ui: &mut egui::Ui,
    name: &mut String,
    typed: &mut String,
    meta: String,
    preview: Option<(&Score, &PreviewLook)>,
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
                    let text_width = ui.available_width() - editing.map_or(0.0, |_| edit_room);
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
                    if let Some(editing) = editing {
                        let edit = Button::new("✏").selected(editing).min_size(edit_size);
                        if ui.add(edit).on_hover_text("Rename or delete").clicked() {
                            action = Some(CardAction::Edit);
                        }
                    }
                });
                if let Some((score, look)) = preview {
                    let width = ui.available_width();
                    pattern_preview(ui, score, width, look);
                }
                if editing == Some(true) {
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

/// What a save button asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SaveAs {
    /// Overwrite the active one.
    Update,
    /// A new one, under a name to type at once.
    New,
}

/// The line under a preset's name: meter, notes, level and tempo.
fn preset_summary(settings: &PresetSettings) -> String {
    let generator = &settings.generator;
    let notes = effective_subdivision(generator.subdivision, generator.time_signature);
    let level = if generator.custom_figures_enabled {
        "Custom".to_owned()
    } else {
        format!("Level {}", generator.complexity)
    };
    format!(
        "{} · {} · {level} · {} BPM",
        generator.time_signature,
        subdivision_short_label(notes),
        settings.bpm
    )
}

/// Bars and tempo, the line under a rhythm's name.
fn pattern_summary(pattern: &SavedPattern) -> String {
    let bars = pattern.score.len();
    let bars = if bars == 1 { "1 bar".to_owned() } else { format!("{bars} bars") };
    format!("{bars} · {} BPM", pattern.bpm)
}

impl Grooph {
    /// Saved rhythms and practice presets, as cards: a tap loads one, ✏
    /// renames or deletes it. Returns a preset tapped, for the caller to open
    /// after its check for generator changes.
    pub(super) fn library_panel(&mut self, ui: &mut egui::Ui) -> Option<PresetId> {
        ui.columns(2, |cols| {
            for (col, (tab, label)) in cols
                .iter_mut()
                .zip([(LibraryTab::Rhythms, "Rhythms"), (LibraryTab::Presets, "Presets")])
            {
                let size = vec2(col.available_width(), ROW_HEIGHT);
                let button = Button::new(label).selected(self.ui.library_tab == tab).min_size(size);
                if col.add(button).clicked() && self.ui.library_tab != tab {
                    // The halves share the name being typed.
                    self.ui.library_tab = tab;
                    self.ui.renaming_pattern = None;
                    self.ui.renaming_preset = None;
                }
            }
        });
        ui.add_space(8.0);
        match self.ui.library_tab {
            LibraryTab::Rhythms => {
                self.rhythms(ui);
                None
            }
            LibraryTab::Presets => {
                ui.weak("A tap opens the practice page with the preset's settings.");
                ui.add_space(4.0);
                self.preset_cards(ui)
            }
        }
    }

    /// The card that says "✓ Saved", and the one to scroll into view once.
    fn card_notices(&mut self, ui: &egui::Ui) -> (Option<LibraryItem>, Option<LibraryItem>) {
        let now = ui.input(|i| i.time);
        let just_saved =
            self.ui.library_saved.filter(|(_, at)| now - at < SAVED_NOTICE_S).map(|(item, _)| item);
        if just_saved.is_some() {
            ui.ctx().request_repaint_after_secs(0.5);
        }
        (just_saved, self.ui.scroll_to_library_item.take())
    }

    /// The card of `item` says "✓ Saved" from UI time `now` on and scrolls
    /// into view.
    fn saved(&mut self, item: LibraryItem, now: f64) {
        self.ui.library_saved = Some((item, now));
        self.ui.scroll_to_library_item = Some(item);
    }

    /// What a save button of the rhythms asked for; a new pattern opens its
    /// name. Returns the pattern's id.
    pub(crate) fn save_rhythm(&mut self, save: SaveAs, now: f64) -> u64 {
        let id = match save {
            SaveAs::Update => self.save_active_pattern(),
            SaveAs::New => {
                let id = self.save_pattern_as();
                self.ui.renaming_pattern = Some(id);
                self.ui.rename_typed =
                    self.editor.library.get(id).map(|p| p.name.clone()).unwrap_or_default();
                id
            }
        };
        self.saved(LibraryItem::Pattern(id), now);
        id
    }

    /// What a save button of the presets asked for; a new preset opens its
    /// name. Returns the preset's id.
    pub(crate) fn save_preset(&mut self, save: SaveAs, now: f64) -> u64 {
        let id = match (save, self.editor.active_preset) {
            (SaveAs::Update, Some(PresetId::User(id))) => {
                self.editor.presets.update(id, self.preset_snapshot());
                id
            }
            _ => {
                let id = self.save_preset_as("");
                self.ui.renaming_preset = Some(id);
                self.ui.rename_typed =
                    self.editor.presets.get(id).map(|p| p.name.clone()).unwrap_or_default();
                id
            }
        };
        self.saved(LibraryItem::Preset(id), now);
        id
    }

    /// "Save" for the rhythms: the active pattern while it has changes, a
    /// new one without an active pattern.
    pub(crate) fn rhythm_save_target(&self) -> SaveTarget {
        let active = self.editor.active_pattern_id.and_then(|id| self.editor.library.get(id));
        SaveTarget {
            overwrites: active.map(|p| p.name.clone()),
            enabled: active.is_none() || self.rhythm_dirty(),
        }
    }

    /// "Save" for the presets: the active user preset while the settings
    /// differ from it; a new one for changed built-in settings or without
    /// an active preset. An unchanged built-in has nothing to save.
    pub(crate) fn preset_save_target(&self) -> SaveTarget {
        let snapshot = self.preset_snapshot();
        let active = self.editor.active_preset;
        let changed = active
            .and_then(|id| presets::lookup(&self.editor.presets, id))
            .is_none_or(|(_, settings)| settings != snapshot);
        let overwrites = match active {
            Some(PresetId::User(id)) => self.editor.presets.get(id).map(|p| p.name.clone()),
            _ => None,
        };
        SaveTarget { overwrites, enabled: changed }
    }

    /// Saved rhythms: a tap loads one onto the staff.
    fn rhythms(&mut self, ui: &mut egui::Ui) {
        let active_id = self.editor.active_pattern_id;
        let dirty = self.rhythm_dirty();
        let target = self.rhythm_save_target();
        if let Some(save) = save_buttons(ui, &target) {
            self.save_rhythm(save, ui.input(|i| i.time));
        }
        if target.overwrites.is_none() {
            ui.weak("The rhythm on the staff is not saved.");
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
            family: self.ui.music_font_id.family.clone(),
            stem_length_factor: self.ui.layout.stem_length_factor,
        };
        let renaming = &mut self.ui.renaming_pattern;
        let typed = &mut self.ui.rename_typed;
        egui::ScrollArea::vertical().max_height(dialog_scroll_height(ui)).show(ui, |ui| {
            for pattern in &mut self.editor.library.patterns {
                let id = pattern.id;
                let item = LibraryItem::Pattern(id);
                let active = active_id == Some(id);
                let state = CardState {
                    active,
                    unsaved: active && dirty,
                    just_saved: just_saved == Some(item),
                    scroll_to: scroll_to == Some(item),
                    editing: Some(*renaming == Some(id)),
                };
                let meta = pattern_summary(pattern);
                let preview = Some((&pattern.score, look));
                let action = library_card(ui, &mut pattern.name, typed, meta, preview, state);
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
                self.ui.renaming_pattern = None;
                self.request_load_pattern(id);
            }
            Some((id, CardAction::Delete)) => {
                self.ui.renaming_pattern = None;
                self.delete_pattern(id);
            }
            _ => {}
        }
    }

    /// Practice presets as cards, the user's and the built-in ones, in the
    /// library and in the Preset picker alike. Returns the one tapped.
    pub(crate) fn preset_cards(&mut self, ui: &mut egui::Ui) -> Option<PresetId> {
        let snapshot = self.preset_snapshot();
        let active = self.editor.active_preset;
        if let Some(save) = save_buttons(ui, &self.preset_save_target()) {
            self.save_preset(save, ui.input(|i| i.time));
        }
        ui.add_space(8.0);

        let mut picked = None;
        let mut to_delete = None;
        let (just_saved, scroll_to) = self.card_notices(ui);
        let renaming = &mut self.ui.renaming_preset;
        let typed = &mut self.ui.rename_typed;
        let builtin = presets::builtin_presets();
        egui::ScrollArea::vertical().max_height(dialog_scroll_height(ui)).show(ui, |ui| {
            for preset in &mut self.editor.presets.presets {
                let id = preset.id;
                let item = LibraryItem::Preset(id);
                let is_active = active == Some(PresetId::User(id));
                let state = CardState {
                    active: is_active,
                    unsaved: is_active && preset.settings != snapshot,
                    just_saved: just_saved == Some(item),
                    scroll_to: scroll_to == Some(item),
                    editing: Some(*renaming == Some(id)),
                };
                let meta = preset_summary(&preset.settings);
                let action = library_card(ui, &mut preset.name, typed, meta, None, state);
                match action {
                    Some(CardAction::Edit) => {
                        *renaming = if *renaming == Some(id) { None } else { Some(id) };
                        typed.clone_from(&preset.name);
                    }
                    Some(CardAction::Renamed) => *renaming = None,
                    Some(CardAction::Load) => picked = Some(PresetId::User(id)),
                    Some(CardAction::Delete) => {
                        *renaming = None;
                        to_delete = Some(id);
                    }
                    None => {}
                }
                ui.add_space(6.0);
            }
            ui.weak("Built in");
            for (i, (name, settings)) in builtin.iter().enumerate() {
                let id = PresetId::BuiltIn(i);
                let is_active = active == Some(id);
                let state = CardState {
                    active: is_active,
                    unsaved: is_active && *settings != snapshot,
                    just_saved: false,
                    scroll_to: false,
                    editing: None,
                };
                let mut name = (*name).to_owned();
                let meta = preset_summary(settings);
                if let Some(CardAction::Load) =
                    library_card(ui, &mut name, typed, meta, None, state)
                {
                    picked = Some(id);
                }
                ui.add_space(6.0);
            }
        });
        if let Some(id) = to_delete {
            self.delete_preset(id);
        }
        picked
    }

    /// Apply preset `id` from the library and open the practice page.
    pub(crate) fn open_preset(&mut self, id: PresetId) {
        // Its rhythm is generated, not the metronome's click bar.
        self.leave_metronome();
        self.apply_preset(id);
        self.ui.renaming_preset = None;
        self.ui.picker = None;
        self.select_tab(Tab::Practice);
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
