use super::{LayoutOpts, Line, NoteLayout, PlacedGlyph, c, requires_flag};
use crate::glyphs;
use crate::render_plan::RenderPlan;
use egui::{Pos2, Rect};
use grooph_measure::duration::{Duration, NoteValue};
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::{Beat, BeatKind, StaffKind, VoiceBar};

/// Number of augmentation dots on a duration (0 for non-dotted).
const fn dot_count_of(duration: Duration) -> u8 {
    match duration {
        Duration::Dotted { dots, .. } => dots,
        _ => 0,
    }
}

/// Where `beat` sits, in half staff spaces up from the staff line. Notes are
/// on the line. In the lower voice a rest hangs a staff space lower, clear of
/// an upper voice's, except half and whole rests: they are told apart only by
/// sitting on the line or hanging from it. A stemlet (`in_beam` rest) reaches
/// from the line like a note's stem.
///
/// On a line shared by two voices ([`LayoutOpts::shared_line`]) each voice
/// keeps to its side: heads sit on the line or hang from it, rests clear it,
/// and a lower voice's half rest, which would sit on it, moves to a ledger
/// line a staff space below (see [`ledger_pos`]).
fn staff_pos_of(beat: &Beat, in_beam: bool, opts: &LayoutOpts) -> i8 {
    if opts.staff == StaffKind::Kit && beat.is_rest() {
        return kit_rest_pos(beat, opts);
    }
    let base = beat.duration.base_note();
    let keeps_line = matches!(base, NoteValue::Half | NoteValue::Whole);
    if opts.shared_line {
        let side = if opts.stem_down { -1 } else { 1 };
        return match beat.kind() {
            BeatKind::Note => side,
            BeatKind::Rest if in_beam => side,
            BeatKind::Rest => match ledger_pos(beat, opts) {
                Some(pos) => pos,
                None if keeps_line => 0,
                None => side * clear_of_line(head_glyph(beat), opts.stem_down),
            },
        };
    }
    match beat.kind() {
        BeatKind::Rest if opts.stem_down && !keeps_line && !in_beam => -2,
        _ => 0,
    }
}

/// Where a rest sits on the kit's staff: in the middle with one voice (a
/// whole rest hangs from the fourth line), in its voice's half with two,
/// clear of the other's notes: half and whole rests on a line, the others
/// in the third or second space from the middle.
fn kit_rest_pos(beat: &Beat, opts: &LayoutOpts) -> i8 {
    let base = beat.duration.base_note();
    match (opts.shared_line, opts.stem_down, base) {
        (false, _, NoteValue::Whole) => 2,
        (false, ..) => 0,
        (true, false, NoteValue::Whole) => 4,
        (true, false, NoteValue::Half) => 2,
        (true, false, _) => 3,
        (true, true, NoteValue::Whole) => -2,
        (true, true, NoteValue::Half) => -4,
        (true, true, _) => -3,
    }
}

/// One head of a note, before it is placed.
#[derive(Clone, Copy)]
struct ChordHead {
    /// In half staff spaces from the middle line.
    pos: i8,
    glyph: glyphs::HeadGlyph,
    soft: bool,
    /// On the other side of the stem, a second from the head before it.
    flipped: bool,
}

/// The heads of the note `beat`, from the stem's base to its tip. On the
/// kit's staff each sits at its instrument's place; a head a second from
/// the one before it, unless that one is flipped already, goes to the other
/// side of the stem so the two don't overlap.
fn chord_of(beat: &Beat, in_beam: bool, opts: &LayoutOpts) -> Vec<ChordHead> {
    let mut heads: Vec<ChordHead> = beat
        .heads()
        .iter()
        .map(|h| ChordHead {
            pos: match opts.staff {
                StaffKind::Kit => h.instrument().staff_pos(),
                StaffKind::Line => staff_pos_of(beat, in_beam, opts),
            },
            glyph: glyphs::head_glyph_for(h.instrument(), h.technique(), beat.duration),
            soft: h.soft(),
            flipped: false,
        })
        .collect();
    heads.sort_by_key(|h| if opts.stem_down { -h.pos } else { h.pos });
    for i in 1..heads.len() {
        let second = (heads[i].pos - heads[i - 1].pos).abs() == 1;
        heads[i].flipped = second && !heads[i - 1].flipped;
    }
    heads
}

/// The x of the origin of `head` hanging off a stem at `stem_x`: left of an
/// up stem and right of a down stem, the other side when `flipped`.
fn head_origin_x(stem_x: f32, head: &glyphs::HeadGlyph, flipped: bool, opts: &LayoutOpts) -> f32 {
    let ss = opts.staff_space();
    let half_stem = opts.stem_thickness() * 0.5;
    if opts.stem_down != flipped {
        stem_x - half_stem - head.stem_down_nw[0] * ss
    } else {
        stem_x + half_stem - head.stem_up_se[0] * ss
    }
}

/// The x the parentheses of a soft `head` center on: the note's
/// `center_x`, or a flipped head's own middle on the other side of the stem.
fn paren_mid(center_x: f32, ink: Rect, head: &ChordHead) -> f32 {
    if head.flipped { ink.center().x } else { center_x }
}

/// From a note's center to its stem, before pixel snapping.
fn stem_dx(opts: &LayoutOpts) -> f32 {
    if opts.stem_down { -opts.stem_offset() } else { opts.stem_offset() }
}

/// Least room between a rest and a staff line it keeps clear of, in staff
/// spaces.
const REST_CLEARANCE_SS: f32 = 0.25;

/// How many half staff spaces `rest` moves off the line, away from it, for
/// its ink to clear the line by [`REST_CLEARANCE_SS`]: up when `below` is
/// false, down when it is true.
fn clear_of_line(rest: glyphs::Glyph, below: bool) -> i8 {
    let reach = if below { rest.ne[1] } else { -rest.sw[1] };
    (2.0 * (REST_CLEARANCE_SS + reach)).ceil() as i8
}

/// The staff position of the ledger line of a rest on a shared line that
/// would otherwise sit in the other voice: a lower voice's half rest sits
/// on one below. (A whole rest, hanging from the line, would need one above
/// in the upper voice, but no meter holds a whole note.)
fn ledger_pos(beat: &Beat, opts: &LayoutOpts) -> Option<i8> {
    let half = beat.duration.base_note() == NoteValue::Half;
    let line = opts.staff == StaffKind::Line;
    (line && opts.shared_line && opts.stem_down && beat.is_rest() && half).then_some(-2)
}

/// How far a ledger line reaches past either side of its rest, in staff
/// spaces (Bravura's `legerLineExtension`).
const LEDGER_EXTENSION_SS: f32 = 0.4;

/// The ledger lines of heads above or below the kit's staff, from its top
/// or bottom line outward to each head, as wide as the heads they hold.
fn kit_ledgers(heads: &[(PlacedGlyph, &ChordHead)], opts: &LayoutOpts) -> Vec<Line> {
    let reach = LEDGER_EXTENSION_SS * opts.staff_space();
    let mut lines: Vec<(i8, f32, f32)> = Vec::new();
    for (glyph, head) in heads {
        let ink = glyph.ink();
        let outward = head.pos.signum();
        let mut pos = 6 * outward;
        while outward != 0 && pos.abs() <= head.pos.abs() {
            match lines.iter_mut().find(|l| l.0 == pos) {
                Some(l) => {
                    l.1 = l.1.min(ink.left());
                    l.2 = l.2.max(ink.right());
                }
                None => lines.push((pos, ink.left(), ink.right())),
            }
            pos += 2 * outward;
        }
    }
    lines
        .into_iter()
        .map(|(pos, left, right)| {
            let y = opts.y_of(pos);
            Line { p1: Pos2::new(left - reach, y), p2: Pos2::new(right + reach, y) }
        })
        .collect()
}

/// The head or rest glyph of `beat`.
fn head_glyph(beat: &Beat) -> glyphs::Glyph {
    match beat.kind() {
        BeatKind::Note => glyphs::notehead_glyph_for_duration(beat.duration),
        BeatKind::Rest => glyphs::rest_glyph_for_duration(beat.duration),
    }
}

/// From a soft note's center to the outer edge of its parentheses' ink.
fn paren_reach(opts: &LayoutOpts) -> f32 {
    let ss = opts.staff_space();
    opts.head_half_width() + c::SOFT_PAREN_GAP_SS * ss + glyphs::NOTEHEAD_PAREN_LEFT.ink(ss).width()
}

/// The parentheses around a soft note's head centered at `center`. They
/// are taller than the head: on a shared line they move off it, away from
/// the other voice, their inner ends level with the head's edge on the line.
fn parens_at(center: Pos2, opts: &LayoutOpts) -> Vec<PlacedGlyph> {
    let ss = opts.staff_space();
    let inner = opts.head_half_width() + c::SOFT_PAREN_GAP_SS * ss;
    let left = glyphs::NOTEHEAD_PAREN_LEFT;
    let right = glyphs::NOTEHEAD_PAREN_RIGHT;
    let overhang = (left.ne[1] - glyphs::NOTEHEAD_BLACK.ne[1]) * ss;
    let y = match (opts.shared_line && opts.staff == StaffKind::Line, opts.stem_down) {
        (false, _) => center.y,
        (true, false) => center.y - overhang,
        (true, true) => center.y + overhang,
    };
    vec![
        PlacedGlyph {
            glyph: left,
            origin: Pos2::new(center.x - inner - left.ink(ss).right(), y),
            size: opts.em,
        },
        PlacedGlyph {
            glyph: right,
            origin: Pos2::new(center.x + inner - right.ink(ss).left(), y),
            size: opts.em,
        },
    ]
}

/// The flag of a note of `duration` on `stem`, its end at the stem's tip.
fn flag_at(
    duration: Duration,
    stem: &Line,
    stem_down: bool,
    opts: &LayoutOpts,
) -> Option<PlacedGlyph> {
    let (glyph, anchor) = glyphs::flag_glyph_for_duration(duration, stem_down)?;
    let ss = opts.staff_space();
    // Flags hang off the stem's left edge, toward the right.
    let x = stem.p1.x - opts.stem_thickness() * 0.5 - anchor[0] * ss;
    Some(PlacedGlyph { glyph, origin: Pos2::new(x, stem.p2.y + anchor[1] * ss), size: opts.em })
}

/// X offset of the first augmentation dot's center, relative to the note
/// center: just right of the head, or of its parentheses for a soft note. An
/// up-stem flag curls down beside the dots and pushes them past it; a
/// down-stem flag hangs below them.
/// `chord` is the note's ([`chord_of`]), empty for a rest.
fn dot_first_dx(beat: &Beat, in_beam: bool, chord: &[ChordHead], opts: &LayoutOpts) -> f32 {
    let is_note = beat.is_note();
    let mut dx = opts.em * c::DOT_FIRST_DX_NO_FLAG_EM;
    // Heads flipped right of an up stem push the dots past them.
    let flipped_right = !opts.stem_down && chord.iter().any(|h| h.flipped);
    if flipped_right {
        dx += glyphs::NOTEHEAD_BLACK.advance * opts.staff_space();
    }
    if is_note && beat.soft() {
        dx += paren_reach(opts) - opts.head_half_width();
    }
    if is_note && !in_beam && requires_flag(beat.duration) && !opts.stem_down {
        dx = dx.max(opts.em * c::DOT_FIRST_DX_WITH_FLAG_EM);
    }
    dx
}

/// The augmentation dots of a head at `cy` of a note centered at
/// `center_x`, the first `first_dx` from it ([`dot_first_dx`]). Returns
/// empty if `dot_count == 0`.
fn dot_glyphs(
    first_dx: f32,
    center_x: f32,
    cy: f32,
    dot_count: u8,
    opts: &LayoutOpts,
) -> Vec<PlacedGlyph> {
    let step_dx = opts.em * c::DOT_STEP_DX_EM;
    // A head on a shared line sits in the space beside it, and its dots
    // with it; one on the line moves them up into the space above. On the
    // kit's staff that goes for heads on its lines.
    let on_line = match opts.staff {
        StaffKind::Line => !opts.shared_line,
        StaffKind::Kit => {
            (((opts.y_center() - cy) / (opts.staff_space() * 0.5)).round() as i32) % 2 == 0
        }
    };
    let dy = if on_line { opts.em * c::DOT_Y_OFFSET_EM } else { 0.0 };
    let y = cy - dy;
    (0..dot_count)
        .map(|d| {
            let x = center_x + first_dx + (d as f32) * step_dx;
            PlacedGlyph::centered(glyphs::AUGMENTATION_DOT, x, y, opts.em)
        })
        .collect()
}

/// Pre-layout horizontal extents (relative to a note's center) used by the
/// shift pass: the head or rest, its parentheses and dots, and an ungrouped
/// flag. Stems are not yet known here, so the flag is placed from
/// `opts.stem_offset()`.
fn compute_x_extents(beat: &Beat, in_beam: bool, opts: &LayoutOpts) -> (f32, f32) {
    let ss = opts.staff_space();
    let is_note = beat.is_note();
    let needs_flag = requires_flag(beat.duration);

    let chord = if is_note { chord_of(beat, in_beam, opts) } else { Vec::new() };
    let (left, mut right) = if is_note {
        // Every head, on its side of the stem, and its parentheses.
        chord
            .iter()
            .map(|h| {
                let x = head_origin_x(stem_dx(opts), &h.glyph, h.flipped, opts);
                let ink =
                    PlacedGlyph { glyph: h.glyph.glyph, origin: Pos2::new(x, 0.0), size: opts.em }
                        .ink();
                if h.soft {
                    let mid = paren_mid(0.0, ink, h);
                    (mid - paren_reach(opts), mid + paren_reach(opts))
                } else {
                    (ink.left(), ink.right())
                }
            })
            .fold((f32::MAX, f32::MIN), |(l, r), (a, b)| (l.min(a), r.max(b)))
    } else {
        // A ledger line may reach past the rest: it doesn't widen it, so the
        // voice is spaced as on a line of its own.
        let head = PlacedGlyph::centered(head_glyph(beat), 0.0, 0.0, opts.em).ink();
        (head.left(), head.right())
    };

    let dot_count = dot_count_of(beat.duration);
    if dot_count > 0 {
        let first_dx = dot_first_dx(beat, in_beam, &chord, opts);
        let step_dx = opts.em * c::DOT_STEP_DX_EM;
        let last_dot_center_rel = first_dx + ((dot_count - 1) as f32) * step_dx;
        let last_dot_right_rel =
            last_dot_center_rel + glyphs::AUGMENTATION_DOT.ink(ss).width() * 0.5;
        right = right.max(last_dot_right_rel);
    }

    if is_note
        && !in_beam
        && needs_flag
        && let Some((flag, anchor)) = glyphs::flag_glyph_for_duration(beat.duration, opts.stem_down)
    {
        let stem_dx = if opts.stem_down { -opts.stem_offset() } else { opts.stem_offset() };
        let flag_x = stem_dx - opts.stem_thickness() * 0.5 - anchor[0] * ss;
        right = right.max(flag_x + flag.ink(ss).right());
    }

    (left, right)
}

/// Post-layout vertical extents in absolute screen coordinates, from the
/// already-placed glyphs and stem.
/// Top and bottom of a note's head, stem, flag and dots.
fn content_extents(note: &NoteLayout) -> (f32, f32) {
    let mut ink = note.head.ink();
    for g in note.chord.iter().chain(&note.flag).chain(&note.dots) {
        ink = ink.union(g.ink());
    }
    if let Some(s) = note.stem {
        ink = ink.union(Rect::from_two_pos(s.p1, s.p2));
    }
    (ink.top(), ink.bottom())
}

/// Put an accent above the note's head, stem, flag and dots (below them
/// when `below`), replacing any accent it had.
fn place_accent(note: &mut NoteLayout, below: bool, opts: &LayoutOpts) {
    let (content_top, content_bottom) = content_extents(note);
    let displacement = opts.em * opts.accent_displacement;
    let mut glyph = PlacedGlyph::centered(glyphs::ACCENT_ABOVE, note.center.x, 0.0, opts.em);
    let ink = glyph.ink();
    glyph.origin.y = if below {
        content_bottom + displacement - ink.top()
    } else {
        content_top - displacement - ink.bottom()
    };
    note.accent = Some(glyph);
}

/// Put the marks of each note on its stem's side, past its heads, stem,
/// flag and dots: the accent nearest, then the sign of a technique (o for an
/// open hi-hat, + for a closed one).
pub(super) fn place_marks(notes: &mut [NoteLayout], beats: &[Beat], opts: &LayoutOpts) {
    let below = opts.accents_below();
    for (note, beat) in notes.iter_mut().zip(beats).filter(|(_, b)| b.is_note()) {
        if beat.accented() {
            place_accent(note, below, opts);
        }
        let sign = beat.heads().iter().find_map(|h| glyphs::technique_sign(h.technique()));
        if let Some(sign) = sign {
            let (mut top, mut bottom) = content_extents(note);
            if let Some(accent) = &note.accent {
                top = top.min(accent.ink().top());
                bottom = bottom.max(accent.ink().bottom());
            }
            let displacement = opts.em * opts.accent_displacement;
            let mut glyph = PlacedGlyph::centered(sign, note.center.x, 0.0, opts.em);
            let ink = glyph.ink();
            glyph.origin.y = if below {
                bottom + displacement - ink.top()
            } else {
                top - displacement - ink.bottom()
            };
            note.sign = Some(glyph);
        }
    }
}

/// Whether each beat sits in a beamed group of two or more (no flag).
fn in_beam_flags(beats: &[Beat], render_plan: &RenderPlan) -> Vec<bool> {
    let mut flags = vec![false; beats.len()];
    for g in &render_plan.beams {
        if g.beat_indices.len() >= 2 {
            for &idx in &g.beat_indices {
                if idx < flags.len() {
                    flags[idx] = true;
                }
            }
        }
    }
    flags
}

/// Least distance (px) each beat needs to the next note head, or to the
/// barline for the last one, so their glyphs don't overlap.
fn min_advances(beats: &[Beat], in_beam: &[bool], opts: &LayoutOpts) -> Vec<f32> {
    let extents: Vec<(f32, f32)> =
        beats.iter().zip(in_beam).map(|(b, &ib)| compute_x_extents(b, ib, opts)).collect();
    let gap = c::COLLISION_MIN_GAP_EM * opts.em;
    let needs: Vec<f32> = (0..beats.len())
        .map(|i| match extents.get(i + 1) {
            Some(&(next_left, _)) => extents[i].1 + gap - next_left,
            None => extents[i].1,
        })
        .collect();
    // Notes of the same length get the same room, so a wide neighbour (a
    // soft note's parentheses) doesn't make the plain ones look shorter.
    let ticks: Vec<_> = beats.iter().map(|b| DEFAULT_GRID.ticks_of(&b.duration)).collect();
    ticks
        .iter()
        .map(|t| {
            ticks.iter().zip(&needs).filter(|(u, _)| *u == t).map(|(_, &n)| n).fold(0.0, f32::max)
        })
        .collect()
}

/// Least distance (px) each beat of `measure` needs to the next note head,
/// or to the barline for the last one (see [`min_advances`]).
pub(super) fn note_advances(
    measure: &VoiceBar,
    render_plan: &RenderPlan,
    opts: &LayoutOpts,
) -> Vec<f32> {
    let beats = measure.beats();
    min_advances(beats, &in_beam_flags(beats, render_plan), opts)
}

/// Narrowest note area in which no glyph overlaps the next one or the
/// barline (see [`crate::calculate_x_centers`]'s `min_advance`).
pub(super) fn min_note_area_width(
    measure: &VoiceBar,
    render_plan: &RenderPlan,
    opts: &LayoutOpts,
) -> f32 {
    c::NOTE_LEAD_EM * opts.em + note_advances(measure, render_plan, opts).iter().sum::<f32>()
}

/// The notes of `measure` in the note area `rect`, at `x_centers` (from the
/// area's left edge) if given, else spread by rhythm over its width.
pub(super) fn build_note_layout(
    measure: &VoiceBar,
    render_plan: &RenderPlan,
    rect: &Rect,
    x_centers: Option<&[f32]>,
    opts: &LayoutOpts,
) -> Vec<NoteLayout> {
    let beats = measure.beats();

    // Base positions, spread evenly by rhythm over the available width
    // Determine which beats are inside any beamed group (for flag suppression)
    let in_beam_flags = in_beam_flags(beats, render_plan);

    let x_centers = match x_centers {
        Some(xs) => xs.to_vec(),
        None => crate::calculate_x_centers(
            beats,
            rect.width(),
            c::NOTE_LEAD_EM * opts.em,
            &min_advances(beats, &in_beam_flags, opts),
        ),
    }
    .into_iter()
    .map(|cx| cx + rect.left())
    .collect::<Vec<_>>();

    // No note is moved after spacing: the spacing already keeps every
    // glyph clear of the next (`min_advances`), and with several voices a
    // note moved in one would leave the beat it shares with the others.
    let mut note_layout: Vec<NoteLayout> = Vec::with_capacity(beats.len());
    let ss = opts.staff_space();
    for (i, b) in beats.iter().enumerate() {
        let ideal_cx = x_centers[i];
        let needs_flag = requires_flag(b.duration);
        let in_beam = in_beam_flags.get(i).copied().unwrap_or(false);
        let is_note = b.is_note();
        // A rest under a beam is a stemlet: a stem without a head.
        let stemlet = !is_note && in_beam;
        let chord = if is_note { chord_of(b, in_beam, opts) } else { Vec::new() };
        // The head at the stem's tip sets the note's place: the beams and
        // tuplets reach from it.
        let staff_pos = chord.last().map_or_else(|| staff_pos_of(b, in_beam, opts), |h| h.pos);
        let cy = opts.y_of(staff_pos);

        // 1. Calculate Stem (if any) and determining pixel-snapping offset.
        // We do this first because the notehead and dots must align with
        // the snapped stem.
        let mut stem: Option<Line> = None;

        if is_note || stemlet {
            let stem_len_factor = if in_beam || needs_flag { 1.0 } else { 0.85 };
            let stem_len = opts.stem_length() * stem_len_factor;
            // Down stems hang off the head's left side.
            let ideal_stem_x = ideal_cx + stem_dx(opts);
            let snapped_stem_x = opts.snap_x(ideal_stem_x, opts.stem_thickness());

            let end = Pos2::new(snapped_stem_x, cy + opts.stem_dir() * stem_len);
            let start = match chord.first() {
                // From the head at the stem's base.
                Some(base) => {
                    let attach = if opts.stem_down {
                        base.glyph.stem_down_nw[1]
                    } else {
                        base.glyph.stem_up_se[1]
                    };
                    Pos2::new(snapped_stem_x, opts.y_of(base.pos) - attach * ss)
                }
                None => Pos2::new(
                    snapped_stem_x,
                    end.y - opts.stem_dir() * stem_len * c::STEMLET_LENGTH_FACTOR,
                ),
            };
            stem = Some(Line { p1: start, p2: end });
        }

        // 2. Heads or rest. A head hangs off its stem's right edge (left
        // edge for a down stem): both are rounded to pixels on their own,
        // and centering the head could leave a hairline between them on
        // small staffs.
        let center = Pos2::new(ideal_cx, cy);
        let placed: Vec<(PlacedGlyph, &ChordHead)> = chord
            .iter()
            .map(|h| {
                let y = opts.y_of(h.pos);
                let glyph = h.glyph.glyph;
                let head = match &stem {
                    Some(s) => PlacedGlyph {
                        glyph,
                        origin: Pos2::new(head_origin_x(s.p1.x, &h.glyph, h.flipped, opts), y),
                        size: opts.em,
                    },
                    None => PlacedGlyph::centered(glyph, center.x, y, opts.em),
                };
                (head, h)
            })
            .collect();
        let head = match placed.last() {
            Some((head, _)) => *head,
            None => PlacedGlyph::centered(head_glyph(b), center.x, cy, opts.em),
        };
        let others: Vec<PlacedGlyph> = placed.iter().rev().skip(1).map(|(g, _)| *g).collect();
        let parens: Vec<PlacedGlyph> = placed
            .iter()
            .filter(|(_, h)| h.soft)
            .flat_map(|(g, h)| {
                parens_at(Pos2::new(paren_mid(center.x, g.ink(), h), opts.y_of(h.pos)), opts)
            })
            .collect();
        let mut ledgers: Vec<Line> = ledger_pos(b, opts)
            .map(|pos| {
                let ink = head.ink();
                let (y, reach) = (opts.y_of(pos), LEDGER_EXTENSION_SS * ss);
                Line { p1: Pos2::new(ink.left() - reach, y), p2: Pos2::new(ink.right() + reach, y) }
            })
            .into_iter()
            .collect();
        if opts.staff == StaffKind::Kit {
            ledgers.extend(kit_ledgers(&placed, opts));
        }

        // 3. Dots, by every head.
        let first_dx = dot_first_dx(b, in_beam, &chord, opts);
        let dot_count = dot_count_of(b.duration);
        let dots = match chord.is_empty() {
            true => dot_glyphs(first_dx, center.x, cy, dot_count, opts),
            false => chord
                .iter()
                .flat_map(|h| dot_glyphs(first_dx, center.x, opts.y_of(h.pos), dot_count, opts))
                .collect(),
        };

        // 4. Flag (if needed)
        let flag = match &stem {
            Some(s) if !in_beam && needs_flag => flag_at(b.duration, s, opts.stem_down, opts),
            _ => None,
        };

        let note = NoteLayout {
            center,
            staff_pos,
            duration: b.duration,
            kind: b.kind(),
            head,
            chord: others,
            stemlet,
            hidden: false,
            parens,
            ledgers,
            dots,
            stem,
            flag,
            accent: None,
            sign: None,
        };

        note_layout.push(note);
    }

    if render_plan.bar_rest {
        bar_rest(&mut note_layout, rect, opts);
    }
    note_layout
}

/// Draw the rests of an empty bar as one whole rest, hanging from the line
/// in the middle of the note area; its other rests are hidden. Every beat
/// keeps its place, for the count labels and the playback cursor.
fn bar_rest(notes: &mut [NoteLayout], rect: &Rect, opts: &LayoutOpts) {
    for (i, note) in notes.iter_mut().enumerate() {
        note.dots.clear();
        if i == 0 {
            let y = opts.y_of(0);
            note.head = PlacedGlyph::centered(glyphs::REST_WHOLE, rect.center().x, y, opts.em);
        } else {
            note.hidden = true;
        }
    }
}
