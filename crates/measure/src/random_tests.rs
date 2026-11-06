//! Randomized tests: long sequences of random edits must keep every measure
//! valid, and random scores must survive a round trip through the text
//! notation. The random numbers come from a fixed seed, so a failure names
//! the seed and step that reproduce it.

use crate::duration::{COMMON_DURATIONS, Duration, NoteValue, TupletSpec};
use crate::editing::{TUPLET_TOOLS, VALUE_BUTTONS};
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
        if !b.duration.is_tuplet() && b.tuplet.is_some() {
            return Err(format!("plain beat {i} is in a tuplet group"));
        }
    }
    let want = DEFAULT_GRID.ticks_per_measure(&m.time_signature());
    if total != want {
        return Err(format!("beats take {total} ticks, the meter {want}"));
    }
    // The rules a saved measure is read back with: whole tuplet groups.
    let saved = ron::to_string(m).map_err(|e| e.to_string())?;
    match ron::from_str::<Measure>(&saved) {
        Ok(back) if back == *m => {}
        Ok(back) => return Err(format!("reads back as {back:?}")),
        Err(e) => return Err(format!("does not read back: {e}")),
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
            let spec = pick(rng, &TUPLET_TOOLS);
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

/// Why `b` differs from `a`, if it does.
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
        if ma != mb {
            let groups = |m: &Measure| m.first_voice().tuplet_groups();
            return Some(format!(
                "measure {i}: {ma:?} {:?} became {mb:?} {:?}",
                groups(ma),
                groups(mb)
            ));
        }
    }
    None
}

fn run_notation_round_trips(seed: u64, scores: usize) {
    let durations = durations();
    let meters = meters();
    let mut rng = Rng::new(seed);
    for i in 0..scores {
        let score = random_score(&mut rng, &durations, &meters);
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
        // Half of them with a second voice of its own rhythm.
        if rng.chance(0.5) {
            let mut lower = Measure::new(m.time_signature());
            for _ in 0..rng.below(30) {
                random_edit(&mut lower, &mut rng, &durations, &[m.time_signature()]);
            }
            m.push_voices(&lower).expect("one meter");
        }
        // A voice that cannot be respelled stays as it is next to one that
        // can; only a measure of such voices is skipped.
        if m.voices().iter().all(|v| v.respelled().is_none()) {
            left += 1;
            continue;
        }
        cleaned += 1;
        let once = m.cleaned_up();
        let place = format!("seed {seed}, measure {i}: {m:?} in {}", m.time_signature());
        for voice in 0..once.voices().len() {
            let single = once.single_voice(voice).expect("voice in range");
            if let Err(e) = check_invariants(&single) {
                panic!("{place}: cleaned up to {once:?}, voice {voice}: {e}");
            }
        }
        let sound = |m: &Measure| -> Vec<_> {
            m.head_onsets().into_iter().map(|o| (o.tick, o.voice, o.head)).collect()
        };
        assert_eq!(sound(&once), sound(&m), "{place}: cleaned up to {once:?}");
        // Only what the editor can write: a tuplet value is one that was
        // there, or a value button's in a ratio that was there or of a
        // tuplet tool.
        let specs = |m: &Measure| -> Vec<TupletSpec> {
            m.voices()
                .iter()
                .flat_map(|v| v.beats().iter().filter_map(|b| b.duration.as_tuplet_spec()))
                .collect()
        };
        let before = specs(&m);
        for spec in specs(&once) {
            let ratio = |s: &TupletSpec| (s.n, s.m);
            let written = before.contains(&spec)
                || (VALUE_BUTTONS.contains(&spec.base)
                    && before.iter().chain(&TUPLET_TOOLS).any(|s| ratio(s) == ratio(&spec)));
            assert!(written, "{place}: cleaned up to {once:?}, with {spec:?}");
        }
        let twice = once.cleaned_up();
        for voice in 0..once.voices().len() {
            let [a, b] = [&once, &twice].map(|m| m.single_voice(voice).expect("voice in range"));
            if let Some(diff) = score_difference(&Score::new(vec![a]), &Score::new(vec![b])) {
                panic!("{place}: a second cleanup changed voice {voice}: {diff}");
            }
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

/// `m` as a measure of the kit: each note of a voice plays one to three
/// heads of the instruments `kit[voice]` (none in two voices), some of them
/// accented or soft.
fn on_the_kit(m: &mut Measure, rng: &mut Rng, kit: &[Vec<crate::Instrument>]) {
    for (v, set) in kit.iter().enumerate().take(m.voices().len()) {
        let voice = m.voice_mut(v).expect("voice in range");
        for beat in voice.beats.iter_mut().filter(|b| b.is_note()) {
            let mut heads = crate::Heads::NONE;
            for _ in 0..1 + rng.below(3) {
                let mut head = crate::Head::new(pick(rng, set));
                head.set_accented(rng.chance(0.2));
                head.set_soft(!head.accented() && rng.chance(0.2));
                let _ = heads.insert(head);
            }
            beat.set_heads(heads);
        }
    }
}

/// Revoice random kit measures of two voices, tuplets among them, by every
/// preset: each result is valid, sounds the same, has every head in the
/// voice of the preset and no drum struck by two voices; and going from
/// Hands/Feet to a preset and back gives every head its voice again.
fn run_random_revoicings(seed: u64, measures: usize) -> (usize, usize, usize) {
    use crate::{Instrument, Voicing};
    let durations = durations();
    let meters = meters();
    let mut rng = Rng::new(seed);
    let (mut revoiced, mut refused, mut stuck) = (0, 0, 0);
    for i in 0..measures {
        let mut m = Measure::new(pick(&mut rng, &meters));
        for _ in 0..rng.below(30) {
            random_edit(&mut m, &mut rng, &durations, &meters);
        }
        let mut lower = Measure::new(m.time_signature());
        for _ in 0..rng.below(30) {
            random_edit(&mut lower, &mut rng, &durations, &[m.time_signature()]);
        }
        m.push_voices(&lower).expect("one meter");
        // The kit split at random between the voices.
        let mut kit: [Vec<Instrument>; 2] = Default::default();
        for i in Instrument::ALL.into_iter().filter(|&i| i != Instrument::Rhythm) {
            kit[rng.below(2)].push(i);
        }
        if kit.iter().any(Vec::is_empty) {
            kit = [vec![Instrument::HiHat, Instrument::Snare], vec![Instrument::Kick]];
        }
        on_the_kit(&mut m, &mut rng, &kit);
        // Only valid measures (a whole rest can slip into a voice of
        // another meter than 4/4; a second voice is not edited elsewhere).
        if (0..2).any(|v| check_invariants(&m.single_voice(v).expect("two voices")).is_err()) {
            continue;
        }
        let place = format!("seed {seed}, measure {i}: {m:?} in {}", m.time_signature());
        let sound = |m: &Measure| -> Vec<_> {
            let mut s: Vec<_> = m.head_onsets().into_iter().map(|o| (o.tick, o.head)).collect();
            s.sort_by_key(|&(t, h)| (t, h.instrument()));
            s
        };
        let voiced = |m: &Measure| -> Vec<_> {
            let mut s: Vec<_> =
                m.head_onsets().into_iter().map(|o| (o.tick, o.voice, o.head)).collect();
            s.sort_by_key(|&(t, v, h)| (t, v, h.instrument()));
            s
        };
        let Some(hands_feet) = m.revoiced(&Voicing::HANDS_FEET) else {
            refused += 1;
            continue;
        };
        for preset in Voicing::PRESETS {
            let Some(r) = hands_feet.revoiced(&preset) else {
                refused += 1;
                continue;
            };
            revoiced += 1;
            for voice in 0..r.voices().len() {
                let single = r.single_voice(voice).expect("voice in range");
                if let Err(e) = check_invariants(&single) {
                    panic!("{place}: by {preset:?} {r:?}, voice {voice}: {e}");
                }
            }
            assert_eq!(r.voices().len(), preset.voices(), "{place}: by {preset:?} {r:?}");
            assert_eq!(sound(&r), sound(&m), "{place}: by {preset:?} {r:?}");
            assert!(r.doubled_instruments().is_empty(), "{place}: by {preset:?} {r:?}");
            for o in r.head_onsets() {
                assert_eq!(o.voice, preset.voice_of(o.head.instrument()), "{place}: {r:?}");
            }
            // Back where it can go: tuplets of both voices that met in one
            // group on its way may not part again.
            let Some(back) = r.revoiced(&Voicing::HANDS_FEET) else {
                stuck += 1;
                continue;
            };
            assert_eq!(voiced(&back), voiced(&hands_feet), "{place}: by {preset:?} {r:?}");
        }
    }
    (revoiced, refused, stuck)
}

#[test]
fn random_revoicings_keep_the_sound() {
    let (revoiced, refused, stuck) = run_random_revoicings(SEED, 1000);
    // Only tuplets of both voices that cannot meet keep a measure as it is.
    assert!(revoiced > 10 * refused, "{revoiced} revoiced, {refused} refused");
    assert!(stuck * 1000 < revoiced, "{stuck} of {revoiced} not back");
}

#[test]
#[ignore = "long run; cargo test -p grooph-measure -- --ignored"]
fn random_revoicings_keep_the_sound_long() {
    for seed in 1..=20 {
        let (revoiced, refused, stuck) = run_random_revoicings(seed, 5000);
        assert!(revoiced > 10 * refused, "seed {seed}: {revoiced} revoiced, {refused} refused");
        assert!(stuck * 1000 < revoiced, "seed {seed}: {stuck} of {revoiced} not back");
    }
}
