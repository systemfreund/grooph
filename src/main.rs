#![allow(dead_code)]

mod beaming;
mod duration;
mod fill;
mod measure;

use duration::{Duration, NoteValue};
use measure::{Measure, TimeSignature};

use crate::duration::NoteValue::*;
use crate::measure::{Beat, BeatKind};
use eframe::egui::{Align2, Context, Key, Rangef, Stroke, pos2};
use eframe::emath::Pos2;
use eframe::epaint::text::{FontInsert, InsertFontFamily};
use eframe::epaint::{Color32, FontFamily, FontId};
use eframe::{App, CreationContext, egui};
use egui::Rect;
use egui::containers::Frame;

struct Grooph {
    font_family: FontFamily,
    font_id: FontId,
    measure: Measure,
    cursor_idx: usize,
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
        Quarter => GLYPH_REST_QUARTER,
        Eighth => GLYPH_REST_EIGHTH,
        Sixteenth => GLYPH_REST_SIXTEENTH,
        ThirtySecond => GLYPH_REST_32ND,
        Half | Whole => GLYPH_REST_QUARTER,
    }
}

fn flag_glyph_for_duration(d: Duration) -> Option<char> {
    match d.base_note() {
        Quarter => None,
        Eighth => Some(GLYPH_FLAG_8TH_UP),
        Sixteenth => Some(GLYPH_FLAG_16TH_UP),
        ThirtySecond => Some(GLYPH_FLAG_32ND_UP),
        Half | Whole => None,
    }
}

// Beam-aware note rendering options
struct NoteRenderOpts {
    color: Color32,
    in_beam: bool,
    stem_offset_x: f32,
    stem_thickness: f32,
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

    // Render rests a bit smaller than notes
    let font_id = if beat.kind == BeatKind::Rest {
        &FontId::new(font_id.size * 0.7, font_id.family.clone())
    } else {
        font_id
    };

    // Draw the glyph (notehead or rest)
    painter.text(pos, Align2::CENTER_CENTER, glyph.to_string(), font_id.clone(), opts.color);

    // If this is a Note, draw a stem and possibly flags/tremolo
    if beat.kind == BeatKind::Note {
        let start = pos2(pos.x + opts.stem_offset_x, pos.y);
        let flag_glyph = flag_glyph_for_duration(duration);
        // It's visually more appealing to reduce the stem length a bit for notes that are neither
        // in a beam nor flagged.
        let stem_len_factor = if opts.in_beam || flag_glyph.is_some() { 1.0 } else { 0.85 };
        let default_stem_len = get_default_stem_length(font_id) * stem_len_factor;
        let end = pos2(start.x, pos.y - default_stem_len);
        painter.line_segment([start, end], Stroke::new(opts.stem_thickness, opts.color));

        // Flag glyph at the stem tip for short durations, only if not in a beam
        if !opts.in_beam {
            if let Some(flag) = flag_glyph {
                painter.text(
                    pos2(
                        start.x - opts.stem_thickness * 0.5,
                        pos.y - get_default_stem_length(font_id),
                    ),
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
                    let y0 = (pos.y - default_stem_len) + (i as f32) * dy;
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

fn draw_measure(
    ui: &mut egui::Ui,
    font_id: &FontId,
    measure: &Measure,
    rect: Rect,
    cursor_idx: Option<usize>,
) {
    const COLOR: Color32 = Color32::WHITE;
    let painter = ui.painter();
    let y = rect.center().y;
    // staff line
    painter.hline(Rangef::new(rect.left(), rect.right()), y, Stroke::new(1.0, COLOR));

    let min_size = 14.0 * ui.ctx().pixels_per_point(); // avoid unreadably small glyphs on HiDPI

    // Make inner rect scale with available height: keep a small vertical padding fraction
    let vpad = (rect.height() * 0.10).clamp(10.0, 200.0);
    let hpad = (rect.width() * 0.10).clamp(80.0, 120.0);
    let inner_rect = Rect::from_min_max(
        pos2(rect.left(), rect.top() + vpad),
        pos2(rect.right() - hpad, rect.bottom() - vpad),
    );

    // Derive font size from available height and width (scaled), keep family from provided font_id
    // Height-first sizing, modulated by window width so very narrow/wide windows adapt.
    let base_size_h = inner_rect.height() * 0.50;
    // Also cap by an estimate from width to prevent overflow on very narrow windows.
    let width_cap = (rect.width() * 0.1).max(min_size);
    let max_size = (inner_rect.height() * 0.80).max(min_size); // avoid overflowing inner rect, but not a fixed cap. also make sure it does not go below min_size
    let target_size = base_size_h.clamp(min_size, max_size.min(width_cap));
    let font_id = FontId::new(target_size, font_id.family.clone());
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
        font_id.clone(),
        COLOR,
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
            pos2(cx, y - em * 0.25),
            Align2::CENTER_CENTER,
            ch.to_string(),
            font_id.clone(),
            COLOR,
        );
    }
    // Bottom row (beat unit)
    for (i, ch) in bot_digits.iter().enumerate() {
        let offset = (ts_cols - bot_digits.len() as f32) * 0.5;
        let cx = ts_left + (i as f32 + 0.5 + offset) * ts_digit_w;
        painter.text(
            pos2(cx, y + em * 0.25),
            Align2::CENTER_CENTER,
            ch.to_string(),
            font_id.clone(),
            COLOR,
        );
    }

    // Content area after clef + time signature
    let content_left = ts_left + ts_w + (em * 0.2);
    let content_right = inner_rect.right();
    let content_w = (content_right - content_left).max(1.0);

    // Compute ticks
    let set = duration::default_duration_set();
    let cap_ticks = ts.measure_duration_ticks();
    let used_ticks: i32 =
        measure.beats().iter().map(|b| set.grid.ticks_of(&b.duration).unwrap_or(0)).sum();

    // Precompute remainder preview durations (virtual rests) for caret/navigation too
    let remaining = cap_ticks - used_ticks;
    let remainder_durs: Vec<Duration> = if remaining > 0 {
        fill::best_fill_for_gap(remaining).unwrap_or_default()
    } else {
        Vec::new()
    };

    // 1) Layout: compute x centers for each committed beat proportionally, and cache per-beat slot left/width
    let mut x_centers: Vec<f32> = vec![0.0; measure.beats().len() + remainder_durs.len()];
    let mut run = 0.0_f32;
    let mut durations: Vec<_> = measure.beats().iter().map(|b| b.duration).collect();
    durations.extend(remainder_durs.iter().copied());

    for (i, duration) in durations.iter().enumerate() {
        let t = set.grid.ticks_of(duration).unwrap_or(0) as f32;
        if cap_ticks > 0 {
            let w = content_w * (t / cap_ticks as f32);
            let cx = content_left + run + w * 0.5;
            x_centers[i] = cx;
            run += w;
        }
    }

    // 2) Metrics for beams and stems
    let beam_render_opts = bream_render_opts(em, y, COLOR, &font_id);
    let stem_dx = font_id.size * 0.13; // keep in sync with draw_beat
    // Precompute stem x positions for all beats (noteheads + stem offset)
    let stem_xs: Vec<f32> = x_centers.iter().map(|&cx| cx + stem_dx).collect();
    // Stem positioning relative to notehead center.
    let stem_offset_x = font_id.size * 0.13; // tweak by eye for Bravura
    let stem_thickness = font_id.size * 0.03;

    // 3) Pass: draw beats (noteheads/rests) with beam-aware stems (flags suppressed when in beam)
    let mut in_beam_flags: Vec<bool> = vec![false; measure.beats().len()];
    if let Some(bp) = measure.beam_plan() {
        for g in &bp.groups {
            // Only consider groups with at least two notes as "beamed".
            // Singleton groups should render with flags, not beams.
            if g.note_indices.len() >= 2 {
                for &idx in &g.note_indices {
                    if idx < in_beam_flags.len() {
                        in_beam_flags[idx] = true;
                    }
                }
            }
        }
    }

    for (i, beat) in measure.beats().iter().copied().enumerate() {
        let in_beam = *in_beam_flags.get(i).unwrap_or(&false);
        let opts = NoteRenderOpts { color: COLOR, in_beam, stem_offset_x, stem_thickness };
        if cap_ticks > 0 {
            draw_beat(&painter, &font_id, pos2(x_centers[i], y), beat, opts);
        }
    }

    // 4) Draw beams per group (horizontal beams for stems up)
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
                let stem_x_offset = stem_thickness / 3.0;
                let x1 = stem_xs[i] - stem_x_offset;
                let x2 = stem_xs[j] + stem_x_offset;
                for lvl in 0..levels {
                    draw_full_beam(&painter, x1, x2, lvl, &beam_render_opts);
                }
            }
        }
    }

    // 4b) Draw broken (partial) beams where a note's beam count exceeds continuity
    if let Some(bp) = measure.beam_plan() {
        let stub_len = em * 0.20; // tune by eye
        for group in &bp.groups {
            if group.note_indices.is_empty() {
                continue;
            }

            let note_idxs = &group.note_indices;
            // Singleton notes should show flags only — no partial beam stubs.
            if note_idxs.len() < 2 {
                continue;
            }
            let counts = &group.beam_counts; // per note
            let cont = &group.continuity; // between neighbors

            for (local_k, &global_i) in note_idxs.iter().enumerate() {
                let count = *counts.get(local_k).unwrap_or(&0);
                if count <= 0 {
                    continue;
                }

                let left_cont = if local_k > 0 { *cont.get(local_k - 1).unwrap_or(&0) } else { 0 };
                let right_cont = if local_k + 1 < note_idxs.len() {
                    *cont.get(local_k).unwrap_or(&0)
                } else {
                    0
                };

                let stem_x = stem_xs[global_i];
                let is_first = local_k == 0;
                let is_last = local_k + 1 == note_idxs.len();

                for lvl in 0..count {
                    let connects_left = lvl < left_cont;
                    let connects_right = lvl < right_cont;

                    match (connects_left, connects_right) {
                        (true, true) => { /* fully connected at this level */ }
                        (true, false) => {
                            // Connected to left neighbor, missing to right
                            // Interior notes: no right stub; full beam terminates cleanly at this stem.
                            // Last note is an outer edge; we also suppress stubs there by policy.
                            // => do nothing
                        }
                        (false, true) => {
                            // Connected to right neighbor, missing to left
                            // Interior notes: no left stub; full beam terminates at this stem.
                            // First note is an outer edge; stubs suppressed by policy.
                            // => do nothing
                        }
                        (false, false) => {
                            // Not connected on either side at this level.
                            // On group edges, draw only the interior-facing stub; on interior notes, choose side by higher continuity (or both if equal).
                            if is_first {
                                // First note: interior faces right
                                draw_full_beam(
                                    &painter,
                                    stem_x,
                                    stem_x + stub_len,
                                    lvl,
                                    &beam_render_opts,
                                );
                            } else if is_last {
                                // Last note: interior faces left
                                draw_full_beam(
                                    &painter,
                                    stem_x - stub_len,
                                    stem_x,
                                    lvl,
                                    &beam_render_opts,
                                );
                            } else {
                                if left_cont > right_cont {
                                    draw_full_beam(
                                        &painter,
                                        stem_x - stub_len,
                                        stem_x,
                                        lvl,
                                        &beam_render_opts,
                                    );
                                } else if right_cont > left_cont {
                                    draw_full_beam(
                                        &painter,
                                        stem_x,
                                        stem_x + stub_len,
                                        lvl,
                                        &beam_render_opts,
                                    );
                                } else {
                                    // Equal continuity: draw both short stubs
                                    draw_full_beam(
                                        &painter,
                                        stem_x - stub_len,
                                        stem_x,
                                        lvl,
                                        &beam_render_opts,
                                    );
                                    draw_full_beam(
                                        &painter,
                                        stem_x,
                                        stem_x + stub_len,
                                        lvl,
                                        &beam_render_opts,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 5) Cursor at current beat index (does not consume width) — blink over time
    if let Some(idx) = cursor_idx {
        if let Some(&x) = x_centers.get(idx) {
            // Blink parameters
            let blink_period = 1.0_f64; // seconds for a full on+off cycle
            let duty = 0.5_f64; // visible fraction of the period
            let t = ui.input(|i| i.time);
            let phase = (t % blink_period) / blink_period; // 0..1
            let visible = phase < duty;
            // Smooth fade near edges optional; for now a simple square wave with two alpha levels
            let alpha_on = 220u8;
            let alpha_off = 40u8; // faint but still present; set to 0 to hide completely
            let alpha = if visible { alpha_on } else { alpha_off };
            let top = inner_rect.top();
            let bottom = inner_rect.bottom();
            painter.vline(
                x,
                Rangef::new(top, bottom),
                Stroke::new(2.0, Color32::from_white_alpha(alpha)),
            );
            // Ensure animation progresses even without input
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    // 6) Remainder preview as faint rests filling the remaining space, continuing run
    if !remainder_durs.is_empty() && cap_ticks > 0 {
        let virt = Color32::from_white_alpha(100);
        // We need a fresh run to draw virtuals, keeping continuity after real beats
        let mut run_draw = 0.0_f32;
        for (_i, beat) in measure.beats().iter().copied().enumerate() {
            let t = set.grid.ticks_of(&beat.duration).unwrap_or(0) as f32;
            let w = content_w * (t / cap_ticks as f32);
            run_draw += w;
        }
        for d in remainder_durs {
            let beat = Beat::rest(d);
            let t = set.grid.ticks_of(&beat.duration).unwrap_or(0) as f32;
            let w = content_w * (t / cap_ticks as f32);
            let cx = content_left + run_draw + w * 0.5;
            draw_beat(
                &painter,
                &font_id,
                pos2(cx, y),
                beat,
                NoteRenderOpts { color: virt, in_beam: false, stem_offset_x, stem_thickness },
            );
            run_draw += w;
        }
    }
}

impl App for Grooph {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            Frame::canvas(ui.style()).show(ui, |ui| {
                let (_id, rect) = ui.allocate_space(ui.available_size());
                ui.input(|i| {
                    let beats_len = self.measure.beats().len();
                    let rem_ticks = self.measure.remaining_ticks();
                    let virtual_len = if rem_ticks > 0 {
                        fill::best_fill_for_gap(rem_ticks).map(|v| v.len()).unwrap_or(0)
                    } else {
                        0
                    };
                    let total_len = beats_len + virtual_len;
                    if total_len > 0 {
                        if i.key_pressed(Key::ArrowLeft) {
                            self.cursor_idx = self.cursor_idx.saturating_sub(1);
                        }
                        if i.key_pressed(Key::ArrowRight) {
                            let max_idx = total_len.saturating_sub(1);
                            if self.cursor_idx < max_idx {
                                self.cursor_idx += 1;
                            }
                        }
                        if i.key_pressed(Key::Home) {
                            self.cursor_idx = 0;
                        }
                        if i.key_pressed(Key::End) {
                            self.cursor_idx = total_len.saturating_sub(1);
                        }
                        if i.key_pressed(Key::Delete) {
                            // Replace note with rest at cursor; commit 'virtual' if needed
                            let idx = self.cursor_idx;
                            self.measure.ensure_committed_position(idx);
                            self.measure.set_beat_to_rest(idx);
                        }
                        if i.key_pressed(Key::Space) {
                            // Toggle between note and rest at cursor (preserve duration); commit 'virtual' if needed
                            let idx = self.cursor_idx;
                            self.measure.ensure_committed_position(idx);
                            self.measure.toggle_beat_kind(idx);
                        }
                        if i.key_pressed(Key::Backspace) {
                            // Remove beat at cursor; commit 'virtual' if needed then remove
                            let idx = self.cursor_idx;
                            self.measure.ensure_committed_position(idx);
                            self.measure.backspace_remove_and_fill(idx);
                            // Move cursor left, like a text editor caret
                            self.cursor_idx = self.cursor_idx.saturating_sub(1);
                        }
                    }
                });
                let idx_opt = Some(self.cursor_idx);
                draw_measure(ui, &self.font_id, &self.measure, rect, idx_opt);
            });
        });
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([800.0, 250.0]),
        ..Default::default()
    };

    eframe::run_native("grooph.app", options, Box::new(|cc| Ok(Box::new(Grooph::new(cc)))))
}

// Beaming metrics and helpers
#[derive(Copy, Clone)]
struct BeamRenderOpts {
    thickness: f32,
    gap: f32,
    beam_y: f32, // primary beam baseline (closest to notehead)
    color: Color32,
}

impl BeamRenderOpts {
    fn get_y_level(&self, lvl: u8) -> f32 {
        self.beam_y + (lvl as f32) * (self.thickness + self.gap)
    }
}

fn bream_render_opts(em: f32, y_center: f32, color: Color32, font_id: &FontId) -> BeamRenderOpts {
    // Approximate staff space relative to font size for a single-line staff context
    let staff_space = em * 0.25; // tuned by eye
    let thickness = 0.5 * staff_space; // Bravura ~0.5 sp
    let gap = 0.25 * staff_space; // distance between beams
    let beam_y = y_center - get_default_stem_length(font_id) + (thickness * 0.95);
    BeamRenderOpts { thickness, gap, beam_y, color }
}

fn draw_full_beam(p: &egui::Painter, x1: f32, x2: f32, lvl: u8, beam_opts: &BeamRenderOpts) {
    let left = x1.min(x2);
    let right = x1.max(x2);
    let y = beam_opts.get_y_level(lvl);
    let top = y - beam_opts.thickness;
    let rect = Rect::from_min_max(pos2(left, top), pos2(right, y));
    p.rect_filled(rect, 0.0, beam_opts.color);
}

fn get_default_stem_length(font_id: &FontId) -> f32 {
    font_id.size * 0.9 // proportional stem length
}

impl Grooph {
    fn new(cc: &CreationContext) -> Self {
        add_font(&cc.egui_ctx);
        let ff = FontFamily::Name("music".into());
        let mut measure = Measure::new(TimeSignature::SEVEN_EIGHT);
        let t8 = Duration::Tuplet { n: 3, m: 2, base: Eighth };
        let t16 = Duration::Tuplet { n: 6, m: 4, base: NoteValue::Sixteenth };
        measure.add_beat(Beat::rest(Duration::Simple(Sixteenth))).unwrap();
        measure.add_beat(Beat::note(Duration::Simple(Sixteenth))).unwrap();
        measure.add_beat(Beat::note(Duration::Simple(ThirtySecond))).unwrap();
        measure.add_beat(Beat::note(Duration::Simple(Eighth))).unwrap();
        measure.add_beat(Beat::note(Duration::Simple(Quarter))).unwrap();

        Self { font_family: ff.clone(), font_id: FontId::new(16.0, ff), measure, cursor_idx: 0 }
    }
}
