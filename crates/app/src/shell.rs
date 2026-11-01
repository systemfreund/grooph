//! The app's layout, made for touch first: a card with the generator settings
//! on top, the staff in the middle, an options sheet with big transport
//! buttons and a tab bar at the bottom. On wide windows the controls stay a
//! centered column and the staff gets the full width.

use crate::documents::Document;
use crate::generator::GeneratorState;
use crate::metronome_view::FillLook;
use crate::metronome_view::ratio_label;
use crate::pickers::{Picker, ROW_HEIGHT, shown_groups, silent_bars_label, swing_label};
use crate::share::paint_share_icon;
use crate::tool_palette::{BAR_TOOLS, notation_size, paint_none, paint_notation, tool};
use crate::tools::{EditOp, Tool, ToolKind};
use crate::{CountingBase, Grooph, Mode, TransportState};
use eframe::egui;
use eframe::egui::layers::ShapeIdx;
use eframe::egui::{
    Align2, Button, Color32, CornerRadius, FontFamily, FontId, Frame, Id, LayerId, Margin, Order,
    Painter, Pos2, Rect, Response, RichText, Sense, Shape, Stroke, ThemePreference, Ui, UiBuilder,
    Vec2, pos2, vec2,
};
use grooph_layout::glyphs::ts_glyphs;
use grooph_layout::pixel_layout::PlacedGlyph;
use grooph_measure::TimeSignature;
use grooph_measure::counter::Ratio;
use grooph_measure::generator::{Subdivision, effective_subdivision};
use grooph_measure::swing::MIN_SWING_PERCENT;
use grooph_render::draw_glyph;
use serde::{Deserialize, Serialize};

/// Windows narrower than this (in points) count as phone-sized: the staff
/// packs its notes tighter there.
pub(crate) const NARROW_MAX_WIDTH: f32 = 600.0;
/// Widest the controls get; wider windows center them.
const COLUMN_MAX_WIDTH: f32 = 720.0;

const TAB_HEIGHT: f32 = 56.0;
const CARD_CELL_HEIGHT: f32 = 64.0;
const CARD_VALUE_SIZE: f32 = 26.0;
/// Music font size of a meter in the card: its two numbers stacked are this high.
const CARD_METER_EM: f32 = 34.0;
const PLAY_BUTTON_SIZE: f32 = 68.0;
const SIDE_BUTTON_SIZE: f32 = 52.0;
/// The width from which an editor's bar holds its own buttons in its one
/// line, next to undo, redo and clear (see [`Grooph::editor_bar`]).
const ONE_LINE_EDIT_BAR: f32 = 360.0;
/// Most height the metronome tab's staff takes, however wide the window;
/// the ring below takes the rest.
const METRONOME_STAFF_MAX: f32 = 220.0;
/// The same with the counter rhythm as a second voice under the click bar.
const METRONOME_TWO_STAFF_MAX: f32 = 340.0;
const CARD_RADIUS: u8 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Tab {
    Metronome,
    Practice,
    /// The score tab, called Edit before it had its own score.
    #[serde(alias = "Edit")]
    Score,
    More,
}

/// UI state of the layout. Not persisted.
pub(crate) struct ShellState {
    /// The options sheet only shows its handle and the transport buttons.
    /// Folded at start, so the staff and the ring get the room.
    pub(crate) options_collapsed: bool,
    /// What the switches of the split rows bring back: the last swing,
    /// count labels and counter rhythm that were not off.
    pub(crate) last_swing_percent: u8,
    pub(crate) last_counting: Option<CountingBase>,
    pub(crate) last_counter: Option<Ratio>,
}

impl Default for ShellState {
    fn default() -> Self {
        Self {
            options_collapsed: true,
            last_swing_percent: DEFAULT_SWING_PERCENT,
            last_counting: None,
            last_counter: None,
        }
    }
}

/// Swing the switch turns on before any other was picked: a triplet feel.
const DEFAULT_SWING_PERCENT: u8 = 66;

/// Bars of the generated score; endless mode never loops.
fn bars_label(generator: &GeneratorState) -> String {
    if generator.endless { "∞".to_owned() } else { generator.effective_bars().to_string() }
}

pub(crate) fn subdivision_short_label(s: Subdivision) -> &'static str {
    match s {
        Subdivision::Eighths => "8ths",
        Subdivision::Sixteenths => "16ths",
        Subdivision::Triplets => "Triplets",
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

/// A value in the settings card.
enum CardValue {
    Text(String),
    /// A meter, stacked in the music font as on the staff.
    Meter(TimeSignature),
}

/// Labelled value in the settings card: caption above, big value below.
fn card_cell(ui: &mut Ui, label: &str, value: &CardValue, music: &FontFamily) -> Response {
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
    // Values that have to shrink stay centered on the line of the full-size ones.
    let value_height = ui.fonts_mut(|f| f.row_height(&FontId::proportional(CARD_VALUE_SIZE)));
    let center = rect.center_bottom() - vec2(0.0, 6.0 + value_height / 2.0);
    match value {
        CardValue::Text(text) => fitted_text(
            painter,
            center,
            Align2::CENTER_CENTER,
            text,
            CARD_VALUE_SIZE,
            width,
            strong,
        ),
        CardValue::Meter(ts) => {
            paint_grouped_meter(painter, *ts, center, CARD_METER_EM, width, music, strong, weak)
        }
    }
    resp
}

/// [`paint_meter`] with the beat groups small underneath in `groups_color`
/// when they are written out (see [`shown_groups`]). Both together take about
/// the height of a meter of size `em` alone, centered on `center`.
#[expect(clippy::too_many_arguments)]
pub(crate) fn paint_grouped_meter(
    painter: &Painter,
    ts: TimeSignature,
    center: Pos2,
    em: f32,
    max_width: f32,
    family: &FontFamily,
    color: Color32,
    groups_color: Color32,
) {
    match shown_groups(ts) {
        None => paint_meter(painter, ts, center, em, max_width, family, color),
        Some(groups) => {
            let meter_center = center - vec2(0.0, 0.18 * em);
            paint_meter(painter, ts, meter_center, 0.74 * em, max_width, family, color);
            fitted_text(
                painter,
                center + vec2(0.0, 0.53 * em),
                Align2::CENTER_BOTTOM,
                &groups,
                0.32 * em,
                max_width,
                groups_color,
            );
        }
    }
}

/// `ts` as stacked time-signature digits centered on `center`, at music font
/// size `em` or smaller to fit `max_width`.
pub(crate) fn paint_meter(
    painter: &Painter,
    ts: TimeSignature,
    center: Pos2,
    em: f32,
    max_width: f32,
    family: &FontFamily,
    color: Color32,
) {
    let advance = |n: u8| ts_glyphs(n).iter().map(|g| g.advance).sum::<f32>() * 0.25;
    let widest = advance(ts.beats).max(advance(ts.beat_unit));
    let em = em.min(max_width / widest);
    let staff_space = 0.25 * em;
    for (n, y) in [(ts.beats, center.y - staff_space), (ts.beat_unit, center.y + staff_space)] {
        let mut pen = center.x - advance(n) * em / 2.0;
        for glyph in ts_glyphs(n) {
            let placed = PlacedGlyph { glyph, origin: pos2(pen, y), size: em };
            draw_glyph(painter, &placed, family, color);
            pen += glyph.advance * staff_space;
        }
    }
}

/// Full-width row with a label and an on/off switch. Returns whether it toggled.
pub(crate) fn switch_row(ui: &mut Ui, label: &str, on: &mut bool) -> bool {
    badged_switch_row(ui, label, None, on)
}

/// [`switch_row`] with a small pill after the label, such as "BETA".
fn badged_switch_row(ui: &mut Ui, label: &str, badge: Option<&str>, on: &mut bool) -> bool {
    let (rect, resp) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    let how_on = ui.ctx().animate_bool_responsive(resp.id, *on);
    let visuals = ui.visuals();
    let painter = ui.painter();
    let label_rect = painter.text(
        rect.left_center() + vec2(4.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(19.0),
        visuals.text_color(),
    );
    if let Some(badge) = badge {
        paint_badge(painter, pos2(label_rect.right() + 8.0, rect.center().y), badge);
    }

    paint_switch(painter, rect, *on, how_on, visuals);
    resp.clicked()
}

/// The switch at the right end of `row`, `how_on` along its way to on.
fn paint_switch(painter: &Painter, row: Rect, on: bool, how_on: f32, visuals: &egui::Visuals) {
    let switch = Rect::from_center_size(row.right_center() - vec2(30.0, 0.0), vec2(52.0, 30.0));
    let fill = if on { visuals.selection.bg_fill } else { switch_off_fill(visuals) };
    painter.rect_filled(switch, 15, fill);
    let radius = 12.0;
    let x = egui::lerp((switch.left() + 15.0)..=(switch.right() - 15.0), how_on);
    painter.circle_filled(pos2(x, switch.center().y), radius, Color32::WHITE);
}

/// Width of the switch's own tap area at the right end of a [`split_row`].
const SPLIT_SWITCH_WIDTH: f32 = 68.0;

/// A [`value_row`] and a switch in one row, split by a thin line: the
/// switch turns the setting on and off, the rest opens its picker. Returns
/// the response of the part that opens the picker.
fn split_row(ui: &mut Ui, label: &str, value: &str, on: &mut bool) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    let split = rect.right() - SPLIT_SWITCH_WIDTH;
    let open_rect = rect.with_max_x(split);
    let switch_rect = rect.with_min_x(split);
    let open = ui.interact(open_rect, Id::unique(("split_open", label)), Sense::click());
    let toggle = ui.interact(switch_rect, Id::unique(("split_switch", label)), Sense::click());
    if toggle.clicked() {
        *on = !*on;
    }
    let how_on = ui.ctx().animate_bool_responsive(toggle.id, *on);
    let visuals = ui.visuals();
    let painter = ui.painter();
    let label = painter.text(
        rect.left_center() + vec2(4.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(19.0),
        visuals.text_color(),
    );
    // Cut short before the label on a narrow screen.
    let value = cut_to_fit(painter, value, split - 30.0 - label.right() - 16.0);
    painter.text(
        pos2(split - 30.0, rect.center().y),
        Align2::RIGHT_CENTER,
        value,
        FontId::proportional(19.0),
        visuals.weak_text_color(),
    );
    painter.text(
        pos2(split - 10.0, rect.center().y),
        Align2::RIGHT_CENTER,
        "›",
        FontId::proportional(26.0),
        visuals.weak_text_color(),
    );
    let divider = visuals.widgets.noninteractive.bg_stroke;
    painter.vline(split, rect.y_range().shrink(10.0), divider);
    paint_switch(painter, rect, *on, how_on, visuals);
    open
}

/// Pill in the accent colour with small text, its left edge vertically
/// centered on `left`.
fn paint_badge(painter: &Painter, left: Pos2, text: &str) {
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(12.0), BADGE_TEXT);
    let size = galley.size() + vec2(12.0, 2.0);
    let pill = Rect::from_min_size(pos2(left.x, left.y - size.y / 2.0), size);
    painter.rect_filled(pill, size.y / 2.0, BADGE_FILL);
    painter.galley(pill.center() - galley.size() / 2.0, galley, Color32::PLACEHOLDER);
}

/// The colours of a badge such as "BETA", in a settings row or on a tab:
/// orange, the same in both themes.
const BADGE_FILL: Color32 = Color32::from_rgb(0xD9, 0x6B, 0x1E);
const BADGE_TEXT: Color32 = Color32::WHITE;

/// Track colour of a switch that is off. It has to differ from the white
/// knob, so the light theme gets a grey instead of its white field colour.
fn switch_off_fill(visuals: &egui::Visuals) -> Color32 {
    if visuals.dark_mode { visuals.extreme_bg_color } else { Color32::from_gray(190) }
}

/// Full-width row with a label, the current value and a chevron.
/// `value`, cut short with "…" to be at most `room` wide in a row's value font.
fn cut_to_fit(painter: &Painter, value: &str, room: f32) -> String {
    let fits = |text: &str| {
        painter.layout_no_wrap(text.to_owned(), FontId::proportional(19.0), Color32::WHITE).size().x
            <= room
    };
    let mut value = value.to_owned();
    if !fits(&value) {
        while !value.is_empty() && !fits(&format!("{value}…")) {
            value.pop();
        }
        value = format!("{}…", value.trim_end());
    }
    value
}

pub(crate) fn value_row(ui: &mut Ui, label: &str, value: &str) -> Response {
    let (rect, resp) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    let visuals = ui.visuals();
    let painter = ui.painter();
    let label = painter.text(
        rect.left_center() + vec2(4.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(19.0),
        visuals.text_color(),
    );
    // A long value (a device or preset name) is cut short before the label.
    let value = cut_to_fit(painter, value, rect.right() - 30.0 - label.right() - 16.0);
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

/// The More menu's theme row: its three choices side by side at the right.
fn theme_row(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    let label = ui.painter().text(
        rect.left_center() + vec2(4.0, 0.0),
        Align2::LEFT_CENTER,
        "🌙   Theme",
        FontId::proportional(19.0),
        ui.visuals().text_color(),
    );
    let theme = ui.options(|o| o.theme_preference);
    let choices = [
        (ThemePreference::System, "System"),
        (ThemePreference::Light, "Light"),
        (ThemePreference::Dark, "Dark"),
    ];
    let spacing = 4.0;
    // Up to 76 px each, narrower on a small phone so they stay clear of the
    // label; their text shrinks to fit.
    let room = rect.right() - label.right() - 12.0;
    let width = ((room - 2.0 * spacing) / 3.0).min(76.0);
    let padding = 2.0 * ui.spacing().button_padding.x;
    let layout = egui::Layout::right_to_left(egui::Align::Center);
    let mut picked = None;
    ui.scope_builder(UiBuilder::new().max_rect(rect).layout(layout), |ui| {
        ui.spacing_mut().item_spacing.x = spacing;
        // Right to left: the last choice comes first.
        for (choice, text) in choices.into_iter().rev() {
            let text_width = ui
                .painter()
                .layout_no_wrap(text.to_owned(), FontId::proportional(16.0), Color32::WHITE)
                .size()
                .x;
            let size = 16.0 * ((width - padding) / text_width).min(1.0);
            let button = Button::new(RichText::new(text).size(size))
                .selected(theme == choice)
                .min_size(vec2(width, 32.0));
            if ui.add(button).clicked() {
                picked = Some(choice);
            }
        }
    });
    if let Some(theme) = picked {
        ui.ctx().set_theme(theme);
    }
}

/// [`value_row`] showing a subdivision as notes or as "none", `text` if
/// it is neither.
fn notation_value_row(
    ui: &mut Ui,
    label: &str,
    look: Option<&FillLook>,
    text: &str,
    family: FontFamily,
) -> Response {
    let Some(look) = look else {
        return value_row(ui, label, text);
    };
    let resp = value_row(ui, label, "");
    // The weak text color is see-through: laid on the card, so notes that
    // overlap (heads, stems, beams) don't show their overlaps.
    let color = card_fill(ui).blend(ui.visuals().weak_text_color());
    let right = resp.rect.right_center() - vec2(28.0, 0.0);
    let at = |size: Vec2| Rect::from_min_size(right - vec2(size.x, size.y / 2.0), size);
    match look {
        FillLook::Notes(notes) => {
            let rect = at(notation_size(notes, ROW_HEIGHT - 4.0));
            paint_notation(ui, rect, notes, family, color);
        }
        FillLook::NoSubdivision => paint_none(ui.painter(), at(Vec2::splat(ROW_HEIGHT)), color),
    }
    resp.on_hover_text(text)
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
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), SHEET_HANDLE_HEIGHT), Sense::hover());
    ui.painter().rect_filled(
        Rect::from_center_size(rect.center(), vec2(40.0, 5.0)),
        3,
        ui.visuals().weak_text_color(),
    );
    // A finger-sized target around the thin handle; the buttons below are
    // registered later, so they keep their own taps where the two overlap.
    let target = Rect::from_center_size(rect.center(), vec2(rect.width(), MIN_TOUCH_TARGET));
    ui.interact(target, ui.auto_id_with("sheet_handle"), Sense::click()).clicked()
}

const SHEET_HANDLE_HEIGHT: f32 = 22.0;
/// Smallest touch target, in points (Apple's and Claude's guidelines).
const MIN_TOUCH_TARGET: f32 = 44.0;

/// The card of an options sheet, and a slot painted before it for the shadow
/// the raised options cast along it.
struct SheetCard {
    rect: Rect,
    shadow_painter: Painter,
    shadow_slot: ShapeIdx,
}

fn sheet_card(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui)) -> SheetCard {
    let shadow_painter = ui.painter().clone();
    let shadow_slot = shadow_painter.add(Shape::Noop);
    let rect = Frame::new()
        .fill(card_fill(ui))
        .corner_radius(CARD_RADIUS)
        .inner_margin(Margin { left: 12, right: 12, top: 0, bottom: 12 })
        .show(ui, add_contents)
        .response
        .rect;
    SheetCard { rect, shadow_painter, shadow_slot }
}

/// How far a finger moves up or down on a sheet before it opens or folds.
const SHEET_FLICK_DISTANCE: f32 = 24.0;

/// A finger (or mouse) dragged up or down on the sheet, made of the `parts`
/// (each a rect on its layer): `Some(true)` to open it, `Some(false)` to fold
/// it. Fires once per gesture, as soon as the drag is far enough and mostly
/// vertical. Only gestures that start on the sheet count, so scrolling the
/// staff never folds it. A drag past the threshold is no click either, so the
/// buttons under the finger stay untouched.
fn sheet_flick(ui: &Ui, parts: &[(LayerId, Rect)]) -> Option<bool> {
    let (origin, start, delta) = ui.input(|i| {
        let p = &i.pointer;
        Some((p.press_origin()?, p.press_start_time()?, p.total_drag_delta()?))
    })?;
    // A picker or dialog on top of the sheet gets the gesture.
    let top = ui.ctx().layer_id_at(origin);
    if !parts.iter().any(|(layer, rect)| rect.contains(origin) && top == Some(*layer)) {
        return None;
    }
    if delta.y.abs() < SHEET_FLICK_DISTANCE || delta.y.abs() < 1.5 * delta.x.abs() {
        return None;
    }
    let fired = Id::unique("sheet_flick_fired");
    if ui.data(|d| d.get_temp::<f64>(fired)) == Some(start) {
        return None;
    }
    ui.data_mut(|d| d.insert_temp(fired, start));
    Some(delta.y < 0.0)
}

pub(crate) fn paint_play_icon(painter: &Painter, rect: Rect, color: Color32, playing: bool) {
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
    /// Tab shown in the tab bar: the More screen, else the page whose
    /// document is shown.
    pub(crate) fn tab(&self) -> Tab {
        match self.ui.mode {
            // More stays in front of the page it was opened from.
            Mode::MoreMenu => Tab::More,
            _ => self.document().tab(),
        }
    }

    /// Open `tab`: a page plays its own document. Another page stops what
    /// plays; More is a screen of its own and leaves it alone.
    pub(crate) fn select_tab(&mut self, tab: Tab) {
        self.ui.mode = match tab {
            Tab::Metronome => {
                self.show_document(Document::Metronome);
                Mode::Playback
            }
            Tab::Practice => {
                self.show_document(Document::Practice);
                Mode::Playback
            }
            Tab::Score => {
                self.show_document(Document::Score);
                Mode::Playback
            }
            Tab::More => Mode::MoreMenu,
        };
    }

    pub(super) fn shell_ui(&mut self, ui: &mut Ui) {
        let tab = self.tab();
        // An editor has the screen to itself: Done leaves it, and the tool
        // grid gets the tab bar's room.
        if !matches!(self.ui.mode, Mode::Edit | Mode::TimeSignature(_)) {
            self.tab_bar(ui, tab);
        }
        let playing = self.ui.mode == Mode::Playback;
        match tab {
            Tab::Practice if playing => {
                self.options_sheet(ui);
                self.settings_card(ui);
            }
            Tab::Score if playing => {
                self.score_sheet(ui);
                self.score_card(ui);
            }
            // A rhythm page's editor leaves the staff all the room above
            // its palette.
            // The editor's bar at the very bottom, the grid above it.
            Tab::Practice | Tab::Score => {
                self.edit_bar(ui);
                self.tool_palette_panel(ui);
            }
            Tab::Metronome if self.ui.mode == Mode::Edit => {
                // No ring here: what was hovered on it is gone, also when
                // Done brings the ring back with the staff of this frame.
                self.editor.metronome.forget_hover();
                self.metronome_edit_bar(ui);
                self.tool_palette_panel(ui);
                self.metronome_card(ui);
            }
            Tab::Metronome => self.metronome_page(ui),
            Tab::More => self.more_page(ui),
        }
        if tab == Tab::More {
            // No staff on this screen, but what plays moves on.
            self.advance_unseen_playback(ui);
            self.editor.metronome.forget_hover();
        } else {
            self.measure_panel(ui);
        }
        self.settings_picker(ui);
    }

    /// The metronome tab above the tab bar: its card, options sheet and ring.
    /// The staff goes in the room left.
    /// Embedded, meter and tempo sit next to the play button instead, so the
    /// staff and the ring get the card's room.
    pub(crate) fn metronome_page(&mut self, ui: &mut Ui) {
        self.metronome_sheet(ui);
        if !self.embedded() {
            self.metronome_card(ui);
        }
        self.staff_and_ring(ui, false);
    }

    /// The ring below the space left, the staff above it as tall as it
    /// needs (see [`Self::metronome_ring`] for `play_in_middle`).
    pub(crate) fn staff_and_ring(&mut self, ui: &mut Ui, play_in_middle: bool) {
        let room = ui.available_height().max(0.0);
        let max = if self.counter_bar().is_some() {
            METRONOME_TWO_STAFF_MAX
        } else {
            METRONOME_STAFF_MAX
        };
        // The staff gets what it needs at this width (up to `max`), the ring
        // the rest, centered in it: worked out in this frame, so a new size
        // or layout shows at once.
        let staff = self.metronome_staff_height(ui, ui.available_width()).min(max).min(room);
        let height = room - staff;
        self.metronome_ring(ui, height, play_in_middle);
    }

    fn tab_bar(&mut self, ui: &mut Ui, current: Tab) {
        const TABS: [(Tab, &str, &str); 4] = [
            (Tab::Metronome, "⏱", "Metronome"),
            (Tab::Practice, "🎲", "Practice"),
            (Tab::Score, "🎼", "Score"),
            (Tab::More, "☰", "More"),
        ];
        // The score tab, not finished yet, only when the developer setting
        // shows it, or while it is open (from a link or a pattern).
        let tabs: Vec<_> = TABS
            .into_iter()
            .filter(|&(tab, ..)| tab != Tab::Score || self.ui.show_score_tab || tab == current)
            .collect();
        let mut picked = None;
        egui::Panel::bottom("shell_tabs")
            .frame(Frame::side_top_panel(ui.style()).inner_margin(Margin::symmetric(0, 4)))
            .show(ui, |ui| {
                centered_column(ui, |ui| {
                    ui.columns(tabs.len(), |cols| {
                        for (col, &(tab, icon, label)) in cols.iter_mut().zip(&tabs) {
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
                            if tab == Tab::Score {
                                beta_pill(painter, rect.center_top() + vec2(14.0, 2.0));
                            }
                            if resp.clicked() {
                                picked = Some(tab);
                            }
                        }
                    });
                });
            });
        if let Some(tab) = picked {
            self.select_tab(tab);
        }
    }

    /// Generator settings and tempo as big values; a tap opens the picker.
    fn settings_card(&mut self, ui: &mut Ui) {
        let settings = &self.editor.generator.settings;
        let subdivision = effective_subdivision(settings.subdivision, settings.time_signature);
        let level = if settings.custom_figures_enabled {
            "Custom".to_owned()
        } else {
            settings.complexity.to_string()
        };
        let cells = [
            (Picker::TimeSignature, "Meter", CardValue::Meter(settings.time_signature)),
            (
                Picker::Subdivision,
                "Notes",
                CardValue::Text(subdivision_short_label(subdivision).to_owned()),
            ),
            (Picker::Complexity, "Level", CardValue::Text(level)),
            (Picker::Bars, "Bars", CardValue::Text(bars_label(&self.editor.generator))),
            // While a tempo ramp runs: the tempo being played.
            match self.playback_ctl.playback.ramp_bpm {
                Some(bpm) => (Picker::Bpm, "BPM now", CardValue::Text(bpm.to_string())),
                None => (Picker::Bpm, "BPM", CardValue::Text(self.playback_ctl.bpm.to_string())),
            },
        ];

        self.card(ui, &cells);
    }

    /// The score tab's score by name (a tap opens the library) and the
    /// tempo, in the same card as Practice.
    fn score_card(&mut self, ui: &mut Ui) {
        let cells = [
            (Picker::Library, "Score", CardValue::Text(self.score_title())),
            match self.playback_ctl.playback.ramp_bpm {
                Some(bpm) => (Picker::Bpm, "BPM now", CardValue::Text(bpm.to_string())),
                None => (Picker::Bpm, "BPM", CardValue::Text(self.playback_ctl.bpm.to_string())),
            },
        ];
        self.card(ui, &cells);
    }

    /// Meter and tempo of the metronome tab, in the same card as Practice.
    fn metronome_card(&mut self, ui: &mut Ui) {
        let cells = self.metronome_cells();
        self.card(ui, &cells);
    }

    /// The metronome's meter and tempo, each opening its picker.
    fn metronome_cells(&self) -> [(Picker, &'static str, CardValue); 2] {
        [
            (Picker::MetronomeMeter, "Meter", CardValue::Meter(self.metronome_meter())),
            match self.playback_ctl.playback.ramp_bpm {
                Some(bpm) => (Picker::Bpm, "BPM now", CardValue::Text(bpm.to_string())),
                None => (Picker::Bpm, "BPM", CardValue::Text(self.playback_ctl.bpm.to_string())),
            },
        ]
    }

    /// Card of labelled values on top; a tap opens the value's picker.
    fn card(&mut self, ui: &mut Ui, cells: &[(Picker, &str, CardValue)]) {
        let music = self.ui.music_font_id.family.clone();
        let mut picked = None;
        egui::Panel::top("shell_card")
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
                                    if card_cell(col, label, value, &music).clicked() {
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
    fn options_sheet(&mut self, ui: &mut Ui) {
        let before = self.editor.generator.settings;
        let endless_before = self.editor.generator.endless;
        let mut roll = false;
        let mut card = None;

        let sheet = egui::Panel::bottom("shell_options")
            .frame(Frame::new().fill(ui.visuals().panel_fill).inner_margin(Margin {
                left: 12,
                right: 12,
                top: 8,
                bottom: 8,
            }))
            .show_separator_line(false)
            .show(ui, |ui| {
                centered_column(ui, |ui| {
                    card = Some(sheet_card(ui, |ui| {
                        if sheet_handle(ui) {
                            self.ui.shell.options_collapsed = !self.ui.shell.options_collapsed;
                        }
                        roll = self.transport(ui);
                    }));
                });
            });
        let options = self.sheet_options(ui, card, Self::option_rows);
        self.fold_sheet_on_flick(ui, sheet.response.rect, options);

        self.finish_generator_change(before, endless_before, roll);
    }

    /// The score tab's options and its transport: ✏ for the editor, play
    /// and share.
    fn score_sheet(&mut self, ui: &mut Ui) {
        let mut card = None;
        let sheet = egui::Panel::bottom("shell_score")
            .frame(Frame::new().fill(ui.visuals().panel_fill).inner_margin(Margin {
                left: 12,
                right: 12,
                top: 8,
                bottom: 8,
            }))
            .show_separator_line(false)
            .show(ui, |ui| {
                centered_column(ui, |ui| {
                    card = Some(sheet_card(ui, |ui| {
                        // Folds with the other sheets: one sheet setting.
                        if sheet_handle(ui) {
                            self.ui.shell.options_collapsed = !self.ui.shell.options_collapsed;
                        }
                        self.score_transport(ui);
                    }));
                });
            });
        let options = self.sheet_options(ui, card, Self::score_rows);
        self.fold_sheet_on_flick(ui, sheet.response.rect, options);
    }

    /// The score tab's options: Practice's, without those of the generator.
    fn score_rows(&mut self, ui: &mut Ui) {
        self.guide_row(ui);
        self.timing_feedback_row(ui);
        self.shared_rows(ui);
    }

    /// ✏ (the editor), play and share, as Practice's 🎲, play and share.
    fn score_transport(&mut self, ui: &mut Ui) {
        let playing = self.playback_ctl.transport_state == TransportState::Playing;
        self.status_line(ui);
        let (row, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), PLAY_BUTTON_SIZE), Sense::hover());
        let size = Vec2::splat(PLAY_BUTTON_SIZE);
        let side_fill = ui.visuals().extreme_bg_color;
        let text = ui.visuals().strong_text_color();
        if round_button_at(
            ui,
            Rect::from_min_size(row.min, size),
            Id::unique("score_edit"),
            side_fill,
            text,
            |painter, rect, color| {
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    "✏",
                    FontId::proportional(30.0),
                    color,
                );
            },
        )
        .on_hover_text("Edit the score")
        .clicked()
        {
            self.edit_page();
        }
        let fill = ui.visuals().selection.bg_fill;
        let icon = ui.visuals().selection.stroke.color;
        if round_button_at(
            ui,
            Rect::from_center_size(row.center(), size),
            Id::unique("score_play"),
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
            Id::unique("score_share"),
            side_fill,
            text,
            paint_share_icon,
        )
        .on_hover_text("Share a link to this score")
        .clicked()
        {
            self.share_link(ui);
        }
    }

    /// The sheet's options, sliding up out of its top edge over the screen
    /// above, so the staff keeps its size. One state for both sheets, so
    /// switching tabs doesn't replay the animation. Returns the layer and rect
    /// they cover while shown.
    fn sheet_options(
        &mut self,
        ui: &Ui,
        card: Option<SheetCard>,
        add_rows: impl FnOnce(&mut Self, &mut Ui),
    ) -> Option<(LayerId, Rect)> {
        let id = Id::unique("shell_sheet_options");
        let openness = ui.ctx().animate_bool_responsive(id, !self.ui.shell.options_collapsed);
        let SheetCard { rect: card, shadow_painter, shadow_slot } = card?;
        if openness <= 0.0 || !card.is_positive() {
            return None;
        }
        // Height of the rows, measured when last shown.
        let rows_id = id.with("rows_height");
        let rows = ui.data(|d| d.get_temp::<f32>(rows_id)).unwrap_or(240.0);
        // Covers the card's own handle, so the two read as one sheet growing.
        let bottom = card.top() + SHEET_HANDLE_HEIGHT;
        let top = bottom - SHEET_HANDLE_HEIGHT - openness * rows;
        let rect = Rect::from_x_y_ranges(card.x_range(), top..=bottom);
        let area = egui::Area::new(id)
            .order(Order::Middle)
            .fixed_pos(rect.min)
            .constrain(false)
            .show(ui.ctx(), |ui| {
                let fill = card_fill(ui);
                let corners = CornerRadius { nw: CARD_RADIUS, ne: CARD_RADIUS, sw: 0, se: 0 };
                // One shadow around the raised options and the card below.
                // Above the card it lies over the staff; along the card it
                // goes behind it, into the slot painted before the card, so
                // the card's rounded corners stay clean. The panel's clip
                // keeps it off the tab bar.
                let mut shadow = ui.visuals().popup_shadow;
                shadow.color = shadow.color.gamma_multiply(openness);
                let shadow = shadow.as_shape(rect.union(card), CARD_RADIUS);
                let above = rect.union(card).expand(64.0).with_max_y(bottom);
                ui.painter().with_clip_rect(above).add(shadow.clone());
                let below = shadow_painter
                    .clip_rect()
                    .intersect(above.with_min_y(bottom).with_max_y(f32::INFINITY));
                shadow_painter.with_clip_rect(below).set(shadow_slot, shadow);
                ui.painter().rect_filled(rect, corners, fill);
                // Taps on the sheet stay on the sheet.
                ui.interact(rect, id.with("background"), Sense::click());

                let inner = rect.shrink2(vec2(12.0, 0.0));
                let builder = UiBuilder::new().max_rect(inner.with_max_y(f32::INFINITY));
                ui.scope_builder(builder, |ui| {
                    ui.set_clip_rect(rect);
                    if sheet_handle(ui) {
                        self.ui.shell.options_collapsed = !self.ui.shell.options_collapsed;
                    }
                    let start = ui.cursor().top();
                    add_rows(self, ui);
                    ui.add_space(8.0);
                    let height = ui.min_rect().bottom() - start;
                    ui.data_mut(|d| d.insert_temp(rows_id, height));
                });
            });
        if openness < 1.0 {
            ui.ctx().request_repaint();
        }
        Some((area.response.layer_id, rect))
    }

    /// Flicking a sheet up opens its options, flicking it down folds them.
    /// Not inline in a chat: a vertical drag there scrolls the chat.
    fn fold_sheet_on_flick(&mut self, ui: &Ui, sheet: Rect, options: Option<(LayerId, Rect)>) {
        if self.chat_scrolls() {
            return;
        }
        let parts: Vec<_> = std::iter::once((ui.layer_id(), sheet)).chain(options).collect();
        if let Some(open) = sheet_flick(ui, &parts) {
            self.ui.shell.options_collapsed = !open;
            ui.ctx().request_repaint();
        }
    }

    fn metronome_rows(&mut self, ui: &mut Ui) {
        let family = self.ui.music_font_id.family.clone();
        let look = self.click_fill_look();
        let label = self.click_fill_label();
        if notation_value_row(ui, "Subdivision", look.as_ref(), &label, family).clicked() {
            self.ui.picker = Some(Picker::ClickFill);
        }
        // Embedded there is no tool palette to edit them with, and the
        // transport row has no room for the link: it is a row here.
        if self.embedded() {
            if value_row(ui, "Open in grooph", "grooph.app").clicked() {
                self.open_in_grooph(ui);
            }
        } else if value_row(ui, "Edit clicks", "✏").clicked() {
            self.ui.mode = Mode::Edit;
        }
        self.polyrhythm_row(ui);
        self.shared_rows(ui);
    }

    /// Off or the counter rhythm; the switch brings back the last one.
    fn polyrhythm_row(&mut self, ui: &mut Ui) {
        if let Some((ratio, _)) = self.counter_ratio() {
            self.ui.shell.last_counter = Some(ratio);
        }
        let Some(restore) = self.counter_to_restore() else {
            // The meter has no counter rhythm to switch on.
            if value_row(ui, "Polyrhythm", &self.counter_label()).clicked() {
                self.ui.picker = Some(Picker::Polyrhythm);
            }
            return;
        };
        let was_on = self.counter_bar().is_some();
        let value = if was_on { self.counter_label() } else { ratio_label((restore, false)) };
        let mut on = was_on;
        if split_row(ui, "Polyrhythm", &value, &mut on).clicked() {
            self.ui.picker = Some(Picker::Polyrhythm);
        }
        if on != was_on {
            self.set_counter_ratio(on.then_some(restore));
        }
    }

    /// Straight or swung; the switch brings back the last swing.
    fn swing_row(&mut self, ui: &mut Ui) {
        let swing = &mut self.playback_ctl.audio_cfg.swing;
        if !swing.is_straight() {
            self.ui.shell.last_swing_percent = swing.percent;
        }
        let shown = self.ui.shell.last_swing_percent;
        let value = format!("{} {}", swing_label(shown), swing.unit.label());
        let was_on = !swing.is_straight();
        let mut on = was_on;
        if split_row(ui, "Swing", &value, &mut on).clicked() {
            self.ui.picker = Some(Picker::Swing);
        }
        if on != was_on {
            swing.percent = if on { shown } else { MIN_SWING_PERCENT };
        }
    }

    /// The last rows of both sheets, the same in each tab.
    fn shared_rows(&mut self, ui: &mut Ui) {
        self.counting_row(ui);
        switch_row(ui, "Count-in", &mut self.playback_ctl.audio_cfg.count_in);
        let gap = &mut self.playback_ctl.audio_cfg.gap;
        let silent = silent_bars_label(gap);
        if split_row(ui, "Silent bars", &silent, &mut gap.enabled).clicked() {
            self.ui.picker = Some(Picker::SilentBars);
        }
        self.swing_row(ui);
    }

    /// The count under the staff, shown in every tab; opens its picker. The
    /// switch brings back the last count that fits the score.
    fn counting_row(&mut self, ui: &mut Ui) {
        if self.ui.counting.base != CountingBase::Off {
            self.ui.shell.last_counting = Some(self.ui.counting.base);
        }
        // Everything read from the score first, so it is borrowed, not copied.
        let score = self.played_score();
        let counting = &self.ui.counting;
        let restore = self.ui.shell.last_counting.filter(|b| b.fits(score)).or_else(|| {
            CountingBase::ALL.into_iter().find(|&b| b != CountingBase::Off && b.fits(score))
        });
        let was_on = counting.base != CountingBase::Off;
        let shown = match restore {
            Some(restore) if !was_on => restore,
            _ => counting.base,
        };
        let label = shown.label(counting.system, score);
        let Some(restore) = restore else {
            if value_row(ui, "Count labels", &label).clicked() {
                self.ui.picker = Some(Picker::Counting);
            }
            return;
        };
        let mut on = was_on;
        if split_row(ui, "Count labels", &label, &mut on).clicked() {
            self.ui.picker = Some(Picker::Counting);
        }
        if on != was_on {
            self.ui.counting.base = if on { restore } else { CountingBase::Off };
        }
    }

    /// Metronome options and the play button.
    fn metronome_sheet(&mut self, ui: &mut Ui) {
        let mut card = None;
        let sheet = egui::Panel::bottom("shell_metronome")
            .frame(Frame::new().fill(ui.visuals().panel_fill).inner_margin(Margin::same(12)))
            .show_separator_line(false)
            .show(ui, |ui| {
                centered_column(ui, |ui| {
                    card = Some(sheet_card(ui, |ui| {
                        // Folds with the Practice options: one sheet setting.
                        if sheet_handle(ui) {
                            self.ui.shell.options_collapsed = !self.ui.shell.options_collapsed;
                        }
                        self.metronome_transport(ui);
                    }));
                });
            });
        let options = self.sheet_options(ui, card, Self::metronome_rows);
        self.fold_sheet_on_flick(ui, sheet.response.rect, options);
    }

    /// Centered play button, with share on the right; embedded, meter and
    /// tempo on either side instead (see [`Self::metronome_page`]).
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
        if self.embedded() {
            self.transport_cells(ui, row);
            return;
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

    /// Meter left and tempo right of the play button in `row`, as in the
    /// card; a tap opens the picker.
    fn transport_cells(&mut self, ui: &mut Ui, row: Rect) {
        const GAP: f32 = 12.0;
        const MAX_WIDTH: f32 = 140.0;
        let music = self.ui.music_font_id.family.clone();
        let width = ((row.width() - PLAY_BUTTON_SIZE) / 2.0 - GAP).min(MAX_WIDTH);
        let size = vec2(width, CARD_CELL_HEIGHT);
        let left = row.center() - vec2(PLAY_BUTTON_SIZE / 2.0 + GAP + width / 2.0, 0.0);
        let right = row.center() + vec2(PLAY_BUTTON_SIZE / 2.0 + GAP + width / 2.0, 0.0);
        let mut picked = None;
        for ((picker, label, value), center) in
            self.metronome_cells().into_iter().zip([left, right])
        {
            let rect = Rect::from_center_size(center, size);
            let builder = UiBuilder::new().max_rect(rect).id_salt(label);
            if ui.scope_builder(builder, |ui| card_cell(ui, label, &value, &music)).inner.clicked()
            {
                picked = Some(picker);
            }
        }
        if picked.is_some() {
            self.ui.picker = picked;
        }
    }

    fn option_rows(&mut self, ui: &mut Ui) {
        if value_row(ui, "Preset", &self.active_preset_label()).clicked() {
            self.ui.picker = Some(Picker::Preset);
        }
        let generator = &mut self.editor.generator;
        switch_row(ui, "Ghost notes", &mut generator.ghost_notes);
        self.guide_row(ui);
        self.timing_feedback_row(ui);
        self.shared_rows(ui);
        // Edited here, as the metronome's "Edit clicks".
        if value_row(ui, "Edit rhythm", "✏").clicked() {
            self.edit_page();
        }
    }

    fn guide_row(&mut self, ui: &mut Ui) {
        let ts = self.guide_meter();
        if value_row(ui, "Guide", self.ui.guide.in_meter(&ts).label()).clicked() {
            self.ui.picker = Some(Picker::Guide);
        }
    }

    fn timing_feedback_row(&mut self, ui: &mut Ui) {
        let mut accuracy = self.playback_ctl.accuracy.enabled;
        if badged_switch_row(ui, "Timing feedback", Some("BETA"), &mut accuracy) {
            self.set_accuracy_enabled(accuracy);
        }
    }

    /// Centered "new rhythm" and play buttons. Returns whether "new" was tapped.
    fn transport(&mut self, ui: &mut Ui) -> bool {
        let playing = self.playback_ctl.transport_state == TransportState::Playing;
        self.status_line(ui);
        let (row, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), PLAY_BUTTON_SIZE), Sense::hover());
        let size = Vec2::splat(PLAY_BUTTON_SIZE);
        let new_rect = Rect::from_min_size(row.min, size);
        let play_rect = Rect::from_center_size(row.center(), size);
        let share_rect = Rect::from_min_size(row.right_top() - vec2(PLAY_BUTTON_SIZE, 0.0), size);

        let side_fill = ui.visuals().extreme_bg_color;
        let text = ui.visuals().strong_text_color();
        // Endless mode rolls new bars by itself, and each play starts afresh.
        let roll = !self.editor.generator.endless
            && round_button_at(
                ui,
                new_rect,
                Id::unique("shell_new"),
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
            Id::unique("shell_play"),
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
            Id::unique("shell_share"),
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
    fn status_line(&mut self, ui: &mut Ui) {
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

    /// Play button and measure controls below the tool palette, as wide as it.
    /// A bar is added by a tap on the ghost bar after the last one.
    fn edit_bar(&mut self, ui: &mut Ui) {
        self.editor_bar(ui, "shell_edit_bar", true, |app, ui| {
            if ui.button("➖").on_hover_text("Remove this bar").clicked() {
                app.remove_current_measure();
            }
        });
    }

    /// Play button and a way back to the ring below the tool palette, while
    /// the click bar is edited note by note. A tap on a voice's line picks
    /// the voice the palette edits.
    fn metronome_edit_bar(&mut self, ui: &mut Ui) {
        self.editor_bar(ui, "metronome_edit_bar", false, |_, _| {});
    }

    /// An editor's bar below the tool grid: play, undo, redo and clear, the
    /// editor's own buttons (`extra`, if `has_extra`), and Done on the right.
    /// Where they do not all fit in one line, the editor's own buttons go
    /// in a second one below.
    fn editor_bar(
        &mut self,
        ui: &mut Ui,
        id: &'static str,
        has_extra: bool,
        extra: impl FnOnce(&mut Self, &mut Ui),
    ) {
        egui::Panel::bottom(id)
            .frame(
                Frame::new().fill(ui.visuals().panel_fill).inner_margin(Margin::symmetric(12, 6)),
            )
            .show_separator_line(false)
            .show(ui, |ui| {
                let one_line = !has_extra || ui.available_width() >= ONE_LINE_EDIT_BAR;
                let mut extra = has_extra.then_some(extra);
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
                    let size = self.bar_tool_buttons(ui);
                    if one_line && let Some(extra) = extra.take() {
                        extra(self, ui);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Back to the page's card and options sheet.
                        // A check mark drawn in, as big as the trash can,
                        // in a button as big as the others.
                        let done = ui.add(Button::new("").min_size(size)).on_hover_text("Done");
                        let r = done.rect.shrink2(vec2(0.32, 0.36) * done.rect.size());
                        let color = ui.style().interact(&done).fg_stroke.color;
                        ui.painter().line(
                            vec![
                                r.left_center() + vec2(0.0, 0.05 * r.height()),
                                r.center_bottom() - vec2(0.12 * r.width(), 0.0),
                                r.right_top(),
                            ],
                            egui::Stroke::new(2.5, color),
                        );
                        if done.clicked() {
                            self.leave_editor();
                        }
                    });
                });
                if let Some(extra) = extra {
                    ui.horizontal(|ui| extra(self, ui));
                }
            });
    }

    /// Undo, redo and clear, in an editor's bar ([`BAR_TOOLS`]).
    /// Their size.
    fn bar_tool_buttons(&mut self, ui: &mut Ui) -> egui::Vec2 {
        let mut size = egui::Vec2::ZERO;
        for id in BAR_TOOLS {
            let t = tool(id);
            let enabled = match t.kind {
                ToolKind::Edit(EditOp::Undo) => self.can_undo(),
                ToolKind::Edit(EditOp::Redo) => self.can_redo(),
                _ => true,
            };
            let hover = match t.shortcuts.first() {
                Some(sc) => format!("{} ({})", bar_tool_name(t), sc.label()),
                None => bar_tool_name(t).to_owned(),
            };
            let button = ui.add_enabled(enabled, Button::new(t.label)).on_hover_text(hover);
            size = size.max(button.rect.size());
            if button.clicked() {
                self.apply_tool(t);
            }
        }
        size
    }

    /// "More" tab: a screen of its own, without the staff. A list at the
    /// bottom, in reach of the thumb, whose rows open dialogs.
    fn more_page(&mut self, ui: &mut Ui) {
        let frame = Frame::new().fill(ui.visuals().panel_fill).inner_margin(Margin::same(12));
        egui::Panel::bottom("shell_more").frame(frame).show_separator_line(false).show(ui, |ui| {
            centered_column(ui, |ui| {
                Frame::new()
                    .fill(card_fill(ui))
                    .corner_radius(CARD_RADIUS)
                    .inner_margin(Margin::symmetric(12, 4))
                    .show(ui, |ui| self.more_menu(ui));
            });
        });
        egui::CentralPanel::default().frame(frame).show(ui, |_| {});
    }

    /// The rows of the More menu.
    fn more_menu(&mut self, ui: &mut Ui) {
        if value_row(ui, "📁   Library", "").clicked() {
            self.ui.picker = Some(Picker::Library);
        }
        if value_row(ui, "🔈   Mixer", "").clicked() {
            self.ui.picker = Some(Picker::Mixer);
        }
        // The sounds are picked here rather than deep in Settings; the
        // click's own parameters stay there.
        if value_row(ui, "🔔   Sounds", "").clicked() {
            self.ui.picker = Some(Picker::Sounds);
        }
        if value_row(ui, "🎹   MIDI input", &self.midi_input_label()).clicked() {
            // A device plugged in since shows up.
            self.refresh_midi_ports();
            self.ui.picker = Some(Picker::MidiInput);
        }
        // Next to the MIDI device it listens to: the other inputs and every
        // latency.
        if value_row(ui, "🎯   Play along", &self.play_along_label()).clicked() {
            self.ui.picker = Some(Picker::PlayAlong);
        }
        if value_row(ui, "⚙   Settings", "").clicked() {
            self.ui.picker = Some(Picker::Settings);
        }
        theme_row(ui);
        if value_row(ui, "?   Help", "").clicked() {
            self.ui.picker = Some(Picker::Help);
        }
    }
}

/// A small "BETA" pill with its left edge at `left_top`, beside a tab's
/// icon: the tab is not finished yet.
fn beta_pill(painter: &egui::Painter, left_top: egui::Pos2) {
    let font = FontId::proportional(9.0);
    let text = painter.layout_no_wrap("BETA".to_owned(), font, BADGE_TEXT);
    let rect = egui::Rect::from_min_size(left_top, text.size() + vec2(8.0, 2.0));
    painter.rect_filled(rect, rect.height() / 2.0, BADGE_FILL);
    painter.galley(rect.min + vec2(4.0, 1.0), text, BADGE_TEXT);
}

/// What a tool of the editor's bar does, on its hover.
fn bar_tool_name(tool: &Tool) -> &'static str {
    match tool.kind {
        ToolKind::Edit(EditOp::Undo) => "Undo",
        ToolKind::Edit(EditOp::Redo) => "Redo",
        _ => "Clear the bar",
    }
}

#[cfg(test)]
mod tests {
    use super::sheet_flick;
    use eframe::egui::{
        self, Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2,
    };

    /// Runs one frame with `events` and returns what `sheet_flick` saw for a
    /// sheet across the bottom of the screen.
    fn frame(ctx: &Context, events: Vec<Event>) -> Option<bool> {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 800.0))),
            events,
            ..Default::default()
        };
        let mut flick = None;
        let mut output = ctx.run_ui(input, |ui| {
            let sheet = Rect::from_min_max(pos2(0.0, 600.0), pos2(400.0, 800.0));
            flick = sheet_flick(ui, &[(ui.layer_id(), sheet)]);
        });
        output.textures_delta.clear();
        flick
    }

    fn button(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    /// Drags from `from` by `by` in a few steps; returns every frame's result.
    fn drag(ctx: &Context, from: Pos2, by: egui::Vec2) -> Vec<Option<bool>> {
        let mut seen = vec![frame(ctx, vec![Event::PointerMoved(from), button(from, true)])];
        for step in 1..=4 {
            let pos = from + by * (step as f32 / 4.0);
            seen.push(frame(ctx, vec![Event::PointerMoved(pos)]));
        }
        seen.push(frame(ctx, vec![button(from + by, false)]));
        seen
    }

    fn fired(seen: &[Option<bool>]) -> Vec<bool> {
        seen.iter().flatten().copied().collect()
    }

    #[test]
    fn flick_up_opens_and_down_folds_once_per_gesture() {
        let ctx = Context::default();
        assert_eq!(fired(&drag(&ctx, pos2(200.0, 700.0), vec2(0.0, -80.0))), [true]);
        assert_eq!(fired(&drag(&ctx, pos2(200.0, 650.0), vec2(0.0, 80.0))), [false]);
    }

    #[test]
    fn drags_off_the_sheet_or_sideways_do_nothing() {
        let ctx = Context::default();
        // Scrolling the staff above, even into the sheet.
        assert!(fired(&drag(&ctx, pos2(200.0, 300.0), vec2(0.0, 400.0))).is_empty());
        // Short and sideways drags on the sheet.
        assert!(fired(&drag(&ctx, pos2(200.0, 700.0), vec2(0.0, -10.0))).is_empty());
        assert!(fired(&drag(&ctx, pos2(100.0, 700.0), vec2(150.0, -40.0))).is_empty());
    }
}
