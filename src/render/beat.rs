use crate::measure::duration::Duration;
use crate::measure::{Beat, BeatKind};
use crate::render::glyphs;
use eframe::egui;
use eframe::egui::{Align2, Color32, FontId, Pos2, Stroke, pos2};

// Beam-aware note rendering options
pub(crate) struct NoteRenderOpts {
    pub font_id: FontId,
    pub color: Color32,
    pub in_beam: bool,
    pub stem_dx: f32,
    pub stem_thickness: f32,
}

pub(crate) fn get_default_stem_length(font_id: &FontId) -> f32 {
    font_id.size * 0.9 // proportional stem length
}

pub(crate) fn draw_beat(painter: &egui::Painter, pos: Pos2, beat: Beat, opts: NoteRenderOpts) {
    let duration = beat.duration;
    let glyph = match beat.kind {
        BeatKind::Note => glyphs::GLYPH_NOTEHEAD_BLACK,
        BeatKind::Rest => glyphs::rest_glyph_for_duration(duration),
    };

    // Render rests a bit smaller than notes
    let font_id = if beat.kind == BeatKind::Rest {
        &FontId::new(opts.font_id.size * 0.8, opts.font_id.family.clone())
    } else {
        &opts.font_id
    };

    // Draw the glyph (notehead or rest)
    painter.text(pos, Align2::CENTER_CENTER, glyph.to_string(), font_id.clone(), opts.color);

    // Draw augmentation dots for dotted durations (notes and rests)
    let dots = match duration {
        Duration::Dotted { dots, .. } => dots,
        _ => 0,
    };
    if dots > 0 {
        // Horizontal spacing tuned by eye relative to font size
        // If this is a flagged note (not in a beam), push dots a bit further right so they don't collide with the flag tail.
        let has_flag_tail = beat.kind == BeatKind::Note
            && !opts.in_beam
            && glyphs::flag_glyph_for_duration(duration).is_some();
        let first_dx = if has_flag_tail { font_id.size * 0.5 } else { font_id.size * 0.28 };
        let step_dx = font_id.size * 0.26;
        for i in 0..dots {
            let x = pos.x + first_dx + (i as f32) * step_dx;
            painter.text(
                pos2(x, pos.y - font_id.size * 0.1),
                Align2::CENTER_CENTER,
                glyphs::GLYPH_AUGMENTATION_DOT.to_string(),
                font_id.clone(),
                opts.color,
            );
        }
    }

    // If this is a Note, draw a stem and possibly flags/tremolo
    if beat.kind == BeatKind::Note {
        let start = pos2(pos.x + opts.stem_dx, pos.y);
        let flag_glyph = glyphs::flag_glyph_for_duration(duration);
        // It's visually more appealing to reduce the stem length a bit for notes that are neither
        // in a beam nor flagged.
        let stem_len_factor = if opts.in_beam || flag_glyph.is_some() { 1.0 } else { 0.85 };
        let default_stem_len = get_default_stem_length(font_id) * stem_len_factor;
        let end = pos2(start.x, pos.y - default_stem_len);
        painter.line_segment([start, end], Stroke::new(opts.stem_thickness, opts.color));

        // Flag glyph at the stem tip for short durations, only if not in a beam
        if !opts.in_beam {
            if let Some(flag) = flag_glyph {
                let flag_font = FontId::new(font_id.size * 1.0, font_id.family.clone());
                painter.text(
                    pos2(
                        start.x - opts.stem_thickness * 0.5,
                        pos.y - get_default_stem_length(font_id), // TODO
                    ),
                    Align2::LEFT_CENTER,
                    flag.to_string(),
                    flag_font,
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

    if beat.accented && beat.kind == BeatKind::Note {
        let accent_font = FontId::new(font_id.size * 0.8, font_id.family.clone());
        painter.text(
            pos2(pos.x, pos.y - font_id.size * 1.2),
            Align2::CENTER_CENTER,
            glyphs::GLYPH_ACCENT_ABOVE.to_string(),
            accent_font,
            opts.color,
        );
    }
}
