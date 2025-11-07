#![allow(dead_code)]

mod duration;
mod fill;
mod measure;
mod beaming;

use duration::{Duration, NoteValue};
use measure::{Measure, TimeSignature};

use crate::measure::{Beat, BeatKind};
use eframe::egui::{Align2, Context, Rangef, Stroke, pos2};
use eframe::emath::Pos2;
use eframe::epaint::text::{FontInsert, InsertFontFamily};
use eframe::epaint::{Color32, FontFamily, FontId};
use eframe::{App, CreationContext, egui};
use egui::containers::Frame;

struct MyApp {
    font_family: FontFamily,
    font_id: FontId,
    measure: Measure,
}

fn add_font(ctx: &Context) {
    ctx.add_font(FontInsert::new(
        "Bravura",
        egui::FontData::from_static(include_bytes!("/usr/share/fonts/OTF/Bravura.otf")),
        vec![InsertFontFamily {
            family: FontFamily::Name("music".into()),
            priority: egui::epaint::text::FontPriority::Highest,
        }],
    ));
}

impl MyApp {
    fn new(cc: &CreationContext) -> Self {
        add_font(&cc.egui_ctx);
        let ff = FontFamily::Name("music".into());
        let mut measure = Measure::new(TimeSignature::SEVEN_EIGHT);
        let t8 = Duration::Tuplet { n: 3, m: 2, base: NoteValue::Eighth };
        measure.add_beat(Beat::note(t8)).unwrap();
        measure.add_beat(Beat::note(t8)).unwrap();
        measure.add_beat(Beat::note(t8)).unwrap();
        // measure.add_beat(Beat::note(Duration::Simple(NoteValue::Eighth))).unwrap();
        Self { font_family: ff.clone(), font_id: FontId::new(64.0, ff), measure }
    }
}

// SMuFL glyphs (Bravura)
// Notehead black: U+E0A4
const GLYPH_NOTEHEAD_BLACK: char = '\u{E0A4}';
// Rests: quarter..32nd: U+E4E5..U+E4E8
const GLYPH_REST_QUARTER: char = '\u{E4E5}';
const GLYPH_REST_EIGHTH: char = '\u{E4E6}';
const GLYPH_REST_SIXTEENTH: char = '\u{E4E7}';
const GLYPH_REST_32ND: char = '\u{E4E8}';

// Up-stem flags (SMuFL): U+E240..U+E244
const GLYPH_FLAG_8TH_UP: char = '\u{E240}';
const GLYPH_FLAG_16TH_UP: char = '\u{E242}';
const GLYPH_FLAG_32ND_UP: char = '\u{E244}';

// Clef and time signature digits
const GLYPH_CLEF_PERCUSSION: char = '\u{E069}';
const TS_DIGITS: [char; 10] = [
    '\u{E080}', // 0
    '\u{E081}', // 1
    '\u{E082}', // 2
    '\u{E083}', // 3
    '\u{E084}', // 4
    '\u{E085}', // 5
    '\u{E086}', // 6
    '\u{E087}', // 7
    '\u{E088}', // 8
    '\u{E089}', // 9
];

fn ts_glyphs(n: u32) -> Vec<char> {
    n.to_string()
        .chars()
        .filter_map(|c| c.to_digit(10).map(|d| TS_DIGITS[d as usize]))
        .collect()
}

fn rest_glyph_for_duration(d: Duration) -> char {
    match d.base_note() {
        NoteValue::Quarter => GLYPH_REST_QUARTER,
        NoteValue::Eighth => GLYPH_REST_EIGHTH,
        NoteValue::Sixteenth => GLYPH_REST_SIXTEENTH,
        NoteValue::ThirtySecond => GLYPH_REST_32ND,
        NoteValue::Half | NoteValue::Whole => GLYPH_REST_QUARTER,
    }
}

fn flag_glyph_for_duration(d: Duration) -> Option<char> {
    match d.base_note() {
        NoteValue::Quarter => None,
        NoteValue::Eighth => Some(GLYPH_FLAG_8TH_UP),
        NoteValue::Sixteenth => Some(GLYPH_FLAG_16TH_UP),
        NoteValue::ThirtySecond => Some(GLYPH_FLAG_32ND_UP),
        NoteValue::Half | NoteValue::Whole => None,
    }
}

fn draw_beat(painter: &egui::Painter, font_id: &FontId, pos: Pos2, beat: Beat, color: Color32) {
    let duration = beat.duration;
    let glyph = match beat.kind {
        BeatKind::Note => GLYPH_NOTEHEAD_BLACK,
        BeatKind::Rest => rest_glyph_for_duration(duration),
    };

    // Draw the glyph (notehead or rest)
    painter.text(pos, Align2::CENTER_CENTER, glyph.to_string(), font_id.clone(), color);

    // If this is a Note, draw a simple upward stem next to the notehead,
    // and add a flag according to the duration (8th=1, 16th=2, 32nd=3; tuplets map similarly).
    if beat.kind == BeatKind::Note {
        // Stem positioning relative to notehead center.
        let stem_offset_x = font_id.size * 0.13; // tweak by eye for Bravura
        let stem_len = font_id.size * 0.9; // proportional stem length
        let stem_thickness = 2.5;
        let start = pos2(pos.x + stem_offset_x, pos.y);
        let end = pos2(start.x, pos.y - stem_len);
        painter.line_segment([start, end], Stroke::new(stem_thickness, color));

        // Flag glyph at the stem tip for short durations
        if let Some(flag) = flag_glyph_for_duration(duration) {
            let fx = end.x + font_id.size * 0.00;
            let fy = end.y + font_id.size * 0.00;
            painter.text(
                pos2(fx, fy),
                Align2::LEFT_CENTER,
                flag.to_string(),
                font_id.clone(),
                color,
            );
        }
    }
}

fn draw_measure(ui: &mut egui::Ui, font_id: &FontId, measure: &Measure, rect: egui::Rect) {
    let painter = ui.painter();
    let y = rect.center().y;
    // staff line
    painter.hline(Rangef::new(rect.left(), rect.right()), y, Stroke::new(1.0, Color32::WHITE));

    // barlines
    let bar_stroke = Stroke::new(2.0, Color32::WHITE);
    painter.vline(rect.left() + 16.0, Rangef::new(y - 24.0, y + 24.0), bar_stroke);
    painter.vline(rect.right() - 16.0, Rangef::new(y - 24.0, y + 24.0), bar_stroke);

    // layout area inside barlines
    let left = rect.left() + 24.0;
    let right = rect.right() - 24.0;
    let inner_rect = egui::Rect::from_min_max(pos2(left, y - 36.0), pos2(right, y + 36.0));

    // Derive font size from available height (scaled), keep family from provided font_id
    let inner_h = inner_rect.height();
    let target_size = (inner_h * 0.65).clamp(24.0, 96.0);
    let music_font = FontId::new(target_size, font_id.family.clone());
    let em = target_size;

    // Left-side: percussion clef and stacked time signature
    let clef_w = em * 0.9;      // reserved visual width for clef
    let ts_digit_w = em * 0.7;  // width per time-signature digit column
    let gap_w = em * 0.3;       // gap between blocks

    // Draw clef
    let clef_x = inner_rect.left() + clef_w * 0.5;
    painter.text(
        pos2(clef_x, y),
        Align2::CENTER_CENTER,
        GLYPH_CLEF_PERCUSSION.to_string(),
        music_font.clone(),
        Color32::WHITE,
    );

    // Time signature digits (SMuFL)
    let ts = measure.time_signature();
    let top_digits = ts_glyphs(ts.beats as u32);
    let bot_digits = ts_glyphs(ts.beat_unit as u32);

    let ts_cols = top_digits.len().max(bot_digits.len()) as f32;
    let ts_w = ts_cols * ts_digit_w;
    let ts_left = inner_rect.left() + clef_w + gap_w;

    // Top row (beats)
    for (i, ch) in top_digits.iter().enumerate() {
        // center narrower row within max columns
        let offset = (ts_cols - top_digits.len() as f32) * 0.5;
        let cx = ts_left + (i as f32 + 0.5 + offset) * ts_digit_w;
        painter.text(pos2(cx, y - em * 0.40), Align2::CENTER_CENTER, ch.to_string(), music_font.clone(), Color32::WHITE);
    }
    // Bottom row (beat unit)
    for (i, ch) in bot_digits.iter().enumerate() {
        let offset = (ts_cols - bot_digits.len() as f32) * 0.5;
        let cx = ts_left + (i as f32 + 0.5 + offset) * ts_digit_w;
        painter.text(pos2(cx, y + em * 0.40), Align2::CENTER_CENTER, ch.to_string(), music_font.clone(), Color32::WHITE);
    }

    // Content area after clef + time signature
    let content_left = ts_left + ts_w + gap_w;
    let content_right = inner_rect.right();
    let content_w = (content_right - content_left).max(1.0);

    // Compute ticks
    let set = crate::duration::default_duration_set();
    let cap_ticks = ts.measure_duration_ticks();
    let used_ticks: i32 = measure
        .beats()
        .iter()
        .map(|b| set.grid.ticks_of(&b.duration).unwrap_or(0))
        .sum();

    // Lay out existing beats proportionally
    let mut run = 0.0_f32;
    for beat in measure.beats().iter().copied() {
        let t = set.grid.ticks_of(&beat.duration).unwrap_or(0) as f32;
        if cap_ticks > 0 {
            let w = content_w * (t / cap_ticks as f32);
            let cx = content_left + run + w * 0.5;
            draw_beat(&painter, &music_font, pos2(cx, y), beat, Color32::WHITE);
            run += w;
        }
    }

    // Cursor at current used position (does not consume width)
    if cap_ticks > 0 {
        let x_cursor = content_left + content_w * (used_ticks as f32 / cap_ticks as f32);
        painter.vline(
            x_cursor,
            Rangef::new(y - em * 0.55, y + em * 0.55),
            Stroke::new(1.5, Color32::from_white_alpha(180)),
        );
    }

    // Remainder preview as faint rests filling the remaining space
    let remaining = cap_ticks - used_ticks;
    if remaining > 0 {
        let remainder_durs = crate::fill::best_fill_for_gap(remaining).unwrap_or_default();
        let ghost = Color32::from_white_alpha(100);
        for d in remainder_durs {
            let beat = Beat::rest(d);
            let t = set.grid.ticks_of(&beat.duration).unwrap_or(0) as f32;
            if cap_ticks > 0 {
                let w = content_w * (t / cap_ticks as f32);
                let cx = content_left + run + w * 0.5;
                draw_beat(&painter, &music_font, pos2(cx, y), beat, ghost);
                run += w;
            }
        }
    }
}

impl App for MyApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            Frame::canvas(ui.style()).show(ui, |ui| {
                let (_id, rect) = ui.allocate_space(ui.available_size());
                draw_measure(ui, &self.font_id, &self.measure, rect);
            });
        });
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([640.0, 200.0]),
        ..Default::default()
    };

    eframe::run_native("grooph.app", options, Box::new(|cc| Ok(Box::new(MyApp::new(cc)))))
}
