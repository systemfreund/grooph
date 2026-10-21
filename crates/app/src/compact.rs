//! The app's layout, made for touch first: a card with the generator settings
//! on top, the staff in the middle, an options sheet with big transport
//! buttons and a tab bar at the bottom. On wide windows the controls stay a
//! centered column and the staff gets the full width.

use crate::pickers::{Picker, ROW_HEIGHT, swing_label, time_signature_label};
use crate::share::paint_share_icon;
use crate::{Grooph, Mode, TransportState};
use eframe::egui;
use eframe::egui::{
    Align2, Button, Color32, FontId, Frame, Id, Margin, Painter, Rect, Response, RichText, Sense,
    Shape, Stroke, Ui, UiBuilder, Vec2, pos2, vec2,
};
use grooph_layout::staff_layout::Voice;
use grooph_measure::generator::{Subdivision, effective_subdivision};
use serde::{Deserialize, Serialize};

/// Windows narrower than this (in points) count as phone-sized: the staff
/// packs its notes tighter there.
pub(crate) const NARROW_MAX_WIDTH: f32 = 600.0;
/// Widest the controls get; wider windows center them.
const COLUMN_MAX_WIDTH: f32 = 720.0;

const TAB_HEIGHT: f32 = 56.0;
const CARD_CELL_HEIGHT: f32 = 64.0;
const PLAY_BUTTON_SIZE: f32 = 68.0;
const SIDE_BUTTON_SIZE: f32 = 52.0;
/// Share of the metronome tab's free height the click bar's staff gets, and
/// its bounds; the ring below takes the rest.
const METRONOME_STAFF_SHARE: f32 = 0.35;
const METRONOME_STAFF_MIN: f32 = 100.0;
const METRONOME_STAFF_MAX: f32 = 220.0;
/// The same with the counter rhythm as a second voice under the click bar.
const METRONOME_TWO_STAFF_SHARE: f32 = 0.45;
const METRONOME_TWO_STAFF_MAX: f32 = 340.0;
const CARD_RADIUS: u8 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum CompactTab {
    Metronome,
    Practice,
    Edit,
    More,
}

/// UI state of the layout. Not persisted.
pub(crate) struct CompactState {
    /// The options sheet only shows its handle and the transport buttons.
    /// Folded at start, so the staff and the ring get the room.
    pub(crate) options_collapsed: bool,
}

impl Default for CompactState {
    fn default() -> Self {
        Self { options_collapsed: true }
    }
}

fn subdivision_short_label(s: Subdivision) -> &'static str {
    match s {
        Subdivision::Eighths => "8ths",
        Subdivision::Sixteenths => "16ths",
        Subdivision::Triplets => "Trips",
        Subdivision::Mixed => "Mixed",
    }
}

/// Lay out `add` in a column at most [`COLUMN_MAX_WIDTH`] wide, centered in
/// the space left.
fn centered_column<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let avail = ui.available_rect_before_wrap();
    let width = avail.width().min(COLUMN_MAX_WIDTH);
    let rect = Rect::from_min_size(
        pos2(avail.center().x - width / 2.0, avail.top()),
        vec2(width, avail.height()),
    );
    ui.scope_builder(UiBuilder::new().max_rect(rect), add).inner
}

/// Fill of cards and sheets: a step off the panel background.
fn card_fill(ui: &Ui) -> Color32 {
    ui.visuals().widgets.inactive.weak_bg_fill
}

/// Text in the accent color (selected tab, values).
fn accent_text(ui: &Ui) -> Color32 {
    ui.visuals().selection.stroke.color
}

/// Text that is as wide as `max_width` at most: shrinks the font if needed.
fn fitted_text(
    painter: &Painter,
    pos: egui::Pos2,
    anchor: Align2,
    text: &str,
    size: f32,
    max_width: f32,
    color: Color32,
) {
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(size), color);
    let width = galley.size().x;
    let size = if width > max_width && width > 0.0 { size * max_width / width } else { size };
    painter.text(pos, anchor, text, FontId::proportional(size), color);
}

/// Labelled value in the settings card: caption above, big value below.
fn card_cell(ui: &mut Ui, label: &str, value: &str) -> Response {
    let (rect, resp) =
        ui.allocate_exact_size(vec2(ui.available_width(), CARD_CELL_HEIGHT), Sense::click());
    if resp.hovered() || resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, 10, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let painter = ui.painter();
    let width = rect.width() - 6.0;
    let weak = ui.visuals().weak_text_color();
    fitted_text(
        painter,
        rect.center_top() + vec2(0.0, 8.0),
        Align2::CENTER_TOP,
        label,
        14.0,
        width,
        weak,
    );
    let strong = ui.visuals().strong_text_color();
    fitted_text(
        painter,
        rect.center_bottom() - vec2(0.0, 6.0),
        Align2::CENTER_BOTTOM,
        value,
        26.0,
        width,
        strong,
    );
    resp
}

/// Full-width row with a label and an on/off switch. Returns whether it toggled.
fn switch_row(ui: &mut Ui, label: &str, on: &mut bool) -> bool {
    let (rect, resp) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    let how_on = ui.ctx().animate_bool_responsive(resp.id, *on);
    let visuals = ui.visuals();
    let painter = ui.painter();
    painter.text(
        rect.left_center() + vec2(4.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(19.0),
        visuals.text_color(),
    );

    let switch = Rect::from_center_size(rect.right_center() - vec2(30.0, 0.0), vec2(52.0, 30.0));
    let fill = if *on { visuals.selection.bg_fill } else { switch_off_fill(visuals) };
    painter.rect_filled(switch, 15, fill);
    let radius = 12.0;
    let x = egui::lerp((switch.left() + 15.0)..=(switch.right() - 15.0), how_on);
    painter.circle_filled(pos2(x, switch.center().y), radius, Color32::WHITE);
    resp.clicked()
}

/// Track colour of a switch that is off. It has to differ from the white
/// knob, so the light theme gets a grey instead of its white field colour.
fn switch_off_fill(visuals: &egui::Visuals) -> Color32 {
    if visuals.dark_mode { visuals.extreme_bg_color } else { Color32::from_gray(190) }
}

/// Full-width row with a label, the current value and a chevron.
fn value_row(ui: &mut Ui, label: &str, value: &str) -> Response {
    let (rect, resp) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    let visuals = ui.visuals();
    let painter = ui.painter();
    painter.text(
        rect.left_center() + vec2(4.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(19.0),
        visuals.text_color(),
    );
    painter.text(
        rect.right_center() - vec2(30.0, 0.0),
        Align2::RIGHT_CENTER,
        value,
        FontId::proportional(19.0),
        visuals.weak_text_color(),
    );
    painter.text(
        rect.right_center() - vec2(8.0, 0.0),
        Align2::RIGHT_CENTER,
        "›",
        FontId::proportional(26.0),
        visuals.weak_text_color(),
    );
    resp
}

/// Round button with an icon painted by `icon` (given the icon color).
fn round_button(
    ui: &mut Ui,
    size: f32,
    fill: Color32,
    icon_color: Color32,
    icon: impl FnOnce(&Painter, Rect, Color32),
) -> Response {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let id = ui.auto_id_with("round_button");
    round_button_at(ui, rect, id, fill, icon_color, icon)
}

/// [`round_button`] filling `rect`, for buttons placed by hand.
fn round_button_at(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    fill: Color32,
    icon_color: Color32,
    icon: impl FnOnce(&Painter, Rect, Color32),
) -> Response {
    let resp = ui.interact(rect, id, Sense::click());
    let fill = if resp.is_pointer_button_down_on() { fill.gamma_multiply(0.8) } else { fill };
    ui.painter().circle_filled(rect.center(), rect.width() / 2.0, fill);
    icon(ui.painter(), rect, icon_color);
    resp
}

/// Handle on top of an options sheet: tap to fold the options away.
/// Returns whether it was tapped.
fn sheet_handle(ui: &mut Ui) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    ui.painter().rect_filled(
        Rect::from_center_size(rect.center(), vec2(40.0, 5.0)),
        3,
        ui.visuals().weak_text_color(),
    );
    resp.clicked()
}

fn paint_play_icon(painter: &Painter, rect: Rect, color: Color32, playing: bool) {
    let c = rect.center();
    let s = rect.width() * 0.17;
    if playing {
        painter.rect_filled(Rect::from_center_size(c, Vec2::splat(2.0 * s)), 3, color);
    } else {
        // Optical center: shift the triangle right a little.
        let c = c + vec2(s * 0.25, 0.0);
        let points = vec![c + vec2(-s, -1.15 * s), c + vec2(1.3 * s, 0.0), c + vec2(-s, 1.15 * s)];
        painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
    }
}

impl Grooph {
    /// Tab shown in the compact layout, derived from the mode so modes set
    /// elsewhere (links, time signature dialog) land on the right tab.
    pub(crate) fn compact_tab(&self) -> CompactTab {
        if self.metronome_mode() {
            return CompactTab::Metronome;
        }
        match self.ui.mode {
            Mode::Edit | Mode::TimeSignature(_) => CompactTab::Edit,
            Mode::MoreMenu | Mode::Mixer | Mode::Settings | Mode::Help | Mode::Library => {
                CompactTab::More
            }
            Mode::Playback | Mode::Generator => CompactTab::Practice,
        }
    }

    pub(crate) fn select_compact_tab(&mut self, tab: CompactTab) {
        // Into or out of the metronome the played score changes: stop first.
        let metronome = tab == CompactTab::Metronome;
        if metronome != self.metronome_mode() {
            self.stop_transport();
            if metronome {
                self.open_metronome();
            } else {
                self.close_metronome();
            }
        }
        self.ui.mode = match tab {
            CompactTab::Practice => Mode::Generator,
            CompactTab::Metronome => Mode::Playback,
            CompactTab::Edit => Mode::Edit,
            CompactTab::More => Mode::MoreMenu,
        };
    }

    pub(super) fn compact_ui(&mut self, ui: &mut Ui) {
        let tab = self.compact_tab();
        self.compact_tab_bar(ui, tab);
        match tab {
            CompactTab::Practice => {
                self.compact_options_sheet(ui);
                self.compact_settings_card(ui);
            }
            CompactTab::Edit => {
                self.tool_palette_panel(ui);
                self.compact_edit_bar(ui);
            }
            CompactTab::Metronome if self.ui.mode == Mode::Edit => {
                self.tool_palette_panel(ui);
                self.metronome_edit_bar(ui);
                self.compact_metronome_card(ui);
            }
            CompactTab::Metronome => {
                self.compact_metronome_sheet(ui);
                self.compact_metronome_card(ui);
                // The staff above takes a share of the room, the ring the rest.
                let room = ui.available_height().max(0.0);
                let (share, max) = if self.counter_bar().is_some() {
                    (METRONOME_TWO_STAFF_SHARE, METRONOME_TWO_STAFF_MAX)
                } else {
                    (METRONOME_STAFF_SHARE, METRONOME_STAFF_MAX)
                };
                let staff = (share * room).clamp(METRONOME_STAFF_MIN, max).min(room);
                let height = (room - staff).min(ui.available_width());
                self.metronome_ring(ui, height);
            }
            CompactTab::More => self.compact_more(ui),
        }
        self.measure_panel(ui);
        self.settings_picker(ui);
    }

    fn compact_tab_bar(&mut self, ui: &mut Ui, current: CompactTab) {
        const TABS: [(CompactTab, &str, &str); 4] = [
            (CompactTab::Metronome, "⏱", "Metronome"),
            (CompactTab::Practice, "🎲", "Practice"),
            (CompactTab::Edit, "🖊", "Edit"),
            (CompactTab::More, "☰", "More"),
        ];
        let mut picked = None;
        egui::Panel::bottom("compact_tabs")
            .frame(Frame::side_top_panel(ui.style()).inner_margin(Margin::symmetric(0, 4)))
            .show(ui, |ui| {
                centered_column(ui, |ui| {
                    ui.columns(TABS.len(), |cols| {
                        for (col, (tab, icon, label)) in cols.iter_mut().zip(TABS) {
                            let (rect, resp) = col.allocate_exact_size(
                                vec2(col.available_width(), TAB_HEIGHT),
                                Sense::click(),
                            );
                            let color = if tab == current {
                                accent_text(col)
                            } else {
                                col.visuals().weak_text_color()
                            };
                            let painter = col.painter();
                            painter.text(
                                rect.center_top() + vec2(0.0, 4.0),
                                Align2::CENTER_TOP,
                                icon,
                                FontId::proportional(24.0),
                                color,
                            );
                            painter.text(
                                rect.center_bottom() - vec2(0.0, 2.0),
                                Align2::CENTER_BOTTOM,
                                label,
                                FontId::proportional(14.0),
                                color,
                            );
                            if resp.clicked() {
                                picked = Some(tab);
                            }
                        }
                    });
                });
            });
        if let Some(tab) = picked {
            self.select_compact_tab(tab);
        }
    }

    /// Generator settings and tempo as big values; a tap opens the picker.
    fn compact_settings_card(&mut self, ui: &mut Ui) {
        let settings = &self.editor.generator.settings;
        let subdivision = effective_subdivision(settings.subdivision, settings.time_signature);
        let level = if settings.custom_figures_enabled {
            "Custom".to_owned()
        } else {
            settings.complexity.to_string()
        };
        let cells = [
            (Picker::TimeSignature, "Meter", time_signature_label(settings.time_signature)),
            (Picker::Subdivision, "Notes", subdivision_short_label(subdivision).to_owned()),
            (Picker::Complexity, "Level", level),
            (Picker::Bars, "Bars", self.editor.generator.effective_bars().to_string()),
            // While a tempo ramp runs: the tempo being played.
            match self.playback_ctl.playback.ramp_bpm {
                Some(bpm) => (Picker::Bpm, "BPM now", bpm.to_string()),
                None => (Picker::Bpm, "BPM", self.playback_ctl.bpm.to_string()),
            },
        ];

        self.compact_card(ui, &cells);
    }

    /// Meter and tempo of the metronome tab, in the same card as Practice.
    fn compact_metronome_card(&mut self, ui: &mut Ui) {
        let cells = [
            (Picker::MetronomeMeter, "Meter", time_signature_label(self.metronome_meter())),
            match self.playback_ctl.playback.ramp_bpm {
                Some(bpm) => (Picker::Bpm, "BPM now", bpm.to_string()),
                None => (Picker::Bpm, "BPM", self.playback_ctl.bpm.to_string()),
            },
        ];
        self.compact_card(ui, &cells);
    }

    /// Card of labelled values on top; a tap opens the value's picker.
    fn compact_card(&mut self, ui: &mut Ui, cells: &[(Picker, &str, String)]) {
        let mut picked = None;
        egui::Panel::top("compact_card")
            .frame(Frame::new().fill(ui.visuals().panel_fill).inner_margin(Margin {
                left: 12,
                right: 12,
                top: 12,
                bottom: 4,
            }))
            .show_separator_line(false)
            .show(ui, |ui| {
                centered_column(ui, |ui| {
                    Frame::new()
                        .fill(card_fill(ui))
                        .corner_radius(CARD_RADIUS)
                        .inner_margin(Margin::symmetric(4, 6))
                        .show(ui, |ui| {
                            ui.columns(cells.len(), |cols| {
                                for (col, (picker, label, value)) in cols.iter_mut().zip(cells) {
                                    if card_cell(col, label, value).clicked() {
                                        picked = Some(*picker);
                                    }
                                }
                            });
                        });
                });
            });
        if picked.is_some() {
            self.ui.picker = picked;
        }
    }

    /// Practice options plus the big "new rhythm" and play buttons.
    fn compact_options_sheet(&mut self, ui: &mut Ui) {
        let before = self.editor.generator.settings;
        let endless_before = self.editor.generator.endless;
        let mut roll = false;

        egui::Panel::bottom("compact_options")
            .frame(Frame::new().fill(ui.visuals().panel_fill).inner_margin(Margin {
                left: 12,
                right: 12,
                top: 8,
                bottom: 8,
            }))
            .show_separator_line(false)
            .show(ui, |ui| {
                centered_column(ui, |ui| {
                    Frame::new()
                        .fill(card_fill(ui))
                        .corner_radius(CARD_RADIUS)
                        .inner_margin(Margin { left: 12, right: 12, top: 0, bottom: 12 })
                        .show(ui, |ui| {
                            if sheet_handle(ui) {
                                self.ui.compact.options_collapsed =
                                    !self.ui.compact.options_collapsed;
                            }

                            if !self.ui.compact.options_collapsed {
                                self.compact_option_rows(ui);
                                ui.add_space(8.0);
                            }

                            roll = self.compact_transport(ui);
                        });
                });
            });

        self.finish_generator_change(before, endless_before, roll);
    }

    fn compact_metronome_rows(&mut self, ui: &mut Ui) {
        if value_row(ui, "Clicks", &self.click_fill_label()).clicked() {
            self.ui.picker = Some(Picker::ClickFill);
        }
        if value_row(ui, "Edit clicks", "✏").clicked() {
            self.ui.mode = Mode::Edit;
        }
        if value_row(ui, "Polyrhythm", &self.counter_label()).clicked() {
            self.ui.picker = Some(Picker::Polyrhythm);
        }
        switch_row(ui, "Count-in", &mut self.playback_ctl.audio_cfg.count_in);
        switch_row(ui, "Mute bars", &mut self.playback_ctl.audio_cfg.gap.enabled);
    }

    /// Metronome options and the play button.
    fn compact_metronome_sheet(&mut self, ui: &mut Ui) {
        egui::Panel::bottom("compact_metronome")
            .frame(Frame::new().fill(ui.visuals().panel_fill).inner_margin(Margin::same(12)))
            .show_separator_line(false)
            .show(ui, |ui| {
                centered_column(ui, |ui| {
                    Frame::new()
                        .fill(card_fill(ui))
                        .corner_radius(CARD_RADIUS)
                        .inner_margin(Margin { left: 12, right: 12, top: 0, bottom: 12 })
                        .show(ui, |ui| {
                            // Folds with the Practice options: one sheet setting.
                            if sheet_handle(ui) {
                                self.ui.compact.options_collapsed =
                                    !self.ui.compact.options_collapsed;
                            }
                            if !self.ui.compact.options_collapsed {
                                self.compact_metronome_rows(ui);
                                ui.add_space(8.0);
                            }
                            self.metronome_transport(ui);
                        });
                });
            });
    }

    /// Centered play button, with share on the right.
    fn metronome_transport(&mut self, ui: &mut Ui) {
        if self.link_copied_recently(ui) {
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Link copied").size(16.0).weak());
            });
            ui.add_space(4.0);
        }
        let playing = self.playback_ctl.transport_state == TransportState::Playing;
        let (row, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), PLAY_BUTTON_SIZE), Sense::hover());
        let size = Vec2::splat(PLAY_BUTTON_SIZE);
        let fill = ui.visuals().selection.bg_fill;
        let icon = ui.visuals().selection.stroke.color;
        if round_button_at(
            ui,
            Rect::from_center_size(row.center(), size),
            Id::unique("metronome_play"),
            fill,
            icon,
            |painter, rect, color| paint_play_icon(painter, rect, color, playing),
        )
        .clicked()
        {
            self.toggle_playback();
        }
        if round_button_at(
            ui,
            Rect::from_min_size(row.right_top() - vec2(PLAY_BUTTON_SIZE, 0.0), size),
            Id::unique("metronome_share"),
            ui.visuals().extreme_bg_color,
            ui.visuals().strong_text_color(),
            paint_share_icon,
        )
        .on_hover_text("Share a link to this metronome")
        .clicked()
        {
            self.share_link(ui);
        }
    }

    fn compact_option_rows(&mut self, ui: &mut Ui) {
        if value_row(ui, "Preset", &self.active_preset_label()).clicked() {
            self.ui.picker = Some(Picker::Preset);
        }
        let generator = &mut self.editor.generator;
        switch_row(ui, "Endless", &mut generator.endless);
        switch_row(ui, "Ghost notes", &mut generator.ghost_notes);
        switch_row(ui, "Count-in", &mut self.playback_ctl.audio_cfg.count_in);
        let swing = self.playback_ctl.audio_cfg.swing;
        let swing_value = if swing.is_straight() {
            swing_label(swing.percent)
        } else {
            format!("{} {}", swing_label(swing.percent), swing.unit.label())
        };
        if value_row(ui, "Swing", &swing_value).clicked() {
            self.ui.picker = Some(Picker::Swing);
        }
        let mut accuracy = self.playback_ctl.accuracy.enabled;
        if switch_row(ui, "Timing feedback", &mut accuracy) {
            self.set_accuracy_enabled(accuracy);
        }
    }

    /// Centered "new rhythm" and play buttons. Returns whether "new" was tapped.
    fn compact_transport(&mut self, ui: &mut Ui) -> bool {
        let playing = self.playback_ctl.transport_state == TransportState::Playing;
        self.compact_status_line(ui);
        let (row, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), PLAY_BUTTON_SIZE), Sense::hover());
        let size = Vec2::splat(PLAY_BUTTON_SIZE);
        let new_rect = Rect::from_min_size(row.min, size);
        let play_rect = Rect::from_center_size(row.center(), size);
        let share_rect = Rect::from_min_size(row.right_top() - vec2(PLAY_BUTTON_SIZE, 0.0), size);

        let side_fill = ui.visuals().extreme_bg_color;
        let text = ui.visuals().strong_text_color();
        let roll = round_button_at(
            ui,
            new_rect,
            Id::unique("compact_new"),
            side_fill,
            text,
            |painter, rect, color| {
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    "🎲",
                    FontId::proportional(32.0),
                    color,
                );
            },
        )
        .on_hover_text("New rhythm")
        .clicked();

        let fill = ui.visuals().selection.bg_fill;
        let icon = ui.visuals().selection.stroke.color;
        if round_button_at(
            ui,
            play_rect,
            Id::unique("compact_play"),
            fill,
            icon,
            |painter, rect, color| paint_play_icon(painter, rect, color, playing),
        )
        .clicked()
        {
            self.toggle_playback();
        }

        if round_button_at(
            ui,
            share_rect,
            Id::unique("compact_share"),
            side_fill,
            text,
            paint_share_icon,
        )
        .on_hover_text("Share a link to this rhythm")
        .clicked()
        {
            self.share_link(ui);
        }
        roll
    }

    /// One line above the transport buttons: the live timing score and the
    /// average deviation while playing along, or a short note after a link
    /// was copied.
    fn compact_status_line(&mut self, ui: &mut Ui) {
        let text = if self.link_copied_recently(ui) {
            Some("Link copied".to_owned())
        } else {
            self.playback_ctl.accuracy.tracker.live_summary().map(|summary| {
                format!("Timing {:.0}%  {:+.0} ms", summary.score_percent(), summary.mean_ms)
            })
        };
        if let Some(text) = text {
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(text).size(16.0).weak());
            });
            ui.add_space(4.0);
        }
    }

    /// Play button and measure controls above the tool palette, as wide as it.
    fn compact_edit_bar(&mut self, ui: &mut Ui) {
        egui::Panel::bottom("compact_edit_bar")
            .frame(
                Frame::new().fill(ui.visuals().panel_fill).inner_margin(Margin::symmetric(12, 6)),
            )
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let playing = self.playback_ctl.transport_state == TransportState::Playing;
                    let fill = ui.visuals().selection.bg_fill;
                    let icon = ui.visuals().selection.stroke.color;
                    if round_button(ui, SIDE_BUTTON_SIZE, fill, icon, |painter, rect, color| {
                        paint_play_icon(painter, rect, color, playing)
                    })
                    .clicked()
                    {
                        self.toggle_playback();
                    }
                    ui.add_space(8.0);
                    ui.label(format!(
                        "{}/{}",
                        self.editor.cursor.measure_idx + 1,
                        self.editor.score.len()
                    ));
                    if ui.button("➕").on_hover_text("Add a bar").clicked() {
                        self.append_measure();
                    }
                    if ui.button("➖").on_hover_text("Remove this bar").clicked() {
                        self.remove_current_measure();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let fill = ui.visuals().extreme_bg_color;
                        let text = ui.visuals().strong_text_color();
                        if round_button(ui, SIDE_BUTTON_SIZE, fill, text, paint_share_icon)
                            .on_hover_text("Share a link to this rhythm")
                            .clicked()
                        {
                            self.share_link(ui);
                        }
                        if self.link_copied_recently(ui) {
                            ui.weak("Copied");
                        }
                    });
                });
            });
    }

    /// Play button and a way back to the ring above the tool palette, while
    /// the click bar is edited note by note.
    fn metronome_edit_bar(&mut self, ui: &mut Ui) {
        egui::Panel::bottom("metronome_edit_bar")
            .frame(
                Frame::new().fill(ui.visuals().panel_fill).inner_margin(Margin::symmetric(12, 6)),
            )
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let playing = self.playback_ctl.transport_state == TransportState::Playing;
                    let fill = ui.visuals().selection.bg_fill;
                    let icon = ui.visuals().selection.stroke.color;
                    if round_button(ui, SIDE_BUTTON_SIZE, fill, icon, |painter, rect, color| {
                        paint_play_icon(painter, rect, color, playing)
                    })
                    .clicked()
                    {
                        self.toggle_playback();
                    }
                    ui.add_space(8.0);
                    // With a counter rhythm: which of the two voices the
                    // palette edits (a tap on its line picks it too).
                    if self.counter_bar().is_some() {
                        let voice = self.edited_voice();
                        for (v, label) in [(Voice::Upper, "Clicks"), (Voice::Lower, "Counter")] {
                            let button =
                                Button::new(RichText::new(label).size(16.0)).selected(voice == v);
                            if ui.add(button).clicked() {
                                self.select_voice(v);
                            }
                        }
                    } else {
                        ui.label("Click bar");
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(RichText::new("Done").size(18.0)).clicked() {
                            self.ui.mode = Mode::Playback;
                        }
                    });
                });
            });
    }

    /// "More" tab: a list of the remaining panels, or the open panel with a
    /// way back to the list.
    fn compact_more(&mut self, ui: &mut Ui) {
        const ENTRIES: [(Mode, &str, &str); 4] = [
            (Mode::Library, "📁", "Library"),
            (Mode::Mixer, "🔈", "Mixer"),
            (Mode::Settings, "⚙", "Settings"),
            (Mode::Help, "?", "Help"),
        ];
        if self.ui.mode == Mode::MoreMenu {
            egui::Panel::bottom("compact_more")
                .frame(Frame::new().fill(ui.visuals().panel_fill).inner_margin(Margin::same(12)))
                .show_separator_line(false)
                .show(ui, |ui| {
                    centered_column(ui, |ui| {
                        Frame::new()
                            .fill(card_fill(ui))
                            .corner_radius(CARD_RADIUS)
                            .inner_margin(Margin::symmetric(12, 4))
                            .show(ui, |ui| {
                                for (mode, icon, label) in ENTRIES {
                                    if value_row(ui, &format!("{icon}   {label}"), "").clicked() {
                                        self.ui.mode = mode;
                                    }
                                }
                            });
                    });
                });
            return;
        }

        self.library_panel(ui);
        self.mixer_panel(ui);
        self.settings_panel(ui);
        self.help_panel(ui);
        let title = ENTRIES.iter().find(|(m, ..)| *m == self.ui.mode).map_or("", |(.., l)| *l);
        egui::Panel::bottom("compact_more_back")
            .frame(Frame::side_top_panel(ui.style()).inner_margin(Margin::symmetric(12, 6)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("‹ Back").clicked() {
                        self.ui.mode = Mode::MoreMenu;
                    }
                    ui.strong(title);
                });
            });
    }
}
