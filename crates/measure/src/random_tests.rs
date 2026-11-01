//! Randomized tests: long sequences of random edits must keep every measure
//! valid, and random scores must survive a round trip through the text
//! notation. The random numbers come from a fixed seed, so a failure names
//! the seed and step that reproduce it.

use crate::duration::{COMMON_DURATIONS, Duration, NoteValue, TupletSpec};
use crate::editing::CYCLE_TUPLET_SPECS;
use crate::generator::Rng;
use crate::grid::DEFAULT_GRID;
use crate::notation::{format_score, parse_score};
use crate::{Beat, Measure, Score, TimeSignature};

/// Tuplets of other ratios the editor offers next to [`COMMON_DURATIONS`].
const GENERAL_TUPLETS: [(u8, u8, NoteValue); 7] = [
    (7, 5, NoteValue::Eighth),
    (5, 7, NoteValue::Eighth),
    (9, 8, NoteValue::Sixteenth),
    (11, 8, NoteValue::ThirtySecond),
    (5, 4, NoteValue::Eighth),
    (4, 3, NoteValue::Eighth),
    (2, 3, NoteValue::Eighth),
];

fn durations() -> Vec<Duration> {
    let general = GENERAL_TUPLETS
        .into_iter()
        .map(|(n, m, base)| Duration::Tuplet(TupletSpec { n, m, base }))
        .filter(|d| DEFAULT_GRID.supports(d));
    COMMON_DURATIONS.into_iter().chain(general).collect()
}

/// Every meter the app offers, in each of its groupings.
fn meters() -> Vec<TimeSignature> {
    let mut out = Vec::new();
    for unit in TimeSignature::BEAT_UNITS {
        for beats in TimeSignature::beat_range(unit).unwrap() {
            let ts = TimeSignature::new(beats, unit);
            out.extend((0..ts.groupings().len()).map(|g| ts.with_grouping(g as u8)));
        }
    }
    out
}

fn pick<T: Copy>(rng: &mut Rng, items: &[T]) -> T {
    items[rng.below(items.len())]
}

/// Why `m` is not a valid measure, if it is not.
fn check_invariants(m: &Measure) -> Result<(), String> {
    let mut total = 0;
    for (i, b) in m.first_voice().beats().iter().enumerate() {
        match DEFAULT_GRID.ticks_of(&b.duration) {
            Some(0) | None => return Err(format!("beat {i} ({:?}) has no length", b.duration)),
            Some(t) => total += t,
        }
        if b.duration.is_tuplet() && m.first_voice().tuplet_group_at(i).is_none() {
            return Err(format!("tuplet beat {i} is outside a group"));
        }
        if !b.duration.is_tuplet() && b.tuplet_group_id.is_some() {
            return Err(format!("plain beat {i} is in a tuplet group"));
        }
    }
    let want = DEFAULT_GRID.ticks_per_measure(&m.time_signature());
    if total != want {
        return Err(format!("beats take {total} ticks, the meter {want}"));
    }
    for g in m.first_voice().tuplet_groups() {
        let Some(anchor) = m.first_voice().tuplets().get(g.id) else {
            return Err(format!("group {:?} has no anchor", g.id));
        };
        // Groups of one ratio can have different note values, so each is
        // measured against its own anchor.
        let span: u32 = m.first_voice().beats()[g.start_idx..=g.end_idx]
            .iter()
            .map(|b| DEFAULT_GRID.ticks_of(&b.duration).unwrap())
            .sum();
        if span != anchor.target_ticks {
            return Err(format!(
                "group {}..={} takes {span} ticks, its anchor {}",
                g.start_idx, g.end_idx, anchor.target_ticks
            ));
        }
    }
    Ok(())
}

/// Apply one random edit to `m` and describe it. Edits may be refused; they
/// must never leave the measure invalid.
fn random_edit(
    m: &mut Measure,
    rng: &mut Rng,
    durations: &[Duration],
    meters: &[TimeSignature],
) -> String {
    let idx = rng.below(m.first_voice().beats().len());
    match rng.below(12) {
        0..=3 => {
            let d = pick(rng, durations);
            let beat = if rng.chance(0.3) { Beat::rest(d) } else { Beat::note(d) };
            let ok = m.first_voice_mut().set_beat(idx, beat).is_ok();
            format!("set_beat({idx}, {beat:?}) ok={ok}")
        }
        4 => {
            m.first_voice_mut().remove(idx);
            format!("remove({idx})")
        }
        5 => {
            m.first_voice_mut().toggle_beat_kind(idx, crate::Instrument::Rhythm);
            format!("toggle_beat_kind({idx})")
        }
        6 => {
            m.first_voice_mut().toggle_dotted(idx);
            format!("toggle_dotted({idx})")
        }
        7 => {
            if rng.chance(0.5) {
                m.first_voice_mut().toggle_accent(idx, crate::Instrument::Rhythm);
                format!("toggle_accent({idx})")
            } else {
                m.first_voice_mut().toggle_soft(idx, crate::Instrument::Rhythm);
                format!("toggle_soft({idx})")
            }
        }
        8 => {
            let spec = if rng.chance(0.3) { None } else { Some(pick(rng, &CYCLE_TUPLET_SPECS)) };
            let overwrite = rng.chance(0.5);
            m.first_voice_mut().set_tuplet(idx, spec, overwrite, crate::Instrument::Rhythm);
            format!("set_tuplet({idx}, {spec:?}, {overwrite})")
        }
        9 => {
            m.first_voice_mut().dissolve_tuplet_group(idx);
            format!("dissolve_tuplet_group({idx})")
        }
        10 => {
            let base = pick(rng, &[NoteValue::Quarter, NoteValue::Eighth, NoteValue::Sixteenth]);
            m.first_voice_mut().modify_beat(idx, base, None, crate::Instrument::Rhythm);
            format!("modify_beat({idx}, {base:?})")
        }
        _ => {
            let ts = pick(rng, meters);
            let ok = m.set_time_signature(ts).is_ok();
            format!("set_time_signature({ts}) ok={ok}")
        }
    }
}

/// Run `steps` random edits on each of `measures` fresh measures and check
/// the invariants after every one.
fn run_random_edits(seed: u64, measures: usize, steps: usize) {
    let durations = durations();
    let meters = meters();
    let mut rng = Rng::new(seed);
    for run in 0..measures {
        let mut m = Measure::new(pick(&mut rng, &meters));
        for step in 0..steps {
            let before = format!("{m:?} in {}", m.time_signature());
            let edit = random_edit(&mut m, &mut rng, &durations, &meters);
            if let Err(e) = check_invariants(&m) {
                panic!(
                    "seed {seed}, measure {run}, step {step}: {e}\n  edit: {edit}\n  before: {before}\n  after: {m:?}"
                );
            }
        }
    }
}

/// A random score of up to four measures, each built from random edits.
fn random_score(rng: &mut Rng, durations: &[Duration], meters: &[TimeSignature]) -> Score {
    let len = 1 + rng.below(4);
    let measures = (0..len)
        .map(|_| {
            let mut m = Measure::new(pick(rng, meters));
            for _ in 0..rng.below(20) {
                random_edit(&mut m, rng, durations, meters);
            }
            m
        })
        .collect();
    Score::new(measures)
}

/// Which beats share a tuplet group, independent of the group ids.
fn group_partition(m: &Measure) -> Vec<Option<usize>> {
    let beats = m.first_voice().beats();
    beats
        .iter()
        .map(|b| {
            let id = b.tuplet_group_id?;
            beats.iter().position(|other| other.tuplet_group_id == Some(id))
        })
        .collect()
}

/// Why `b` differs from `a`, if it does. Beats are compared with their
/// tuplet groups, which `Beat`'s `PartialEq` ignores.
fn score_difference(a: &Score, b: &Score) -> Option<String> {
    if a.len() != b.len() {
        return Some(format!("{} measures, then {}", a.len(), b.len()));
    }
    for (i, (ma, mb)) in a.measures().iter().zip(b.measures()).enumerate() {
        if ma.time_signature() != mb.time_signature() {
            return Some(format!(
                "measure {i}: meter {} became {}",
                ma.time_signature(),
                mb.time_signature()
            ));
        }
        if ma.first_voice().beats() != mb.first_voice().beats() {
            return Some(format!("measure {i}: {ma:?} became {mb:?}"));
        }
        if group_partition(ma) != group_partition(mb) {
            return Some(format!("measure {i}: tuplet groups changed in {ma:?}"));
        }
    }
    None
}

/// Whether a tuplet group of `m` holds notes of another value than the one
/// its span was built from, like `t4 t8` in a group of eighth triplets. The
/// model allows them, but the editor does not build them and the notation
/// cannot write them: it reads a group's span from its first note.
fn has_regrouped_tuplets(m: &Measure) -> bool {
    m.first_voice().tuplet_groups().iter().any(|g| {
        let base = m.first_voice().tuplets().get(g.id).map(|a| a.base_hint);
        m.first_voice().beats()[g.start_idx..=g.end_idx]
            .iter()
            .any(|b| b.duration.as_tuplet_spec().map(|s| s.base) != base)
    })
}

fn run_notation_round_trips(seed: u64, scores: usize) {
    let durations = durations();
    let meters = meters();
    let mut rng = Rng::new(seed);
    for i in 0..scores {
        let score = random_score(&mut rng, &durations, &meters);
        if score.measures().iter().any(has_regrouped_tuplets) {
            continue;
        }
        let text = format_score(&score)
            .unwrap_or_else(|| panic!("seed {seed}, score {i}: no notation for {score:?}"));
        let parsed = parse_score(&text)
            .unwrap_or_else(|e| panic!("seed {seed}, score {i}: '{text}' does not parse: {e}"));
        if let Some(diff) = score_difference(&score, &parsed) {
            panic!("seed {seed}, score {i}: '{text}' parses differently: {diff}");
        }
    }
}

const SEED: u64 = 0x2545_F491_4F6C_DD1D;

#[test]
fn random_edits_keep_measures_valid() {
    run_random_edits(SEED, 200, 30);
}

#[test]
#[ignore = "long run; cargo test -p grooph-measure -- --ignored"]
fn random_edits_keep_measures_valid_long() {
    for seed in 1..=20 {
        run_random_edits(seed, 1000, 50);
    }
}

#[test]
fn random_scores_round_trip_through_notation() {
    run_notation_round_trips(SEED, 1000);
}

#[test]
#[ignore = "long run; cargo test -p grooph-measure -- --ignored"]
fn random_scores_round_trip_through_notation_long() {
    for seed in 1..=20 {
        run_notation_round_trips(seed, 5000);
    }
}

/// Clean up random measures: the result is valid, sounds the same (every
/// head at the same tick) and a second cleanup changes nothing.
fn run_random_cleanups(seed: u64, measures: usize) -> (usize, usize) {
    let durations = durations();
    let meters = meters();
    let mut rng = Rng::new(seed);
    let (mut cleaned, mut left) = (0, 0);
    for i in 0..measures {
        let mut m = Measure::new(pick(&mut rng, &meters));
        for _ in 0..rng.below(30) {
            random_edit(&mut m, &mut rng, &durations, &meters);
        }
        if m.first_voice().respelled().is_none() {
            left += 1;
            continue;
        }
        cleaned += 1;
        let once = m.cleaned_up();
        let place = format!("seed {seed}, measure {i}: {m:?} in {}", m.time_signature());
        if let Err(e) = check_invariants(&once) {
            panic!("{place}: cleaned up to {once:?}: {e}");
        }
        let sound = |m: &Measure| -> Vec<_> {
            m.head_onsets().into_iter().map(|o| (o.tick, o.voice, o.head)).collect()
        };
        assert_eq!(sound(&once), sound(&m), "{place}: cleaned up to {once:?}");
        let twice = once.cleaned_up();
        if let Some(diff) =
            score_difference(&Score::new(vec![once.clone()]), &Score::new(vec![twice]))
        {
            panic!("{place}: a second cleanup changed it: {diff}");
        }
    }
    (cleaned, left)
}

#[test]
fn random_cleanups_keep_the_rhythm() {
    let (cleaned, left) = run_random_cleanups(SEED, 2000);
    // Most measures can be cleaned up; tuplets inside beats may not.
    assert!(cleaned > 4 * left, "{cleaned} cleaned, {left} left as they are");
}

#[test]
#[ignore = "long run; cargo test -p grooph-measure -- --ignored"]
fn random_cleanups_keep_the_rhythm_long() {
    for seed in 1..=20 {
        run_random_cleanups(seed, 10_000);
    }
}
