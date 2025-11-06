#![allow(dead_code)]

mod duration;
mod fill;
mod measure;
mod rhythm;

use duration::{Duration, NoteValue};
use measure::TimeSignature;
use rhythm::{RhythmMeasure, RhythmNode, SlotContent};

use crate::fill::best_fill_for_gap;
use eframe::egui::{Align2, Context, Id, Rangef, Sense, Stroke, pos2};
use eframe::emath::Pos2;
use eframe::epaint::text::{FontInsert, InsertFontFamily};
use eframe::epaint::{Color32, FontFamily, FontId, StrokeKind};
use eframe::{App, CreationContext, egui};
use egui::containers::Frame;

struct MyApp {
    font_family: FontFamily,
    font_id: FontId,
    measure: RhythmMeasure,
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
        let mut measure = RhythmMeasure::new(TimeSignature::SEVEN_EIGHT);
        println!("{:?}", measure);
        // println!("{}", measure.subdivide(&[], 4, SlotContent::Note));
        measure.flatten_to_measure().map(|m| println!("{}", m));

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

// Up-stem flags (SMuFL): U+E240..U+E242
const GLYPH_FLAG_8TH_UP: char = '\u{E240}';
const GLYPH_FLAG_16TH_UP: char = '\u{E242}';
const GLYPH_FLAG_32ND_UP: char = '\u{E244}';

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

// Helper: choose rest glyph by duration already defined above

#[derive(Clone, Debug)]
struct SlotBox {
    rect: egui::Rect,
    span_ticks: i32,
    content: SlotContent,
    path: Vec<usize>,
}

fn layout_rhythm_boxes(
    node: &RhythmNode,
    span_ticks: i32,
    rect: egui::Rect,
    path: &mut Vec<usize>,
    out: &mut Vec<SlotBox>,
) {
    match node {
        RhythmNode::Leaf(content) => {
            out.push(SlotBox {
                rect,
                span_ticks,
                content: *content,
                path: path.clone(),
            });
        }
        RhythmNode::Group { n, children } => {
            let n_i = *n as i32;
            let slot_ticks = span_ticks / n_i;
            let child_w = rect.width() / (*n as f32);
            for (i, child) in children.iter().enumerate() {
                let left = rect.left() + child_w * (i as f32);
                let child_rect = egui::Rect::from_min_max(
                    pos2(left, rect.top()),
                    pos2(left + child_w, rect.bottom()),
                );
                path.push(i);
                layout_rhythm_boxes(child, slot_ticks, child_rect, path, out);
                path.pop();
            }
        }
    }
}

fn draw_slot_overlays(
    ui: &mut egui::Ui,
    font_id: &FontId,
    rm: &mut RhythmMeasure,
    inner_rect: egui::Rect,
) {
    let painter = ui.painter();

    let mut boxes = Vec::new();
    let total_ticks = rm.time_signature.measure_duration_ticks();
    let mut path: Vec<usize> = Vec::new();
    layout_rhythm_boxes(&rm.root, total_ticks, inner_rect, &mut path, &mut boxes);

    let border = Stroke::new(1.0, Color32::from_gray(170));
    let fill_a = Color32::from_rgba_unmultiplied(80, 160, 255, 40);
    let fill_b = Color32::from_rgba_unmultiplied(80, 255, 160, 24);

    for (idx, sb) in boxes.iter().enumerate() {
        // Interactivity: toggle on click
        let id = Id::new(("slot_box", &sb.path));
        let resp = ui.interact(sb.rect, id, Sense::click());
        if resp.clicked() {
            // Toggle between Rest and Note at this slot path
            rm.toggle_leaf(&sb.path);
        }

        let fill = if idx % 2 == 0 { fill_a } else { fill_b };
        painter.rect_filled(sb.rect, 3.0, fill);
        painter.rect_stroke(sb.rect, 3.0, border, StrokeKind::Inside);

        // Within each slot, draw the local minimal spelling.
        if let Some(seq) = best_fill_for_gap(sb.span_ticks) {
            let mut x = sb.rect.left();
            let width = sb.rect.width();
            let mut acc_ticks = 0.0_f32;
            let total = sb.span_ticks as f32;

            for (j, d) in seq.iter().enumerate() {
                // Use default dynamic grid to compute tick proportions
                let grid = duration::default_grid();
                acc_ticks += grid.ticks_of(d).unwrap() as f32;
                let next_x = if j == seq.len() - 1 {
                    sb.rect.right()
                } else {
                    sb.rect.left() + width * (acc_ticks / total)
                };
                let mid = pos2(0.5 * (x + next_x), 0.5 * (sb.rect.top() + sb.rect.bottom()));
                draw_note(painter, font_id, mid, *d, sb.content);
                x = next_x;
            }
        }
    }
}

fn draw_note(
    painter: &egui::Painter,
    font_id: &FontId,
    pos: Pos2,
    duration: Duration,
    slot_content: SlotContent,
) {
    let glyph = match slot_content {
        SlotContent::Note => GLYPH_NOTEHEAD_BLACK,
        SlotContent::Rest => rest_glyph_for_duration(duration),
    };

    // Draw the glyph (notehead or rest)
    painter.text(
        pos,
        Align2::CENTER_CENTER,
        glyph.to_string(),
        font_id.clone(),
        Color32::WHITE,
    );

    // If this is a Note, draw a simple upward stem next to the notehead,
    // and add a flag according to the duration (8th=1, 16th=2, 32nd=3; tuplets map similarly).
    if slot_content == SlotContent::Note {
        // Stem positioning relative to notehead center.
        let stem_offset_x = font_id.size * 0.13; // tweak by eye for Bravura
        let stem_len = font_id.size * 0.9; // proportional stem length
        let stem_thickness = 2.5;
        let start = pos2(pos.x + stem_offset_x, pos.y);
        let end = pos2(start.x, pos.y - stem_len);
        painter.line_segment([start, end], Stroke::new(stem_thickness, Color32::WHITE));

        // Flag glyph at the stem tip for short durations
        if let Some(flag) = flag_glyph_for_duration(duration) {
            let fx = end.x + font_id.size * 0.00;
            let fy = end.y + font_id.size * 0.00;
            painter.text(
                pos2(fx, fy),
                Align2::LEFT_CENTER,
                flag.to_string(),
                font_id.clone(),
                Color32::WHITE,
            );
        }
    }
}

fn draw_measure(ui: &mut egui::Ui, font_id: &FontId, rm: &mut RhythmMeasure, rect: egui::Rect) {
    let painter = ui.painter();
    let y = rect.center().y;
    // staff line
    painter.hline(
        Rangef::new(rect.left(), rect.right()),
        y,
        Stroke::new(1.0, Color32::WHITE),
    );

    // barlines
    let bar_stroke = Stroke::new(2.0, Color32::WHITE);
    painter.vline(
        rect.left() + 16.0,
        Rangef::new(y - 24.0, y + 24.0),
        bar_stroke,
    );
    painter.vline(
        rect.right() - 16.0,
        Rangef::new(y - 24.0, y + 24.0),
        bar_stroke,
    );

    // layout area inside barlines
    let left = rect.left() + 24.0;
    let right = rect.right() - 24.0;
    let inner_rect = egui::Rect::from_min_max(pos2(left, y - 36.0), pos2(right, y + 36.0));

    // Draw semi-transparent slot overlays containing local spelling
    draw_slot_overlays(ui, font_id, rm, inner_rect);
}

impl App for MyApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            Frame::canvas(ui.style()).show(ui, |ui| {
                let (_id, rect) = ui.allocate_space(ui.available_size());
                draw_measure(ui, &self.font_id, &mut self.measure, rect);
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
        "grooph.app",
        options,
        Box::new(|cc| Ok(Box::new(MyApp::new(cc)))),
    )
}
