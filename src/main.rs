#![allow(dead_code)]

mod duration;
mod fill;
mod measure;
mod rhythm;

use duration::Duration;
use measure::{BeatKind, TimeSignature};
use rhythm::RhythmMeasure;

use eframe::egui::{pos2, Align2, Context, Rangef, Stroke};
use eframe::epaint::text::{FontInsert, InsertFontFamily};
use eframe::epaint::{Color32, FontFamily, FontId};
use eframe::{App, CreationContext, egui};
use egui::containers::Frame;

struct MyApp {
    font_family: FontFamily,
    font_id: FontId,
    measure: RhythmMeasure,
}

fn add_font(ctx: &egui::Context) {
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
        // Example rhythm: empty 7/8 measure (will render as rests)
        let measure = RhythmMeasure::new(TimeSignature::SEVEN_EIGHT);
        Self {
            font_family: ff.clone(),
            font_id: FontId::new(64.0, ff),
            measure,
        }
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

fn rest_glyph_for_duration(d: Duration) -> char {
    match d {
        Duration::Quarter => GLYPH_REST_QUARTER,
        Duration::Eighth | Duration::TripletEighth => GLYPH_REST_EIGHTH,
        Duration::Sixteenth | Duration::QuintupletSixteenth | Duration::SextupletSixteenth | Duration::SeptupletSixteenth => GLYPH_REST_SIXTEENTH,
        Duration::ThirtySecond | Duration::NonupletThirtySecond => GLYPH_REST_32ND,
    }
}

fn draw_measure(ui: &mut egui::Ui, font_id: &FontId, rm: &RhythmMeasure, rect: egui::Rect) {
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

    let m = rm.flatten_to_measure();
    let beats = m.beats();
    if beats.is_empty() { return; }

    let total_ticks = rm.time_signature.measure_duration_ticks() as f32;
    let mut x = left;
    for (i, b) in beats.iter().enumerate() {
        let next_x = if i == beats.len() - 1 { right } else { left + (right - left) * (beats[..=i].iter().map(|bb| bb.duration.ticks() as f32).sum::<f32>() / total_ticks) };
        let mid_x = (x + next_x) * 0.5;
        // Choose glyph
        let glyph = match b.kind {
            BeatKind::Note(_) => GLYPH_NOTEHEAD_BLACK,
            BeatKind::Rest => rest_glyph_for_duration(b.duration),
        };
        // Draw glyph centered at mid_x on the staff line
        painter.text(pos2(mid_x, y), Align2::CENTER_CENTER, glyph.to_string(), font_id.clone(), Color32::WHITE);
        x = next_x;
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

    eframe::run_native(
        "Rustronome",
        options,
        Box::new(|cc| Ok(Box::new(MyApp::new(cc)))),
    )
}
