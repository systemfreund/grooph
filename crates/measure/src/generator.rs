//! Random rhythm generator for sight-reading practice.
//!
//! A measure is generated beat by beat: for every beat of the time signature
//! one *cell* (a figure spanning exactly one beat, e.g. `16-16-8` or
//! `8-rest + 8`) is drawn from a library. Each cell belongs to a
//! [`CellFamily`] (which [`Subdivision`] settings may use it) and has a
//! difficulty level. [`GeneratorSettings::complexity`] caps the level that may
//! be drawn; the highest allowed level is favoured so a higher setting audibly
//! changes the result, while easier cells stay in the mix.
//!
//! Level ladder for 8th/16th subdivisions (derived from reference examples):
//! 1. quarters, on-beat eighths and quarter rests
//! 2. eighths on the "and" (first syncopations)
//! 3. sixteenth figures starting on the beat, no rests inside the beat
//! 4. dotted figures and rests inside the beat
//! 5. figures starting with a sixteenth rest (notes on "e" / "a")
//!
//! Level ladder for triplets (per the reference app, `x` = note, `-` = rest):
//! 1. `xxx` or a quarter note, no rests
//! 2. + `x-x`, quarter rest
//! 3. + `--x`
//! 4. + `xx-`, `-xx`
//! 5. + `-x-`
//!
//! Besides these library quarter rests (level 1 for 8ths/16ths, level 2 for
//! triplets), [`GeneratorSettings::space`] adds whole-beat rests, which replaces entire beats (never part
//! of a figure, so a triplet is always complete) with a quarter rest. At
//! maximum space a measure may consist of rests only.
//!
//! Only time signatures whose beat unit is a quarter (`x/4`) are supported for
//! now; cells are written for a quarter-note beat.

use crate::BeatKind::{Note, Rest};
use crate::duration::{Duration, NoteValue, e, q, s, t8};
use crate::{Beat, Measure, MeasureError, Score, TimeSignature};
use serde::{Deserialize, Serialize};

/// Highest supported complexity level.
pub const MAX_COMPLEXITY: u8 = 5;

/// Finest grid the generator may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Subdivision {
    Eighths,
    Sixteenths,
    Triplets,
    /// Eighths, sixteenths and triplets combined.
    Mixed,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeneratorSettings {
    pub subdivision: Subdivision,
    /// 1..=[`MAX_COMPLEXITY`]; out-of-range values are clamped.
    pub complexity: u8,
    /// Number of measures in a generated score (at least 1).
    pub bars: usize,
    pub time_signature: TimeSignature,
    /// 0.0..=1.0 — the higher, the more (and longer) rests between notes.
    pub space: f32,
}

impl Default for GeneratorSettings {
    fn default() -> Self {
        Self {
            subdivision: Subdivision::Sixteenths,
            complexity: 1,
            bars: 1,
            time_signature: TimeSignature::FOUR_FOUR,
            space: 0.0,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum GeneratorError {
    /// The generator only knows cells for quarter-note beats (`x/4`).
    UnsupportedTimeSignature(TimeSignature),
    /// Writing a cell into the measure failed (library bug).
    Measure(MeasureError),
}

impl From<MeasureError> for GeneratorError {
    fn from(err: MeasureError) -> Self { GeneratorError::Measure(err) }
}

/// Small deterministic PRNG (SplitMix64). Seedable so generated rhythms are
/// reproducible in tests; the app seeds it from the clock.
#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self { Self { state: seed } }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform integer in `0..n`. `n` must be non-zero.
    pub fn below(&mut self, n: usize) -> usize { (self.next_u64() % n as u64) as usize }

    /// Uniform float in `[0, 1)`.
    pub fn unit(&mut self) -> f32 { (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32 }

    pub fn chance(&mut self, p: f32) -> bool { self.unit() < p }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CellFamily {
    /// A plain quarter note; available to every subdivision.
    Quarter,
    /// Straight eighths; not used with the triplet subdivision.
    Eighth,
    Sixteenth,
    Triplet,
}

impl CellFamily {
    fn allowed_in(self, subdivision: Subdivision) -> bool {
        match self {
            CellFamily::Quarter => true,
            CellFamily::Eighth => subdivision != Subdivision::Triplets,
            CellFamily::Sixteenth => {
                matches!(subdivision, Subdivision::Sixteenths | Subdivision::Mixed)
            }
            CellFamily::Triplet => {
                matches!(subdivision, Subdivision::Triplets | Subdivision::Mixed)
            }
        }
    }
}

/// A figure spanning exactly one quarter-note beat.
#[derive(Clone, Copy, Debug)]
struct Cell {
    level: u8,
    family: CellFamily,
    beats: &'static [(Duration, bool)],
}

const fn de() -> Duration { Duration::Dotted { base: NoteValue::Eighth, dots: 1 } }

const N: bool = true;
const R: bool = false;

/// Whole-beat rest inserted by the "space" setting; not part of the library.
static QUARTER_REST: Cell = Cell { level: 1, family: CellFamily::Quarter, beats: &[(q(), R)] };

#[rustfmt::skip]
const CELLS: &[Cell] = &[
    // Level 1: quarters and on-beat eighths.
    Cell { level: 1, family: CellFamily::Quarter, beats: &[(q(), N)] },
    Cell { level: 1, family: CellFamily::Eighth, beats: &[(e(), N), (e(), N)] },
    Cell { level: 1, family: CellFamily::Eighth, beats: &[(q(), R)] },
    // Level 2: eighth syncopation. (Eighth + eighth rest is left out on
    // purpose: rhythmically it is the same single hit as a quarter note.)
    Cell { level: 2, family: CellFamily::Eighth, beats: &[(e(), R), (e(), N)] },
    // Sixteenths (onsets on 1 e & a; one spelling per rhythm).
    // Levels 1-2 use the eighth cells only (same as the eighth subdivision).
    // Level 3: sixteenth figures on the beat, no inner rests.
    Cell { level: 3, family: CellFamily::Sixteenth, beats: &[(s(), N), (s(), N), (s(), N), (s(), N)] },
    Cell { level: 3, family: CellFamily::Sixteenth, beats: &[(s(), N), (s(), N), (e(), N)] },
    Cell { level: 3, family: CellFamily::Sixteenth, beats: &[(e(), N), (s(), N), (s(), N)] },
    // Level 4: dotted figures, rests inside the beat.
    Cell { level: 4, family: CellFamily::Sixteenth, beats: &[(de(), N), (s(), N)] },
    Cell { level: 4, family: CellFamily::Sixteenth, beats: &[(s(), N), (s(), N), (e(), R)] },
    Cell { level: 4, family: CellFamily::Sixteenth, beats: &[(e(), R), (s(), N), (s(), N)] },
    Cell { level: 4, family: CellFamily::Sixteenth, beats: &[(s(), N), (e(), N), (s(), N)] },
    // Level 5: notes on "e" and "a".
    Cell { level: 5, family: CellFamily::Sixteenth, beats: &[(s(), R), (s(), N), (e(), N)] },
    Cell { level: 5, family: CellFamily::Sixteenth, beats: &[(de(), R), (s(), N)] },
    Cell { level: 5, family: CellFamily::Sixteenth, beats: &[(s(), R), (s(), N), (s(), N), (s(), N)] },
    Cell { level: 5, family: CellFamily::Sixteenth, beats: &[(s(), R), (e(), N), (s(), N)] },
    Cell { level: 5, family: CellFamily::Sixteenth, beats: &[(s(), R), (de(), N)] },
    // Triplets (eighth-note triplets per quarter).
    Cell { level: 1, family: CellFamily::Triplet, beats: &[(t8(), N), (t8(), N), (t8(), N)] },
    Cell { level: 2, family: CellFamily::Triplet, beats: &[(t8(), N), (t8(), R), (t8(), N)] },
    Cell { level: 2, family: CellFamily::Triplet, beats: &[(q(), R)] },
    Cell { level: 3, family: CellFamily::Triplet, beats: &[(t8(), R), (t8(), R), (t8(), N)] },
    Cell { level: 4, family: CellFamily::Triplet, beats: &[(t8(), N), (t8(), N), (t8(), R)] },
    Cell { level: 4, family: CellFamily::Triplet, beats: &[(t8(), R), (t8(), N), (t8(), N)] },
    Cell { level: 5, family: CellFamily::Triplet, beats: &[(t8(), R), (t8(), N), (t8(), R)] },
];

/// Weight of the highest allowed level relative to each easier level.
const TOP_LEVEL_WEIGHT: usize = 3;

/// Probability of a whole-beat rest at `space == 1.0` (a 4/4 measure is then
/// entirely rests about a third of the time).
const MAX_SPACE_REST_PROBABILITY: f32 = 0.75;

fn candidates(subdivision: Subdivision, max_level: u8) -> Vec<&'static Cell> {
    CELLS.iter().filter(|c| c.family.allowed_in(subdivision) && c.level <= max_level).collect()
}

/// Draw one cell: pick a level (favouring the highest one available), then a
/// cell of that level uniformly.
fn pick_cell(pool: &[&'static Cell], rng: &mut Rng) -> &'static Cell {
    let mut levels: Vec<u8> = pool.iter().map(|c| c.level).collect();
    levels.sort_unstable();
    levels.dedup();
    let top = *levels.last().expect("cell pool is never empty");

    let total: usize = levels.iter().map(|&l| if l == top { TOP_LEVEL_WEIGHT } else { 1 }).sum();
    let mut roll = rng.below(total);
    let mut level = top;
    for &l in &levels {
        let w = if l == top { TOP_LEVEL_WEIGHT } else { 1 };
        if roll < w {
            level = l;
            break;
        }
        roll -= w;
    }

    let at_level: Vec<&'static Cell> = pool.iter().copied().filter(|c| c.level == level).collect();
    at_level[rng.below(at_level.len())]
}

fn pick_cells(settings: &GeneratorSettings, rng: &mut Rng) -> Vec<&'static Cell> {
    let complexity = settings.complexity.clamp(1, MAX_COMPLEXITY);
    let space = settings.space.clamp(0.0, 1.0);
    let pool = candidates(settings.subdivision, complexity);
    let beat_count = settings.time_signature.beats as usize;

    (0..beat_count)
        .map(|_| {
            if rng.chance(space * MAX_SPACE_REST_PROBABILITY) {
                &QUARTER_REST
            } else {
                pick_cell(&pool, rng)
            }
        })
        .collect()
}

/// Write `cells` (one per beat) into a fresh measure via `set_beat`, so the
/// result carries the same invariants (tuplet groups, anchors) as an edited one.
fn build_measure(ts: TimeSignature, cells: &[&Cell]) -> Result<Measure, MeasureError> {
    let mut measure = Measure::new(ts);
    let mut idx = 0;
    for cell in cells {
        for &(duration, is_note) in cell.beats {
            let beat = Beat::new(duration, if is_note { Note } else { Rest });
            measure.set_beat(idx, beat)?;
            idx += 1;
        }
    }
    Ok(measure)
}

fn check_time_signature(ts: TimeSignature) -> Result<(), GeneratorError> {
    if ts.beat_unit == 4 && ts.beats > 0 {
        Ok(())
    } else {
        Err(GeneratorError::UnsupportedTimeSignature(ts))
    }
}

/// Generate a single measure according to `settings`.
pub fn generate_measure(
    settings: &GeneratorSettings,
    rng: &mut Rng,
) -> Result<Measure, GeneratorError> {
    check_time_signature(settings.time_signature)?;
    let cells = pick_cells(settings, rng);
    Ok(build_measure(settings.time_signature, &cells)?)
}

/// Generate a score of `settings.bars` measures.
pub fn generate_score(
    settings: &GeneratorSettings,
    rng: &mut Rng,
) -> Result<Score, GeneratorError> {
    let measures = (0..settings.bars.max(1))
        .map(|_| generate_measure(settings, rng))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Score { measures })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duration::{e, q, s, t8};
    use crate::grid::DEFAULT_GRID;

    fn settings(subdivision: Subdivision, complexity: u8) -> GeneratorSettings {
        GeneratorSettings { subdivision, complexity, ..GeneratorSettings::default() }
    }

    fn expected_beats(cells: &[&Cell]) -> Vec<Beat> {
        cells
            .iter()
            .flat_map(|c| c.beats.iter())
            .map(|&(d, is_note)| Beat::new(d, if is_note { Note } else { Rest }))
            .collect()
    }

    #[test]
    fn every_cell_spans_exactly_one_quarter() {
        let quarter = DEFAULT_GRID.ticks_of(&q()).unwrap();
        for cell in CELLS {
            let ticks: u32 =
                cell.beats.iter().map(|(d, _)| DEFAULT_GRID.ticks_of(d).unwrap()).sum();
            assert_eq!(ticks, quarter, "cell {:?}", cell.beats);
        }
    }

    #[test]
    fn every_cell_is_written_verbatim_at_every_beat_position() {
        for cell in CELLS {
            let cells = [cell; 4];
            let m = build_measure(TimeSignature::FOUR_FOUR, &cells).expect("cell must build");
            assert_eq!(m.beats(), &expected_beats(&cells), "cell {:?}", cell.beats);
        }
    }

    #[test]
    fn mixed_neighbours_are_written_verbatim() {
        // Every ordered pair of cells, so fills from one cell never leak into the next.
        for a in CELLS {
            for b in CELLS {
                let cells = [a, b, b, a];
                let m = build_measure(TimeSignature::FOUR_FOUR, &cells).expect("pair must build");
                assert_eq!(m.beats(), &expected_beats(&cells));
            }
        }
    }

    #[test]
    fn triplet_cells_form_one_group_per_beat() {
        let triplets = CELLS.iter().find(|c| c.family == CellFamily::Triplet).unwrap();
        let m = build_measure(TimeSignature::TWO_FOUR, &[triplets, triplets]).unwrap();
        assert_eq!(m.tuplet_groups().len(), 2);
        assert_ne!(m.beats()[0].tuplet_group_id, m.beats()[3].tuplet_group_id);
    }

    #[test]
    fn same_seed_gives_same_score() {
        let st = GeneratorSettings { bars: 4, ..settings(Subdivision::Mixed, 5) };
        let a = generate_score(&st, &mut Rng::new(42)).unwrap();
        let b = generate_score(&st, &mut Rng::new(42)).unwrap();
        for (ma, mb) in a.measures.iter().zip(&b.measures) {
            assert_eq!(ma.beats(), mb.beats());
        }
    }

    #[test]
    fn score_has_requested_bars_and_time_signature() {
        let st = GeneratorSettings {
            bars: 3,
            time_signature: TimeSignature::THREE_FOUR,
            ..settings(Subdivision::Sixteenths, 3)
        };
        let score = generate_score(&st, &mut Rng::new(1)).unwrap();
        assert_eq!(score.len(), 3);
        assert!(score.measures.iter().all(|m| m.time_signature() == TimeSignature::THREE_FOUR));
    }

    #[test]
    fn unsupported_time_signature_is_rejected() {
        let st = GeneratorSettings {
            time_signature: TimeSignature::SIX_EIGHT,
            ..GeneratorSettings::default()
        };
        assert_eq!(
            generate_measure(&st, &mut Rng::new(1)).unwrap_err(),
            GeneratorError::UnsupportedTimeSignature(TimeSignature::SIX_EIGHT)
        );
    }

    #[test]
    fn complexity_caps_the_cell_level() {
        let mut rng = Rng::new(7);
        for complexity in 1..=MAX_COMPLEXITY {
            let st = settings(Subdivision::Mixed, complexity);
            for _ in 0..200 {
                assert!(pick_cells(&st, &mut rng).iter().all(|c| c.level <= complexity));
            }
        }
    }

    #[test]
    fn highest_level_is_actually_used() {
        let mut rng = Rng::new(11);
        let st = settings(Subdivision::Sixteenths, 5);
        let top = (0..200).flat_map(|_| pick_cells(&st, &mut rng)).filter(|c| c.level == 5).count();
        // 4 beats * 200 measures; the top level carries 3/7 of the weight.
        assert!(top > 200, "top-level cells drawn: {top}");
    }

    #[test]
    fn subdivision_restricts_durations() {
        let mut rng = Rng::new(3);
        for _ in 0..200 {
            let eighths = generate_measure(&settings(Subdivision::Eighths, 5), &mut rng).unwrap();
            assert!(eighths.beats().iter().all(|b| b.duration == q() || b.duration == e()));

            let triplets = generate_measure(&settings(Subdivision::Triplets, 5), &mut rng).unwrap();
            assert!(triplets.beats().iter().all(|b| b.duration != s() && b.duration != de()));

            let sixteenths =
                generate_measure(&settings(Subdivision::Sixteenths, 5), &mut rng).unwrap();
            assert!(sixteenths.beats().iter().all(|b| b.duration != t8()));
        }
    }

    /// Triplet-cell patterns (`x` = note, `-` = rest) drawn at `complexity`.
    fn triplet_patterns(complexity: u8) -> std::collections::BTreeSet<String> {
        let mut rng = Rng::new(21);
        let st = settings(Subdivision::Triplets, complexity);
        (0..500)
            .flat_map(|_| pick_cells(&st, &mut rng))
            .map(|c| c.beats.iter().map(|&(_, n)| if n { 'x' } else { '-' }).collect())
            .collect()
    }

    fn set(items: &[&str]) -> std::collections::BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn triplet_ladder_matches_reference_app() {
        // "x" alone is the quarter note.
        assert_eq!(triplet_patterns(1), set(&["x", "xxx"]));
        // "-" alone is the quarter rest.
        assert_eq!(triplet_patterns(2), set(&["x", "-", "xxx", "x-x"]));
        assert_eq!(triplet_patterns(3), set(&["x", "-", "xxx", "x-x", "--x"]));
        assert_eq!(triplet_patterns(4), set(&["x", "-", "xxx", "x-x", "--x", "xx-", "-xx"]));
        assert_eq!(triplet_patterns(5), set(&["x", "-", "xxx", "x-x", "--x", "xx-", "-xx", "-x-"]));
    }

    #[test]
    fn eighth_ladder_is_rhythmically_distinct() {
        // Onset patterns per beat on an eighth grid: level 1 = x- (quarter)
        // and xx; level 2 adds -x. No cell repeats another's onsets.
        fn patterns(complexity: u8) -> std::collections::BTreeSet<String> {
            let mut rng = Rng::new(3);
            let st = settings(Subdivision::Eighths, complexity);
            (0..300)
                .flat_map(|_| pick_cells(&st, &mut rng))
                .map(|c| c.beats.iter().map(|&(_, n)| if n { 'x' } else { '-' }).collect())
                .collect()
        }
        // "-" alone is the quarter rest.
        assert_eq!(patterns(1), set(&["x", "xx", "-"]));
        assert_eq!(patterns(2), set(&["x", "xx", "-", "-x"]));
        assert_eq!(patterns(5), patterns(2));
    }

    #[test]
    fn sixteenth_ladder_covers_every_rhythm_once() {
        // Onsets on the 16th grid (x = note start). Quarter "x" = x---,
        // 8-8 "xx" = x-x-, 8-rest + 8 "-x" = --x-.
        fn onsets(c: &Cell) -> String {
            let sx = DEFAULT_GRID.ticks_of(&s()).unwrap();
            let mut slots = ['-'; 4];
            let mut t = 0;
            for &(d, n) in c.beats {
                if n {
                    slots[(t / sx) as usize] = 'x';
                }
                t += DEFAULT_GRID.ticks_of(&d).unwrap();
            }
            slots.iter().collect()
        }
        let pool = |level| {
            candidates(Subdivision::Sixteenths, level).into_iter().map(onsets).collect::<Vec<_>>()
        };
        // Levels 1-2 match the eighth subdivision; sixteenths start at 3.
        for level in [1, 2] {
            let eighths: Vec<String> =
                candidates(Subdivision::Eighths, level).into_iter().map(onsets).collect();
            assert_eq!(pool(level), eighths);
        }
        assert!(pool(3).contains(&"xxxx".to_string()));
        let all = pool(5);
        let unique: std::collections::BTreeSet<_> = all.iter().cloned().collect();
        assert_eq!(unique.len(), all.len(), "duplicate rhythms: {all:?}");
        // All 16 four-slot rhythms, the empty one being the quarter rest.
        assert_eq!(unique.len(), 16, "all 4-slot rhythms: {unique:?}");
    }

    #[test]
    fn sixteenth_levels_match_reference_groupings() {
        // Reference app "Custom Groupings", unlocked cumulatively per level.
        fn onsets(c: &Cell) -> String {
            let sx = DEFAULT_GRID.ticks_of(&s()).unwrap();
            let mut slots = ['-'; 4];
            let mut t = 0;
            for &(d, n) in c.beats {
                if n {
                    slots[(t / sx) as usize] = 'x';
                }
                t += DEFAULT_GRID.ticks_of(&d).unwrap();
            }
            slots.iter().collect()
        }
        let at = |level: u8| -> std::collections::BTreeSet<String> {
            candidates(Subdivision::Sixteenths, level)
                .into_iter()
                .filter(|c| c.level == level)
                .map(onsets)
                .collect()
        };
        assert_eq!(at(1), set(&["----", "x---", "x-x-"]));
        assert_eq!(at(2), set(&["--x-"]));
        assert_eq!(at(3), set(&["xxxx", "x-xx", "xxx-"]));
        assert_eq!(at(4), set(&["xx-x", "x--x", "--xx", "xx--"]));
        assert_eq!(at(5), set(&["-xx-", "-xxx", "-x-x", "-x--", "---x"]));
    }

    #[test]
    fn triplet_level_one_without_space_has_no_rests() {
        let mut rng = Rng::new(5);
        for sub in [Subdivision::Triplets] {
            for _ in 0..200 {
                let m = generate_measure(&settings(sub, 1), &mut rng).unwrap();
                assert!(m.beats().iter().all(|b| b.kind == Note), "{sub:?}: {m:?}");
            }
        }
    }

    #[test]
    fn space_only_replaces_whole_beats() {
        // With space, every beat is either a complete library cell or a
        // quarter rest; triplets are never cut apart.
        let mut rng = Rng::new(13);
        let st = GeneratorSettings { space: 1.0, ..settings(Subdivision::Triplets, 1) };
        for _ in 0..300 {
            for cell in pick_cells(&st, &mut rng) {
                let is_space = std::ptr::eq(cell, &QUARTER_REST);
                assert!(is_space || cell.beats.iter().all(|&(_, n)| n), "{:?}", cell.beats);
            }
        }
    }

    #[test]
    fn space_increases_rests() {
        fn rest_ratio(space: f32) -> f32 {
            let mut rng = Rng::new(9);
            let st = GeneratorSettings { space, ..settings(Subdivision::Sixteenths, 3) };
            let mut rest_beats = 0;
            for _ in 0..300 {
                let m = generate_measure(&st, &mut rng).unwrap();
                rest_beats +=
                    m.beats().iter().filter(|b| b.kind == Rest && b.duration == q()).count();
            }
            rest_beats as f32 / (300.0 * 4.0)
        }
        assert!(rest_ratio(1.0) > rest_ratio(0.0) + 0.5);
    }

    #[test]
    fn full_space_can_produce_an_empty_measure() {
        let mut rng = Rng::new(17);
        let st = GeneratorSettings { space: 1.0, ..settings(Subdivision::Sixteenths, 3) };
        let empty = (0..100)
            .map(|_| generate_measure(&st, &mut rng).unwrap())
            .filter(|m| m.beats().iter().all(|b| b.kind == Rest))
            .count();
        assert!(empty > 0, "expected some all-rest measures at maximum space");
    }

    #[test]
    fn rng_unit_stays_in_range() {
        let mut rng = Rng::new(0);
        for _ in 0..10_000 {
            let u = rng.unit();
            assert!((0.0..1.0).contains(&u));
        }
    }
}
