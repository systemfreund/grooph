//! The staffs of the last few frames, so that a frame in which nothing on the
//! staff changed does not lay it out again.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use egui::Vec2;
use grooph_layout::staff_layout::{StaffLayout, StaffOpts, build_staff_layout};
use grooph_measure::score::Score;

/// [`build_staff_layout`] of `score` with `opts`, kept for later frames.
///
/// The measure panel asks for its staff every frame, and the metronome for
/// the height of its own, but the score, the panel's size and its zoom
/// change only now and then. A staff kept from an earlier frame is the
/// answer when the score and every option are the same. When only the
/// origin moved (the view scrolled) by whole device pixels, the kept staff
/// is moved there: lines are snapped to device pixels, so a move by whole
/// pixels is the staff laid out at the new origin (see
/// [`StaffLayout::translate`]). Any other change lays the staff out again.
pub(crate) fn staff_layout(score: &Score, opts: &StaffOpts) -> Rc<StaffLayout> {
    /// The staffs kept: the panel's, its version shrunk to fit a short
    /// panel, the metronome's height probe, and one for a score just swapped.
    const KEPT: usize = 4;
    type Kept = VecDeque<(Score, StaffOpts, Rc<StaffLayout>)>;
    thread_local! {
        static KEPT_STAFFS: RefCell<Kept> = const { RefCell::new(VecDeque::new()) };
    }
    KEPT_STAFFS.with_borrow_mut(|kept| {
        if let Some((_, _, staff)) = kept.iter().find(|(s, o, _)| o == opts && s == score) {
            return Rc::clone(staff);
        }
        let moved = kept.iter().find_map(|(s, o, staff)| {
            let d = whole_pixel_move(o, opts)?;
            (s == score).then(|| {
                let mut staff = StaffLayout::clone(staff);
                staff.translate(d);
                staff
            })
        });
        let staff = Rc::new(moved.unwrap_or_else(|| build_staff_layout(score, opts)));
        kept.push_front((score.clone(), opts.clone(), Rc::clone(&staff)));
        kept.truncate(KEPT);
        staff
    })
}

/// How far `to`'s rect lies from `from`'s, when that is all that tells them
/// apart and it is a whole number of device pixels on both axes.
fn whole_pixel_move(from: &StaffOpts, to: &StaffOpts) -> Option<Vec2> {
    let d = to.rect.min - from.rect.min;
    let same_otherwise = StaffOpts { rect: from.rect.translate(d), ..from.clone() } == *to;
    let whole = |v: f32| {
        let px = v * to.pixels_per_point;
        (px - px.round()).abs() < 1e-3
    };
    (same_otherwise && whole(d.x) && whole(d.y)).then_some(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{FontFamily, FontId, Pos2, Rect};
    use grooph_layout::staff_layout::ROOM_BELOW_EM;
    use grooph_measure::notation::parse_score;

    fn opts(min: Pos2) -> StaffOpts {
        StaffOpts {
            rect: Rect::from_min_size(min, egui::vec2(320.0, 600.0)),
            font_id: FontId::new(20.0, FontFamily::Proportional),
            pixels_per_point: 2.0,
            em: 20.0,
            y_offset: 0.0,
            stem_length_factor: 3.5,
            stem_thickness_factor: 0.1,
            accent_displacement: 0.0,
            min_measure_width_em: 6.0,
            note_width_em: 0.6,
            system_spacing_em: 0.5,
            layout_clef_first: true,
            time_signature_first: false,
            room_below_em: ROOM_BELOW_EM,
            lower_label_room_em: 0.0,
            bar_rests: false,
            veil_hidden_rests: false,
        }
    }

    #[test]
    fn a_staff_is_kept_until_its_score_or_options_change() {
        let score = parse_score("4/4 q q e e q | 3/4 q h").unwrap();
        let here = opts(Pos2::new(10.0, 20.0));
        let first = staff_layout(&score, &here);
        assert!(Rc::ptr_eq(&first, &staff_layout(&score.clone(), &here)));

        let edited = parse_score("4/4 q q e e q | 3/4 h q").unwrap();
        assert!(!Rc::ptr_eq(&first, &staff_layout(&edited, &here)));
        let wider = StaffOpts { rect: here.rect.expand2(egui::vec2(20.0, 0.0)), ..here.clone() };
        assert!(!Rc::ptr_eq(&first, &staff_layout(&score, &wider)));
        assert!(Rc::ptr_eq(&first, &staff_layout(&score, &here)));
    }

    #[test]
    fn a_scrolled_staff_is_moved_by_whole_pixels_only() {
        let at = |min| opts(min);
        let here = at(Pos2::new(10.0, 20.0));
        // Half a point is one device pixel at 2 pixels per point.
        let scrolled = at(Pos2::new(10.0, -35.5));
        let nudged = at(Pos2::new(10.0, -35.25));
        assert_eq!(whole_pixel_move(&here, &scrolled), Some(egui::vec2(0.0, -55.5)));
        assert_eq!(whole_pixel_move(&here, &nudged), None);
        let zoomed = StaffOpts { em: 24.0, ..scrolled.clone() };
        assert_eq!(whole_pixel_move(&here, &zoomed), None);

        let score = parse_score("4/4 q q e e q | 3/4 q h").unwrap();
        staff_layout(&score, &here);
        let moved = staff_layout(&score, &scrolled);
        let built = build_staff_layout(&score, &scrolled);
        assert_eq!(format!("{moved:.3?}"), format!("{built:.3?}"));
    }
}
