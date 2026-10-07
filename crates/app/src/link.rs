//! Shareable links: open a rhythm, a generated exercise or a plain metronome
//! from URL query parameters, and build a link for the working score.
//!
//! | Parameter | Meaning |
//! |-----------|---------|
//! | `bpm`     | Tempo, 20-300 |
//! | `r`       | Rhythm in the text notation of [`grooph_measure::notation`] |
//! | `ts`      | Time signature, e.g. `7/8`; default for `r`, the generator's meter (x/4 only), or on its own a one-measure metronome |
//! | `sub`     | Generator subdivision: `8`, `16`, `3` (triplets) or `mix` |
//! | `lvl`     | Generator complexity 1-5 |
//! | `bars`    | Generator bars 1-8 |
//! | `space`   | Generator rests 0.0-1.0 |
//! | `seed`    | Generator seed; the same parameters and seed give the same rhythm |
//!
//! Any of `sub`, `lvl`, `bars`, `space` or `seed` selects the generator;
//! unknown parameters are ignored.

use grooph_measure::generator::{GeneratorSettings, MAX_BARS, MAX_COMPLEXITY, Subdivision};
use grooph_measure::notation::{format_score, parse_score};
use grooph_measure::tempo::{MAX_BPM, MIN_BPM};
use grooph_measure::{BeatKind, Measure, Score, TimeSignature};

/// What a link opens. `bpm` and `content` are both optional; a link with only
/// `bpm` just sets the tempo.
#[derive(Clone, Debug)]
pub(crate) struct SharedLink {
    pub(crate) bpm: Option<u32>,
    pub(crate) content: Option<LinkContent>,
}

#[derive(Clone, Debug)]
pub(crate) enum LinkContent {
    /// A fixed score (from `r`, or a metronome measure from `ts`).
    Score(Score),
    /// Generator settings; `seed: None` draws a random rhythm.
    Generator { settings: GeneratorSettings, seed: Option<u64> },
}

/// Parse a URL query string (with or without the leading `?`). Returns
/// `Ok(None)` if it contains none of the link parameters.
pub(crate) fn parse_query(query: &str) -> Result<Option<SharedLink>, String> {
    let mut bpm = None;
    let mut rhythm = None;
    let mut ts = None;
    let mut sub = None;
    let mut lvl = None;
    let mut bars = None;
    let mut space = None;
    let mut seed = None;

    for pair in query.trim_start_matches('?').split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = percent_decode(value);
        match key {
            "bpm" => bpm = Some(parse_bpm(&value)?),
            "r" => rhythm = Some(value),
            "ts" => {
                ts = Some(
                    value
                        .parse::<TimeSignature>()
                        .map_err(|e| format!("time signature '{value}': {e}"))?,
                )
            }
            "sub" => sub = Some(parse_subdivision(&value)?),
            "lvl" => lvl = Some(parse_in_range("lvl", &value, 1, MAX_COMPLEXITY)?),
            "bars" => bars = Some(parse_in_range("bars", &value, 1, MAX_BARS)?),
            "space" => space = Some(parse_space(&value)?),
            "seed" => {
                seed = Some(value.parse::<u64>().map_err(|_| format!("invalid seed '{value}'"))?)
            }
            _ => {}
        }
    }

    let generator =
        sub.is_some() || lvl.is_some() || bars.is_some() || space.is_some() || seed.is_some();
    let content = if let Some(text) = rhythm {
        if generator {
            return Err("use either a rhythm (r) or generator parameters, not both".into());
        }
        // A leading time signature in `r` overrides `ts`.
        let text = match ts {
            Some(ts) => format!("{ts} {text}"),
            None => text,
        };
        Some(LinkContent::Score(parse_score(&text).map_err(|e| format!("rhythm: {e}"))?))
    } else if generator {
        let defaults = GeneratorSettings::default();
        let time_signature = ts.unwrap_or(defaults.time_signature);
        if time_signature.beat_unit != 4 {
            return Err("the generator supports only x/4 time signatures".into());
        }
        let settings = GeneratorSettings {
            subdivision: sub.unwrap_or(defaults.subdivision),
            complexity: lvl.unwrap_or(defaults.complexity),
            bars: bars.unwrap_or(defaults.bars),
            time_signature,
            space: space.unwrap_or(defaults.space),
            custom_groupings_enabled: false,
            ..defaults
        };
        Some(LinkContent::Generator { settings, seed })
    } else {
        ts.map(|ts| LinkContent::Score(Score::single(Measure::new_init(ts, BeatKind::Note))))
    };

    if bpm.is_none() && content.is_none() {
        Ok(None)
    } else {
        Ok(Some(SharedLink { bpm, content }))
    }
}

/// Link that opens `score` at `bpm`, or `None` if the score cannot be written
/// in the text notation.
pub(crate) fn share_url(base: &str, score: &Score, bpm: u32) -> Option<String> {
    let rhythm = format_score(score)?;
    Some(format!("{base}?bpm={bpm}&r={}", percent_encode(&rhythm)))
}

fn parse_bpm(value: &str) -> Result<u32, String> { parse_in_range("bpm", value, MIN_BPM, MAX_BPM) }

fn parse_in_range<T>(name: &str, value: &str, min: T, max: T) -> Result<T, String>
where
    T: std::str::FromStr + PartialOrd + std::fmt::Display,
{
    match value.parse::<T>() {
        Ok(v) if v >= min && v <= max => Ok(v),
        _ => Err(format!("{name} must be a number from {min} to {max}, got '{value}'")),
    }
}

fn parse_subdivision(value: &str) -> Result<Subdivision, String> {
    match value {
        "8" => Ok(Subdivision::Eighths),
        "16" => Ok(Subdivision::Sixteenths),
        "3" => Ok(Subdivision::Triplets),
        "mix" => Ok(Subdivision::Mixed),
        _ => Err(format!("sub must be 8, 16, 3 or mix, got '{value}'")),
    }
}

fn parse_space(value: &str) -> Result<f32, String> {
    match value.parse::<f32>() {
        Ok(v) if (0.0..=1.0).contains(&v) => Ok(v),
        _ => Err(format!("space must be a number from 0 to 1, got '{value}'")),
    }
}

/// Decode `application/x-www-form-urlencoded` text (`+` is a space).
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = value.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(b) => {
                        out.push(b);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Encode a query value; keeps the notation's `/`, `:` and `.` readable and
/// writes spaces as `+`.
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b':' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use grooph_measure::duration::{e, q};
    use grooph_measure::{Beat, generator::Subdivision};

    fn parse(query: &str) -> SharedLink { parse_query(query).unwrap().unwrap() }

    #[test]
    fn query_without_link_parameters_is_ignored() {
        assert!(parse_query("").unwrap().is_none());
        assert!(parse_query("?utm_source=x").unwrap().is_none());
    }

    #[test]
    fn rhythm_link_with_tempo() {
        let link = parse("?bpm=90&r=q+e+e+r:q+%3Eq&utm_source=chat");
        assert_eq!(link.bpm, Some(90));
        let Some(LinkContent::Score(score)) = link.content else { panic!("expected a score") };
        let beats = score.measures[0].beats();
        assert_eq!(beats[0], Beat::note(q()));
        assert_eq!(beats[1], Beat::note(e()));
        assert!(beats[4].accented);
    }

    #[test]
    fn literal_bar_lines_and_ts_parameter() {
        let link = parse("ts=3/4&r=q q q | q r:q q");
        let Some(LinkContent::Score(score)) = link.content else { panic!("expected a score") };
        assert_eq!(score.len(), 2);
        assert_eq!(score.measures[1].time_signature(), TimeSignature::THREE_FOUR);
    }

    #[test]
    fn time_signature_in_rhythm_overrides_ts() {
        let link = parse("ts=3/4&r=4/4+q+q+q+q");
        let Some(LinkContent::Score(score)) = link.content else { panic!("expected a score") };
        assert_eq!(score.measures[0].time_signature(), TimeSignature::FOUR_FOUR);
    }

    #[test]
    fn generator_link() {
        let link = parse("bpm=80&ts=3/4&sub=16&lvl=3&bars=4&space=0.25&seed=1234");
        let Some(LinkContent::Generator { settings, seed }) = link.content else {
            panic!("expected generator settings")
        };
        assert_eq!(settings.subdivision, Subdivision::Sixteenths);
        assert_eq!(settings.complexity, 3);
        assert_eq!(settings.bars, 4);
        assert_eq!(settings.space, 0.25);
        assert_eq!(settings.time_signature, TimeSignature::THREE_FOUR);
        assert!(!settings.custom_groupings_enabled);
        assert_eq!(seed, Some(1234));
    }

    #[test]
    fn generator_defaults_and_random_seed() {
        let link = parse("sub=mix");
        let Some(LinkContent::Generator { settings, seed }) = link.content else {
            panic!("expected generator settings")
        };
        assert_eq!(settings.subdivision, Subdivision::Mixed);
        assert_eq!(settings.complexity, GeneratorSettings::default().complexity);
        assert_eq!(seed, None);
    }

    #[test]
    fn metronome_link() {
        let link = parse("bpm=140&ts=7/8");
        let Some(LinkContent::Score(score)) = link.content else { panic!("expected a score") };
        let m = &score.measures[0];
        assert_eq!(m.time_signature(), TimeSignature::SEVEN_EIGHT);
        assert_eq!(m.beats().len(), 7);
        assert!(m.beats().iter().all(|b| b.kind == BeatKind::Note));
    }

    #[test]
    fn tempo_only_link() {
        let link = parse("bpm=60");
        assert_eq!(link.bpm, Some(60));
        assert!(link.content.is_none());
    }

    #[test]
    fn invalid_parameters_are_reported() {
        assert!(parse_query("bpm=500").unwrap_err().contains("bpm"));
        assert!(parse_query("lvl=9").unwrap_err().contains("lvl"));
        assert!(parse_query("sub=4").unwrap_err().contains("sub"));
        assert!(parse_query("ts=7/8&sub=16").unwrap_err().contains("x/4"));
        assert!(parse_query("r=q+q+q").unwrap_err().contains("too short"));
        assert!(parse_query("r=q+q+q+q&seed=1").unwrap_err().contains("not both"));
    }

    #[test]
    fn share_url_round_trips() {
        let score = parse_score("3/4 >q t8 t8 r:t8 e e | e. s q r:q").unwrap();
        let url = share_url("https://grooph.app/", &score, 96).unwrap();
        assert_eq!(url, "https://grooph.app/?bpm=96&r=3/4+%3Eq+t8+t8+r:t8+e+e+%7C+e.+s+q+r:q");
        let link = parse(url.split_once('?').unwrap().1);
        assert_eq!(link.bpm, Some(96));
        let Some(LinkContent::Score(parsed)) = link.content else { panic!("expected a score") };
        assert_eq!(format_score(&parsed), format_score(&score));
    }

    #[test]
    fn percent_decoding_handles_utf8_and_bad_escapes() {
        assert_eq!(percent_decode("a%20b+c"), "a b c");
        assert_eq!(percent_decode("%E2%86%92"), "→");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }
}
