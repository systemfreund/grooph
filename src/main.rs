#![allow(dead_code)]

mod beaming;
mod duration;
mod fill;
mod measure;

use duration::{Duration, NoteValue};
use measure::{Measure, TimeSignature};

use crate::measure::{Beat, BeatKind};
use eframe::egui::{Align2, Context, Rangef, Stroke, pos2};
use eframe::emath::Pos2;
use eframe::epaint::text::{FontInsert, InsertFontFamily};
use eframe::epaint::{Color32, FontFamily, FontId};
use eframe::{App, CreationContext, egui};
use egui::Rect;
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
    n.to_string().chars().filter_map(|c| c.to_digit(10).map(|d| TS_DIGITS[d as usize])).collect()
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

// Beam-aware note rendering options
struct NoteRenderOpts {
    color: Color32,
    in_beam: bool,
    stem_end_y: Option<f32>, // when Some, draw stem to this y and suppress flags
}

fn draw_beat(
    painter: &egui::Painter,
    font_id: &FontId,
    pos: Pos2,
    beat: Beat,
    opts: NoteRenderOpts,
) {
    let duration = beat.duration;
    let glyph = match beat.kind {
        BeatKind::Note => GLYPH_NOTEHEAD_BLACK,
        BeatKind::Rest => rest_glyph_for_duration(duration),
    };

    // Draw the glyph (notehead or rest)
    painter.text(pos, Align2::CENTER_CENTER, glyph.to_string(), font_id.clone(), opts.color);

    // If this is a Note, draw a stem and possibly flags/tremolo
    if beat.kind == BeatKind::Note {
        // Stem positioning relative to notehead center.
        let stem_offset_x = font_id.size * 0.13; // tweak by eye for Bravura
        let stem_thickness = 2.5;
        let start = pos2(pos.x + stem_offset_x, pos.y);
        let default_stem_len = font_id.size * 0.9; // proportional stem length
        let end_y = if let Some(y) = opts.stem_end_y { y } else { pos.y - default_stem_len };
        let end = pos2(start.x, end_y);
        painter.line_segment([start, end], Stroke::new(stem_thickness, opts.color));

        // Flag glyph at the stem tip for short durations, only if not in a beam
        if !opts.in_beam {
            if let Some(flag) = flag_glyph_for_duration(duration) {
                let fx = end.x + font_id.size * 0.00;
                let fy = end.y + font_id.size * 0.00;
                painter.text(
                    pos2(fx, fy),
                    Align2::LEFT_CENTER,
                    flag.to_string(),
                    font_id.clone(),
                    opts.color,
                );
            }
        }

        // Tremolo slashes (single-note measured tremolo)
        if let Some(trem) = beat.tremolo {
            if trem.measured {
                let sl = trem.slashes.min(3);
                let dx = font_id.size * 0.12; // slight right offset per slash
                let dy = font_id.size * 0.12; // spacing along stem
                let ang = 0.6; // tilt factor (down-right)
                for i in 0..sl {
                    let y0 = end_y + (i as f32) * dy;
                    let x0 = start.x + (i as f32) * dx;
                    let len = font_id.size * 0.45;
                    painter.line_segment(
                        [pos2(x0, y0), pos2(x0 + len, y0 - len * ang)],
                        Stroke::new(2.0, opts.color),
                    );
                }
            }
        }
    }
}

fn draw_measure(ui: &mut egui::Ui, font_id: &FontId, measure: &Measure, rect: Rect) {
    let painter = ui.painter();
    let y = rect.center().y;
    // staff line
    painter.hline(Rangef::new(rect.left(), rect.right()), y, Stroke::new(1.0, Color32::WHITE));

    let y_offset = 60.0;
    let inner_rect =
        Rect::from_min_max(pos2(rect.left(), y - y_offset), pos2(rect.right(), y + y_offset));

    // Derive font size from available height (scaled), keep family from provided font_id
    let target_size = (inner_rect.height() * 0.80).clamp(24.0, 96.0);
    let music_font = FontId::new(target_size, font_id.family.clone());
    let em = target_size;

    // Left-side: percussion clef and stacked time signature
    let clef_w = em * 0.9; // reserved visual width for clef
    let ts_digit_w = em * 0.7; // width per time-signature digit column

    // Draw clef
    let clef_x = inner_rect.left() + clef_w * 0.4;
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
    let ts_left = inner_rect.left() + clef_w - (em * 0.2);

    // Top row (beats)
    for (i, ch) in top_digits.iter().enumerate() {
        // center narrower row within max columns
        let offset = (ts_cols - top_digits.len() as f32) * 0.5;
        let cx = ts_left + (i as f32 + 0.5 + offset) * ts_digit_w;
        painter.text(
            pos2(cx, y - em * 0.40),
            Align2::CENTER_CENTER,
            ch.to_string(),
            music_font.clone(),
            Color32::WHITE,
        );
    }
    // Bottom row (beat unit)
    for (i, ch) in bot_digits.iter().enumerate() {
        let offset = (ts_cols - bot_digits.len() as f32) * 0.5;
        let cx = ts_left + (i as f32 + 0.5 + offset) * ts_digit_w;
        painter.text(
            pos2(cx, y + em * 0.40),
            Align2::CENTER_CENTER,
            ch.to_string(),
            music_font.clone(),
            Color32::WHITE,
        );
    }

    // Content area after clef + time signature
    let content_left = ts_left + ts_w + (em * 0.2);
    let content_right = inner_rect.right();
    let content_w = (content_right - content_left).max(1.0);

    // Compute ticks
    let set = crate::duration::default_duration_set();
    let cap_ticks = ts.measure_duration_ticks();
    let used_ticks: i32 =
        measure.beats().iter().map(|b| set.grid.ticks_of(&b.duration).unwrap_or(0)).sum();

    // 1) Layout: compute x centers for each committed beat proportionally
    let mut x_centers: Vec<f32> = vec![0.0; measure.beats().len()];
    let mut run = 0.0_f32;
    for (i, beat) in measure.beats().iter().copied().enumerate() {
        let t = set.grid.ticks_of(&beat.duration).unwrap_or(0) as f32;
        if cap_ticks > 0 {
            let w = content_w * (t / cap_ticks as f32);
            let cx = content_left + run + w * 0.5;
            x_centers[i] = cx;
            run += w;
        }
    }

    // 2) Metrics for beams and stems
    let metrics = beam_metrics(em, y);
    let stem_dx = music_font.size * 0.13; // keep in sync with draw_beat_with_opts

    // 3) Pass: draw beats (noteheads/rests) with beam-aware stems (flags suppressed when in beam)
    let mut in_beam_flags: Vec<bool> = vec![false; measure.beats().len()];
    if let Some(bp) = measure.beam_plan() {
        for g in &bp.groups {
            for &idx in &g.note_indices {
                if idx < in_beam_flags.len() {
                    in_beam_flags[idx] = true;
                }
            }
        }
    }

    for (i, beat) in measure.beats().iter().copied().enumerate() {
        let in_beam = *in_beam_flags.get(i).unwrap_or(&false);
        let opts = if in_beam {
            NoteRenderOpts {
                color: Color32::WHITE,
                in_beam: true,
                stem_end_y: Some(metrics.beam_y),
            }
        } else {
            NoteRenderOpts { color: Color32::WHITE, in_beam: false, stem_end_y: None }
        };
        if cap_ticks > 0 {
            draw_beat(&painter, &music_font, pos2(x_centers[i], y), beat, opts);
        }
    }

    // 4) Draw beams and hooks per group (horizontal beams for stems up)
    if let Some(bp) = measure.beam_plan() {
        for group in &bp.groups {
            // Full beams between adjacent stems according to continuity
            for (pair_idx, win) in group.note_indices.windows(2).enumerate() {
                let i = win[0];
                let j = win[1];
                let levels = *group.continuity.get(pair_idx).unwrap_or(&0);
                if levels == 0 {
                    continue;
                }
                let x1 = x_centers[i] + stem_dx;
                let x2 = x_centers[j] + stem_dx;
                for lvl in 0..levels {
                    let y_lvl = metrics.beam_y - (lvl as f32) * (metrics.thickness + metrics.gap);
                    draw_full_beam(&painter, x1, x2, y_lvl, metrics.thickness, Color32::WHITE);
                }
            }
            // Hooks (partial beams) for extra levels not continued to neighbors
            for (idx_in_group, &note_i) in group.note_indices.iter().enumerate() {
                let bc = *group.beam_counts.get(idx_in_group).unwrap_or(&0);
                if bc == 0 {
                    continue;
                }
                let x_stem = x_centers[note_i] + stem_dx;
                let cont_left = if idx_in_group == 0 {
                    0
                } else {
                    *group.continuity.get(idx_in_group - 1).unwrap_or(&0)
                };
                let cont_right = if idx_in_group + 1 >= group.note_indices.len() {
                    0
                } else {
                    *group.continuity.get(idx_in_group).unwrap_or(&0)
                };
                // Left hooks extend to the right for levels not continued from the left
                if bc > cont_left {
                    for lvl in cont_left..bc {
                        let y_lvl =
                            metrics.beam_y - (lvl as f32) * (metrics.thickness + metrics.gap);
                        draw_hook(
                            &painter,
                            x_stem,
                            y_lvl,
                            metrics.thickness,
                            metrics.hook_len,
                            1.0,
                            Color32::WHITE,
                        );
                    }
                }
                // Right hooks extend to the left for levels not continued to the right
                if bc > cont_right {
                    for lvl in cont_right..bc {
                        let y_lvl =
                            metrics.beam_y - (lvl as f32) * (metrics.thickness + metrics.gap);
                        draw_hook(
                            &painter,
                            x_stem,
                            y_lvl,
                            metrics.thickness,
                            metrics.hook_len,
                            -1.0,
                            Color32::WHITE,
                        );
                    }
                }
            }
        }
    }

    // 5) Cursor at current used position (does not consume width)
    if cap_ticks > 0 {
        let x_cursor = content_left + content_w * (used_ticks as f32 / cap_ticks as f32);
        painter.vline(
            x_cursor,
            Rangef::new(y - em * 0.55, y + em * 0.55),
            Stroke::new(1.5, Color32::from_white_alpha(180)),
        );
    }

    // 6) Remainder preview as faint rests filling the remaining space, continuing run
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
                draw_beat(
                    &painter,
                    &music_font,
                    pos2(cx, y),
                    beat,
                    NoteRenderOpts { color: ghost, in_beam: false, stem_end_y: None },
                );
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
        viewport: egui::ViewportBuilder::default().with_inner_size([800.0, 250.0]),
        ..Default::default()
    };

    eframe::run_native("grooph.app", options, Box::new(|cc| Ok(Box::new(MyApp::new(cc)))))
}

// Beaming metrics and helpers
#[derive(Copy, Clone)]
struct BeamMetrics {
    thickness: f32,
    gap: f32,
    hook_len: f32,
    beam_y: f32, // primary beam baseline (closest to notehead)
}

fn beam_metrics(em: f32, y_center: f32) -> BeamMetrics {
    // Approximate staff space relative to font size for a single-line staff context
    let staff_space = em * 0.20; // tuned by eye
    let thickness = 0.5 * staff_space; // Bravura ~0.5 sp
    let gap = 0.5 * staff_space; // distance between beams
    let hook_len = 0.6 * em; // horizontal extent of partial beams (hooks)
    let beam_y = y_center - 0.75 * em; // height above notehead center for stems up
    BeamMetrics { thickness, gap, hook_len, beam_y }
}

fn draw_full_beam(p: &egui::Painter, x1: f32, x2: f32, y: f32, thickness: f32, color: Color32) {
    let left = x1.min(x2);
    let right = x1.max(x2);
    let top = y - thickness;
    let rect = Rect::from_min_max(pos2(left, top), pos2(right, y));
    p.rect_filled(rect, 0.0, color);
}

// dir: +1 => hook extends to right; -1 => hook extends to left
fn draw_hook(
    p: &egui::Painter,
    x_stem: f32,
    y: f32,
    thickness: f32,
    len: f32,
    dir: f32,
    color: Color32,
) {
    let x2 = x_stem + dir * len;
    let left = x_stem.min(x2);
    let right = x_stem.max(x2);
    let top = y - thickness;
    let rect = Rect::from_min_max(pos2(left, top), pos2(right, y));
    p.rect_filled(rect, 0.0, color);
}
