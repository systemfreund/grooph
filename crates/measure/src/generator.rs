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
//! 1. `xxx`, quarter note or quarter rest
//! 2. + `x-x`
//! 3. + `--x`
//! 4. + `xx-`, `-xx`
//! 5. + `-x-`
//!
//! Besides the level-1 quarter rest, [`GeneratorSettings::space`] adds
//! whole-beat rests: it replaces entire beats (never part of a figure, so a
//! triplet is always complete) with a quarter rest. At
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
    /// Draw from [`Self::custom_groupings`] instead of the complexity level.
    #[serde(default)]
    pub custom_groupings_enabled: bool,
    /// Hand-picked groupings; kept while custom groupings are switched off.
    #[serde(default)]
    pub custom_groupings: GroupingSet,
}

impl Default for GeneratorSettings {
    fn default() -> Self {
        Self {
            subdivision: Subdivision::Sixteenths,
            complexity: 1,
            bars: 1,
            time_signature: TimeSignature::FOUR_FOUR,
            space: 0.0,
            custom_groupings_enabled: false,
            custom_groupings: GroupingSet::default(),
        }
    }
}

/// Stable identifier of a grouping (one-beat figure) in the catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GroupingId(pub u8);

/// A set of groupings, stored as a bit mask over [`GroupingId`]s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupingSet(u32);

impl GroupingSet {
    pub fn contains(&self, id: GroupingId) -> bool { self.0 & (1 << id.0) != 0 }
    pub fn insert(&mut self, id: GroupingId) {
        self.0 |= 1 << id.0;
    }
    pub fn remove(&mut self, id: GroupingId) {
        self.0 &= !(1 << id.0);
    }
    pub fn toggle(&mut self, id: GroupingId) {
        self.0 ^= 1 << id.0;
    }
    pub fn is_empty(&self) -> bool { self.0 == 0 }
}

/// One entry of the grouping catalog, for display and selection.
#[derive(Clone, Copy, Debug)]
pub struct Grouping {
    pub id: GroupingId,
    /// Complexity level that unlocks this grouping.
    pub level: u8,
    cell: &'static Cell,
}

impl Grouping {
    /// The grouping written into a one-beat (1/4) measure, e.g. for a thumbnail.
    pub fn measure(&self) -> Measure {
        build_measure(TimeSignature::ONE_FOUR, &[self.cell])
            .expect("library cells always fit one beat")
    }
}

/// All groupings available with `subdivision`, in display order.
pub fn groupings(subdivision: Subdivision) -> Vec<Grouping> {
    catalog(subdivision)
        .into_iter()
        .map(|cell| Grouping { id: cell.id, level: cell.level, cell })
        .collect()
}

/// The groupings a complexity level unlocks: the level is a preset subset of
/// the catalog.
pub fn complexity_groupings(subdivision: Subdivision, complexity: u8) -> GroupingSet {
    let mut set = GroupingSet::default();
    for c in candidates(subdivision, complexity.clamp(1, MAX_COMPLEXITY)) {
        set.insert(c.id);
    }
    set
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
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

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
    /// A plain quarter note or quarter rest; available to every subdivision.
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
    /// Stable grouping id (bit index in [`GroupingSet`]); persisted, never reuse.
    id: GroupingId,
    level: u8,
    family: CellFamily,
    beats: &'static [(Duration, bool)],
}

const fn de() -> Duration {
    Duration::Dotted { base: NoteValue::Eighth, dots: 1 }
}

const N: bool = true;
const R: bool = false;

/// Whole-beat rest inserted by the "space" setting; not part of the library.
static QUARTER_REST: Cell =
    Cell { id: g(0), level: 1, family: CellFamily::Quarter, beats: &[(q(), R)] };

const fn g(id: u8) -> GroupingId { GroupingId(id) }

/// Groupings in the reference app's display order ("Custom Groupings"
/// screen, read left to right). Complexity N unlocks every grouping whose
/// level is at most N.
#[rustfmt::skip]
const CELLS: &[Cell] = &[
    // Level 1: quarter rest, quarter, two eighths.
    Cell { id: g(0), level: 1, family: CellFamily::Quarter, beats: &[(q(), R)] },
    Cell { id: g(1), level: 1, family: CellFamily::Quarter, beats: &[(q(), N)] },
    Cell { id: g(2), level: 1, family: CellFamily::Eighth, beats: &[(e(), N), (e(), N)] },
    // Level 2: eighth syncopation. (Eighth + eighth rest is left out on
    // purpose: rhythmically it is the same single hit as a quarter note.)
    Cell { id: g(3), level: 2, family: CellFamily::Eighth, beats: &[(e(), R), (e(), N)] },
    // Sixteenths (onsets on 1 e & a; one spelling per rhythm).
    // Level 3: sixteenth figures on the beat, no inner rests.
    Cell { id: g(4), level: 3, family: CellFamily::Sixteenth, beats: &[(s(), N), (s(), N), (s(), N), (s(), N)] },
    Cell { id: g(5), level: 3, family: CellFamily::Sixteenth, beats: &[(e(), N), (s(), N), (s(), N)] },
    Cell { id: g(6), level: 3, family: CellFamily::Sixteenth, beats: &[(s(), N), (s(), N), (e(), N)] },
    // Level 4: dotted figures, rests inside the beat.
    Cell { id: g(7), level: 4, family: CellFamily::Sixteenth, beats: &[(s(), N), (e(), N), (s(), N)] },
    Cell { id: g(8), level: 4, family: CellFamily::Sixteenth, beats: &[(de(), N), (s(), N)] },
    Cell { id: g(9), level: 4, family: CellFamily::Sixteenth, beats: &[(e(), R), (s(), N), (s(), N)] },
    Cell { id: g(10), level: 4, family: CellFamily::Sixteenth, beats: &[(s(), N), (s(), N), (e(), R)] },
    // Level 5: notes on "e" and "a".
    Cell { id: g(11), level: 5, family: CellFamily::Sixteenth, beats: &[(s(), R), (s(), N), (e(), N)] },
    Cell { id: g(12), level: 5, family: CellFamily::Sixteenth, beats: &[(s(), R), (s(), N), (s(), N), (s(), N)] },
    Cell { id: g(13), level: 5, family: CellFamily::Sixteenth, beats: &[(s(), R), (e(), N), (s(), N)] },
    Cell { id: g(14), level: 5, family: CellFamily::Sixteenth, beats: &[(s(), R), (de(), N)] },
    Cell { id: g(15), level: 5, family: CellFamily::Sixteenth, beats: &[(de(), R), (s(), N)] },
    // Triplets (eighth-note triplets per quarter).
    Cell { id: g(16), level: 1, family: CellFamily::Triplet, beats: &[(t8(), N), (t8(), N), (t8(), N)] },
    Cell { id: g(17), level: 2, family: CellFamily::Triplet, beats: &[(t8(), N), (t8(), R), (t8(), N)] },
    Cell { id: g(18), level: 3, family: CellFamily::Triplet, beats: &[(t8(), R), (t8(), R), (t8(), N)] },
    Cell { id: g(19), level: 4, family: CellFamily::Triplet, beats: &[(t8(), N), (t8(), N), (t8(), R)] },
    Cell { id: g(20), level: 4, family: CellFamily::Triplet, beats: &[(t8(), R), (t8(), N), (t8(), N)] },
    Cell { id: g(21), level: 5, family: CellFamily::Triplet, beats: &[(t8(), R), (t8(), N), (t8(), R)] },
];

/// Weight of the highest allowed level relative to each easier level.
const TOP_LEVEL_WEIGHT: usize = 3;

/// Probability of a whole-beat rest at `space == 1.0` (a 4/4 measure is then
/// entirely rests about a third of the time).
const MAX_SPACE_REST_PROBABILITY: f32 = 0.75;

/// Library cells usable with `subdivision`, one per grouping id (the first in
/// catalog order wins, so a shared id keeps its lowest unlock level).
fn catalog(subdivision: Subdivision) -> Vec<&'static Cell> {
    let mut seen = GroupingSet::default();
    CELLS
        .iter()
        .filter(|c| c.family.allowed_in(subdivision))
        .filter(|c| {
            let new = !seen.contains(c.id);
            seen.insert(c.id);
            new
        })
        .collect()
}

fn candidates(subdivision: Subdivision, max_level: u8) -> Vec<&'static Cell> {
    catalog(subdivision).into_iter().filter(|c| c.level <= max_level).collect()
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
    let custom: Vec<&'static Cell> = if settings.custom_groupings_enabled {
        catalog(settings.subdivision)
            .into_iter()
            .filter(|c| settings.custom_groupings.contains(c.id))
            .collect()
    } else {
        Vec::new()
    };
    // Custom groupings are drawn uniformly; an empty selection falls back to
    // the complexity level so the generator never runs dry.
    let (pool, weighted) = if custom.is_empty() {
        (candidates(settings.subdivision, complexity), true)
    } else {
        (custom, false)
    };
    let beat_count = settings.time_signature.beats as usize;

    (0..beat_count)
        .map(|_| {
            if rng.chance(space * MAX_SPACE_REST_PROBABILITY) {
                &QUARTER_REST
            } else if weighted {
                pick_cell(&pool, rng)
            } else {
                pool[rng.below(pool.len())]
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
        // "x" alone is the quarter note, "-" alone the quarter rest.
        assert_eq!(triplet_patterns(1), set(&["x", "-", "xxx"]));
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
    fn space_only_replaces_whole_beats() {
        // With space, every beat is either a complete library cell or a
        // quarter rest; triplets are never cut apart.
        let mut rng = Rng::new(13);
        let st = GeneratorSettings { space: 1.0, ..settings(Subdivision::Triplets, 1) };
        for _ in 0..300 {
            for cell in pick_cells(&st, &mut rng) {
                let whole_rest = cell.beats == [(q(), R)];
                assert!(whole_rest || cell.beats.iter().all(|&(_, n)| n), "{:?}", cell.beats);
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
    fn grouping_ids_are_unique_per_subdivision() {
        for sub in [
            Subdivision::Eighths,
            Subdivision::Sixteenths,
            Subdivision::Triplets,
            Subdivision::Mixed,
        ] {
            let ids: Vec<_> = groupings(sub).iter().map(|g| g.id).collect();
            let unique: std::collections::HashSet<_> = ids.iter().collect();
            assert_eq!(ids.len(), unique.len(), "{sub:?}");
            assert!(ids.iter().all(|id| id.0 < 32));
        }
        assert_eq!(groupings(Subdivision::Sixteenths).len(), 16);
        assert_eq!(groupings(Subdivision::Eighths).len(), 4);
        // Triplets: rest, quarter and six triplet figures; the rest comes
        // first in every subdivision.
        assert_eq!(groupings(Subdivision::Triplets).len(), 8);
        for sub in [Subdivision::Eighths, Subdivision::Sixteenths, Subdivision::Triplets] {
            let first = groupings(sub)[0].measure();
            assert!(first.beats().iter().all(|b| b.kind == Rest), "{sub:?}");
        }
    }

    #[test]
    fn complexity_is_a_growing_subset_of_the_catalog() {
        let sub = Subdivision::Sixteenths;
        let mut prev = GroupingSet::default();
        for level in 1..=MAX_COMPLEXITY {
            let set = complexity_groupings(sub, level);
            assert_eq!(set.0 & prev.0, prev.0, "level {level} must contain level {}", level - 1);
            prev = set;
        }
        let all = groupings(sub).iter().fold(GroupingSet::default(), |mut s, g| {
            s.insert(g.id);
            s
        });
        assert_eq!(prev, all);
    }

    #[test]
    fn custom_groupings_restrict_the_pool() {
        let mut custom = GroupingSet::default();
        let pick = groupings(Subdivision::Sixteenths)[8]; // dotted eighth + sixteenth
        custom.insert(pick.id);
        let st = GeneratorSettings {
            custom_groupings_enabled: true,
            custom_groupings: custom,
            ..settings(Subdivision::Sixteenths, 1)
        };
        let mut rng = Rng::new(4);
        for _ in 0..100 {
            let m = generate_measure(&st, &mut rng).unwrap();
            let expected: Vec<Beat> =
                std::iter::repeat_n(pick.measure().beats().clone(), 4).flatten().collect();
            assert_eq!(m.beats(), &expected);
        }
    }

    #[test]
    fn empty_or_disabled_custom_selection_uses_complexity() {
        let mut custom = GroupingSet::default();
        custom.insert(groupings(Subdivision::Sixteenths)[15].id);
        let off =
            GeneratorSettings { custom_groupings: custom, ..settings(Subdivision::Sixteenths, 1) };
        let empty = GeneratorSettings {
            custom_groupings_enabled: true,
            ..settings(Subdivision::Sixteenths, 1)
        };
        let mut rng = Rng::new(8);
        for st in [off, empty] {
            for _ in 0..100 {
                assert!(pick_cells(&st, &mut rng).iter().all(|c| c.level == 1));
            }
        }
    }

    #[test]
    fn custom_selection_outside_subdivision_is_ignored() {
        // A triplet grouping selected while generating sixteenths: falls back.
        let mut custom = GroupingSet::default();
        custom.insert(groupings(Subdivision::Triplets).iter().find(|g| g.level == 5).unwrap().id);
        let st = GeneratorSettings {
            custom_groupings_enabled: true,
            custom_groupings: custom,
            ..settings(Subdivision::Sixteenths, 2)
        };
        let mut rng = Rng::new(2);
        assert!(pick_cells(&st, &mut rng).iter().all(|c| c.level <= 2));
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
