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
//!   `q` `e` `s` `th` (quarter to thirty-second), dotted with a trailing `.`
//!   (`q.` `e.` `s.`), tuplets `t4` `t8` `t16` `t32` (triplets), `qt16`
//!   (quintuplet), `st8` `st16` (sextuplets), `spt16` (septuplet) and `nt16`
//!   (nonuplet). Every note of a tuplet group is written out.
//! - `r:` makes a rest (`r:q`), `>` accents a note (`>e`).
//!
//! Parsing goes through [`Measure::set_beat`], so a parsed score obeys the
//! same invariants as an edited one. Every measure must be filled exactly;
//! errors name the measure and token so the input can be corrected.

use crate::BeatKind::{Note, Rest};
use crate::duration::NoteValue::{Eighth, Quarter, Sixteenth, ThirtySecond};
use crate::duration::{Duration, NoteValue, TupletSpec};
use crate::grid::DEFAULT_GRID;
use crate::{Beat, Measure, MeasureError, Score, TimeSignature};
use std::fmt::{Display, Formatter};

/// Upper bound for the number of measures in a parsed score.
pub const MAX_MEASURES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotationError {
    /// The input contains no beats at all.
    Empty,
    /// More than [`MAX_MEASURES`] measures.
    TooManyMeasures,
    /// A token that is not a duration, rest, accent or time signature.
    UnknownToken { measure: usize, token: String },
    /// A time signature that is malformed or not supported.
    InvalidTimeSignature { measure: usize, token: String },
    /// A time signature in the middle of a measure.
    MisplacedTimeSignature { measure: usize, token: String },
    /// `>` on a rest.
    AccentedRest { measure: usize, token: String },
    /// A measure without beats (e.g. `q q q q | | q q q q`).
    EmptyMeasure { measure: usize },
    /// The beats exceed the measure length.
    TooLong { measure: usize, token: String },
    /// The beats do not fill the measure; `missing` is a fraction of a whole note.
    TooShort { measure: usize, missing: String },
    /// The beat cannot be placed here, e.g. an incomplete tuplet group or a
    /// duration that would leave an unfillable gap.
    DoesNotFit { measure: usize, token: String },
}

impl Display for NotationError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        use NotationError::*;
        match self {
            Empty => write!(f, "the rhythm is empty"),
            TooManyMeasures => write!(f, "more than {MAX_MEASURES} measures"),
            UnknownToken { measure, token } => write!(
                f,
                "measure {measure}: unknown token '{token}' (expected e.g. q, e, s, th, q., t8, qt16, r:q, >e or 4/4)"
            ),
            InvalidTimeSignature { measure, token } => write!(
                f,
                "measure {measure}: unsupported time signature '{token}' (1-17 beats over 4, 8 or 16)"
            ),
            MisplacedTimeSignature { measure, token } => write!(
                f,
                "measure {measure}: time signature '{token}' must come before the first beat of the measure"
            ),
            AccentedRest { measure, token } => {
                write!(f, "measure {measure}: '{token}' - rests cannot be accented")
            }
            EmptyMeasure { measure } => write!(f, "measure {measure} has no beats"),
            TooLong { measure, token } => {
                write!(f, "measure {measure} is too long: '{token}' does not fit anymore")
            }
            TooShort { measure, missing } => {
                write!(f, "measure {measure} is too short: {missing} of a whole note missing")
            }
            DoesNotFit { measure, token } => write!(
                f,
                "measure {measure}: '{token}' cannot be placed here (tuplet groups must be complete, e.g. t8 t8 t8)"
            ),
        }
    }
}

impl std::error::Error for NotationError {}

/// Durations with a notation code, in the order of [`crate::duration::COMMON_DURATIONS`].
const CODES: [(&str, Duration); 16] = {
    use crate::duration::{e, nt16, q, qt16, s, spt16, st8, st16, t8, t16, t32, th};
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
    ]
};

fn duration_from_code(code: &str) -> Option<Duration> {
    CODES.iter().find(|(c, _)| *c == code).map(|(_, d)| *d)
}

fn code_of(duration: &Duration) -> Option<&'static str> {
    CODES.iter().find(|(_, d)| d == duration).map(|(c, _)| *c)
}

fn parse_time_signature(token: &str) -> Option<Result<TimeSignature, ()>> {
    let (beats, unit) = token.split_once('/')?;
    let parsed = match (beats.parse::<u8>(), unit.parse::<u8>()) {
        (Ok(beats @ 1..=17), Ok(beat_unit @ (4 | 8 | 16))) => {
            Ok(TimeSignature { beats, beat_unit })
        }
        _ => Err(()),
    };
    Some(parsed)
}

fn parse_beat(token: &str, measure: usize) -> Result<Beat, NotationError> {
    let (accented, rest_of) = match token.strip_prefix('>') {
        Some(t) => (true, t),
        None => (false, token),
    };
    let (kind, code) = match rest_of.strip_prefix("r:") {
        Some(t) => (Rest, t),
        None => (Note, rest_of),
    };
    let duration = duration_from_code(code)
        .ok_or_else(|| NotationError::UnknownToken { measure, token: token.to_string() })?;
    if accented && kind == Rest {
        return Err(NotationError::AccentedRest { measure, token: token.to_string() });
    }
    let mut beat = Beat::new(duration, kind);
    beat.accented = accented;
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
    measure: usize,
) -> Result<Measure, NotationError> {
    let mut m = Measure::new(ts);
    for (idx, (token, beat)) in tokens.iter().enumerate() {
        if idx >= m.beats().len() {
            return Err(NotationError::TooLong { measure, token: token.to_string() });
        }
        m.set_beat(idx, *beat).map_err(|err| match err {
            MeasureError::Overflow { .. } => {
                NotationError::TooLong { measure, token: token.to_string() }
            }
            MeasureError::Unfillable { .. } => {
                NotationError::DoesNotFit { measure, token: token.to_string() }
            }
        })?;
        // `set_beat` keeps the accent of the slot it overwrites; apply ours.
        if m.beats()[idx].accented != beat.accented {
            m.toggle_accent(idx);
        }
    }
    if m.beats().len() > tokens.len() {
        let missing: u32 = m.beats()[tokens.len()..]
            .iter()
            .map(|b| DEFAULT_GRID.ticks_of(&b.duration).unwrap_or(0))
            .sum();
        return Err(NotationError::TooShort { measure, missing: ticks_as_fraction(missing) });
    }
    Ok(m)
}

/// Parse a score from the text notation described in the [module docs](self).
pub fn parse_score(input: &str) -> Result<Score, NotationError> {
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
        let measure = i + 1;
        let mut beats: Vec<(&str, Beat)> = Vec::new();
        for token in chunk.split_whitespace() {
            if let Some(parsed) = parse_time_signature(token) {
                let Ok(new_ts) = parsed else {
                    return Err(NotationError::InvalidTimeSignature {
                        measure,
                        token: token.to_string(),
                    });
                };
                if !beats.is_empty() {
                    return Err(NotationError::MisplacedTimeSignature {
                        measure,
                        token: token.to_string(),
                    });
                }
                ts = new_ts;
                continue;
            }
            beats.push((token, parse_beat(token, measure)?));
        }
        if beats.is_empty() {
            return Err(NotationError::EmptyMeasure { measure });
        }
        measures.push(build_measure(ts, &beats, measure)?);
    }
    Ok(Score { measures })
}

/// Format `score` in the text notation. The first measure always states its
/// time signature; later ones only when it changes. Returns `None` if a beat
/// has a duration without a notation code (not produced by the editor or the
/// generator).
pub fn format_score(score: &Score) -> Option<String> {
    let mut out = String::new();
    let mut prev_ts: Option<TimeSignature> = None;
    for (i, m) in score.measures.iter().enumerate() {
        if i > 0 {
            out.push_str(" | ");
        }
        let ts = m.time_signature();
        if prev_ts != Some(ts) {
            out.push_str(&format!("{}/{} ", ts.beats, ts.beat_unit));
            prev_ts = Some(ts);
        }
        let tokens: Option<Vec<String>> = m
            .beats()
            .iter()
            .map(|b| {
                let code = code_of(&b.duration)?;
                Some(match (b.kind, b.accented) {
                    (Rest, _) => format!("r:{code}"),
                    (Note, true) => format!(">{code}"),
                    (Note, false) => code.to_string(),
                })
            })
            .collect();
        out.push_str(&tokens?.join(" "));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duration::{e, q, qt16, s, t8};
    use crate::generator::{GeneratorSettings, Rng, Subdivision, generate_score};

    fn beats_of(score: &Score, measure: usize) -> Vec<Beat> {
        score.measures[measure].beats().clone()
    }

    #[test]
    fn parses_simple_measure_with_default_time_signature() {
        let score = parse_score("q e e r:q >q").unwrap();
        assert_eq!(score.len(), 1);
        assert_eq!(score.measures[0].time_signature(), TimeSignature::FOUR_FOUR);
        let mut accented = Beat::note(q());
        accented.accented = true;
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
    fn dotted_and_accented_beats() {
        let score = parse_score("q. >e s s s s q").unwrap();
        let beats = beats_of(&score, 0);
        assert_eq!(beats[0].duration, Duration::Dotted { base: Quarter, dots: 1 });
        assert!(beats[1].accented);
        assert_eq!(beats[2].duration, s());
    }

    #[test]
    fn too_short_measure_reports_missing_amount() {
        assert_eq!(
            parse_score("q q q").unwrap_err(),
            NotationError::TooShort { measure: 1, missing: "1/4".into() }
        );
        assert_eq!(
            parse_score("q q q q | q q q e").unwrap_err(),
            NotationError::TooShort { measure: 2, missing: "1/8".into() }
        );
    }

    #[test]
    fn too_long_measure_is_rejected() {
        assert!(matches!(
            parse_score("q q q q e").unwrap_err(),
            NotationError::TooLong { measure: 1, .. }
        ));
        assert!(matches!(
            parse_score("q q q e. s s").unwrap_err(),
            NotationError::TooLong { measure: 1, .. }
        ));
    }

    #[test]
    fn incomplete_tuplet_is_rejected() {
        assert_eq!(
            parse_score("t8 t8 q q q").unwrap_err(),
            NotationError::DoesNotFit { measure: 1, token: "q".into() }
        );
    }

    #[test]
    fn invalid_tokens_are_reported() {
        assert_eq!(
            parse_score("q q x q").unwrap_err(),
            NotationError::UnknownToken { measure: 1, token: "x".into() }
        );
        assert_eq!(
            parse_score(">r:q q q q").unwrap_err(),
            NotationError::AccentedRest { measure: 1, token: ">r:q".into() }
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
            NotationError::EmptyMeasure { measure: 2 }
        );
        assert_eq!(parse_score("|").unwrap_err(), NotationError::EmptyMeasure { measure: 1 });
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
                for (a, b) in score.measures.iter().zip(&parsed.measures) {
                    assert_eq!(a.beats(), b.beats(), "{text}");
                    assert_eq!(a.time_signature(), b.time_signature());
                }
            }
        }
    }

    #[test]
    fn eighth_based_meters_start_from_eighth_rests() {
        let score = parse_score("6/8 e e e q.").unwrap();
        assert_eq!(beats_of(&score, 0).len(), 4);
        assert_eq!(format_score(&score).unwrap(), "6/8 e e e q.");
    }
}
