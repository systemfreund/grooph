//! Compact layout for narrow screens (phones): a card with the generator
//! settings on top, the staff in the middle, an options sheet with big
//! transport buttons and a tab bar at the bottom. Picked every frame from the
//! window width, so the desktop layout is unchanged.

use crate::generator::ENDLESS_MIN_BARS;
use crate::generator_panel::{
    offered_subdivisions, subdivision_label, time_signature_label, time_signatures,
};
use crate::library_panel::LINK_COPIED_NOTICE_S;
use crate::presets::{self, PresetId};
use crate::{Grooph, Mode, TransportState, platform};
use eframe::egui;
use eframe::egui::{
    Align2, Button, Color32, CornerRadius, FontId, Frame, Id, Margin, Modal, Painter, Rect,
    Response, RichText, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2,
};
use grooph_link::{LinkContent, SharedLink};
use grooph_measure::generator::{MAX_BARS, MAX_COMPLEXITY, Subdivision, effective_subdivision};
use grooph_measure::swing::{MAX_SWING_PERCENT, MIN_SWING_PERCENT, SwingUnit};
use grooph_measure::tempo::{MAX_BPM, MIN_BPM};
use log::warn;

/// Windows narrower than this (in points) get the compact layout.
pub(crate) const COMPACT_MAX_WIDTH: f32 = 600.0;

const ROW_HEIGHT: f32 = 44.0;
const TAB_HEIGHT: f32 = 56.0;
const CARD_CELL_HEIGHT: f32 = 64.0;
const PLAY_BUTTON_SIZE: f32 = 68.0;
const SIDE_BUTTON_SIZE: f32 = 52.0;
const CARD_RADIUS: u8 = 16;
/// Taps further apart than this start a new tap-tempo measurement.
const TAP_RESET_S: f64 = 2.0;
/// Swing amounts offered in the swing picker (percent of a pair).
const SWING_PRESETS: [u8; 6] = [MIN_SWING_PERCENT, 55, 60, 66, 70, MAX_SWING_PERCENT];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CompactTab {
    Practice,
    Edit,
    More,
}

/// Setting edited in the picker dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Picker {
    TimeSignature,
    Subdivision,
    Complexity,
    Bars,
    Bpm,
    Swing,
    Preset,
}

/// UI state of the compact layout. Not persisted.
#[derive(Default)]
pub(crate) struct CompactState {
    /// "More" tab is showing its list (mode stays `Playback` there).
    pub(crate) more_menu: bool,
    pub(crate) picker: Option<Picker>,
    /// The options sheet only shows its handle and the transport buttons.
    pub(crate) options_collapsed: bool,
    /// UI times of recent tap-tempo taps.
    pub(crate) taps: Vec<f64>,
}

fn subdivision_short_label(s: Subdivision) -> &'static str {
    match s {
        Subdivision::Eighths => "8ths",
        Subdivision::Sixteenths => "16ths",
        Subdivision::Triplets => "Trips",
        Subdivision::Mixed => "Mixed",
    }
}

fn swing_label(percent: u8) -> String {
    if percent <= MIN_SWING_PERCENT { "Off".to_owned() } else { format!("{percent}%") }
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
    let fill = if *on { visuals.selection.bg_fill } else { visuals.extreme_bg_color };
    painter.rect_filled(switch, 15, fill);
    let radius = 12.0;
    let x = egui::lerp((switch.left() + 15.0)..=(switch.right() - 15.0), how_on);
    painter.circle_filled(pos2(x, switch.center().y), radius, Color32::WHITE);
    resp.clicked()
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

/// The iOS share symbol: an open box with an arrow pointing up out of it.
pub(crate) fn paint_share_icon(painter: &Painter, rect: Rect, color: Color32) {
    let c = rect.center() + vec2(0.0, rect.height() * 0.03);
    let s = rect.width() * 0.14;
    let stroke = Stroke::new(2.5, color);
    let box_top = c.y - 0.3 * s;
    let box_bottom = c.y + 1.2 * s;
    painter.add(Shape::line(
        vec![
            pos2(c.x - 0.45 * s, box_top),
            pos2(c.x - s, box_top),
            pos2(c.x - s, box_bottom),
            pos2(c.x + s, box_bottom),
            pos2(c.x + s, box_top),
            pos2(c.x + 0.45 * s, box_top),
        ],
        stroke,
    ));
    let tip = pos2(c.x, c.y - 1.6 * s);
    painter.line_segment([pos2(c.x, c.y + 0.4 * s), tip], stroke);
    painter.line_segment([tip, tip + vec2(-0.5 * s, 0.5 * s)], stroke);
    painter.line_segment([tip, tip + vec2(0.5 * s, 0.5 * s)], stroke);
}

/// Grid of option buttons; returns the one that was tapped.
fn choice_grid<T: Copy + PartialEq>(
    ui: &mut Ui,
    options: &[(T, String)],
    current: T,
    columns: usize,
) -> Option<T> {
    let mut picked = None;
    let spacing = ui.spacing().item_spacing.x;
    let width = (ui.available_width() - spacing * (columns as f32 - 1.0)) / columns as f32;
    for row in options.chunks(columns) {
        ui.horizontal(|ui| {
            for (value, label) in row {
                // Long labels (e.g. "13/16 (3+3+3+2+2)") shrink to fit the cell.
                let text_width = ui
                    .painter()
                    .layout_no_wrap(label.clone(), FontId::proportional(18.0), Color32::WHITE)
                    .size()
                    .x;
                let room = width - 2.0 * ui.spacing().button_padding.x;
                let size = if text_width > room { 18.0 * room / text_width } else { 18.0 };
                let button = Button::new(RichText::new(label).size(size))
                    .selected(*value == current)
                    .min_size(vec2(width, ROW_HEIGHT));
                if ui.add(button).clicked() {
                    picked = Some(*value);
                }
            }
        });
    }
    picked
}

/// Full-width button with a centered label.
fn wide_button(ui: &mut Ui, label: &str, size: f32, height: f32) -> Response {
    let width = ui.available_width();
    ui.horizontal(|ui| {
        ui.add(Button::new(RichText::new(label).size(size)).min_size(vec2(width, height)))
    })
    .inner
}

/// Add a tap at UI time `now` and return the tempo of the recent taps, once
/// there are two. A pause longer than [`TAP_RESET_S`] starts over.
pub(crate) fn record_tap(taps: &mut Vec<f64>, now: f64) -> Option<u32> {
    const MAX_TAPS: usize = 8;
    if taps.last().is_some_and(|&last| now - last > TAP_RESET_S) {
        taps.clear();
    }
    taps.push(now);
    if taps.len() > MAX_TAPS {
        taps.remove(0);
    }
    let (first, last) = (*taps.first()?, *taps.last()?);
    let interval = (last - first) / (taps.len() - 1).max(1) as f64;
    (interval > 0.0).then(|| ((60.0 / interval).round() as u32).clamp(MIN_BPM, MAX_BPM))
}

impl Grooph {
    /// Tab shown in the compact layout, derived from the mode so modes set
    /// elsewhere (links, time signature dialog) land on the right tab.
    pub(crate) fn compact_tab(&self) -> CompactTab {
        match self.ui.mode {
            Mode::Edit | Mode::TimeSignature { .. } => CompactTab::Edit,
            Mode::Mixer | Mode::Settings | Mode::Help | Mode::Library => CompactTab::More,
            Mode::Playback | Mode::Generator => {
                if self.ui.compact.more_menu {
                    CompactTab::More
                } else {
                    CompactTab::Practice
                }
            }
        }
    }

    fn select_compact_tab(&mut self, tab: CompactTab) {
        self.ui.compact.more_menu = tab == CompactTab::More;
        self.ui.mode = match tab {
            CompactTab::Practice => Mode::Generator,
            CompactTab::Edit => Mode::Edit,
            CompactTab::More => Mode::Playback,
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
            CompactTab::More => self.compact_more(ui),
        }
        self.measure_panel(ui);
        self.compact_picker(ui);
    }

    fn compact_tab_bar(&mut self, ui: &mut Ui, current: CompactTab) {
        const TABS: [(CompactTab, &str, &str); 3] = [
            (CompactTab::Practice, "🎲", "Practice"),
            (CompactTab::Edit, "🖊", "Edit"),
            (CompactTab::More, "☰", "More"),
        ];
        let mut picked = None;
        egui::Panel::bottom("compact_tabs")
            .frame(Frame::side_top_panel(ui.style()).inner_margin(Margin::symmetric(0, 4)))
            .show_inside(ui, |ui| {
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
        if let Some(tab) = picked {
            self.select_compact_tab(tab);
        }
    }

    /// Generator settings and tempo as big values; a tap opens the picker.
    fn compact_settings_card(&mut self, ui: &mut Ui) {
        let settings = &self.editor.generator.settings;
        let subdivision = effective_subdivision(settings.subdivision, settings.time_signature);
        let level = if settings.custom_groupings_enabled {
            "Custom".to_owned()
        } else {
            settings.complexity.to_string()
        };
        let cells = [
            (Picker::TimeSignature, "Meter", settings.time_signature.to_string()),
            (Picker::Subdivision, "Notes", subdivision_short_label(subdivision).to_owned()),
            (Picker::Complexity, "Level", level),
            (Picker::Bars, "Bars", self.editor.generator.effective_bars().to_string()),
            (Picker::Bpm, "BPM", self.playback_ctl.bpm.to_string()),
        ];

        let mut picked = None;
        egui::Panel::top("compact_card")
            .frame(Frame::new().fill(ui.visuals().panel_fill).inner_margin(Margin {
                left: 12,
                right: 12,
                top: 12,
                bottom: 4,
            }))
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                Frame::new()
                    .fill(card_fill(ui))
                    .corner_radius(CARD_RADIUS)
                    .inner_margin(Margin::symmetric(4, 6))
                    .show(ui, |ui| {
                        ui.columns(cells.len(), |cols| {
                            for (col, (picker, label, value)) in cols.iter_mut().zip(&cells) {
                                if card_cell(col, label, value).clicked() {
                                    picked = Some(*picker);
                                }
                            }
                        });
                    });
            });
        if picked.is_some() {
            self.ui.compact.picker = picked;
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
            .show_inside(ui, |ui| {
                Frame::new()
                    .fill(card_fill(ui))
                    .corner_radius(CARD_RADIUS)
                    .inner_margin(Margin { left: 12, right: 12, top: 0, bottom: 12 })
                    .show(ui, |ui| {
                        // Handle: tap to fold the options away.
                        let (rect, resp) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
                        ui.painter().rect_filled(
                            Rect::from_center_size(rect.center(), vec2(40.0, 5.0)),
                            3,
                            ui.visuals().weak_text_color(),
                        );
                        if resp.clicked() {
                            self.ui.compact.options_collapsed = !self.ui.compact.options_collapsed;
                        }

                        if !self.ui.compact.options_collapsed {
                            self.compact_option_rows(ui);
                            ui.add_space(8.0);
                        }

                        roll = self.compact_transport(ui);
                    });
            });

        self.finish_generator_change(before, endless_before, roll);
    }

    fn compact_option_rows(&mut self, ui: &mut Ui) {
        if value_row(ui, "Preset", &self.active_preset_label()).clicked() {
            self.ui.compact.picker = Some(Picker::Preset);
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
            self.ui.compact.picker = Some(Picker::Swing);
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
            Id::new("compact_new"),
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
            Id::new("compact_play"),
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
            Id::new("compact_share"),
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

    /// One line above the transport buttons: the live timing score while
    /// playing along, or a short note after a link was copied.
    fn compact_status_line(&mut self, ui: &mut Ui) {
        let now = ui.input(|i| i.time);
        let copied = self.ui.link_copied_at.is_some_and(|at| now - at < LINK_COPIED_NOTICE_S);
        let text = if copied {
            ui.ctx().request_repaint_after_secs(0.5);
            Some("Link copied".to_owned())
        } else {
            self.playback_ctl
                .accuracy
                .tracker
                .live_summary()
                .map(|summary| format!("Timing {:.0}%", summary.score_percent()))
        };
        if let Some(text) = text {
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(text).size(16.0).weak());
            });
            ui.add_space(4.0);
        }
    }

    /// Share a link to the working score with tempo and swing: the system
    /// share sheet where the browser has one, else the clipboard.
    /// Link that opens the working score at the current tempo and swing.
    pub(crate) fn share_link_url(&self) -> Option<String> {
        let swing = self.playback_ctl.audio_cfg.swing;
        let link = SharedLink {
            bpm: Some(self.playback_ctl.bpm),
            swing: (!swing.is_straight()).then_some(swing),
            content: Some(LinkContent::Score(self.editor.score.clone())),
        };
        let url = link.to_url(&platform::share_base_url());
        if url.is_none() {
            warn!("The score cannot be written as a link");
        }
        url
    }

    /// Open the share sheet with a link to the score, or copy the link where
    /// there is no share sheet.
    pub(crate) fn share_link(&mut self, ui: &Ui) {
        let Some(url) = self.share_link_url() else {
            return;
        };
        if !platform::open_share_sheet(&url) {
            ui.ctx().copy_text(url);
            self.ui.link_copied_at = Some(ui.input(|i| i.time));
        }
    }

    /// Play button and measure controls above the tool palette.
    fn compact_edit_bar(&mut self, ui: &mut Ui) {
        egui::Panel::bottom("compact_edit_bar")
            .frame(
                Frame::new().fill(ui.visuals().panel_fill).inner_margin(Margin::symmetric(12, 6)),
            )
            .show_separator_line(false)
            .show_inside(ui, |ui| {
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
                        let now = ui.input(|i| i.time);
                        if self.ui.link_copied_at.is_some_and(|at| now - at < LINK_COPIED_NOTICE_S)
                        {
                            ui.ctx().request_repaint_after_secs(0.5);
                            ui.weak("Copied");
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
        if self.ui.mode == Mode::Playback {
            egui::Panel::bottom("compact_more")
                .frame(Frame::new().fill(ui.visuals().panel_fill).inner_margin(Margin::same(12)))
                .show_separator_line(false)
                .show_inside(ui, |ui| {
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
            return;
        }

        self.library_panel(ui);
        self.mixer_panel(ui);
        self.settings_panel(ui);
        self.help_panel(ui);
        let title = ENTRIES.iter().find(|(m, ..)| *m == self.ui.mode).map_or("", |(.., l)| *l);
        egui::Panel::bottom("compact_more_back")
            .frame(Frame::side_top_panel(ui.style()).inner_margin(Margin::symmetric(12, 6)))
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("‹ Back").clicked() {
                        self.ui.mode = Mode::Playback;
                    }
                    ui.strong(title);
                });
            });
    }

    /// Dialog for the setting tapped in the card or the options sheet.
    /// Picker dialog; the desktop layout opens it for the time signature.
    pub(crate) fn compact_picker(&mut self, ui: &mut Ui) {
        let Some(picker) = self.ui.compact.picker else {
            return;
        };
        let before = self.editor.generator.settings;
        let endless_before = self.editor.generator.endless;
        let width = (ui.ctx().content_rect().width() - 48.0).clamp(200.0, 380.0);
        let mut preset = None;

        let modal = Modal::new(Id::new("compact_picker")).show(ui.ctx(), |ui| {
            ui.set_width(width);
            let title = match picker {
                Picker::TimeSignature => "Time signature",
                Picker::Subdivision => "Notes",
                Picker::Complexity => "Level",
                Picker::Bars => "Bars",
                Picker::Bpm => "Tempo",
                Picker::Swing => "Swing",
                Picker::Preset => "Preset",
            };
            ui.heading(title);
            ui.add_space(8.0);
            let picked = match picker {
                Picker::TimeSignature => self.time_signature_choices(ui),
                Picker::Subdivision => self.subdivision_choices(ui),
                Picker::Complexity => self.complexity_choices(ui),
                Picker::Bars => self.bars_choices(ui),
                Picker::Bpm => {
                    self.bpm_editor(ui);
                    false
                }
                Picker::Swing => {
                    self.swing_choices(ui);
                    false
                }
                Picker::Preset => {
                    preset = self.preset_choices(ui);
                    preset.is_some()
                }
            };
            ui.add_space(12.0);
            picked || wide_button(ui, "Done", 19.0, ROW_HEIGHT).clicked()
        });
        if modal.inner || modal.should_close() {
            self.ui.compact.picker = None;
            self.ui.compact.taps.clear();
        }

        self.finish_generator_change(before, endless_before, false);
        if let Some(id) = preset {
            self.apply_preset(id);
        }
    }

    /// Presets to load, and saving the current settings as a new one.
    fn preset_choices(&mut self, ui: &mut Ui) -> Option<PresetId> {
        let options: Vec<_> =
            presets::all(&self.editor.presets).into_iter().map(|(id, n)| (Some(id), n)).collect();
        let current = self.editor.active_preset;
        let picked = egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| choice_grid(ui, &options, current, 1))
            .inner
            .flatten();
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.ui.preset_name_buffer)
                    .hint_text("Name")
                    .desired_width(ui.available_width() - 80.0),
            );
            if ui.add(Button::new("Save").min_size(vec2(72.0, 32.0))).clicked() {
                let name = std::mem::take(&mut self.ui.preset_name_buffer);
                self.save_preset_as(&name);
            }
        });
        picked
    }

    fn time_signature_choices(&mut self, ui: &mut Ui) -> bool {
        let settings = &mut self.editor.generator.settings;
        let options: Vec<_> =
            time_signatures().into_iter().map(|ts| (ts, time_signature_label(ts))).collect();
        let picked = egui::ScrollArea::vertical()
            .max_height(360.0)
            .show(ui, |ui| choice_grid(ui, &options, settings.time_signature, 2))
            .inner;
        if let Some(ts) = picked {
            settings.time_signature = ts;
        }
        picked.is_some()
    }

    fn subdivision_choices(&mut self, ui: &mut Ui) -> bool {
        let settings = &mut self.editor.generator.settings;
        let shown = effective_subdivision(settings.subdivision, settings.time_signature);
        let options: Vec<_> = offered_subdivisions(settings.time_signature)
            .iter()
            .map(|&s| (s, subdivision_label(s).to_owned()))
            .collect();
        let picked = choice_grid(ui, &options, shown, 2);
        if let Some(s) = picked {
            settings.subdivision = s;
        }
        picked.is_some()
    }

    /// Complexity levels; picking one turns custom groupings off. The grouping
    /// picker below lets the user hand-pick instead.
    fn complexity_choices(&mut self, ui: &mut Ui) -> bool {
        let settings = &mut self.editor.generator.settings;
        let current = if settings.custom_groupings_enabled { 0 } else { settings.complexity };
        let options: Vec<_> = (1..=MAX_COMPLEXITY).map(|l| (l, l.to_string())).collect();
        let picked = choice_grid(ui, &options, current, MAX_COMPLEXITY as usize);
        if let Some(level) = picked {
            settings.complexity = level;
            settings.custom_groupings_enabled = false;
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Space").size(18.0));
            ui.label(RichText::new(format!("{:.0}%", settings.space * 100.0)).size(18.0).weak());
        });
        ui.spacing_mut().slider_width = ui.available_width();
        ui.add(egui::Slider::new(&mut settings.space, 0.0..=1.0).show_value(false))
            .on_hover_text("More rests in the rhythm");
        ui.add_space(8.0);
        egui::CollapsingHeader::new(RichText::new("Groupings").size(18.0))
            .id_salt("compact_groupings")
            .show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    self.groupings_picker(ui);
                });
            });
        picked.is_some()
    }

    fn bars_choices(&mut self, ui: &mut Ui) -> bool {
        let generator = &mut self.editor.generator;
        let min_bars = if generator.endless { ENDLESS_MIN_BARS } else { 1 };
        let options: Vec<_> = (min_bars..=MAX_BARS).map(|b| (b, b.to_string())).collect();
        let picked = choice_grid(ui, &options, generator.effective_bars(), 4);
        if let Some(bars) = picked {
            generator.settings.bars = bars;
        }
        picked.is_some()
    }

    /// Big tempo display with steppers, a slider and tap tempo.
    fn bpm_editor(&mut self, ui: &mut Ui) {
        let mut bpm = self.playback_ctl.bpm;
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(bpm.to_string()).size(56.0).strong());
        });

        let steps: [i32; 4] = [-5, -1, 1, 5];
        let spacing = ui.spacing().item_spacing.x;
        let width = (ui.available_width() - spacing * 3.0) / 4.0;
        ui.horizontal(|ui| {
            for step in steps {
                let label = if step > 0 { format!("+{step}") } else { format!("−{}", -step) };
                let button =
                    Button::new(RichText::new(label).size(20.0)).min_size(vec2(width, ROW_HEIGHT));
                if ui.add(button).clicked() {
                    bpm = (bpm as i32 + step).clamp(MIN_BPM as i32, MAX_BPM as i32) as u32;
                }
            }
        });
        ui.add_space(4.0);
        ui.spacing_mut().slider_width = ui.available_width();
        ui.add(egui::Slider::new(&mut bpm, MIN_BPM..=MAX_BPM).show_value(false));
        ui.add_space(4.0);

        let tap = wide_button(ui, "Tap", 22.0, 72.0);
        if tap.clicked()
            && let Some(tapped) = record_tap(&mut self.ui.compact.taps, ui.input(|i| i.time))
        {
            bpm = tapped;
        }

        if bpm != self.playback_ctl.bpm {
            self.playback_ctl.bpm = bpm;
            self.editor.dirty = true;
            self.handle_bpm_change();
        }
    }

    fn swing_choices(&mut self, ui: &mut Ui) {
        let swing = &mut self.playback_ctl.audio_cfg.swing;
        let options: Vec<_> = SWING_PRESETS.iter().map(|&p| (p, swing_label(p))).collect();
        if let Some(percent) = choice_grid(ui, &options, swing.percent, 3) {
            swing.percent = percent;
        }
        ui.add_space(4.0);
        ui.spacing_mut().slider_width = ui.available_width();
        ui.add(
            egui::Slider::new(&mut swing.percent, MIN_SWING_PERCENT..=MAX_SWING_PERCENT)
                .show_value(false),
        );
        if !swing.is_straight() {
            ui.add_space(8.0);
            ui.label("Swung note value");
            let units: Vec<_> = SwingUnit::ALL.iter().map(|&u| (u, u.label().to_owned())).collect();
            if let Some(unit) = choice_grid(ui, &units, swing.unit, 2) {
                swing.unit = unit;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_tempo_needs_two_taps() {
        let mut taps = Vec::new();
        assert_eq!(record_tap(&mut taps, 10.0), None);
        assert_eq!(record_tap(&mut taps, 10.5), Some(120));
        assert_eq!(record_tap(&mut taps, 11.0), Some(120));
    }

    #[test]
    fn tap_tempo_averages_and_restarts_after_a_pause() {
        let mut taps = Vec::new();
        for t in [0.0, 0.6, 1.2, 1.8] {
            record_tap(&mut taps, t);
        }
        assert_eq!(taps.len(), 4);
        // 0.6 s per beat = 100 BPM.
        assert_eq!(record_tap(&mut taps, 2.4), Some(100));
        // A long pause starts a new measurement.
        assert_eq!(record_tap(&mut taps, 10.0), None);
        assert_eq!(taps, vec![10.0]);
    }

    #[test]
    fn tap_tempo_is_clamped() {
        let mut taps = Vec::new();
        record_tap(&mut taps, 0.0);
        assert_eq!(record_tap(&mut taps, 0.01), Some(MAX_BPM));
    }
}
