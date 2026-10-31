//! Compact text notation for scores, meant to be written and read by people
//! and AI agents (e.g. in shareable links).
//!
//! ```text
//! 4/4 q e e >s s s s t8 t8 r:t8 | q. e r:q q
//! ```
//!
//! - Tokens are separated by whitespace; `|` separates measures (a trailing
//!   `|` is allowed).
//! - A time signature (`4/4`, `7/8`, ...) may start a measure and applies to
//!   it and all following measures. Without one, the first measure is 4/4.
//! - Durations use the names of the helpers in [`crate::duration`]:
//!   `h` `q` `e` `s` `th` (half to thirty-second), dotted with a trailing `.`
//!   (`q.` `e.` `s.`), tuplets `t2` `t4` `t8` `t16` `t32` (triplets), `qt4`
//!   `qt8` `qt16` (quintuplets), `st8` `st16` (sextuplets), `spt4` `spt8`
//!   `spt16` (septuplets) and `nt16` (nonuplet), and over a dotted quarter
//!   `qt8_3` (five eighths in the time of three) and `spt16_6` (seven
//!   sixteenths in the time of six). Tuplets of other ratios are written `n:m` and their
//!   note value (`w` `h` `q` `e` `s` `th`), e.g. `7:5e` for seven eighths in
//!   the time of five. Every note of a tuplet group is written out.
//! - `r:` makes a rest (`r:q`), `>` accents a note (`>e`), parentheses make
//!   it a soft (ghost) note (`(e)`).
//!
//! ## Drum kit
//!
//! ```text
//! 4/4 H:e S-H:e H:e H:e H:e S-H:e Ho:e Hc:e ; K:q r:q K:q r:q
//! ```
//!
//! - A note of the kit names its heads before the duration: `S:e` is a snare
//!   eighth. The instruments are `K` kick, `S` snare, `H` hi-hat, `HF` hi-hat
//!   foot, `RD` ride, `CR` crash, `T1` high tom, `T2` mid tom and `FT` floor
//!   tom (see [`Instrument`]). A note without them is the one line's
//!   ([`Instrument::Rhythm`]), as above.
//! - A chord joins its heads with `-`: `K-H:e`. They are written from the
//!   foot up; any order reads.
//! - Each head is accented with `>` or a ghost note in parentheses: `>S-H:q`,
//!   `K-(S):e`.
//! - A lowercase letter after an instrument plays it with a [`Technique`]: `Ho`
//!   open, `Hh` half-open, `Hc` closed hi-hat, `Sx` cross-stick, `Sr` (`T1r`,
//!   ...) rimshot, `RDb` ride bell, `CRc` choked crash.
//! - `;` separates the voices of a measure: the first with its stems up, the
//!   second below it (see [`Measure::voices`]). Every voice fills the measure;
//!   a time signature comes before the first.
//! - A score with a note of the kit is on the kit's staff. One without, such
//!   as an empty bar of the kit (`kit 4/4 r:h r:h`), starts with `kit`;
//!   [`format_score`] writes it only there.
//!
//! Parsing goes through [`Measure::set_beat`], so a parsed score obeys the
//! same invariants as an edited one. Every measure must be filled exactly;
//! errors name the measure, the voice and the token so the input can be
//! corrected.

use crate::BeatKind::{Note, Rest};
use crate::duration::NoteValue::{Eighth, Quarter, Sixteenth};
use crate::duration::{Duration, NoteValue, TupletSpec};
use crate::grid::DEFAULT_GRID;
use crate::{
    Beat, Head, Heads, Instrument, Measure, MeasureError, ParseTimeSignatureError, Score,
    StaffKind, Technique, TimeSignature,
};
use std::fmt::{Display, Formatter};

/// Upper bound for the number of measures in a parsed score.
pub const MAX_MEASURES: usize = 64;

/// Where in the input an error is: a measure and one of its voices, both
/// counted from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place {
    pub measure: usize,
    pub voice: usize,
}

impl Place {
    /// The first voice of `measure`.
    pub const fn measure(measure: usize) -> Self {
        Place { measure, voice: 1 }
    }
}

impl Display for Place {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self.voice {
            1 => write!(f, "measure {}", self.measure),
            voice => write!(f, "measure {}, voice {voice}", self.measure),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotationError {
    /// The input contains no beats at all.
    Empty,
    /// More than [`MAX_MEASURES`] measures.
    TooManyMeasures,
    /// A token that is not a duration, rest, accent, note of the kit or time
    /// signature.
    UnknownToken { at: Place, token: String },
    /// A time signature that is malformed or not supported.
    InvalidTimeSignature { at: Place, token: String, reason: ParseTimeSignatureError },
    /// A time signature in the middle of a measure, or before a voice other
    /// than the first.
    MisplacedTimeSignature { at: Place, token: String },
    /// `>` on a rest.
    AccentedRest { at: Place, token: String },
    /// A soft note that is also a rest or accented, e.g. `(r:q)` or `>(e)`.
    InvalidSoft { at: Place, token: String },
    /// A head played with a technique its instrument has not, e.g. `Sb:e`.
    InvalidTechnique { at: Place, token: String },
    /// A chord with an instrument twice or more heads than a beat holds.
    InvalidChord { at: Place, token: String },
    /// A measure or voice without beats (e.g. `q q q q | | q q q q`).
    EmptyMeasure { at: Place },
    /// The beats exceed the measure length.
    TooLong { at: Place, token: String },
    /// The beats do not fill the measure; `missing` is a fraction of a whole note.
    TooShort { at: Place, missing: String },
    /// The beat cannot be placed here, e.g. an incomplete tuplet group or a
    /// duration that would leave an unfillable gap.
    DoesNotFit { at: Place, token: String },
}

impl Display for NotationError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        use NotationError::*;
        match self {
            Empty => write!(f, "the rhythm is empty"),
            TooManyMeasures => write!(f, "more than {MAX_MEASURES} measures"),
            UnknownToken { at, token } => write!(
                f,
                "{at}: unknown token '{token}' (expected e.g. q, e, s, th, q., t8, qt16, r:q, >e or 4/4)"
            ),
            InvalidTimeSignature { at, token, reason } => {
                write!(f, "{at}: time signature '{token}': {reason}")
            }
            MisplacedTimeSignature { at, token } => write!(
                f,
                "{at}: time signature '{token}' must come before the first beat of the measure"
            ),
            AccentedRest { at, token } => {
                write!(f, "{at}: '{token}' - rests cannot be accented")
            }
            InvalidSoft { at, token } => write!(
                f,
                "{at}: '{token}' - a soft note is written (e) and cannot be a rest or accented"
            ),
            InvalidTechnique { at, token } => {
                write!(f, "{at}: '{token}' - the instrument is not played that way")
            }
            InvalidChord { at, token } => write!(
                f,
                "{at}: '{token}' - a chord has each instrument once and at most {} heads",
                crate::head::MAX_HEADS
            ),
            EmptyMeasure { at } => write!(f, "{at} has no beats"),
            TooLong { at, token } => {
                write!(f, "{at} is too long: '{token}' does not fit anymore")
            }
            TooShort { at, missing } => {
                write!(f, "{at} is too short: {missing} of a whole note missing")
            }
            DoesNotFit { at, token } => write!(
                f,
                "{at}: '{token}' cannot be placed here (tuplet groups must be complete, e.g. t8 t8 t8)"
            ),
        }
    }
}

impl std::error::Error for NotationError {}

/// Durations with a notation code, in the order of [`crate::duration::COMMON_DURATIONS`].
const CODES: [(&str, Duration); 24] = {
    use crate::duration::{
        e, h, nt16, q, qt4, qt8, qt8_3, qt16, s, spt4, spt8, spt16, spt16_6, st8, st16, t2, t8,
        t16, t32, th,
    };
    const fn dotted(base: NoteValue) -> Duration {
        Duration::Dotted { base, dots: 1 }
    }
    [
        ("q", q()),
        ("e", e()),
        ("s", s()),
        ("th", th()),
        ("q.", dotted(Quarter)),
        ("e.", dotted(Eighth)),
        ("s.", dotted(Sixteenth)),
        ("t4", Duration::Tuplet(TupletSpec { n: 3, m: 2, base: Quarter })),
        ("t8", t8()),
        ("t16", t16()),
        ("t32", t32()),
        ("qt16", qt16()),
        ("st8", st8()),
        ("st16", st16()),
        ("spt16", spt16()),
        ("nt16", nt16()),
        ("qt4", qt4()),
        ("spt4", spt4()),
        ("h", h()),
        ("t2", t2()),
        ("qt8", qt8()),
        ("spt8", spt8()),
        ("qt8_3", qt8_3()),
        ("spt16_6", spt16_6()),
    ]
};

/// Codes of the note values a tuplet of any ratio is written with.
const BASE_CODES: [(&str, NoteValue); 6] = [
    ("w", NoteValue::Whole),
    ("h", NoteValue::Half),
    ("q", Quarter),
    ("e", Eighth),
    ("s", Sixteenth),
    ("th", NoteValue::ThirtySecond),
];

pub(crate) fn duration_of_code(code: &str) -> Option<Duration> {
    CODES.iter().find(|(c, _)| *c == code).map(|(_, d)| *d).or_else(|| general_tuplet(code))
}

/// A tuplet written `n:m` and its note value, e.g. `7:5e` for seven eighths
/// in the time of five.
fn general_tuplet(code: &str) -> Option<Duration> {
    let (n, rest) = code.split_once(':')?;
    let digits = rest.find(|c: char| !c.is_ascii_digit())?;
    let (m, base) = rest.split_at(digits);
    let (_, base) = BASE_CODES.iter().find(|(c, _)| *c == base)?;
    let d = Duration::Tuplet(TupletSpec { n: n.parse().ok()?, m: m.parse().ok()?, base: *base });
    DEFAULT_GRID.supports(&d).then_some(d)
}

pub(crate) fn code_of(duration: &Duration) -> Option<String> {
    if let Some((c, _)) = CODES.iter().find(|(_, d)| d == duration) {
        return Some(c.to_string());
    }
    let Duration::Tuplet(TupletSpec { n, m, base }) = *duration else {
        return None;
    };
    let (code, _) = BASE_CODES.iter().find(|(_, b)| *b == base)?;
    DEFAULT_GRID.supports(duration).then(|| format!("{n}:{m}{code}"))
}

/// Codes of the drum kit's instruments.
const INSTRUMENT_CODES: [(&str, Instrument); 9] = [
    ("K", Instrument::Kick),
    ("S", Instrument::Snare),
    ("H", Instrument::HiHat),
    ("HF", Instrument::HiHatFoot),
    ("RD", Instrument::Ride),
    ("CR", Instrument::Crash),
    ("T1", Instrument::HighTom),
    ("T2", Instrument::MidTom),
    ("FT", Instrument::FloorTom),
];

/// The letter written after an instrument for `technique`, none for the
/// usual stroke. A letter means one technique per instrument (`c` is a
/// closed hi-hat and a choked crash).
const fn technique_letter(technique: Technique) -> Option<char> {
    match technique {
        Technique::Normal => None,
        Technique::Open => Some('o'),
        Technique::HalfOpen => Some('h'),
        Technique::Closed => Some('c'),
        Technique::CrossStick => Some('x'),
        Technique::Rimshot => Some('r'),
        Technique::Bell => Some('b'),
        Technique::Choke => Some('c'),
    }
}

/// Whether `token` is a note of the kit: its heads (after `>` or `(`) start
/// with an instrument's capital, where a plain note starts with a duration
/// and a rest with `r:`.
fn is_kit_token(token: &str) -> bool {
    token.trim_start_matches(['>', '(']).starts_with(|c: char| c.is_ascii_uppercase())
}

/// A head written `S`, `Ho`, `>S` or `(S)`.
fn parse_head(text: &str, token: &str, at: Place) -> Result<Head, NotationError> {
    let unknown = || NotationError::UnknownToken { at, token: token.to_string() };
    let (accented, text) = match text.strip_prefix('>') {
        Some(t) => (true, t),
        None => (false, text),
    };
    let (soft, text) = match text.strip_prefix('(').and_then(|t| t.strip_suffix(')')) {
        Some(t) => (true, t),
        None => (false, text),
    };
    if accented && soft {
        return Err(NotationError::InvalidSoft { at, token: token.to_string() });
    }
    let split = text.find(|c: char| c.is_ascii_lowercase()).unwrap_or(text.len());
    let (code, letter) = text.split_at(split);
    let (_, instrument) = INSTRUMENT_CODES.iter().find(|(c, _)| *c == code).ok_or_else(unknown)?;
    let mut head = Head::new(*instrument);
    if !letter.is_empty() {
        let mut letters = letter.chars();
        let (Some(letter), None) = (letters.next(), letters.next()) else {
            return Err(unknown());
        };
        let technique = instrument
            .techniques()
            .iter()
            .find(|t| technique_letter(**t) == Some(letter))
            .ok_or_else(|| NotationError::InvalidTechnique { at, token: token.to_string() })?;
        head = head.with_technique(*technique).expect("one of the instrument's techniques");
    }
    head.set_accented(accented);
    head.set_soft(soft);
    Ok(head)
}

/// A note of the kit: its heads, joined by `-`, a colon and the duration,
/// e.g. `K-(S):e`.
fn parse_kit_beat(token: &str, at: Place) -> Result<Beat, NotationError> {
    let unknown = || NotationError::UnknownToken { at, token: token.to_string() };
    let (heads, code) = token.split_once(':').ok_or_else(unknown)?;
    let duration = duration_of_code(code).ok_or_else(unknown)?;
    let heads =
        heads.split('-').map(|head| parse_head(head, token, at)).collect::<Result<Vec<_>, _>>()?;
    let heads = Heads::new(heads)
        .map_err(|_| NotationError::InvalidChord { at, token: token.to_string() })?;
    Ok(Beat::with_heads(duration, heads))
}

fn parse_beat(token: &str, at: Place) -> Result<Beat, NotationError> {
    if is_kit_token(token) {
        return parse_kit_beat(token, at);
    }
    if let Some(inner) = token.strip_prefix('(').and_then(|t| t.strip_suffix(')')) {
        let mut beat = parse_beat(inner, at)?;
        if beat.is_rest() || beat.accented() {
            return Err(NotationError::InvalidSoft { at, token: token.to_string() });
        }
        beat.set_soft(true);
        return Ok(beat);
    }
    let (accented, rest_of) = match token.strip_prefix('>') {
        Some(t) => (true, t),
        None => (false, token),
    };
    let (kind, code) = match rest_of.strip_prefix("r:") {
        Some(t) => (Rest, t),
        None => (Note, rest_of),
    };
    let duration = duration_of_code(code)
        .ok_or_else(|| NotationError::UnknownToken { at, token: token.to_string() })?;
    if accented && kind == Rest {
        return Err(NotationError::AccentedRest { at, token: token.to_string() });
    }
    let mut beat = Beat::new(duration, kind);
    beat.set_accented(accented);
    Ok(beat)
}

/// Format `ticks` as a reduced fraction of a whole note, e.g. "1/4".
fn ticks_as_fraction(ticks: u32) -> String {
    let whole = DEFAULT_GRID.ticks_per_whole;
    let g = crate::math::gcd(ticks, whole);
    format!("{}/{}", ticks / g, whole / g)
}

fn build_measure(
    ts: TimeSignature,
    tokens: &[(&str, Beat)],
    at: Place,
) -> Result<Measure, NotationError> {
    let mut m = Measure::new(ts);
    for (idx, (token, beat)) in tokens.iter().enumerate() {
        if idx >= m.first_voice().beats().len() {
            return Err(NotationError::TooLong { at, token: token.to_string() });
        }
        m.first_voice_mut().set_beat(idx, *beat).map_err(|err| match err {
            MeasureError::Overflow { .. } => {
                NotationError::TooLong { at, token: token.to_string() }
            }
            MeasureError::Unfillable { .. } => {
                NotationError::DoesNotFit { at, token: token.to_string() }
            }
        })?;
        // `set_beat` keeps the accent of the slot it overwrites; play ours.
        m.first_voice_mut().set_heads(idx, *beat.heads());
    }
    if m.first_voice().beats().len() > tokens.len() {
        let missing: u32 = m.first_voice().beats()[tokens.len()..]
            .iter()
            .map(|b| DEFAULT_GRID.ticks_of(&b.duration).unwrap_or(0))
            .sum();
        return Err(NotationError::TooShort { at, missing: ticks_as_fraction(missing) });
    }
    Ok(m)
}

/// Parse a score from the text notation described in the [module docs](self).
pub fn parse_score(input: &str) -> Result<Score, NotationError> {
    let (marked_kit, input) = match input.trim_start().split_once(char::is_whitespace) {
        Some((KIT_MARKER, rest)) => (true, rest),
        _ if input.trim() == KIT_MARKER => (true, ""),
        _ => (false, input),
    };
    if input.trim().is_empty() {
        return Err(NotationError::Empty);
    }
    // A trailing `|` (closing bar line) is allowed.
    let input = input.trim_end();
    let input = input.strip_suffix('|').unwrap_or(input);
    let chunks: Vec<&str> = input.split('|').collect();
    if chunks.len() > MAX_MEASURES {
        return Err(NotationError::TooManyMeasures);
    }

    let mut ts = TimeSignature::FOUR_FOUR;
    let mut measures = Vec::with_capacity(chunks.len());
    for (i, chunk) in chunks.iter().enumerate() {
        let mut measure: Option<Measure> = None;
        for (v, text) in chunk.split(';').enumerate() {
            let at = Place { measure: i + 1, voice: v + 1 };
            let mut beats: Vec<(&str, Beat)> = Vec::new();
            for token in text.split_whitespace() {
                // Duration codes never contain '/', so any token with one is a meter.
                if token.contains('/') {
                    let new_ts = token.parse::<TimeSignature>().map_err(|reason| {
                        NotationError::InvalidTimeSignature { at, token: token.to_string(), reason }
                    })?;
                    if !beats.is_empty() || v > 0 {
                        return Err(NotationError::MisplacedTimeSignature {
                            at,
                            token: token.to_string(),
                        });
                    }
                    ts = new_ts;
                    continue;
                }
                beats.push((token, parse_beat(token, at)?));
            }
            if beats.is_empty() {
                return Err(NotationError::EmptyMeasure { at });
            }
            let voice = build_measure(ts, &beats, at)?;
            match &mut measure {
                None => measure = Some(voice),
                Some(m) => m.push_voices(&voice).expect("every voice in the measure's meter"),
            }
        }
        measures.extend(measure);
    }
    // A score with an instrument of the kit is written on the kit's staff.
    let mut score = Score::new(measures);
    if marked_kit || score.has_kit_heads() {
        score.staff = StaffKind::Kit;
    }
    Ok(score)
}

/// The token that puts a score without notes of the kit on the kit's staff.
const KIT_MARKER: &str = "kit";

/// Format `score` in the text notation. The first measure always states its
/// time signature; later ones only when it changes. Returns `None` if a beat
/// has a duration without a notation code (not produced by the editor or the
/// generator), or a head no notation (the one line's in a chord).
pub fn format_score(score: &Score) -> Option<String> {
    let mut out = String::new();
    // Its notes do not tell the kit's staff.
    if score.staff == StaffKind::Kit && !score.has_kit_heads() {
        out.push_str(KIT_MARKER);
        out.push(' ');
    }
    let mut prev_ts: Option<TimeSignature> = None;
    for (i, m) in score.measures.iter().enumerate() {
        if i > 0 {
            out.push_str(" | ");
        }
        let ts = m.time_signature();
        if prev_ts != Some(ts) {
            out.push_str(&format!("{ts} "));
            prev_ts = Some(ts);
        }
        let voices: Option<Vec<String>> = m
            .voices()
            .iter()
            .map(|v| Some(v.beats().iter().map(format_beat).collect::<Option<Vec<_>>>()?.join(" ")))
            .collect();
        out.push_str(&voices?.join(" ; "));
    }
    Some(out)
}

/// `beat` as a token: a rest, a note of the one line, or a note of the kit.
fn format_beat(beat: &Beat) -> Option<String> {
    let code = code_of(&beat.duration)?;
    let heads: Vec<&Head> = beat.heads().iter().collect();
    Some(match heads[..] {
        [] => format!("r:{code}"),
        [head] if head.instrument() == Instrument::Rhythm => match (head.accented(), head.soft()) {
            (true, _) => format!(">{code}"),
            (false, true) => format!("({code})"),
            (false, false) => code,
        },
        _ => {
            let heads: Option<Vec<String>> = heads.into_iter().map(format_head).collect();
            format!("{}:{code}", heads?.join("-"))
        }
    })
}

/// `head` of the kit as written in a chord, e.g. `>S`, `(Sx)` or `Ho`.
fn format_head(head: &Head) -> Option<String> {
    let (code, _) = INSTRUMENT_CODES.iter().find(|(_, i)| *i == head.instrument())?;
    let letter = technique_letter(head.technique()).map(String::from).unwrap_or_default();
    let text = format!("{code}{letter}");
    Some(match (head.accented(), head.soft()) {
        (true, _) => format!(">{text}"),
        (false, true) => format!("({text})"),
        (false, false) => text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duration::{e, q, qt16, s, t8};
    use crate::generator::{GeneratorSettings, Rng, Subdivision, generate_score};

    fn q_dot() -> Duration {
        Duration::Dotted { base: Quarter, dots: 1 }
    }

    fn beats_of(score: &Score, measure: usize) -> Vec<Beat> {
        score.measures[measure].first_voice().beats().clone()
    }

    /// Which beats share a tuplet group, independent of the group ids: each
    /// beat maps to the index of the first beat in its group (`None` outside
    /// tuplets).
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

    #[test]
    fn parses_simple_measure_with_default_time_signature() {
        let score = parse_score("q e e r:q >q").unwrap();
        assert_eq!(score.len(), 1);
        assert_eq!(score.measures[0].time_signature(), TimeSignature::FOUR_FOUR);
        let mut accented = Beat::note(q());
        accented.set_accented(true);
        assert_eq!(
            beats_of(&score, 0),
            vec![Beat::note(q()), Beat::note(e()), Beat::note(e()), Beat::rest(q()), accented]
        );
    }

    #[test]
    fn parses_tuplets_into_one_group() {
        let score = parse_score("t8 r:t8 t8 q qt16 qt16 qt16 qt16 qt16 q").unwrap();
        let beats = beats_of(&score, 0);
        assert_eq!(beats.len(), 10);
        assert_eq!(beats[1], Beat::rest(t8()));
        let group = beats[0].tuplet_group_id;
        assert!(group.is_some());
        assert!(beats[..3].iter().all(|b| b.tuplet_group_id == group));
        assert!(beats[4..9].iter().all(|b| b.duration == qt16()));
        assert_ne!(beats[4].tuplet_group_id, group);
    }

    #[test]
    fn time_signature_carries_over_and_changes() {
        let score = parse_score("3/4 q q q | q q q | 7/8 q q q e").unwrap();
        let ts: Vec<_> = score.measures.iter().map(|m| m.time_signature()).collect();
        assert_eq!(
            ts,
            vec![TimeSignature::THREE_FOUR, TimeSignature::THREE_FOUR, TimeSignature::SEVEN_EIGHT]
        );
    }

    #[test]
    fn soft_notes_are_written_in_parentheses() {
        let score = parse_score("4/4 q (q) >q (e) e").unwrap();
        let beats = beats_of(&score, 0);
        assert!(beats[1].soft() && !beats[1].accented());
        assert!(beats[2].accented() && !beats[2].soft());
        assert!(beats[3].soft());
        assert_eq!(format_score(&score).unwrap(), "4/4 q (q) >q (e) e");
        assert!(matches!(parse_score("(r:q) q q q"), Err(NotationError::InvalidSoft { .. })));
        assert!(parse_score(">(q) q q q").is_err());
    }

    #[test]
    fn dotted_and_accented_beats() {
        let score = parse_score("q. >e s s s s q").unwrap();
        let beats = beats_of(&score, 0);
        assert_eq!(beats[0].duration, Duration::Dotted { base: Quarter, dots: 1 });
        assert!(beats[1].accented());
        assert_eq!(beats[2].duration, s());
    }

    #[test]
    fn too_short_measure_reports_missing_amount() {
        assert_eq!(
            parse_score("q q q").unwrap_err(),
            NotationError::TooShort { at: Place::measure(1), missing: "1/4".into() }
        );
        assert_eq!(
            parse_score("q q q q | q q q e").unwrap_err(),
            NotationError::TooShort { at: Place::measure(2), missing: "1/8".into() }
        );
    }

    #[test]
    fn too_long_measure_is_rejected() {
        assert!(matches!(
            parse_score("q q q q e").unwrap_err(),
            NotationError::TooLong { at: Place { measure: 1, voice: 1 }, .. }
        ));
        assert!(matches!(
            parse_score("q q q e. s s").unwrap_err(),
            NotationError::TooLong { at: Place { measure: 1, voice: 1 }, .. }
        ));
    }

    #[test]
    fn incomplete_tuplet_is_rejected() {
        assert_eq!(
            parse_score("t8 t8 q q q").unwrap_err(),
            NotationError::DoesNotFit { at: Place::measure(1), token: "q".into() }
        );
    }

    #[test]
    fn tuplets_of_any_ratio_are_written_n_m_and_their_value() {
        let score = parse_score("5/8 7:5e 7:5e r:7:5e 7:5e >7:5e 7:5e (7:5e)").unwrap();
        let beats = beats_of(&score, 0);
        let seven = Duration::Tuplet(TupletSpec { n: 7, m: 5, base: Eighth });
        assert!(beats.iter().all(|b| b.duration == seven));
        assert_eq!(group_partition(&score.measures[0]), vec![Some(0); 7]);
        assert_eq!(format_score(&score).unwrap(), "5/8 7:5e 7:5e r:7:5e 7:5e >7:5e 7:5e (7:5e)");
        // A listed tuplet keeps its code.
        assert_eq!(
            format_score(&parse_score("3:2e 3:2e 3:2e q q q").unwrap()).unwrap(),
            "4/4 t8 t8 t8 q q q"
        );
        // Off the grid, too many notes, or a plain value in disguise.
        for token in ["11:8th", "17:16th", "4:4q", "7:5x", "7:e", ":5e"] {
            assert!(
                matches!(parse_score(token), Err(NotationError::UnknownToken { .. })),
                "{token}"
            );
        }
    }

    #[test]
    fn invalid_tokens_are_reported() {
        assert_eq!(
            parse_score("q q x q").unwrap_err(),
            NotationError::UnknownToken { at: Place::measure(1), token: "x".into() }
        );
        assert_eq!(
            parse_score(">r:q q q q").unwrap_err(),
            NotationError::AccentedRest { at: Place::measure(1), token: ">r:q".into() }
        );
        assert!(matches!(
            parse_score("4/5 q").unwrap_err(),
            NotationError::InvalidTimeSignature { .. }
        ));
        assert!(matches!(
            parse_score("q 3/4 q q").unwrap_err(),
            NotationError::MisplacedTimeSignature { .. }
        ));
        assert_eq!(
            parse_score("q q q q | | q q q q").unwrap_err(),
            NotationError::EmptyMeasure { at: Place::measure(2) }
        );
        assert_eq!(
            parse_score("|").unwrap_err(),
            NotationError::EmptyMeasure { at: Place::measure(1) }
        );
        assert_eq!(parse_score("  ").unwrap_err(), NotationError::Empty);
    }

    #[test]
    fn trailing_bar_line_is_allowed() {
        assert_eq!(parse_score("q q q q | e e q q q |").unwrap().len(), 2);
    }

    #[test]
    fn too_many_measures_are_rejected() {
        let input = vec!["q q q q"; MAX_MEASURES + 1].join(" | ");
        assert_eq!(parse_score(&input).unwrap_err(), NotationError::TooManyMeasures);
    }

    #[test]
    fn format_states_time_signature_only_on_change() {
        let score = parse_score("3/4 q q q | q r:q q | 4/4 >q q t8 t8 t8 q").unwrap();
        assert_eq!(format_score(&score).unwrap(), "3/4 q q q | q r:q q | 4/4 >q q t8 t8 t8 q");
    }

    #[test]
    fn groupings_carry_over_and_change_like_meters() {
        let text = "7/8 q e q q | 2+2+3/8 q q e q | q q q e";
        let score = parse_score(text).unwrap();
        let ts: Vec<_> = score.measures.iter().map(|m| m.time_signature()).collect();
        let grouped = TimeSignature::SEVEN_EIGHT.with_grouping(1);
        assert_eq!(ts, [TimeSignature::SEVEN_EIGHT, grouped, grouped]);
        assert_eq!(format_score(&score).unwrap(), text);
    }

    #[test]
    fn round_trip_of_generated_scores() {
        for subdivision in [
            Subdivision::Eighths,
            Subdivision::Sixteenths,
            Subdivision::Triplets,
            Subdivision::Mixed,
        ] {
            for complexity in 1..=5 {
                let settings = GeneratorSettings {
                    subdivision,
                    complexity,
                    bars: 4,
                    space: 0.3,
                    ..GeneratorSettings::default()
                };
                let score = generate_score(&settings, &mut Rng::new(complexity as u64)).unwrap();
                let text = format_score(&score).unwrap();
                let parsed = parse_score(&text).unwrap();
                assert_eq!(score.len(), parsed.len(), "{text}");
                for (a, b) in score.measures.iter().zip(&parsed.measures) {
                    assert_eq!(a.first_voice().beats(), b.first_voice().beats(), "{text}");
                    assert_eq!(a.time_signature(), b.time_signature());
                    // `Beat`'s `PartialEq` ignores tuplet groups; compare them too.
                    assert_eq!(group_partition(a), group_partition(b), "{text}");
                }
            }
        }
    }

    #[test]
    fn a_kit_groove_reads_and_writes_back() {
        use crate::Technique::{Closed, Open};
        let text = "4/4 H:e S-H:e H:e H:e H:e S-H:e Ho:e Hc:e ; K:q r:q K:q r:q";
        let score = parse_score(text).unwrap();
        let m = &score.measures[0];
        assert_eq!(m.voices().len(), 2);
        let hands = m.voice(0).unwrap().beats();
        let heads = |b: &Beat| b.heads().iter().map(|h| h.instrument()).collect::<Vec<_>>();
        assert_eq!(heads(&hands[1]), [Instrument::Snare, Instrument::HiHat]);
        assert_eq!(hands[6].heads().get(Instrument::HiHat).unwrap().technique(), Open);
        assert_eq!(hands[7].heads().get(Instrument::HiHat).unwrap().technique(), Closed);
        let feet = m.voice(1).unwrap().beats();
        assert_eq!(heads(&feet[0]), [Instrument::Kick]);
        assert!(feet[1].is_rest());
        assert_eq!(format_score(&score).unwrap(), text);
    }

    #[test]
    fn a_kit_score_without_kit_notes_keeps_its_staff() {
        let score = parse_score("kit 4/4 r:h r:h ; r:h r:h").unwrap();
        assert_eq!(score.staff, StaffKind::Kit);
        assert_eq!(format_score(&score).unwrap(), "kit 4/4 r:h r:h ; r:h r:h");
        let empty = Score {
            staff: StaffKind::Kit,
            ..Score::single(Measure::new(TimeSignature::new(3, 4)))
        };
        let text = format_score(&empty).unwrap();
        assert!(text.starts_with("kit 3/4 "), "{text}");
        assert_eq!(parse_score(&text).unwrap().staff, StaffKind::Kit);
        // Notes of the kit tell the staff themselves; the marker reads anyway.
        assert_eq!(format_score(&parse_score("kit K:h r:h").unwrap()).unwrap(), "4/4 K:h r:h");
        // Without it a score of rests is on the one line, as before.
        assert_eq!(parse_score("4/4 r:h r:h").unwrap().staff, StaffKind::Line);
        assert_eq!(parse_score("kit").unwrap_err(), NotationError::Empty);
        assert_eq!(parse_score("  kit  ").unwrap_err(), NotationError::Empty);
    }

    #[test]
    fn kit_heads_carry_their_own_accent_ghost_and_technique() {
        // Any order reads; heads are written from the foot up.
        let score = parse_score("2/4 H-K:e >S-H:e K-(Sx):e RDb-CRc-T1r-HF:e").unwrap();
        assert_eq!(format_score(&score).unwrap(), "2/4 K-H:e >S-H:e K-(Sx):e HF-T1r-RDb-CRc:e");
        let beats = beats_of(&score, 0);
        let snare = beats[1].heads().get(Instrument::Snare).unwrap();
        assert!(snare.accented() && !beats[1].heads().get(Instrument::HiHat).unwrap().accented());
        let ghost = beats[2].heads().get(Instrument::Snare).unwrap();
        assert!(ghost.soft() && ghost.technique() == Technique::CrossStick);
        // Tuplets, dotted notes and ratios work as for any note of the line.
        let text = "2/4 S:t8 (S):t8 >S:t8 K:q | K:q. H:e | 5/8 S:7:5e (S):7:5e r:7:5e S:7:5e S:7:5e \
                    S:7:5e K:7:5e";
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let score = parse_score(&text).unwrap();
        assert_eq!(format_score(&score).unwrap(), text);
        let line =
            parse_score("2/4 t8 t8 t8 q | q. e | 5/8 7:5e 7:5e 7:5e 7:5e 7:5e 7:5e 7:5e").unwrap();
        for (kit, plain) in score.measures.iter().zip(&line.measures) {
            assert_eq!(group_partition(kit), group_partition(plain));
        }
    }

    #[test]
    fn every_head_of_the_kit_reads_back_as_written() {
        for &(_, instrument) in &INSTRUMENT_CODES {
            for &technique in instrument.techniques() {
                for (accented, soft) in [(false, false), (true, false), (false, true)] {
                    let mut head = Head::new(instrument).with_technique(technique).unwrap();
                    head.set_accented(accented);
                    head.set_soft(soft);
                    let beat = Beat::with_heads(q(), Heads::one(head));
                    let text = format!("1/4 {}", format_beat(&beat).unwrap());
                    let back = parse_score(&text).unwrap();
                    assert_eq!(beats_of(&back, 0), [beat], "{text}");
                }
            }
        }
        // Every kit instrument has a code; the one line has none.
        let coded = INSTRUMENT_CODES.map(|(_, i)| i);
        assert!(Instrument::ALL.iter().all(|i| coded.contains(i) == (*i != Instrument::Rhythm)));
    }

    #[test]
    fn voices_read_back_in_every_meter_and_measure() {
        // Simple, compound and odd meters; voices in some measures only, a
        // meter change, tuplets in a second voice.
        let text = "4/4 H:e S-H:e H:e H:e H:e S-H:e H:e H:e ; K:q r:q K:q r:q \
                    | 6/8 RD:e RD:e RD:e >RDb:e RD:e RD:e ; K:q. HF:q. \
                    | (S):e S:e S:e S:e S:e S:e \
                    | 7/8 H:e H:e H:e H:e H:e H:e H:e ; K:q r:e K:t8 K:t8 K:t8 r:e K:e";
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let score = parse_score(&text).unwrap();
        let voices: Vec<usize> = score.measures.iter().map(|m| m.voices().len()).collect();
        assert_eq!(voices, [2, 2, 1, 2]);
        assert_eq!(score.measures[1].time_signature(), TimeSignature::SIX_EIGHT);
        // The 6/8 feet keep their meter's beats: two dotted quarters.
        let feet = score.measures[1].voice(1).unwrap().beats();
        assert_eq!(feet.iter().map(|b| b.duration).collect::<Vec<_>>(), [q_dot(), q_dot()]);
        let feet = score.measures[3].voice(1).unwrap();
        assert!(feet.tuplet_group_at(2).is_some());
        assert_eq!(format_score(&score).unwrap(), text);
    }

    #[test]
    fn kit_mistakes_name_measure_voice_and_token() {
        let err = |text: &str| parse_score(text).unwrap_err();
        let at = Place::measure(1);
        let token = |t: &str| t.to_string();
        assert_eq!(err("Sb:e e q h"), NotationError::InvalidTechnique { at, token: token("Sb:e") });
        assert_eq!(err("K-K:e e q h"), NotationError::InvalidChord { at, token: token("K-K:e") });
        assert_eq!(
            err("K-S-H-HF-CR:e e q h"),
            NotationError::InvalidChord { at, token: token("K-S-H-HF-CR:e") }
        );
        assert_eq!(err(">(S):e e q h"), NotationError::InvalidSoft { at, token: token(">(S):e") });
        for bad in ["X:e", "K:", "K:x", "Hoo:e", "H", "k:e", "K-:e"] {
            assert_eq!(
                err(&format!("{bad} e q h")),
                NotationError::UnknownToken { at, token: token(bad) },
                "{bad}"
            );
        }
        let second = Place { measure: 2, voice: 2 };
        let e = err("h h | h h ; h q");
        assert_eq!(e, NotationError::TooShort { at: second, missing: "1/4".into() });
        assert!(e.to_string().starts_with("measure 2, voice 2 is too short"), "{e}");
        assert_eq!(
            err("h h ;"),
            NotationError::EmptyMeasure { at: Place { measure: 1, voice: 2 } }
        );
        assert_eq!(
            err("h h ; 3/4 h h"),
            NotationError::MisplacedTimeSignature {
                at: Place { measure: 1, voice: 2 },
                token: token("3/4")
            }
        );
    }

    #[test]
    fn eighth_based_meters_start_from_eighth_rests() {
        let score = parse_score("6/8 e e e q.").unwrap();
        assert_eq!(beats_of(&score, 0).len(), 4);
        assert_eq!(format_score(&score).unwrap(), "6/8 e e e q.");
    }
}
