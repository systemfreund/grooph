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
//! With the mixed subdivision the triplet ladder is reversed: 1 `-x-`,
//! 2 + `--x`, 3 + `x-x`, 4 + `xx-`, `-xx`, 5 + `xxx`. Straight figures keep
//! their levels.
//!
//! Besides the level-1 quarter rest, [`GeneratorSettings::space`] adds
//! whole-beat rests: it replaces entire beats (never part of a figure, so a
//! triplet is always complete) with a quarter rest. At
//! maximum space a measure may consist of rests only.
//!
//! In `x/8` time signatures a "beat" is a beat group of the meter (see
//! [`beat_groups`]): 6/8 is 3+3, 7/8 2+3+2 and so on. Two-eighth groups use the
//! straight quarter-beat cells; three-eighth groups (a dotted quarter) have
//! their own cells, written straight. With eighths their ladder follows the
//! triplet ladder (`xxx`, `x-x`, `--x`, `xx-` / `-xx`, `-x-`); with
//! sixteenths the eighth figures sit on levels 1-2 (offbeat starts higher) and
//! sixteenth figures start at level 3. Triplets and mixed are not offered in
//! `x/8`; they fall back to eighths and sixteenths (see
//! [`effective_subdivision`]).
//!
//! `x/16` works the same way one note value down and always in sixteenths:
//! two-sixteenth groups (an eighth: `x-`, `xx`, then `-x`) and
//! three-sixteenth groups (a dotted eighth) with the triplet ladder.

use crate::BeatKind::{Note, Rest};
use crate::duration::{Duration, NoteValue, e, q, s, t8};
use crate::grouping::default_groups_for;
use crate::{Beat, Measure, MeasureError, Score, TimeSignature};
use serde::{Deserialize, Serialize};

/// Highest supported complexity level.
pub const MAX_COMPLEXITY: u8 = 5;

/// Most bars the generator offers.
pub const MAX_BARS: usize = 8;

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
pub struct GroupingSet(u64);

impl GroupingSet {
    pub fn contains(&self, id: GroupingId) -> bool {
        self.0 & (1 << id.0) != 0
    }
    pub fn insert(&mut self, id: GroupingId) {
        self.0 |= 1 << id.0;
    }
    pub fn remove(&mut self, id: GroupingId) {
        self.0 &= !(1 << id.0);
    }
    pub fn toggle(&mut self, id: GroupingId) {
        self.0 ^= 1 << id.0;
    }
    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }
}

/// Length of one generator beat: a quarter (`x/4` beats and two-eighth
/// groups in `x/8`), a dotted quarter (three-eighth groups), an eighth or a
/// dotted eighth (two- and three-sixteenth groups in `x/16`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BeatSpan {
    Quarter,
    DottedQuarter,
    Eighth,
    DottedEighth,
}

impl BeatSpan {
    pub const ALL: [BeatSpan; 4] =
        [BeatSpan::Quarter, BeatSpan::DottedQuarter, BeatSpan::Eighth, BeatSpan::DottedEighth];

    /// The span written as its own measure, e.g. for a thumbnail.
    pub fn time_signature(self) -> TimeSignature {
        let (beats, beat_unit) = match self {
            BeatSpan::Quarter => (1, 4),
            BeatSpan::DottedQuarter => (3, 8),
            BeatSpan::Eighth => (2, 16),
            BeatSpan::DottedEighth => (3, 16),
        };
        TimeSignature::new(beats, beat_unit)
    }

    /// Length of the span in beats of `beat_unit`, e.g. 2 for a quarter in
    /// `x/8` or 3 for a dotted eighth in `x/16`.
    pub fn beats_of(self, beat_unit: u8) -> u32 {
        let sixteenths: u32 = match self {
            BeatSpan::Quarter => 4,
            BeatSpan::DottedQuarter => 6,
            BeatSpan::Eighth => 2,
            BeatSpan::DottedEighth => 3,
        };
        sixteenths * beat_unit as u32 / 16
    }

    /// Whole-span rest for the "space" setting.
    fn rest(self) -> &'static Cell {
        match self {
            BeatSpan::Quarter => &QUARTER_REST,
            BeatSpan::DottedQuarter => &DOTTED_QUARTER_REST,
            BeatSpan::Eighth => &EIGHTH_REST,
            BeatSpan::DottedEighth => &DOTTED_EIGHTH_REST,
        }
    }
}

/// One entry of the grouping catalog, for display and selection.
#[derive(Clone, Copy, Debug)]
pub struct Grouping {
    pub id: GroupingId,
    /// Complexity level that unlocks this grouping.
    pub level: u8,
    pub span: BeatSpan,
    cell: &'static Cell,
}

impl Grouping {
    /// The grouping written into a one-beat measure (1/4, or 3/8 for a
    /// three-eighth group), e.g. for a thumbnail.
    pub fn measure(&self) -> Measure {
        build_measure(self.span.time_signature(), &[self.cell])
            .expect("library cells always fit one beat")
    }
}

/// Subdivision actually used in `ts`: triplets and mixed have no cells for
/// `x/8` beat groups and fall back to eighths and sixteenths; `x/16` is
/// always written in sixteenths.
pub fn effective_subdivision(subdivision: Subdivision, ts: TimeSignature) -> Subdivision {
    match (ts.beat_unit, subdivision) {
        (8, Subdivision::Triplets) => Subdivision::Eighths,
        (8, Subdivision::Mixed) => Subdivision::Sixteenths,
        (16, _) => Subdivision::Sixteenths,
        _ => subdivision,
    }
}

/// Subdivisions the generator offers for `ts`: triplets and mixed have no x/8
/// cells, x/16 is always written in sixteenths.
pub fn offered_subdivisions(ts: TimeSignature) -> &'static [Subdivision] {
    match ts.beat_unit {
        8 => &[Subdivision::Eighths, Subdivision::Sixteenths],
        16 => &[Subdivision::Sixteenths],
        _ => &[
            Subdivision::Eighths,
            Subdivision::Sixteenths,
            Subdivision::Triplets,
            Subdivision::Mixed,
        ],
    }
}

/// Generator beats of `ts`, in order: one quarter per beat in `x/4`, the
/// meter's beat groups in `x/8` and `x/16` (groups longer than three are
/// split into twos, with a three at the end for an odd length). `None` if
/// the generator has no cells for `ts`.
pub fn beat_groups(ts: TimeSignature) -> Option<Vec<BeatSpan>> {
    let (two, three) = match ts.beat_unit {
        4 if ts.beats > 0 => return Some(vec![BeatSpan::Quarter; ts.beats as usize]),
        8 if ts.beats >= 2 => (BeatSpan::Quarter, BeatSpan::DottedQuarter),
        16 if ts.beats >= 2 => (BeatSpan::Eighth, BeatSpan::DottedEighth),
        _ => return None,
    };
    let mut spans = Vec::new();
    for size in default_groups_for(&ts) {
        if size < 2 {
            return None;
        }
        let threes = size % 2;
        let twos = (size - 3 * threes) / 2;
        spans.extend(std::iter::repeat_n(two, twos as usize));
        spans.extend(std::iter::repeat_n(three, threes as usize));
    }
    Some(spans)
}

/// Whether the generator can write measures in `ts`.
pub fn supports_time_signature(ts: TimeSignature) -> bool {
    beat_groups(ts).is_some()
}

/// All groupings available with `subdivision` in `ts`, in display order: only
/// the beat spans `ts` uses (quarter-beat figures first).
pub fn groupings(subdivision: Subdivision, ts: TimeSignature) -> Vec<Grouping> {
    let subdivision = effective_subdivision(subdivision, ts);
    let spans = beat_groups(ts).unwrap_or_default();
    catalog(subdivision)
        .into_iter()
        .filter(|cell| spans.contains(&cell.span()))
        .map(|cell| Grouping {
            id: cell.id,
            level: cell.level_in(subdivision),
            span: cell.span(),
            cell,
        })
        .collect()
}

/// The groupings a complexity level unlocks in `ts`: the level is a preset
/// subset of the catalog.
pub fn complexity_groupings(
    subdivision: Subdivision,
    ts: TimeSignature,
    complexity: u8,
) -> GroupingSet {
    let complexity = complexity.clamp(1, MAX_COMPLEXITY);
    let mut set = GroupingSet::default();
    for g in groupings(subdivision, ts).into_iter().filter(|g| g.level <= complexity) {
        set.insert(g.id);
    }
    set
}

#[derive(Debug, PartialEq)]
pub enum GeneratorError {
    /// The generator has no cells for this time signature (see
    /// [`supports_time_signature`]).
    UnsupportedTimeSignature(TimeSignature),
    /// Writing a cell into the measure failed (library bug).
    Measure(MeasureError),
}

impl From<MeasureError> for GeneratorError {
    fn from(err: MeasureError) -> Self {
        GeneratorError::Measure(err)
    }
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
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// Uniform float in `[0, 1)`.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CellFamily {
    /// A plain quarter note or quarter rest; available to every subdivision.
    Quarter,
    /// Straight eighths; not used with the triplet subdivision.
    Eighth,
    Sixteenth,
    Triplet,
    /// Eighth figures over a three-eighth group (dotted quarter) in `x/8`.
    CompoundEighth,
    /// Sixteenth figures over a three-eighth group.
    CompoundSixteenth,
    /// Figures over a two-sixteenth group (an eighth) in `x/16`.
    SixteenthPair,
    /// Figures over a three-sixteenth group (a dotted eighth) in `x/16`.
    SixteenthTriple,
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
            CellFamily::CompoundEighth => {
                matches!(subdivision, Subdivision::Eighths | Subdivision::Sixteenths)
            }
            CellFamily::CompoundSixteenth
            | CellFamily::SixteenthPair
            | CellFamily::SixteenthTriple => subdivision == Subdivision::Sixteenths,
        }
    }

    fn span(self) -> BeatSpan {
        match self {
            CellFamily::CompoundEighth | CellFamily::CompoundSixteenth => BeatSpan::DottedQuarter,
            CellFamily::SixteenthPair => BeatSpan::Eighth,
            CellFamily::SixteenthTriple => BeatSpan::DottedEighth,
            _ => BeatSpan::Quarter,
        }
    }
}

/// A figure spanning exactly one generator beat ([`BeatSpan`]).
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

const fn dq() -> Duration {
    Duration::Dotted { base: NoteValue::Quarter, dots: 1 }
}

const N: bool = true;
const R: bool = false;

/// Whole-beat rest inserted by the "space" setting; not part of the library.
static QUARTER_REST: Cell =
    Cell { id: g(0), level: 1, family: CellFamily::Quarter, beats: &[(q(), R)] };

/// Whole-group rest for three-eighth groups; same as grouping 22.
static DOTTED_QUARTER_REST: Cell =
    Cell { id: g(22), level: 1, family: CellFamily::CompoundEighth, beats: &[(dq(), R)] };

/// Whole-group rests for two- and three-sixteenth groups (groupings 41, 45).
static EIGHTH_REST: Cell =
    Cell { id: g(41), level: 1, family: CellFamily::SixteenthPair, beats: &[(e(), R)] };
static DOTTED_EIGHTH_REST: Cell =
    Cell { id: g(45), level: 1, family: CellFamily::SixteenthTriple, beats: &[(de(), R)] };

const fn g(id: u8) -> GroupingId {
    GroupingId(id)
}

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
    // Three-eighth groups in x/8 (dotted-quarter beat), written straight.
    // Eighths follow the triplet ladder: xxx, x-x, --x, xx- / -xx, -x-.
    Cell { id: g(22), level: 1, family: CellFamily::CompoundEighth, beats: &[(dq(), R)] },
    Cell { id: g(23), level: 1, family: CellFamily::CompoundEighth, beats: &[(dq(), N)] },
    Cell { id: g(24), level: 1, family: CellFamily::CompoundEighth, beats: &[(e(), N), (e(), N), (e(), N)] },
    Cell { id: g(25), level: 2, family: CellFamily::CompoundEighth, beats: &[(q(), N), (e(), N)] },
    Cell { id: g(26), level: 3, family: CellFamily::CompoundEighth, beats: &[(q(), R), (e(), N)] },
    Cell { id: g(27), level: 4, family: CellFamily::CompoundEighth, beats: &[(e(), N), (q(), N)] },
    Cell { id: g(28), level: 4, family: CellFamily::CompoundEighth, beats: &[(e(), R), (e(), N), (e(), N)] },
    Cell { id: g(29), level: 5, family: CellFamily::CompoundEighth, beats: &[(e(), R), (q(), N)] },
    // Sixteenths in three-eighth groups.
    // Level 3: sixteenths on the beat, no inner rests.
    Cell { id: g(30), level: 3, family: CellFamily::CompoundSixteenth, beats: &[(s(), N), (s(), N), (s(), N), (s(), N), (s(), N), (s(), N)] },
    Cell { id: g(31), level: 3, family: CellFamily::CompoundSixteenth, beats: &[(s(), N), (s(), N), (e(), N), (e(), N)] },
    Cell { id: g(32), level: 3, family: CellFamily::CompoundSixteenth, beats: &[(e(), N), (s(), N), (s(), N), (e(), N)] },
    Cell { id: g(33), level: 3, family: CellFamily::CompoundSixteenth, beats: &[(e(), N), (e(), N), (s(), N), (s(), N)] },
    // Level 4: dotted figures and longer notes.
    Cell { id: g(34), level: 4, family: CellFamily::CompoundSixteenth, beats: &[(de(), N), (s(), N), (e(), N)] },
    Cell { id: g(35), level: 4, family: CellFamily::CompoundSixteenth, beats: &[(s(), N), (s(), N), (s(), N), (s(), N), (e(), N)] },
    Cell { id: g(36), level: 4, family: CellFamily::CompoundSixteenth, beats: &[(e(), N), (s(), N), (s(), N), (s(), N), (s(), N)] },
    Cell { id: g(37), level: 4, family: CellFamily::CompoundSixteenth, beats: &[(q(), N), (s(), N), (s(), N)] },
    // Level 5: rests inside the group.
    Cell { id: g(38), level: 5, family: CellFamily::CompoundSixteenth, beats: &[(s(), R), (s(), N), (e(), N), (e(), N)] },
    Cell { id: g(39), level: 5, family: CellFamily::CompoundSixteenth, beats: &[(e(), R), (s(), N), (s(), N), (e(), N)] },
    Cell { id: g(40), level: 5, family: CellFamily::CompoundSixteenth, beats: &[(de(), N), (s(), N), (s(), N), (s(), N)] },
    // Two-sixteenth groups in x/16 (an eighth).
    Cell { id: g(41), level: 1, family: CellFamily::SixteenthPair, beats: &[(e(), R)] },
    Cell { id: g(42), level: 1, family: CellFamily::SixteenthPair, beats: &[(e(), N)] },
    Cell { id: g(43), level: 1, family: CellFamily::SixteenthPair, beats: &[(s(), N), (s(), N)] },
    Cell { id: g(44), level: 2, family: CellFamily::SixteenthPair, beats: &[(s(), R), (s(), N)] },
    // Three-sixteenth groups in x/16 (a dotted eighth), triplet ladder.
    Cell { id: g(45), level: 1, family: CellFamily::SixteenthTriple, beats: &[(de(), R)] },
    Cell { id: g(46), level: 1, family: CellFamily::SixteenthTriple, beats: &[(de(), N)] },
    Cell { id: g(47), level: 1, family: CellFamily::SixteenthTriple, beats: &[(s(), N), (s(), N), (s(), N)] },
    Cell { id: g(48), level: 2, family: CellFamily::SixteenthTriple, beats: &[(e(), N), (s(), N)] },
    Cell { id: g(49), level: 3, family: CellFamily::SixteenthTriple, beats: &[(e(), R), (s(), N)] },
    Cell { id: g(50), level: 4, family: CellFamily::SixteenthTriple, beats: &[(s(), N), (e(), N)] },
    Cell { id: g(51), level: 4, family: CellFamily::SixteenthTriple, beats: &[(s(), R), (s(), N), (s(), N)] },
    Cell { id: g(52), level: 5, family: CellFamily::SixteenthTriple, beats: &[(s(), R), (e(), N)] },
];

/// Triplet levels with the mixed subdivision (reference app): the offbeat
/// figures come first, the full triplet last.
const MIXED_TRIPLET_LEVELS: &[(GroupingId, u8)] =
    &[(g(21), 1), (g(18), 2), (g(17), 3), (g(19), 4), (g(20), 4), (g(16), 5)];

/// Levels of the three-eighth eighth figures with the sixteenth subdivision:
/// like the quarter-beat ladder, eighths take levels 1-2 and only the
/// figures starting with a rest come later.
const COMPOUND_SIXTEENTH_LEVELS: &[(GroupingId, u8)] =
    &[(g(25), 2), (g(26), 2), (g(27), 2), (g(28), 4), (g(29), 5)];

impl Cell {
    /// Complexity level that unlocks this cell with `subdivision`.
    fn level_in(&self, subdivision: Subdivision) -> u8 {
        let table = match (subdivision, self.family) {
            (Subdivision::Mixed, CellFamily::Triplet) => MIXED_TRIPLET_LEVELS,
            (Subdivision::Sixteenths, CellFamily::CompoundEighth) => COMPOUND_SIXTEENTH_LEVELS,
            _ => return self.level,
        };
        table.iter().find(|(id, _)| *id == self.id).map_or(self.level, |&(_, level)| level)
    }

    fn span(&self) -> BeatSpan {
        self.family.span()
    }
}

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

fn candidates(subdivision: Subdivision, span: BeatSpan, max_level: u8) -> Vec<&'static Cell> {
    catalog(subdivision)
        .into_iter()
        .filter(|c| c.span() == span && c.level_in(subdivision) <= max_level)
        .collect()
}

/// Draw one cell: pick a level (favouring the highest one available), then a
/// cell of that level uniformly.
fn pick_cell(pool: &[&'static Cell], subdivision: Subdivision, rng: &mut Rng) -> &'static Cell {
    let mut levels: Vec<u8> = pool.iter().map(|c| c.level_in(subdivision)).collect();
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

    let at_level: Vec<&'static Cell> =
        pool.iter().copied().filter(|c| c.level_in(subdivision) == level).collect();
    at_level[rng.below(at_level.len())]
}

/// Where cells of one span are drawn from.
struct Pool {
    cells: Vec<&'static Cell>,
    /// Weighted by level (complexity) or uniform (custom groupings).
    weighted: bool,
}

impl Pool {
    fn new(settings: &GeneratorSettings, subdivision: Subdivision, span: BeatSpan) -> Self {
        let custom: Vec<&'static Cell> = if settings.custom_groupings_enabled {
            catalog(subdivision)
                .into_iter()
                .filter(|c| c.span() == span && settings.custom_groupings.contains(c.id))
                .collect()
        } else {
            Vec::new()
        };
        // Custom groupings are drawn uniformly; an empty selection falls back
        // to the complexity level so the generator never runs dry.
        if custom.is_empty() {
            let complexity = settings.complexity.clamp(1, MAX_COMPLEXITY);
            Self { cells: candidates(subdivision, span, complexity), weighted: true }
        } else {
            Self { cells: custom, weighted: false }
        }
    }
}

fn pick_cells(settings: &GeneratorSettings, rng: &mut Rng) -> Vec<&'static Cell> {
    let ts = settings.time_signature;
    let Some(spans) = beat_groups(ts) else {
        return Vec::new();
    };
    let subdivision = effective_subdivision(settings.subdivision, ts);
    let space = settings.space.clamp(0.0, 1.0);
    let pools = BeatSpan::ALL.map(|span| Pool::new(settings, subdivision, span));

    spans
        .into_iter()
        .map(|span| {
            let pool = &pools[BeatSpan::ALL.iter().position(|&s| s == span).unwrap()];
            if rng.chance(space * MAX_SPACE_REST_PROBABILITY) {
                span.rest()
            } else if pool.weighted {
                pick_cell(&pool.cells, subdivision, rng)
            } else {
                pool.cells[rng.below(pool.cells.len())]
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
    if supports_time_signature(ts) {
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

    #[test]
    fn effective_subdivision_is_always_offered() {
        let all = [
            Subdivision::Eighths,
            Subdivision::Sixteenths,
            Subdivision::Triplets,
            Subdivision::Mixed,
        ];
        for beat_unit in TimeSignature::BEAT_UNITS {
            for beats in TimeSignature::beat_range(beat_unit).into_iter().flatten() {
                let ts = TimeSignature::new(beats, beat_unit);
                if !supports_time_signature(ts) {
                    continue;
                }
                for s in all {
                    let used = effective_subdivision(s, ts);
                    assert!(offered_subdivisions(ts).contains(&used), "{s:?} in {ts}");
                }
            }
        }
    }

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

    fn cells_of(span: BeatSpan) -> impl Iterator<Item = &'static Cell> {
        CELLS.iter().filter(move |c| c.span() == span)
    }

    fn ts(beats: u8, beat_unit: u8) -> TimeSignature {
        TimeSignature::new(beats, beat_unit)
    }

    #[test]
    fn every_cell_spans_exactly_one_beat() {
        for cell in CELLS {
            let ticks: u32 =
                cell.beats.iter().map(|(d, _)| DEFAULT_GRID.ticks_of(d).unwrap()).sum();
            let span = DEFAULT_GRID.ticks_per_measure(&cell.span().time_signature());
            assert_eq!(ticks, span, "cell {:?}", cell.beats);
        }
    }

    #[test]
    fn every_cell_is_written_verbatim_at_every_beat_position() {
        for cell in cells_of(BeatSpan::Quarter) {
            let cells = [cell; 4];
            let m = build_measure(TimeSignature::FOUR_FOUR, &cells).expect("cell must build");
            assert_eq!(m.beats(), &expected_beats(&cells), "cell {:?}", cell.beats);
        }
        for cell in cells_of(BeatSpan::DottedQuarter) {
            let cells = [cell; 4];
            let m = build_measure(TimeSignature::TWELVE_EIGHT, &cells).expect("cell must build");
            assert_eq!(m.beats(), &expected_beats(&cells), "cell {:?}", cell.beats);
        }
        // 12/16 = 3+3+3+3, 10/16 = 3+3+2+2.
        for three in cells_of(BeatSpan::DottedEighth) {
            let cells = [three; 4];
            let m = build_measure(ts(12, 16), &cells).expect("cell must build");
            assert_eq!(m.beats(), &expected_beats(&cells), "cell {:?}", three.beats);
            for two in cells_of(BeatSpan::Eighth) {
                let cells = [three, three, two, two];
                let m = build_measure(ts(10, 16), &cells).expect("cell must build");
                assert_eq!(m.beats(), &expected_beats(&cells));
            }
        }
    }

    #[test]
    fn mixed_neighbours_are_written_verbatim() {
        // Every ordered pair of cells, so fills from one cell never leak into the next.
        for a in cells_of(BeatSpan::Quarter) {
            for b in cells_of(BeatSpan::Quarter) {
                let cells = [a, b, b, a];
                let m = build_measure(TimeSignature::FOUR_FOUR, &cells).expect("pair must build");
                assert_eq!(m.beats(), &expected_beats(&cells));
            }
        }
        for a in cells_of(BeatSpan::DottedQuarter) {
            for b in cells_of(BeatSpan::DottedQuarter) {
                let cells = [a, b];
                let m = build_measure(TimeSignature::SIX_EIGHT, &cells).expect("pair must build");
                assert_eq!(m.beats(), &expected_beats(&cells));
            }
        }
    }

    #[test]
    fn two_and_three_eighth_groups_mix_in_odd_meters() {
        // 5/8 is 3+2 and 7/8 2+3+2: every straight two-eighth cell next to
        // every three-eighth cell.
        for three in cells_of(BeatSpan::DottedQuarter) {
            for two in cells_of(BeatSpan::Quarter).filter(|c| c.family != CellFamily::Triplet) {
                let five = [three, two];
                let m = build_measure(TimeSignature::FIVE_EIGHT, &five).expect("5/8 must build");
                assert_eq!(m.beats(), &expected_beats(&five));
                let seven = [two, three, two];
                let m = build_measure(TimeSignature::SEVEN_EIGHT, &seven).expect("7/8 must build");
                assert_eq!(m.beats(), &expected_beats(&seven));
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
        for bad in [ts(1, 8), TimeSignature::ONE_SIXTEENTH, ts(0, 4), ts(2, 2)] {
            let st = GeneratorSettings { time_signature: bad, ..GeneratorSettings::default() };
            assert_eq!(
                generate_measure(&st, &mut Rng::new(1)).unwrap_err(),
                GeneratorError::UnsupportedTimeSignature(bad)
            );
        }
    }

    #[test]
    fn beat_groups_follow_the_meter() {
        use BeatSpan::{DottedQuarter as D, Quarter as Q};
        assert_eq!(beat_groups(TimeSignature::THREE_FOUR), Some(vec![Q, Q, Q]));
        assert_eq!(beat_groups(TimeSignature::SIX_EIGHT), Some(vec![D, D]));
        assert_eq!(beat_groups(TimeSignature::NINE_EIGHT), Some(vec![D, D, D]));
        assert_eq!(beat_groups(TimeSignature::TWELVE_EIGHT), Some(vec![D, D, D, D]));
        assert_eq!(beat_groups(TimeSignature::FIVE_EIGHT), Some(vec![D, Q]));
        assert_eq!(beat_groups(TimeSignature::SEVEN_EIGHT), Some(vec![Q, D, Q]));
        assert_eq!(beat_groups(ts(3, 8)), Some(vec![D]));
        assert_eq!(beat_groups(TimeSignature::FOUR_EIGHT), Some(vec![Q, Q]));
        assert_eq!(beat_groups(ts(8, 8)), Some(vec![D, D, Q]));
        assert_eq!(beat_groups(ts(10, 8)), Some(vec![D, D, Q, Q]));
        assert_eq!(beat_groups(ts(11, 8)), Some(vec![D, D, D, Q]));
        assert_eq!(beat_groups(ts(13, 8)), Some(vec![D, D, D, Q, Q]));
        // Every x/8 and x/16 meter the app offers can be generated.
        for unit in [4, 8, 16] {
            for beats in TimeSignature::beat_range(unit).unwrap() {
                assert!(supports_time_signature(ts(beats, unit)), "{beats}/{unit}");
            }
        }
        use BeatSpan::{DottedEighth as DE, Eighth as E};
        assert_eq!(beat_groups(ts(5, 16)), Some(vec![DE, E]));
        assert_eq!(beat_groups(ts(7, 16)), Some(vec![E, DE, E]));
        assert_eq!(beat_groups(ts(12, 16)), Some(vec![DE, DE, DE, DE]));
        // Spans add up to the measure.
        for unit in [4, 8, 16] {
            for beats in TimeSignature::beat_range(unit).unwrap() {
                let spans = beat_groups(ts(beats, unit)).unwrap();
                let sum: u32 = spans.iter().map(|g| g.beats_of(unit)).sum();
                assert_eq!(sum, beats as u32, "{beats}/{unit}");
            }
        }
    }

    /// Onsets of a cell on the sixteenth grid (`x` = note start).
    fn sixteenth_onsets(c: &Cell) -> String {
        let sx = DEFAULT_GRID.ticks_of(&s()).unwrap();
        let span = DEFAULT_GRID.ticks_per_measure(&c.span().time_signature());
        let mut slots = vec!['-'; (span / sx) as usize];
        let mut t = 0;
        for &(d, n) in c.beats {
            if n {
                slots[(t / sx) as usize] = 'x';
            }
            t += DEFAULT_GRID.ticks_of(&d).unwrap();
        }
        slots.iter().collect()
    }

    /// Onsets of a three-eighth cell on the eighth grid.
    fn eighth_onsets(c: &Cell) -> String {
        sixteenth_onsets(c).chars().step_by(2).collect()
    }

    #[test]
    fn compound_meters_generate_in_dotted_quarter_groups() {
        let eighth = DEFAULT_GRID.ticks_of(&e()).unwrap();
        for meter in
            [TimeSignature::SIX_EIGHT, TimeSignature::NINE_EIGHT, TimeSignature::TWELVE_EIGHT]
        {
            for sub in [Subdivision::Eighths, Subdivision::Sixteenths] {
                let st = GeneratorSettings {
                    bars: 4,
                    time_signature: meter,
                    ..settings(sub, MAX_COMPLEXITY)
                };
                let score = generate_score(&st, &mut Rng::new(5)).unwrap();
                for m in &score.measures {
                    assert_eq!(m.time_signature(), meter);
                    // No note or rest crosses a dotted-quarter boundary.
                    let onsets = DEFAULT_GRID.compute_onset_ticks(m.beats());
                    for (b, &t) in m.beats().iter().zip(&onsets) {
                        let end = t + DEFAULT_GRID.ticks_of(&b.duration).unwrap();
                        assert_eq!(t / (3 * eighth), (end - 1) / (3 * eighth), "{m:?}");
                        assert!(!b.duration.is_tuplet());
                    }
                }
            }
        }
    }

    #[test]
    fn every_x8_and_x16_meter_respects_its_beat_groups() {
        // No note or rest crosses a primary boundary (beam group / click).
        let meters = [8, 16]
            .into_iter()
            .flat_map(|unit| {
                TimeSignature::beat_range(unit).unwrap().map(move |beats| ts(beats, unit))
            })
            .flat_map(|ts| (0..ts.groupings().len() as u8).map(move |g| ts.with_grouping(g)));
        for meter in meters {
            let beats = meter.beats;
            let bounds = DEFAULT_GRID.primary_boundaries(&meter);
            let st = GeneratorSettings {
                bars: 8,
                time_signature: meter,
                ..settings(Subdivision::Sixteenths, MAX_COMPLEXITY)
            };
            for m in generate_score(&st, &mut Rng::new(beats as u64)).unwrap().measures {
                let onsets = DEFAULT_GRID.compute_onset_ticks(m.beats());
                for (b, &t) in m.beats().iter().zip(&onsets) {
                    let end = t + DEFAULT_GRID.ticks_of(&b.duration).unwrap();
                    assert!(bounds.iter().all(|&x| x <= t || x >= end), "{meter}: {m:?}");
                }
            }
        }
    }

    #[test]
    fn sixteenth_meters_use_sixteenth_groups() {
        // x/16 is always written in sixteenths: only 16ths, 8ths and dotted
        // 8ths, whatever subdivision is selected.
        let allowed = [s(), e(), de()];
        let mut rng = Rng::new(16);
        for sub in [Subdivision::Eighths, Subdivision::Triplets, Subdivision::Mixed] {
            let st =
                GeneratorSettings { time_signature: ts(7, 16), ..settings(sub, MAX_COMPLEXITY) };
            for _ in 0..50 {
                let m = generate_measure(&st, &mut rng).unwrap();
                assert!(m.beats().iter().all(|b| allowed.contains(&b.duration)), "{m:?}");
            }
        }
        let spans = groupings(Subdivision::Sixteenths, ts(7, 16))
            .iter()
            .map(|g| g.span)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(spans, [BeatSpan::Eighth, BeatSpan::DottedEighth].into());
    }

    #[test]
    fn three_sixteenth_ladder_follows_the_triplet_ladder() {
        let at = |level: u8| -> std::collections::BTreeSet<String> {
            candidates(Subdivision::Sixteenths, BeatSpan::DottedEighth, level)
                .into_iter()
                .map(sixteenth_onsets)
                .collect()
        };
        assert_eq!(at(1), set(&["---", "x--", "xxx"]));
        assert_eq!(at(2), set(&["---", "x--", "xxx", "x-x"]));
        assert_eq!(at(3), set(&["---", "x--", "xxx", "x-x", "--x"]));
        assert_eq!(at(4), set(&["---", "x--", "xxx", "x-x", "--x", "xx-", "-xx"]));
        assert_eq!(at(5).len(), 8);
        let pairs: std::collections::BTreeSet<String> =
            candidates(Subdivision::Sixteenths, BeatSpan::Eighth, 5)
                .into_iter()
                .map(sixteenth_onsets)
                .collect();
        assert_eq!(pairs, set(&["--", "x-", "xx", "-x"]));
    }

    #[test]
    fn odd_meters_follow_the_grouping() {
        // 7/8 = 2+3+2: the middle group holds three-eighth cells, the outer
        // ones two-eighth cells.
        let st = GeneratorSettings {
            time_signature: TimeSignature::SEVEN_EIGHT,
            ..settings(Subdivision::Sixteenths, MAX_COMPLEXITY)
        };
        let mut rng = Rng::new(6);
        for _ in 0..100 {
            let spans: Vec<BeatSpan> = pick_cells(&st, &mut rng).iter().map(|c| c.span()).collect();
            assert_eq!(spans, [BeatSpan::Quarter, BeatSpan::DottedQuarter, BeatSpan::Quarter]);
        }
    }

    #[test]
    fn triplets_and_mixed_fall_back_to_straight_in_x8() {
        let six = TimeSignature::SIX_EIGHT;
        assert_eq!(effective_subdivision(Subdivision::Triplets, six), Subdivision::Eighths);
        assert_eq!(effective_subdivision(Subdivision::Mixed, six), Subdivision::Sixteenths);
        assert_eq!(
            effective_subdivision(Subdivision::Mixed, TimeSignature::FOUR_FOUR),
            Subdivision::Mixed
        );
        let mut rng = Rng::new(12);
        for sub in [Subdivision::Triplets, Subdivision::Mixed] {
            let st = GeneratorSettings { time_signature: six, ..settings(sub, MAX_COMPLEXITY) };
            for _ in 0..50 {
                let m = generate_measure(&st, &mut rng).unwrap();
                assert!(m.beats().iter().all(|b| !b.duration.is_tuplet()));
            }
        }
    }

    #[test]
    fn compound_eighth_ladder_follows_the_triplet_ladder() {
        let at = |level: u8| -> std::collections::BTreeSet<String> {
            candidates(Subdivision::Eighths, BeatSpan::DottedQuarter, level)
                .into_iter()
                .map(eighth_onsets)
                .collect()
        };
        assert_eq!(at(1), set(&["---", "x--", "xxx"]));
        assert_eq!(at(2), set(&["---", "x--", "xxx", "x-x"]));
        assert_eq!(at(3), set(&["---", "x--", "xxx", "x-x", "--x"]));
        assert_eq!(at(4), set(&["---", "x--", "xxx", "x-x", "--x", "xx-", "-xx"]));
        // Level 5 adds -x- and completes all eight rhythms of the group.
        assert_eq!(at(5).len(), 8);
        assert!(at(5).contains("-x-"));
    }

    #[test]
    fn compound_sixteenth_ladder() {
        let at = |level: u8| -> std::collections::BTreeSet<String> {
            candidates(Subdivision::Sixteenths, BeatSpan::DottedQuarter, level)
                .into_iter()
                .filter(|c| c.level_in(Subdivision::Sixteenths) == level)
                .map(sixteenth_onsets)
                .collect()
        };
        // Eighth figures first, as with quarter beats.
        assert_eq!(at(1), set(&["------", "x-----", "x-x-x-"]));
        assert_eq!(at(2), set(&["x---x-", "----x-", "x-x---"]));
        assert_eq!(at(3), set(&["xxxxxx", "xxx-x-", "x-xxx-", "x-x-xx"]));
        assert_eq!(at(4), set(&["--x-x-", "x--xx-", "xxxxx-", "x-xxxx", "x---xx"]));
        assert_eq!(at(5), set(&["--x---", "-xx-x-", "--xxx-", "x--xxx"]));
        // No rhythm appears twice across levels.
        let all: Vec<String> = candidates(Subdivision::Sixteenths, BeatSpan::DottedQuarter, 5)
            .into_iter()
            .map(sixteenth_onsets)
            .collect();
        let unique: std::collections::BTreeSet<_> = all.iter().cloned().collect();
        assert_eq!(unique.len(), all.len(), "duplicate rhythms: {all:?}");
    }

    #[test]
    fn space_uses_dotted_quarter_rests_in_compound_groups() {
        let st = GeneratorSettings {
            space: 1.0,
            time_signature: TimeSignature::SIX_EIGHT,
            ..settings(Subdivision::Eighths, 1)
        };
        let mut rng = Rng::new(14);
        let rests = (0..200)
            .flat_map(|_| pick_cells(&st, &mut rng))
            .filter(|c| c.beats == [(dq(), R)])
            .count();
        assert!(rests > 200, "dotted-quarter rests: {rests}");
    }

    #[test]
    fn groupings_list_only_the_spans_of_the_meter() {
        let spans = |meter| {
            groupings(Subdivision::Sixteenths, meter)
                .iter()
                .map(|g| g.span)
                .collect::<std::collections::HashSet<_>>()
        };
        assert_eq!(spans(TimeSignature::FOUR_FOUR), [BeatSpan::Quarter].into());
        assert_eq!(spans(TimeSignature::SIX_EIGHT), [BeatSpan::DottedQuarter].into());
        assert_eq!(
            spans(TimeSignature::SEVEN_EIGHT),
            [BeatSpan::Quarter, BeatSpan::DottedQuarter].into()
        );
        // Two-eighth groups never offer triplets, even with mixed selected.
        assert!(
            groupings(Subdivision::Mixed, TimeSignature::FIVE_EIGHT)
                .iter()
                .all(|g| g.cell.family != CellFamily::Triplet)
        );
        for g in groupings(Subdivision::Sixteenths, TimeSignature::SIX_EIGHT) {
            assert_eq!(g.measure().time_signature(), ts(3, 8));
        }
    }

    #[test]
    fn custom_groupings_apply_per_group_size() {
        // Only a three-eighth grouping picked: 5/8 uses it for the three
        // and falls back to the complexity level for the two.
        let pick = groupings(Subdivision::Eighths, TimeSignature::SIX_EIGHT)
            .into_iter()
            .find(|g| g.cell.id == g_id(25))
            .unwrap();
        let mut custom = GroupingSet::default();
        custom.insert(pick.id);
        let st = GeneratorSettings {
            custom_groupings_enabled: true,
            custom_groupings: custom,
            time_signature: TimeSignature::FIVE_EIGHT,
            ..settings(Subdivision::Eighths, 1)
        };
        let mut rng = Rng::new(15);
        for _ in 0..50 {
            let cells = pick_cells(&st, &mut rng);
            assert_eq!(cells[0].id, pick.id);
            assert_eq!(cells[1].span(), BeatSpan::Quarter);
            assert_eq!(cells[1].level, 1);
        }
    }

    fn g_id(id: u8) -> GroupingId {
        GroupingId(id)
    }

    #[test]
    fn complexity_caps_the_cell_level() {
        let mut rng = Rng::new(7);
        for complexity in 1..=MAX_COMPLEXITY {
            let st = settings(Subdivision::Mixed, complexity);
            for _ in 0..200 {
                assert!(
                    pick_cells(&st, &mut rng)
                        .iter()
                        .all(|c| c.level_in(Subdivision::Mixed) <= complexity)
                );
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
            candidates(Subdivision::Sixteenths, BeatSpan::Quarter, level)
                .into_iter()
                .map(onsets)
                .collect::<Vec<_>>()
        };
        // Levels 1-2 match the eighth subdivision; sixteenths start at 3.
        for level in [1, 2] {
            let eighths: Vec<String> = candidates(Subdivision::Eighths, BeatSpan::Quarter, level)
                .into_iter()
                .map(onsets)
                .collect();
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
            candidates(Subdivision::Sixteenths, BeatSpan::Quarter, level)
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
    fn mixed_reverses_the_triplet_ladder() {
        fn triplets_at(level: u8) -> std::collections::BTreeSet<String> {
            candidates(Subdivision::Mixed, BeatSpan::Quarter, level)
                .into_iter()
                .filter(|c| c.family == CellFamily::Triplet)
                .map(|c| c.beats.iter().map(|&(_, n)| if n { 'x' } else { '-' }).collect())
                .collect()
        }
        assert_eq!(triplets_at(1), set(&["-x-"]));
        assert_eq!(triplets_at(2), set(&["-x-", "--x"]));
        assert_eq!(triplets_at(3), set(&["-x-", "--x", "x-x"]));
        assert_eq!(triplets_at(4), set(&["-x-", "--x", "x-x", "xx-", "-xx"]));
        assert_eq!(triplets_at(5), set(&["-x-", "--x", "x-x", "xx-", "-xx", "xxx"]));
        // Straight figures keep their levels in mixed.
        let straight = |sub| {
            candidates(sub, BeatSpan::Quarter, 3)
                .into_iter()
                .filter(|c| c.family != CellFamily::Triplet)
                .map(|c| c.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(straight(Subdivision::Mixed), straight(Subdivision::Sixteenths));
        // The triplet subdivision itself is unaffected.
        assert_eq!(triplet_patterns(1), set(&["x", "-", "xxx"]));
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
            let ids: Vec<_> =
                groupings(sub, TimeSignature::FOUR_FOUR).iter().map(|g| g.id).collect();
            let unique: std::collections::HashSet<_> = ids.iter().collect();
            assert_eq!(ids.len(), unique.len(), "{sub:?}");
            assert!(ids.iter().all(|id| id.0 < 64));
        }
        assert_eq!(groupings(Subdivision::Sixteenths, TimeSignature::FOUR_FOUR).len(), 16);
        assert_eq!(groupings(Subdivision::Eighths, TimeSignature::FOUR_FOUR).len(), 4);
        // Triplets: rest, quarter and six triplet figures; the rest comes
        // first in every subdivision.
        assert_eq!(groupings(Subdivision::Triplets, TimeSignature::FOUR_FOUR).len(), 8);
        for sub in [Subdivision::Eighths, Subdivision::Sixteenths, Subdivision::Triplets] {
            let first = groupings(sub, TimeSignature::FOUR_FOUR)[0].measure();
            assert!(first.beats().iter().all(|b| b.kind == Rest), "{sub:?}");
        }
    }

    #[test]
    fn complexity_is_a_growing_subset_of_the_catalog() {
        let sub = Subdivision::Sixteenths;
        let mut prev = GroupingSet::default();
        for level in 1..=MAX_COMPLEXITY {
            let set = complexity_groupings(sub, TimeSignature::FOUR_FOUR, level);
            assert_eq!(set.0 & prev.0, prev.0, "level {level} must contain level {}", level - 1);
            prev = set;
        }
        let all = groupings(sub, TimeSignature::FOUR_FOUR).iter().fold(
            GroupingSet::default(),
            |mut s, g| {
                s.insert(g.id);
                s
            },
        );
        assert_eq!(prev, all);
    }

    #[test]
    fn custom_groupings_restrict_the_pool() {
        let mut custom = GroupingSet::default();
        let pick = groupings(Subdivision::Sixteenths, TimeSignature::FOUR_FOUR)[8]; // dotted eighth + sixteenth
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
        custom.insert(groupings(Subdivision::Sixteenths, TimeSignature::FOUR_FOUR)[15].id);
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
        custom.insert(
            groupings(Subdivision::Triplets, TimeSignature::FOUR_FOUR)
                .iter()
                .find(|g| g.level == 5)
                .unwrap()
                .id,
        );
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
