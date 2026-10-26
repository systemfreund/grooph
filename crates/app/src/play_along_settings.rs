//! The Play along row of the More menu and its dialog: which inputs count as
//! hits (taps, the microphone, a MIDI device), the microphone's threshold and
//! every input's latency with its calibration.

use crate::Grooph;
use crate::clock;
use crate::pickers::{Picker, ROW_HEIGHT, choice_grid};
use crate::play_along::{HitSource, MAX_INPUT_OFFSET_MS, TapKey};
use crate::shell::{switch_row, value_row};
use eframe::egui;
use eframe::egui::{Button, Color32, FontId, RichText, Ui, vec2};
use grooph_mic::MicStatus;

/// Lowest level the meter shows (dBFS).
const METER_FLOOR_DB: f32 = -50.0;
/// How far the meter falls back per repaint (dB).
const METER_FALL_DB: f32 = 1.5;
/// A held − or + steps again after this long (s) …
const REPEAT_DELAY: f64 = 0.4;
/// … and then every this long (s).
const REPEAT_EVERY: f64 = 0.05;

impl Grooph {
    /// The inputs that count as hits, for the More menu's row, or "Off".
    pub(crate) fn play_along_label(&self) -> String {
        let settings = &self.playback_ctl.play_along.settings;
        let midi = self.playback_ctl.midi.input.as_ref().is_some_and(|input| input.is_connected());
        let inputs: Vec<_> =
            [(settings.tap_enabled, "Tap"), (settings.mic_enabled, "Mic"), (midi, "MIDI")]
                .into_iter()
                .filter_map(|(on, name)| on.then_some(name))
                .collect();
        if inputs.is_empty() { "Off".to_owned() } else { inputs.join(" · ") }
    }

    /// One group per input: its switch (or device), what it needs and its
    /// latency. Returns the input to calibrate; the caller starts it once
    /// the dialog is drawn, because the run swaps the score.
    pub(crate) fn play_along_choices(&mut self, ui: &mut Ui) -> Option<HitSource> {
        let mut calibrate = None;
        // Everything open at once is taller than a phone.
        let height = (ui.ctx().content_rect().height() - 200.0).max(240.0);
        egui::ScrollArea::vertical().id_salt("play_along").max_height(height).show(ui, |ui| {
            ui.weak("Play along and see how early or late every hit is.");
            ui.add_space(4.0);

            let settings = &mut self.playback_ctl.play_along.settings;
            switch_row(ui, "Tap", &mut settings.tap_enabled);
            if settings.tap_enabled {
                ui.weak("Touch the staff or press the key while it plays.");
                let keys: Vec<_> = TapKey::ALL.map(|k| (k, k.label().to_owned())).into();
                if let Some(key) = choice_grid(ui, &keys, settings.tap_key, 4) {
                    settings.tap_key = key;
                }
                if latency_row(ui, &mut settings.tap_offset_ms) {
                    calibrate = Some(HitSource::Tap);
                }
            }
            ui.separator();

            switch_row(ui, "Microphone", &mut settings.mic_enabled);
            if settings.mic_enabled {
                ui.weak("Sticks, a pad or claps. With a headset on a phone, the phone listens.");
                self.mic_settings(ui);
                let settings = &mut self.playback_ctl.play_along.settings;
                if latency_row(ui, &mut settings.mic_offset_ms) {
                    calibrate = Some(HitSource::Mic);
                }
            }
            ui.separator();

            let midi_connected =
                self.playback_ctl.midi.input.as_ref().is_some_and(|input| input.is_connected());
            if value_row(ui, "MIDI", &self.midi_input_label()).clicked() {
                // A device plugged in since shows up.
                self.refresh_midi_ports();
                self.ui.picker = Some(Picker::MidiInput);
            }
            if midi_connected {
                let settings = &mut self.playback_ctl.play_along.settings;
                if latency_row(ui, &mut settings.midi_offset_ms) {
                    calibrate = Some(HitSource::Midi);
                }
            }
            ui.add_space(8.0);
            ui.weak(
                "Latency is taken off your hits. Calibrate plays a click: play along on every \
                 beat and the median deviation becomes the latency.",
            );
        });
        calibrate
    }

    /// Microphone status, a level meter and the hit threshold. Keeps the
    /// microphone listening while shown.
    fn mic_settings(&mut self, ui: &mut Ui) {
        let mic = &mut self.playback_ctl.mic;
        mic.monitor_until = clock::now_seconds() + 1.0;
        let status = mic.input.as_ref().map(|input| input.status());
        match status {
            None if mic.permission_asked => {
                ui.label(
                    "Waiting for microphone access. If you denied it, allow it in the \
                     system settings.",
                );
            }
            None | Some(MicStatus::Starting) => {
                ui.label("Starting the microphone…");
            }
            Some(MicStatus::Running(name)) => {
                ui.label(format!("Listening: {name}"));
            }
            Some(MicStatus::Failed(err)) => {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("Microphone unavailable: {err}"),
                );
            }
        }
        if let Some(input) = &mic.input {
            mic.level_db = input.take_peak_db().max(mic.level_db - METER_FALL_DB);
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        }
        let threshold_db = &mut self.playback_ctl.play_along.settings.mic_threshold_db;
        ui.add_space(4.0);
        level_meter(ui, mic.level_db, *threshold_db);
        ui.add_space(4.0);
        ui.label(format!("Threshold {threshold_db:.0} dB: louder sounds count as hits"));
        // The whole width, so the dialog does not grow.
        ui.spacing_mut().slider_width = ui.available_width();
        ui.add(egui::Slider::new(threshold_db, METER_FLOOR_DB..=-3.0).show_value(false));
    }
}

/// "Latency", a − / + stepper for `offset_ms` (held, it keeps stepping) and
/// a Calibrate button. Returns whether Calibrate was tapped.
fn latency_row(ui: &mut Ui, offset_ms: &mut f32) -> bool {
    // The app's wide button padding would not leave room on a phone.
    let padding = 4.0;
    let spacing = ui.spacing().item_spacing.x;
    let text_width = |ui: &Ui, text: &str, size: f32| {
        ui.painter()
            .layout_no_wrap(text.to_owned(), FontId::proportional(size), Color32::WHITE)
            .size()
            .x
    };
    let label = text_width(ui, "Latency", 17.0) + 8.0;
    let number = text_width(ui, "−300 ms", 18.0) + 4.0;
    let calibrate_min = text_width(ui, "Calibrate", 17.0) + 2.0 * padding;
    // On a narrow phone the − and + give way first.
    let room = ui.available_width() - label - number - calibrate_min - 4.0 * spacing;
    let step_width = (room / 2.0).clamp(32.0, ROW_HEIGHT);
    let mut calibrate = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().button_padding.x = padding;
        let (rect, _) = ui.allocate_exact_size(vec2(label, ROW_HEIGHT), egui::Sense::hover());
        ui.painter().text(
            rect.left_center() + vec2(4.0, 0.0),
            egui::Align2::LEFT_CENTER,
            "Latency",
            FontId::proportional(17.0),
            ui.visuals().text_color(),
        );
        let current = *offset_ms;
        let mut stepped = None;
        let mut step = |ui: &mut Ui, text: &str, by: f32| {
            let to = (current + by).round().clamp(-MAX_INPUT_OFFSET_MS, MAX_INPUT_OFFSET_MS);
            let button =
                Button::new(RichText::new(text).size(22.0)).min_size(vec2(step_width, ROW_HEIGHT));
            if repeat_clicked(&ui.add_enabled(to != current, button)) {
                stepped = Some(to);
            }
        };
        step(ui, "−", -1.0);
        ui.add_sized(
            vec2(number, ROW_HEIGHT),
            egui::Label::new(RichText::new(format!("{current:+.0} ms")).size(18.0).strong()),
        );
        step(ui, "+", 1.0);
        if let Some(to) = stepped {
            *offset_ms = to;
        }
        let width = ui.available_width().max(calibrate_min);
        let button =
            Button::new(RichText::new("Calibrate").size(17.0)).min_size(vec2(width, ROW_HEIGHT));
        calibrate = ui.add(button).clicked();
    });
    calibrate
}

/// Clicked, or held down long enough to step again.
fn repeat_clicked(resp: &egui::Response) -> bool {
    if resp.clicked() {
        return true;
    }
    if !resp.is_pointer_button_down_on() {
        return false;
    }
    let (now, pressed_at, dt) =
        resp.ctx.input(|i| (i.time, i.pointer.press_start_time(), i.stable_dt.min(0.1) as f64));
    resp.ctx.request_repaint();
    let Some(pressed_at) = pressed_at else {
        return false;
    };
    let held = now - pressed_at - REPEAT_DELAY;
    // Fires once per REPEAT_EVERY: when this frame crossed a multiple of it.
    held >= 0.0 && (held / REPEAT_EVERY).floor() != ((held - dt) / REPEAT_EVERY).floor()
}

/// A bar for the microphone level with a mark at the threshold.
fn level_meter(ui: &mut Ui, level_db: f32, threshold_db: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 10.0), egui::Sense::hover());
    let x = |db: f32| {
        let t = ((db - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0);
        egui::lerp(rect.left()..=rect.right(), t)
    };
    let visuals = ui.visuals();
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, visuals.extreme_bg_color);
    let fill = if level_db >= threshold_db {
        visuals.selection.bg_fill
    } else {
        visuals.widgets.inactive.fg_stroke.color
    };
    let level = egui::Rect::from_min_max(rect.min, egui::pos2(x(level_db), rect.max.y));
    painter.rect_filled(level, 3.0, fill);
    let mark = x(threshold_db);
    painter.vline(mark, rect.y_range(), egui::Stroke::new(2.0_f32, visuals.error_fg_color));
}
