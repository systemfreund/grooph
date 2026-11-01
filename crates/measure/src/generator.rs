//! Random rhythm generator for sight-reading practice.
//!
//! A measure is generated beat by beat: for every beat of the time signature
//! one *figure* is drawn, a pattern of where notes start within the beat
//! (`x-xx`: notes on "1", "&" and "a"). The figures are not a hand-made list
//! but every pattern of the beat's grid (see [`all_patterns`]): sixteenths,
//! or eighth triplets over a quarter. Which [`Subdivision`] may use a pattern
//! follows from its grid, and its difficulty level from a few rules.
//! [`GeneratorSettings::complexity`] caps the level that may be drawn; the
//! highest allowed level fills half of the beats so a higher setting audibly
//! changes the result; the other half repeats the easier levels, each level
//! equally often, so easy figures stay in the mix.
//!
//! Level ladder for 8th/16th figures (derived from reference examples):
//! 1. quarters, on-beat eighths and quarter rests
//! 2. eighths on the "and" (first syncopations)
//! 3. sixteenth figures starting on the beat, without dotted or syncopated
//!    notes
//! 4. one hard element: a dotted or syncopated note, or a start on a later
//!    eighth
//! 5. figures starting on "e" or "a", or with two hard elements or more
//!
//! Patterns of three slots (eighth triplets, eighths over a dotted quarter,
//! sixteenths over a dotted eighth) follow the triplet ladder of the
//! reference app (`x` = note, `-` = rest):
//! 1. `xxx`, `x--` (one long note) and `---`
//! 2. + `x-x`
//! 3. + `--x`
//! 4. + `xx-`, `-xx`
//! 5. + `-x-`
//!
//! With the mixed subdivision the triplet ladder is reversed: 1 `-x-`,
//! 2 + `--x`, 3 + `x-x`, 4 + `xx-`, `-xx`, 5 + `xxx`. With sixteenths the
//! eighth figures of a dotted quarter take levels 1-2 like those of a
//! quarter, and only the ones starting with a rest come later. Two-slot
//! patterns (sixteenths over an eighth) are `x-`, `xx`, then `-x`.
//!
//! Besides the level-1 quarter rest, [`GeneratorSettings::space`] adds
//! whole-beat rests: it replaces entire beats (never part of a figure, so a
//! triplet is always complete) with a quarter rest. At
//! maximum space a measure may consist of rests only.
//!
//! In `x/8` time signatures a "beat" is a beat group of the meter (see
//! [`beat_groups`]): 6/8 is 3+3, 7/8 2+3+2 and so on. Two-eighth groups use the
//! quarter-beat figures, three-eighth groups (a dotted quarter) the patterns
//! of six sixteenths, written straight. Triplets and mixed are not offered in
//! `x/8`; they fall back to eighths and sixteenths (see
//! [`effective_subdivision`]). `x/16` works the same way one note value down
//! and always in sixteenths: two- and three-sixteenth groups.
//!
//! A figure only fixes where its notes start. How long each note sounds, and
//! so how the measure is written (`8 8` or `16 16-rest 16 16-rest`, a half
//! note or a quarter and a quarter rest, a quarter over the beat), is
//! decided afterwards for the whole measure (see [`spelling`]). From level
//! [`SHORT_NOTES_LEVEL`] notes may also stop before the next onset inside
//! their beat.

mod spelling;

use crate::BeatKind::{Note, Rest};
use crate::duration::{Duration, t8};
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
    /// Draw from [`Self::custom_patterns`] instead of the complexity level.
    /// (Stored as `custom_groupings_enabled` before the rename.)
    #[serde(default, alias = "custom_groupings_enabled")]
    pub custom_figures_enabled: bool,
    /// Hand-picked figures; kept while custom figures are switched off.
    /// Replaces `custom_figures`, whose ids predate the computed patterns:
    /// a selection stored under that name is dropped.
    #[serde(default)]
    pub custom_patterns: FigureSet,
}

impl Default for GeneratorSettings {
    fn default() -> Self {
        Self {
            subdivision: Subdivision::Sixteenths,
            complexity: 1,
            bars: 1,
            time_signature: TimeSignature::FOUR_FOUR,
            space: 0.0,
            custom_figures_enabled: false,
            custom_patterns: FigureSet::default(),
        }
    }
}

/// Identifier of a one-beat figure: the position of its pattern in
/// [`all_patterns`], so it follows from the pattern itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FigureId(pub u8);

/// A set of figures, stored as a bit mask over [`FigureId`]s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FigureSet([u64; 2]);

impl FigureSet {
    fn bit(id: FigureId) -> (usize, u64) {
        (id.0 as usize / 64, 1 << (id.0 % 64))
    }
    pub fn contains(&self, id: FigureId) -> bool {
        let (word, bit) = Self::bit(id);
        self.0[word] & bit != 0
    }
    pub fn insert(&mut self, id: FigureId) {
        let (word, bit) = Self::bit(id);
        self.0[word] |= bit;
    }
    pub fn remove(&mut self, id: FigureId) {
        let (word, bit) = Self::bit(id);
        self.0[word] &= !bit;
    }
    pub fn toggle(&mut self, id: FigureId) {
        let (word, bit) = Self::bit(id);
        self.0[word] ^= bit;
    }
    pub fn is_empty(&self) -> bool {
        self.0 == [0; 2]
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

    /// Length of the span in sixteenths.
    fn sixteenths(self) -> u32 {
        match self {
            BeatSpan::Quarter => 4,
            BeatSpan::DottedQuarter => 6,
            BeatSpan::Eighth => 2,
            BeatSpan::DottedEighth => 3,
        }
    }

    /// Whole-span rest for the "space" setting.
    fn rest(self) -> Figure {
        Pattern { span: self, triplet: false, onsets: 0 }.figure(FigureId(u8::MAX), 1)
    }
}

/// One entry of the figure catalog, for display and selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Figure {
    pub id: FigureId,
    /// Complexity level that unlocks this figure.
    pub level: u8,
    pub span: BeatSpan,
    pattern: Pattern,
}

impl Figure {
    /// The figure written into a one-beat measure (1/4, or 3/8 for a
    /// three-eighth group), e.g. for a thumbnail: every note up to the next.
    pub fn measure(&self) -> Measure {
        write_measure(self.span.time_signature(), &self.pattern.beats())
            .expect("figures always fit one beat")
    }

    /// Where the notes start, one character per slot (a sixteenth, or an
    /// eighth triplet): `x` a note starts, `-` none. `x-xx` is `8 16 16`.
    pub fn pattern(&self) -> String {
        (0..self.pattern.slots())
            .map(|slot| if self.pattern.onsets & 1 << slot != 0 { 'x' } else { '-' })
            .collect()
    }
}

/// Where the notes of a one-beat figure start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pattern {
    span: BeatSpan,
    /// Three eighth-triplet slots over a quarter instead of sixteenths.
    triplet: bool,
    /// Bit `i` set: a note starts on slot `i`.
    onsets: u8,
}

/// Levels of the three-slot patterns, indexed by their onset bits (bit 0 =
/// first slot): the triplet ladder of the reference app.
const TRIPLET_LADDER: [u8; 8] = [1, 1, 5, 4, 3, 2, 4, 1];

/// The triplet ladder with the mixed subdivision: offbeat figures first,
/// the full triplet last. (No triplet has none or only its first note.)
const MIXED_TRIPLET_LADDER: [u8; 8] = [1, 1, 1, 4, 2, 3, 4, 5];

/// Eighth figures over a dotted quarter with the sixteenth subdivision:
/// like the quarter-beat ladder, those starting on the beat take levels
/// 1-2 and only the ones starting with a rest come later.
const COMPOUND_SIXTEENTH_LADDER: [u8; 8] = [1, 1, 5, 2, 2, 2, 4, 1];

impl Pattern {
    fn figure(self, id: FigureId, level: u8) -> Figure {
        Figure { id, level, span: self.span, pattern: self }
    }

    fn slots(&self) -> u32 {
        if self.triplet { 3 } else { self.span.sixteenths() }
    }

    /// Slots on which a note starts, in order.
    fn onset_slots(&self) -> impl Iterator<Item = u32> + '_ {
        (0..self.slots()).filter(|slot| self.onsets & 1 << slot != 0)
    }

    /// Notes on eighths only (no odd sixteenth).
    fn eighths_only(&self) -> bool {
        self.onsets & 0b10_1010 == 0
    }

    fn allowed_in(&self, subdivision: Subdivision) -> bool {
        match (self.triplet, self.span, subdivision) {
            (true, ..) => matches!(subdivision, Subdivision::Triplets | Subdivision::Mixed),
            // With triplets the straight beats are a quarter note or rest.
            (false, BeatSpan::Quarter, Subdivision::Triplets) => self.onsets <= 1,
            (false, BeatSpan::Quarter | BeatSpan::DottedQuarter, Subdivision::Eighths) => {
                self.eighths_only()
            }
            _ => true,
        }
    }

    /// Complexity level that unlocks the pattern with `subdivision`.
    fn level_in(&self, subdivision: Subdivision) -> u8 {
        if self.triplet {
            let ladder = match subdivision {
                Subdivision::Mixed => &MIXED_TRIPLET_LADDER,
                _ => &TRIPLET_LADDER,
            };
            return ladder[self.onsets as usize];
        }
        match self.span {
            BeatSpan::Eighth => {
                if self.onsets == 0b10 {
                    2
                } else {
                    1
                }
            }
            BeatSpan::DottedEighth => TRIPLET_LADDER[self.onsets as usize],
            _ if !self.eighths_only() => self.sixteenth_level(),
            BeatSpan::Quarter => {
                if self.onsets == 0b100 {
                    2
                } else {
                    1
                }
            }
            BeatSpan::DottedQuarter => {
                let o = self.onsets;
                let eighths = (o & 1) | (o >> 1 & 0b10) | (o >> 2 & 0b100);
                let ladder = match subdivision {
                    Subdivision::Sixteenths => &COMPOUND_SIXTEENTH_LADDER,
                    _ => &TRIPLET_LADDER,
                };
                ladder[eighths as usize]
            }
        }
    }

    /// Level of a pattern with notes on odd sixteenths. It starts on "e"
    /// or "a": 5. Otherwise it counts the hard elements: a start on a later
    /// eighth, and every dotted or syncopated note (a dotted value, or a
    /// note from an offbeat sixteenth held over the next eighth, each note
    /// sounding up to the next). None: 3, one: 4, more: 5, so each level
    /// brings one new idea.
    fn sixteenth_level(&self) -> u8 {
        let starts: Vec<u32> = self.onset_slots().collect();
        if starts[0] % 2 == 1 {
            return 5;
        }
        let ends = starts.iter().skip(1).copied().chain([self.slots()]);
        let hard_notes = starts.iter().zip(ends).filter(|&(&start, end)| {
            let len = end - start;
            len == 3 || len == 5 || (len >= 2 && start % 2 == 1)
        });
        match usize::from(starts[0] != 0) + hard_notes.count() {
            0 => 3,
            1 => 4,
            _ => 5,
        }
    }

    /// The pattern written out: straight with every note up to the next
    /// one, triplets one eighth triplet per slot.
    fn beats(&self) -> Vec<(Duration, bool)> {
        if self.triplet {
            (0..3).map(|slot| (t8(), self.onsets & 1 << slot != 0)).collect()
        } else {
            spelling::legato(self.span.sixteenths(), self.onsets)
        }
    }
}

/// Every one-beat pattern, in [`FigureId`] order: sixteenths over a quarter,
/// eighth triplets (none of them a plain quarter note or rest), then
/// sixteenths over a dotted quarter, an eighth and a dotted eighth.
fn all_patterns() -> impl Iterator<Item = Pattern> {
    let straight = |span: BeatSpan| {
        (0..1u8 << span.sixteenths()).map(move |onsets| Pattern { span, triplet: false, onsets })
    };
    let triplets = (2..8).map(|onsets| Pattern { span: BeatSpan::Quarter, triplet: true, onsets });
    straight(BeatSpan::Quarter)
        .chain(triplets)
        .chain(straight(BeatSpan::DottedQuarter))
        .chain(straight(BeatSpan::Eighth))
        .chain(straight(BeatSpan::DottedEighth))
}

/// Subdivision actually used in `ts`: triplets and mixed have no figures for
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
/// figures, x/16 is always written in sixteenths.
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
/// the generator has no figures for `ts`.
pub fn beat_groups(ts: TimeSignature) -> Option<Vec<BeatSpan>> {
    let (two, three) = match ts.beat_unit {
        4 if ts.beats > 0 => return Some(vec![BeatSpan::Quarter; ts.beats as usize]),
        8 if ts.beats >= 2 => (BeatSpan::Quarter, BeatSpan::DottedQuarter),
        16 if ts.beats >= 2 => (BeatSpan::Eighth, BeatSpan::DottedEighth),
        _ => return None,
    };
    let mut spans = Vec::new();
    for &size in default_groups_for(&ts).iter() {
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

/// All figures available with `subdivision` in `ts`, in display order: only
/// the beat spans `ts` uses (quarter-beat figures first), easier levels
/// first.
pub fn figures(subdivision: Subdivision, ts: TimeSignature) -> Vec<Figure> {
    let subdivision = effective_subdivision(subdivision, ts);
    let spans = beat_groups(ts).unwrap_or_default();
    let mut figures: Vec<Figure> =
        catalog(subdivision).into_iter().filter(|f| spans.contains(&f.span)).collect();
    // Stable: within a level, figures keep the pattern order.
    figures.sort_by_key(|f| (BeatSpan::ALL.iter().position(|&s| s == f.span), f.level));
    figures
}

/// The figures a complexity level unlocks in `ts`: the level is a preset
/// subset of the catalog.
pub fn complexity_figures(
    subdivision: Subdivision,
    ts: TimeSignature,
    complexity: u8,
) -> FigureSet {
    let complexity = complexity.clamp(1, MAX_COMPLEXITY);
    let mut set = FigureSet::default();
    for g in figures(subdivision, ts).into_iter().filter(|g| g.level <= complexity) {
        set.insert(g.id);
    }
    set
}

#[derive(Debug, PartialEq)]
pub enum GeneratorError {
    /// The generator has no figures for this time signature (see
    /// [`supports_time_signature`]).
    UnsupportedTimeSignature(TimeSignature),
    /// Writing a figure into the measure failed (library bug).
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

/// Level from which notes may stop early inside their beat (the ladder's
/// "rests inside the beat").
const SHORT_NOTES_LEVEL: u8 = 4;

/// Probability that a beat draws from the highest allowed level; the other
/// beats repeat one of the easier levels.
const TOP_LEVEL_PROBABILITY: f32 = 0.5;

/// Probability of a whole-beat rest at `space == 1.0` (a 4/4 measure is then
/// entirely rests about a third of the time).
const MAX_SPACE_REST_PROBABILITY: f32 = 0.75;

/// Figures usable with `subdivision`, with their levels.
fn catalog(subdivision: Subdivision) -> Vec<Figure> {
    all_patterns()
        .enumerate()
        .filter(|(_, p)| p.allowed_in(subdivision))
        .map(|(id, p)| p.figure(FigureId(id as u8), p.level_in(subdivision)))
        .collect()
}

fn candidates(subdivision: Subdivision, span: BeatSpan, max_level: u8) -> Vec<Figure> {
    catalog(subdivision).into_iter().filter(|f| f.span == span && f.level <= max_level).collect()
}

/// Draw one figure: half of the time from the highest level available,
/// otherwise from one of the easier levels, each level equally likely; then
/// a figure of that level uniformly.
fn pick_figure(pool: &[Figure], rng: &mut Rng) -> Figure {
    let mut levels: Vec<u8> = pool.iter().map(|f| f.level).collect();
    levels.sort_unstable();
    levels.dedup();
    let (&top, easier) = levels.split_last().expect("figure pool is never empty");
    let level = if easier.is_empty() || rng.chance(TOP_LEVEL_PROBABILITY) {
        top
    } else {
        easier[rng.below(easier.len())]
    };

    let at_level: Vec<Figure> = pool.iter().copied().filter(|f| f.level == level).collect();
    at_level[rng.below(at_level.len())]
}

/// Where figures of one span are drawn from.
struct Pool {
    figures: Vec<Figure>,
    /// Weighted by level (complexity) or uniform (custom figures).
    weighted: bool,
    /// Highest level the pool may draw.
    max_level: u8,
}

impl Pool {
    fn new(settings: &GeneratorSettings, subdivision: Subdivision, span: BeatSpan) -> Self {
        let custom: Vec<Figure> = if settings.custom_figures_enabled {
            catalog(subdivision)
                .into_iter()
                .filter(|f| f.span == span && settings.custom_patterns.contains(f.id))
                .collect()
        } else {
            Vec::new()
        };
        // Custom figures are drawn uniformly; an empty selection falls back
        // to the complexity level so the generator never runs dry.
        if custom.is_empty() {
            let complexity = settings.complexity.clamp(1, MAX_COMPLEXITY);
            let figures = candidates(subdivision, span, complexity);
            Self { figures, weighted: true, max_level: complexity }
        } else {
            let max_level = custom.iter().map(|f| f.level).max().unwrap_or(1);
            Self { figures: custom, weighted: false, max_level }
        }
    }
}

/// Draw one figure per generator beat, and how the notes may be written.
fn draw(settings: &GeneratorSettings, rng: &mut Rng) -> (Vec<spelling::Drawn>, spelling::Style) {
    let ts = settings.time_signature;
    let subdivision = effective_subdivision(settings.subdivision, ts);
    let spans = beat_groups(ts).unwrap_or_default();
    let space = settings.space.clamp(0.0, 1.0);
    let pools = BeatSpan::ALL.map(|span| Pool::new(settings, subdivision, span));
    let pool_of = |span: BeatSpan| &pools[BeatSpan::ALL.iter().position(|&s| s == span).unwrap()];

    let drawn = spans
        .iter()
        .map(|&span| {
            let pool = pool_of(span);
            if rng.chance(space * MAX_SPACE_REST_PROBABILITY) {
                spelling::Drawn { figure: span.rest(), space: true }
            } else if pool.weighted {
                spelling::Drawn { figure: pick_figure(&pool.figures, rng), space: false }
            } else {
                let figure = pool.figures[rng.below(pool.figures.len())];
                spelling::Drawn { figure, space: false }
            }
        })
        .collect();
    let style = spelling::Style {
        step: match subdivision {
            Subdivision::Eighths => 2,
            // Only quarters are straight.
            Subdivision::Triplets => 4,
            Subdivision::Sixteenths | Subdivision::Mixed => 1,
        },
        short_notes: spans.iter().any(|&span| pool_of(span).max_level >= SHORT_NOTES_LEVEL),
    };
    (drawn, style)
}

/// Write `beats` (duration, note?) into a fresh measure via `set_beat`, so
/// the result carries the same invariants (whole tuplet groups) as an
/// edited one.
fn write_measure(ts: TimeSignature, beats: &[(Duration, bool)]) -> Result<Measure, MeasureError> {
    let mut measure = Measure::new(ts);
    for (idx, &(duration, is_note)) in beats.iter().enumerate() {
        measure
            .first_voice_mut()
            .set_beat(idx, Beat::new(duration, if is_note { Note } else { Rest }))?;
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
    let ts = settings.time_signature;
    check_time_signature(ts)?;
    let (drawn, style) = draw(settings, rng);
    let beats = spelling::spell(ts.beat_unit, &drawn, style, rng);
    Ok(write_measure(ts, &beats)?)
}

/// Generate a score of `settings.bars` measures.
pub fn generate_score(
    settings: &GeneratorSettings,
    rng: &mut Rng,
) -> Result<Score, GeneratorError> {
    let measures = (0..settings.bars.max(1))
        .map(|_| generate_measure(settings, rng))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Score::new(measures))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beat::unmarked;
    use crate::duration::{Duration, NoteValue, e, h, q, s};
    use crate::grid::DEFAULT_GRID;

    fn de() -> Duration {
        Duration::Dotted { base: NoteValue::Eighth, dots: 1 }
    }

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

    /// Ticks at which the notes of `m` start.
    fn note_onsets(m: &Measure) -> Vec<u32> {
        let onsets = DEFAULT_GRID.compute_onset_ticks(m.first_voice().beats());
        m.first_voice()
            .beats()
            .iter()
            .zip(onsets)
            .filter(|(b, _)| b.is_note())
            .map(|(_, t)| t)
            .collect()
    }

    /// The figures drawn for one measure.
    fn pick_figures(settings: &GeneratorSettings, rng: &mut Rng) -> Vec<Figure> {
        draw(settings, rng).0.into_iter().map(|d| d.figure).collect()
    }

    /// `figures` (one per beat) written into a measure.
    fn build_measure(ts: TimeSignature, figures: &[Figure]) -> Result<Measure, MeasureError> {
        let beats: Vec<_> = figures.iter().flat_map(|f| f.pattern.beats()).collect();
        write_measure(ts, &beats)
    }

    fn settings(subdivision: Subdivision, complexity: u8) -> GeneratorSettings {
        GeneratorSettings { subdivision, complexity, ..GeneratorSettings::default() }
    }

    fn expected_beats(figures: &[Figure]) -> Vec<Beat> {
        figures
            .iter()
            .flat_map(|f| f.pattern.beats())
            .map(|(d, is_note)| Beat::new(d, if is_note { Note } else { Rest }))
            .collect()
    }

    /// Every figure of `span`, triplets included.
    fn figures_of(span: BeatSpan) -> impl Iterator<Item = Figure> {
        let sub =
            if span == BeatSpan::Quarter { Subdivision::Mixed } else { Subdivision::Sixteenths };
        catalog(sub).into_iter().filter(move |f| f.span == span)
    }

    fn ts(beats: u8, beat_unit: u8) -> TimeSignature {
        TimeSignature::new(beats, beat_unit)
    }

    #[test]
    fn every_figure_spans_exactly_one_beat_and_starts_its_notes_on_the_pattern() {
        let sixteenth = DEFAULT_GRID.ticks_of(&crate::duration::s()).unwrap();
        for f in all_patterns().map(|p| p.figure(FigureId(0), 1)) {
            let m = f.measure();
            let ticks: u32 = m
                .first_voice()
                .beats()
                .iter()
                .map(|b| DEFAULT_GRID.ticks_of(&b.duration).unwrap())
                .sum();
            assert_eq!(ticks, DEFAULT_GRID.ticks_per_measure(&f.span.time_signature()), "{f:?}");
            let slot =
                if f.pattern.triplet { DEFAULT_GRID.ticks_of(&t8()).unwrap() } else { sixteenth };
            let written: String = (0..f.pattern.slots())
                .map(|i| if note_onsets(&m).contains(&(i * slot)) { 'x' } else { '-' })
                .collect();
            assert_eq!(written, f.pattern(), "{m:?}");
        }
    }

    #[test]
    fn every_figure_is_written_verbatim_at_every_beat_position() {
        for cell in figures_of(BeatSpan::Quarter) {
            let cells = [cell; 4];
            let m = build_measure(TimeSignature::FOUR_FOUR, &cells).expect("figure must build");
            assert_eq!(
                unmarked(m.first_voice().beats()),
                expected_beats(&cells),
                "{:?}",
                cell.pattern()
            );
        }
        for cell in figures_of(BeatSpan::DottedQuarter) {
            let cells = [cell; 4];
            let m = build_measure(TimeSignature::TWELVE_EIGHT, &cells).expect("figure must build");
            assert_eq!(
                unmarked(m.first_voice().beats()),
                expected_beats(&cells),
                "{:?}",
                cell.pattern()
            );
        }
        // 12/16 = 3+3+3+3, 10/16 = 3+3+2+2.
        for three in figures_of(BeatSpan::DottedEighth) {
            let cells = [three; 4];
            let m = build_measure(ts(12, 16), &cells).expect("figure must build");
            assert_eq!(
                unmarked(m.first_voice().beats()),
                expected_beats(&cells),
                "{:?}",
                three.pattern()
            );
            for two in figures_of(BeatSpan::Eighth) {
                let cells = [three, three, two, two];
                let m = build_measure(ts(10, 16), &cells).expect("figure must build");
                assert_eq!(unmarked(m.first_voice().beats()), expected_beats(&cells));
            }
        }
    }

    #[test]
    fn mixed_neighbours_are_written_verbatim() {
        // Every ordered pair of cells, so fills from one cell never leak into the next.
        for a in figures_of(BeatSpan::Quarter) {
            for b in figures_of(BeatSpan::Quarter) {
                let cells = [a, b, b, a];
                let m = build_measure(TimeSignature::FOUR_FOUR, &cells).expect("pair must build");
                assert_eq!(unmarked(m.first_voice().beats()), expected_beats(&cells));
            }
        }
        for a in figures_of(BeatSpan::DottedQuarter) {
            for b in figures_of(BeatSpan::DottedQuarter) {
                let cells = [a, b];
                let m = build_measure(TimeSignature::SIX_EIGHT, &cells).expect("pair must build");
                assert_eq!(unmarked(m.first_voice().beats()), expected_beats(&cells));
            }
        }
    }

    #[test]
    fn two_and_three_eighth_groups_mix_in_odd_meters() {
        // 5/8 is 3+2 and 7/8 2+3+2: every straight two-eighth figure next to
        // every three-eighth figure.
        for three in figures_of(BeatSpan::DottedQuarter) {
            for two in figures_of(BeatSpan::Quarter).filter(|c| !c.pattern.triplet) {
                let five = [three, two];
                let m = build_measure(TimeSignature::FIVE_EIGHT, &five).expect("5/8 must build");
                assert_eq!(unmarked(m.first_voice().beats()), expected_beats(&five));
                let seven = [two, three, two];
                let m = build_measure(TimeSignature::SEVEN_EIGHT, &seven).expect("7/8 must build");
                assert_eq!(unmarked(m.first_voice().beats()), expected_beats(&seven));
            }
        }
    }

    #[test]
    fn triplet_cells_form_one_group_per_beat() {
        let triplets = figures_of(BeatSpan::Quarter).find(|f| f.pattern() == "xxx").unwrap();
        let m = build_measure(TimeSignature::TWO_FOUR, &[triplets, triplets]).unwrap();
        assert_eq!(m.first_voice().tuplet_groups().len(), 2);
        let starts: Vec<_> = m.first_voice().tuplet_groups().iter().map(|g| g.start_idx).collect();
        assert_eq!(starts, [0, 3]);
    }

    #[test]
    fn same_seed_gives_same_score() {
        let st = GeneratorSettings { bars: 4, ..settings(Subdivision::Mixed, 5) };
        let a = generate_score(&st, &mut Rng::new(42)).unwrap();
        let b = generate_score(&st, &mut Rng::new(42)).unwrap();
        for (ma, mb) in a.measures().iter().zip(b.measures()) {
            assert_eq!(ma.first_voice().beats(), mb.first_voice().beats());
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
        assert!(score.measures().iter().all(|m| m.time_signature() == TimeSignature::THREE_FOUR));
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

    /// Patterns of the figures of `span` up to `level`.
    fn patterns(sub: Subdivision, span: BeatSpan, level: u8) -> std::collections::BTreeSet<String> {
        candidates(sub, span, level).iter().map(Figure::pattern).collect()
    }

    /// Patterns of the figures of `span` at exactly `level`.
    fn patterns_at(
        sub: Subdivision,
        span: BeatSpan,
        level: u8,
    ) -> std::collections::BTreeSet<String> {
        candidates(sub, span, level)
            .iter()
            .filter(|f| f.level == level)
            .map(Figure::pattern)
            .collect()
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
                for m in score.measures() {
                    assert_eq!(m.time_signature(), meter);
                    // No note or rest crosses a dotted-quarter boundary.
                    let onsets = DEFAULT_GRID.compute_onset_ticks(m.first_voice().beats());
                    for (b, &t) in m.first_voice().beats().iter().zip(&onsets) {
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
            for m in generate_score(&st, &mut Rng::new(beats as u64)).unwrap().measures() {
                let onsets = DEFAULT_GRID.compute_onset_ticks(m.first_voice().beats());
                for (b, &t) in m.first_voice().beats().iter().zip(&onsets) {
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
                assert!(
                    m.first_voice().beats().iter().all(|b| allowed.contains(&b.duration)),
                    "{m:?}"
                );
            }
        }
        let spans = figures(Subdivision::Sixteenths, ts(7, 16))
            .iter()
            .map(|g| g.span)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(spans, [BeatSpan::Eighth, BeatSpan::DottedEighth].into());
    }

    #[test]
    fn three_sixteenth_ladder_follows_the_triplet_ladder() {
        let at = |level| patterns(Subdivision::Sixteenths, BeatSpan::DottedEighth, level);
        assert_eq!(at(1), set(&["---", "x--", "xxx"]));
        assert_eq!(at(2), set(&["---", "x--", "xxx", "x-x"]));
        assert_eq!(at(3), set(&["---", "x--", "xxx", "x-x", "--x"]));
        assert_eq!(at(4), set(&["---", "x--", "xxx", "x-x", "--x", "xx-", "-xx"]));
        assert_eq!(at(5).len(), 8);
        let pairs = patterns(Subdivision::Sixteenths, BeatSpan::Eighth, 5);
        assert_eq!(pairs, set(&["--", "x-", "xx", "-x"]));
        assert_eq!(patterns_at(Subdivision::Sixteenths, BeatSpan::Eighth, 2), set(&["-x"]));
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
            let spans: Vec<BeatSpan> = pick_figures(&st, &mut rng).iter().map(|c| c.span).collect();
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
                assert!(m.first_voice().beats().iter().all(|b| !b.duration.is_tuplet()));
            }
        }
    }

    #[test]
    fn compound_eighth_ladder_follows_the_triplet_ladder() {
        let at = |level: u8| -> std::collections::BTreeSet<String> {
            patterns(Subdivision::Eighths, BeatSpan::DottedQuarter, level)
                .iter()
                .map(|p| p.chars().step_by(2).collect())
                .collect()
        };
        assert_eq!(at(1), set(&["---", "x--", "xxx"]));
        assert_eq!(at(2), set(&["---", "x--", "xxx", "x-x"]));
        assert_eq!(at(3), set(&["---", "x--", "xxx", "x-x", "--x"]));
        assert_eq!(at(4), set(&["---", "x--", "xxx", "x-x", "--x", "xx-", "-xx"]));
        // Level 5 adds -x- and completes all eight rhythms of the group.
        assert_eq!(at(5).len(), 8);
        assert!(at(5).contains("-x-"));
        // Only eighths with the eighth subdivision.
        assert!(
            patterns(Subdivision::Eighths, BeatSpan::DottedQuarter, 5).iter().all(|p| p
                .chars()
                .skip(1)
                .step_by(2)
                .all(|c| c == '-'))
        );
    }

    #[test]
    fn compound_sixteenth_ladder() {
        let at = |level| patterns_at(Subdivision::Sixteenths, BeatSpan::DottedQuarter, level);
        // Eighth figures first, as with quarter beats.
        assert_eq!(at(1), set(&["------", "x-----", "x-x-x-"]));
        assert_eq!(at(2), set(&["x---x-", "----x-", "x-x---"]));
        // Every one of the 64 patterns, once.
        let all = candidates(Subdivision::Sixteenths, BeatSpan::DottedQuarter, 5);
        let unique: std::collections::BTreeSet<_> = all.iter().map(Figure::pattern).collect();
        assert_eq!((all.len(), unique.len()), (64, 64));
        // Sixteenths on the beat, plain values: level 3.
        for p in ["xxxxxx", "xxx-x-", "x-xxx-", "x-x-xx", "x---xx", "xxx---"] {
            assert!(at(3).contains(p), "{p}");
        }
        // One dotted or syncopated note, or a start on a later eighth: 4.
        for p in ["x--xx-", "xx--x-", "xx----", "x----x", "--xxx-", "--x-xx", "----xx"] {
            assert!(at(4).contains(p), "{p}");
        }
        // A start on a sixteenth offbeat, or two hard elements: 5.
        for p in ["-xx-x-", "-x----", "---x--", "-----x", "xx-x-x", "x--x--", "--xx--"] {
            assert!(at(5).contains(p), "{p}");
        }
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
            .flat_map(|_| pick_figures(&st, &mut rng))
            .filter(|f| f.span == BeatSpan::DottedQuarter && f.pattern() == "------")
            .count();
        assert!(rests > 200, "dotted-quarter rests: {rests}");
    }

    #[test]
    fn figures_list_only_the_spans_of_the_meter() {
        let spans = |meter| {
            figures(Subdivision::Sixteenths, meter)
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
            figures(Subdivision::Mixed, TimeSignature::FIVE_EIGHT)
                .iter()
                .all(|g| !g.pattern.triplet)
        );
        for g in figures(Subdivision::Sixteenths, TimeSignature::SIX_EIGHT) {
            assert_eq!(g.measure().time_signature(), ts(3, 8));
        }
    }

    #[test]
    fn custom_figures_apply_per_group_size() {
        // Only a three-eighth figure picked: 5/8 uses it for the three
        // and falls back to the complexity level for the two.
        let pick = figures(Subdivision::Eighths, TimeSignature::SIX_EIGHT)
            .into_iter()
            .find(|g| g.pattern() == "x---x-")
            .unwrap();
        let mut custom = FigureSet::default();
        custom.insert(pick.id);
        let st = GeneratorSettings {
            custom_figures_enabled: true,
            custom_patterns: custom,
            time_signature: TimeSignature::FIVE_EIGHT,
            ..settings(Subdivision::Eighths, 1)
        };
        let mut rng = Rng::new(15);
        for _ in 0..50 {
            let cells = pick_figures(&st, &mut rng);
            assert_eq!(cells[0].id, pick.id);
            assert_eq!(cells[1].span, BeatSpan::Quarter);
            assert_eq!(cells[1].level, 1);
        }
    }

    #[test]
    fn complexity_caps_the_cell_level() {
        let mut rng = Rng::new(7);
        for complexity in 1..=MAX_COMPLEXITY {
            let st = settings(Subdivision::Mixed, complexity);
            for _ in 0..200 {
                assert!(pick_figures(&st, &mut rng).iter().all(|c| c.level <= complexity));
            }
        }
    }

    #[test]
    fn highest_level_is_actually_used() {
        let mut rng = Rng::new(11);
        let st = settings(Subdivision::Sixteenths, 5);
        let top =
            (0..200).flat_map(|_| pick_figures(&st, &mut rng)).filter(|c| c.level == 5).count();
        // 4 beats * 200 measures, half of them from the top level.
        assert!((320..=480).contains(&top), "top-level cells drawn: {top}");
    }

    #[test]
    fn easier_levels_share_the_other_half_equally() {
        // Quarter beats with sixteenths at level 5: level 2 has one figure,
        // level 4 four, yet each easier level gets an eighth of the draws.
        let pool = candidates(Subdivision::Sixteenths, BeatSpan::Quarter, 5);
        let mut rng = Rng::new(5);
        let mut per_level = [0u32; 6];
        for _ in 0..8000 {
            per_level[pick_figure(&pool, &mut rng).level as usize] += 1;
        }
        assert!((3700..=4300).contains(&per_level[5]), "{per_level:?}");
        for level in 1..=4 {
            assert!((800..=1200).contains(&per_level[level]), "{per_level:?}");
        }
    }

    #[test]
    fn subdivision_restricts_durations() {
        let mut rng = Rng::new(3);
        for _ in 0..200 {
            let eighths = generate_measure(&settings(Subdivision::Eighths, 5), &mut rng).unwrap();
            assert!(
                eighths
                    .first_voice()
                    .beats()
                    .iter()
                    .all(|b| b.duration != s() && b.duration != de())
            );
            assert!(eighths.first_voice().beats().iter().all(|b| !b.duration.is_tuplet()));

            let triplets = generate_measure(&settings(Subdivision::Triplets, 5), &mut rng).unwrap();
            assert!(
                triplets
                    .first_voice()
                    .beats()
                    .iter()
                    .all(|b| b.duration != s() && b.duration != de())
            );

            let sixteenths =
                generate_measure(&settings(Subdivision::Sixteenths, 5), &mut rng).unwrap();
            assert!(sixteenths.first_voice().beats().iter().all(|b| b.duration != t8()));
        }
    }

    /// Patterns drawn at `complexity` with the triplet subdivision.
    fn triplet_patterns(complexity: u8) -> std::collections::BTreeSet<String> {
        let mut rng = Rng::new(21);
        let st = settings(Subdivision::Triplets, complexity);
        (0..500).flat_map(|_| pick_figures(&st, &mut rng)).map(|f| f.pattern()).collect()
    }

    fn set(items: &[&str]) -> std::collections::BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn triplet_ladder_matches_reference_app() {
        // "x---" is the quarter note, "----" the quarter rest.
        let base = ["x---", "----", "xxx"];
        assert_eq!(triplet_patterns(1), set(&base));
        assert_eq!(triplet_patterns(2), set(&[&base[..], &["x-x"]].concat()));
        assert_eq!(triplet_patterns(3), set(&[&base[..], &["x-x", "--x"]].concat()));
        assert_eq!(triplet_patterns(4), set(&[&base[..], &["x-x", "--x", "xx-", "-xx"]].concat()));
        assert_eq!(triplet_patterns(5).len(), 8);
    }

    #[test]
    fn eighth_ladder_is_rhythmically_distinct() {
        // Level 1 = quarter, two eighths and the quarter rest; level 2 adds
        // the eighth on the "and".
        fn drawn(complexity: u8) -> std::collections::BTreeSet<String> {
            let mut rng = Rng::new(3);
            let st = settings(Subdivision::Eighths, complexity);
            (0..300).flat_map(|_| pick_figures(&st, &mut rng)).map(|f| f.pattern()).collect()
        }
        assert_eq!(drawn(1), set(&["x---", "x-x-", "----"]));
        assert_eq!(drawn(2), set(&["x---", "x-x-", "----", "--x-"]));
        assert_eq!(drawn(5), drawn(2));
    }

    #[test]
    fn sixteenth_ladder_covers_every_rhythm_once() {
        let pool = |level| patterns(Subdivision::Sixteenths, BeatSpan::Quarter, level);
        // Levels 1-2 match the eighth subdivision; sixteenths start at 3.
        for level in [1, 2] {
            assert_eq!(pool(level), patterns(Subdivision::Eighths, BeatSpan::Quarter, level));
        }
        assert!(pool(3).contains("xxxx"));
        let all = candidates(Subdivision::Sixteenths, BeatSpan::Quarter, 5);
        // All 16 four-slot rhythms, the empty one being the quarter rest.
        assert_eq!((all.len(), pool(5).len()), (16, 16));
    }

    #[test]
    fn sixteenth_levels_match_reference_figures() {
        // Reference app "Custom Groupings", unlocked cumulatively per level;
        // the rules reproduce it exactly.
        let at = |level| patterns_at(Subdivision::Sixteenths, BeatSpan::Quarter, level);
        assert_eq!(at(1), set(&["----", "x---", "x-x-"]));
        assert_eq!(at(2), set(&["--x-"]));
        assert_eq!(at(3), set(&["xxxx", "x-xx", "xxx-"]));
        assert_eq!(at(4), set(&["xx-x", "x--x", "--xx", "xx--"]));
        assert_eq!(at(5), set(&["-xx-", "-xxx", "-x-x", "-x--", "---x"]));
    }

    #[test]
    fn mixed_reverses_the_triplet_ladder() {
        let triplets_at = |level| -> std::collections::BTreeSet<String> {
            candidates(Subdivision::Mixed, BeatSpan::Quarter, level)
                .iter()
                .filter(|f| f.pattern.triplet)
                .map(Figure::pattern)
                .collect()
        };
        assert_eq!(triplets_at(1), set(&["-x-"]));
        assert_eq!(triplets_at(2), set(&["-x-", "--x"]));
        assert_eq!(triplets_at(3), set(&["-x-", "--x", "x-x"]));
        assert_eq!(triplets_at(4), set(&["-x-", "--x", "x-x", "xx-", "-xx"]));
        assert_eq!(triplets_at(5), set(&["-x-", "--x", "x-x", "xx-", "-xx", "xxx"]));
        // Straight figures keep their levels in mixed.
        let straight = |sub| {
            candidates(sub, BeatSpan::Quarter, 3)
                .into_iter()
                .filter(|f| !f.pattern.triplet)
                .map(|f| f.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(straight(Subdivision::Mixed), straight(Subdivision::Sixteenths));
        // The triplet subdivision itself is unaffected.
        assert_eq!(triplet_patterns(1), set(&["x---", "----", "xxx"]));
    }

    #[test]
    fn space_only_replaces_whole_beats() {
        // With space, every beat is either a complete figure or a quarter
        // rest; triplets are never cut apart.
        let mut rng = Rng::new(13);
        let st = GeneratorSettings { space: 1.0, ..settings(Subdivision::Triplets, 1) };
        for _ in 0..300 {
            for f in pick_figures(&st, &mut rng) {
                assert!(["----", "x---", "xxx"].contains(&f.pattern().as_str()), "{f:?}");
            }
        }
    }

    #[test]
    fn space_increases_rests() {
        fn rest_ratio(space: f32) -> f32 {
            let mut rng = Rng::new(9);
            let st = GeneratorSettings { space, ..settings(Subdivision::Sixteenths, 3) };
            let mut rest_sixteenths = 0;
            for _ in 0..300 {
                let m = generate_measure(&st, &mut rng).unwrap();
                rest_sixteenths += m
                    .first_voice()
                    .beats()
                    .iter()
                    .filter(|b| b.is_rest())
                    .map(|b| {
                        DEFAULT_GRID.ticks_of(&b.duration).unwrap()
                            / DEFAULT_GRID.ticks_of(&s()).unwrap()
                    })
                    .sum::<u32>();
            }
            rest_sixteenths as f32 / (300.0 * 16.0)
        }
        assert!(rest_ratio(1.0) > rest_ratio(0.0) + 0.5);
    }

    #[test]
    fn full_space_can_produce_an_empty_measure() {
        let mut rng = Rng::new(17);
        let st = GeneratorSettings { space: 1.0, ..settings(Subdivision::Sixteenths, 3) };
        let empty = (0..100)
            .map(|_| generate_measure(&st, &mut rng).unwrap())
            .filter(|m| m.first_voice().beats().iter().all(|b| b.is_rest()))
            .count();
        assert!(empty > 0, "expected some all-rest measures at maximum space");
    }

    #[test]
    fn figure_ids_are_unique_per_subdivision() {
        for sub in [
            Subdivision::Eighths,
            Subdivision::Sixteenths,
            Subdivision::Triplets,
            Subdivision::Mixed,
        ] {
            let ids: Vec<_> = figures(sub, TimeSignature::FOUR_FOUR).iter().map(|g| g.id).collect();
            let unique: std::collections::HashSet<_> = ids.iter().collect();
            assert_eq!(ids.len(), unique.len(), "{sub:?}");
        }
        // Every pattern of every grid once: 16 + 6 + 64 + 4 + 8 ids, all in
        // the set's 128 bits.
        let all: Vec<Pattern> = all_patterns().collect();
        let unique: std::collections::HashSet<_> =
            all.iter().map(|p| (p.span, p.triplet, p.onsets)).collect();
        assert_eq!((all.len(), unique.len()), (98, 98));
        let mut set = FigureSet::default();
        set.insert(FigureId(97));
        assert!(set.contains(FigureId(97)) && !set.contains(FigureId(33)));
        assert_eq!(figures(Subdivision::Sixteenths, TimeSignature::FOUR_FOUR).len(), 16);
        assert_eq!(figures(Subdivision::Eighths, TimeSignature::FOUR_FOUR).len(), 4);
        // Triplets: rest, quarter and six triplet figures; the rest comes
        // first in every subdivision.
        assert_eq!(figures(Subdivision::Triplets, TimeSignature::FOUR_FOUR).len(), 8);
        for sub in [Subdivision::Eighths, Subdivision::Sixteenths, Subdivision::Triplets] {
            let first = figures(sub, TimeSignature::FOUR_FOUR)[0].measure();
            assert!(first.first_voice().beats().iter().all(|b| b.is_rest()), "{sub:?}");
        }
    }

    #[test]
    fn complexity_is_a_growing_subset_of_the_catalog() {
        let sub = Subdivision::Sixteenths;
        let mut prev = FigureSet::default();
        for level in 1..=MAX_COMPLEXITY {
            let set = complexity_figures(sub, TimeSignature::FOUR_FOUR, level);
            for g in figures(sub, TimeSignature::FOUR_FOUR).iter().filter(|g| prev.contains(g.id)) {
                assert!(set.contains(g.id), "level {level} must contain level {}", level - 1);
            }
            prev = set;
        }
        let all =
            figures(sub, TimeSignature::FOUR_FOUR).iter().fold(FigureSet::default(), |mut s, g| {
                s.insert(g.id);
                s
            });
        assert_eq!(prev, all);
    }

    #[test]
    fn custom_figures_restrict_the_pool() {
        let mut custom = FigureSet::default();
        let pick = figures(Subdivision::Sixteenths, TimeSignature::FOUR_FOUR)[8]; // dotted eighth + sixteenth
        custom.insert(pick.id);
        let st = GeneratorSettings {
            custom_figures_enabled: true,
            custom_patterns: custom,
            ..settings(Subdivision::Sixteenths, 1)
        };
        let mut rng = Rng::new(4);
        // Only where the notes start is fixed; their lengths vary.
        let beat = DEFAULT_GRID.ticks_per_beat(&TimeSignature::FOUR_FOUR);
        let expected: Vec<u32> = (0..4)
            .flat_map(|i| note_onsets(&pick.measure()).into_iter().map(move |t| i * beat + t))
            .collect();
        for _ in 0..100 {
            let m = generate_measure(&st, &mut rng).unwrap();
            assert_eq!(note_onsets(&m), expected);
        }
    }

    #[test]
    fn empty_or_disabled_custom_selection_uses_complexity() {
        let mut custom = FigureSet::default();
        custom.insert(figures(Subdivision::Sixteenths, TimeSignature::FOUR_FOUR)[15].id);
        let off =
            GeneratorSettings { custom_patterns: custom, ..settings(Subdivision::Sixteenths, 1) };
        let empty = GeneratorSettings {
            custom_figures_enabled: true,
            ..settings(Subdivision::Sixteenths, 1)
        };
        let mut rng = Rng::new(8);
        for st in [off, empty] {
            for _ in 0..100 {
                assert!(pick_figures(&st, &mut rng).iter().all(|c| c.level == 1));
            }
        }
    }

    #[test]
    fn custom_selection_outside_subdivision_is_ignored() {
        // A triplet figure selected while generating sixteenths: falls back.
        let mut custom = FigureSet::default();
        custom.insert(
            figures(Subdivision::Triplets, TimeSignature::FOUR_FOUR)
                .iter()
                .find(|g| g.level == 5)
                .unwrap()
                .id,
        );
        let st = GeneratorSettings {
            custom_figures_enabled: true,
            custom_patterns: custom,
            ..settings(Subdivision::Sixteenths, 2)
        };
        let mut rng = Rng::new(2);
        assert!(pick_figures(&st, &mut rng).iter().all(|c| c.level <= 2));
    }

    #[test]
    fn rng_unit_stays_in_range() {
        let mut rng = Rng::new(0);
        for _ in 0..10_000 {
            let u = rng.unit();
            assert!((0.0..1.0).contains(&u));
        }
    }

    /// Every supported meter with every subdivision, complexity and some
    /// space, `n` measures each, with the cells they were drawn from.
    fn generated(n: usize) -> Vec<(GeneratorSettings, Vec<Figure>, Measure)> {
        let mut rng = Rng::new(77);
        let mut out = Vec::new();
        for beat_unit in TimeSignature::BEAT_UNITS {
            for beats in TimeSignature::beat_range(beat_unit).into_iter().flatten() {
                let ts = TimeSignature::new(beats, beat_unit);
                if !supports_time_signature(ts) {
                    continue;
                }
                for &subdivision in offered_subdivisions(ts) {
                    for complexity in 1..=MAX_COMPLEXITY {
                        for space in [0.0, 0.5] {
                            let st = GeneratorSettings {
                                time_signature: ts,
                                space,
                                ..settings(subdivision, complexity)
                            };
                            for _ in 0..n {
                                let (drawn, style) = draw(&st, &mut rng);
                                let written = spelling::spell(beat_unit, &drawn, style, &mut rng);
                                let m = write_measure(ts, &written).expect("spelling must build");
                                let expected: Vec<Beat> = written
                                    .iter()
                                    .map(|&(d, n)| Beat::new(d, if n { Note } else { Rest }))
                                    .collect();
                                assert_eq!(
                                    unmarked(m.first_voice().beats()),
                                    expected,
                                    "{ts} written verbatim"
                                );
                                let cells = drawn.iter().map(|d| d.figure).collect();
                                out.push((st, cells, m));
                            }
                        }
                    }
                }
            }
        }
        out
    }

    #[test]
    fn spelling_keeps_the_drawn_onsets() {
        for (st, cells, m) in generated(6) {
            let drawn = build_measure(st.time_signature, &cells).unwrap();
            assert_eq!(note_onsets(&m), note_onsets(&drawn), "{m:?} from {drawn:?}");
        }
    }

    /// Start and end tick of every beat of `m` and of the generator beats.
    fn bounds(st: &GeneratorSettings, m: &Measure) -> (Vec<(Beat, u32, u32)>, Vec<u32>) {
        let ts = st.time_signature;
        let onsets = DEFAULT_GRID.compute_onset_ticks(m.first_voice().beats());
        let beats = m
            .first_voice()
            .beats()
            .iter()
            .zip(onsets)
            .map(|(b, t)| (*b, t, t + DEFAULT_GRID.ticks_of(&b.duration).unwrap()))
            .collect();
        let mut at = 0;
        let mut starts = vec![0];
        for span in beat_groups(ts).unwrap() {
            at += DEFAULT_GRID.ticks_per_measure(&span.time_signature());
            starts.push(at);
        }
        (beats, starts)
    }

    #[test]
    fn notes_cross_a_half_bar_only_centred_on_it() {
        for (st, _, m) in generated(6) {
            let ts = st.time_signature;
            let (beats, starts) = bounds(&st, &m);
            // x/4: quarters grouped like the eighths of n/8.
            let mut boundaries = vec![0];
            if ts.beat_unit == 4 {
                let quarter = DEFAULT_GRID.ticks_of(&q()).unwrap();
                for g in crate::grouping::compound_groups(ts.beats).iter() {
                    boundaries.push(boundaries.last().unwrap() + *g as u32 * quarter);
                }
            } else {
                boundaries = starts.clone();
            }
            for (b, start, end) in beats {
                let crosses = |cuts: &[u32]| cuts.iter().any(|&c| start < c && c < end);
                // `4 2 4` over the boundary of two x/4 groups.
                let centred = |c: u32| {
                    ts.beat_unit == 4 && b.is_note() && start + end == 2 * c && b.duration == h()
                };
                let ok = boundaries.iter().all(|&c| !(start < c && c < end) || centred(c));
                assert!(ok, "{b:?} at {start} crosses a group in {m:?}");
                if b.is_rest() && b.duration != h() {
                    assert!(!crosses(&starts), "rest {b:?} at {start} crosses a beat in {m:?}");
                }
            }
        }
    }

    #[test]
    fn rests_inside_a_beat_need_the_short_notes_level() {
        for (st, _, m) in generated(6) {
            if st.complexity >= SHORT_NOTES_LEVEL || st.subdivision == Subdivision::Triplets {
                continue;
            }
            let (beats, starts) = bounds(&st, &m);
            for pair in beats.windows(2) {
                let ((prev, _, _), (b, start, _)) = (pair[0], pair[1]);
                // Triplet cells bring their own rests.
                let straight = !b.duration.is_tuplet() && !prev.duration.is_tuplet();
                if straight && b.is_rest() && prev.is_note() {
                    assert!(starts.contains(&start), "rest after a note at {start} in {m:?}");
                }
            }
        }
    }

    #[test]
    fn sixteenths_come_in_several_spellings() {
        let mut rng = Rng::new(5);
        let st = settings(Subdivision::Sixteenths, 5);
        let written: Vec<String> =
            (0..300).map(|_| format!("{:?}", generate_measure(&st, &mut rng).unwrap())).collect();
        // A sixteenth rest between two notes inside a beat, as in
        // `16 16-rest 16 16-rest`.
        assert!(written.iter().any(|m| m.contains("1/16 (1/16) 1/16")), "{written:?}");
        let mut rng = Rng::new(5);
        let low = settings(Subdivision::Sixteenths, 1);
        let written: Vec<String> =
            (0..300).map(|_| format!("{:?}", generate_measure(&low, &mut rng).unwrap())).collect();
        // Half notes and half rests from level 1 on.
        assert!(written.iter().any(|m| m.contains("1/2 ")), "{written:?}");
        assert!(written.iter().any(|m| m.contains("(1/2)")), "{written:?}");
    }

    #[test]
    fn only_a_half_crosses_the_middle_of_a_four_four_bar() {
        let quarter = DEFAULT_GRID.ticks_of(&q()).unwrap();
        let mut rng = Rng::new(8);
        let (mut half_on_two, mut quarter_on_and_of_one) = (false, false);
        for i in 0..1000u32 {
            let m = generate_measure(&settings(Subdivision::Eighths, 1 + (i % 2) as u8), &mut rng)
                .unwrap();
            let onsets = DEFAULT_GRID.compute_onset_ticks(m.first_voice().beats());
            for (b, t) in m.first_voice().beats().iter().zip(onsets) {
                half_on_two |= b.is_note() && b.duration == h() && t == quarter;
                quarter_on_and_of_one |= b.is_note() && b.duration == q() && 2 * t == quarter;
                let over_beat_three = b.is_note() && b.duration == q() && 2 * t == 3 * quarter;
                assert!(!over_beat_three, "`8 4 8` over beat 3 in {m:?}");
            }
        }
        assert!(half_on_two, "no `4 2 4`");
        // The same syncopation inside a half bar stays.
        assert!(quarter_on_and_of_one, "no `8 4 8` over beat 2");
    }

    #[test]
    fn syncopations_over_a_beat_keep_eighths_around_them() {
        let sixteenth = DEFAULT_GRID.ticks_of(&s()).unwrap();
        let quarter = DEFAULT_GRID.ticks_of(&q()).unwrap();
        let mut seen = false;
        for (st, _, m) in generated(6) {
            if st.time_signature.beat_unit != 4 {
                continue;
            }
            let (beats, _) = bounds(&st, &m);
            // Quarter beats read in sixteenths: a straight value in them off
            // the eighth grid or of an odd number of sixteenths.
            let fine = |beat: u32| {
                beats.iter().any(|(b, start, end)| {
                    let straight = !b.duration.is_tuplet();
                    let odd = |t: u32| (t / sixteenth) % 2 == 1;
                    straight && start / quarter == beat && (odd(*start) || odd(end - start))
                })
            };
            for (b, start, end) in &beats {
                let from_and = start % quarter != 0;
                if b.is_rest() || !from_and || end - start <= quarter - start % quarter {
                    continue;
                }
                seen = true;
                let (first, last) = (start / quarter, (end - 1) / quarter);
                assert!(!fine(first) && !fine(last), "{b:?} at {start} beside sixteenths in {m:?}");
            }
        }
        assert!(seen, "no syncopation over a beat");
    }
}
