use crate::measure::duration::Duration;
use crate::measure::{Beat, BeatKind};
use crate::render::glyphs;
use eframe::egui;
use eframe::egui::{Align2, Color32, FontId, Pos2, Stroke, pos2};
use crate::layout::pixel_layout::NoteLayoutPx;

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
        if !opts.in_beam
            && let Some(flag) = flag_glyph
        {
            let flag_font = FontId::new(font_id.size * 1.0, font_id.family.clone());
            painter.text(
                pos2(start.x - opts.stem_thickness * 0.5, pos.y - default_stem_len),
                Align2::LEFT_CENTER,
                flag.to_string(),
                flag_font,
                opts.color,
            );
        }

        // Tremolo slashes (single-note measured tremolo)
        if let Some(trem) = beat.tremolo
            && trem.measured
        {
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

/// Phase B: Zeichnet eine Note/Rest basierend auf vorbereiteten Pixel-Geometrien aus dem Layout.
pub(crate) fn draw_note_from_layout(
    painter: &egui::Painter,
    note: &NoteLayoutPx,
    base_font: &FontId,
    color: Color32,
) {
    // 1) Glyph auswählen (Notehead/Rest)
    let glyph = match note.is_rest {
        true => glyphs::rest_glyph_for_duration(note.duration),
        false => glyphs::GLYPH_NOTEHEAD_BLACK,
    };

    // 2) Schriftauswahl: Rests etwas kleiner
    let glyph_font = if note.is_rest {
        FontId::new(base_font.size * 0.8, base_font.family.clone())
    } else {
        base_font.clone()
    };

    // 3) Haupt-Glyph zeichnen
    painter.text(note.center, Align2::CENTER_CENTER, glyph.to_string(), glyph_font, color);

    // 4) Dots
    if !note.dots.is_empty() {
        for p in &note.dots {
            painter.text(
                *p,
                Align2::CENTER_CENTER,
                glyphs::GLYPH_AUGMENTATION_DOT.to_string(),
                base_font.clone(),
                color,
            );
        }
    }

    // 5) Stem
    if let Some(stem) = &note.stem {
        painter.line_segment([stem.p1, stem.p2], Stroke::new(stem.thickness, color));
    }

    // 6) Flagge (nur wenn vorhanden)
    if let Some(pos) = note.flag_pos
        && let Some(flag) = glyphs::flag_glyph_for_duration(note.duration)
    {
        let flag_font = FontId::new(base_font.size * 1.0, base_font.family.clone());
        painter.text(pos, Align2::LEFT_CENTER, flag.to_string(), flag_font, color);
    }

    // 7) Tremolo-Linien
    if !note.tremolo.is_empty() {
        for seg in &note.tremolo {
            painter.line_segment([seg.p1, seg.p2], Stroke::new(seg.thickness, color));
        }
    }

    // 8) Akzent
    if let Some(p) = note.accent_pos {
        let accent_font = FontId::new(base_font.size * 0.8, base_font.family.clone());
        painter.text(
            p,
            Align2::CENTER_CENTER,
            glyphs::GLYPH_ACCENT_ABOVE.to_string(),
            accent_font,
            color,
        );
    }
}
