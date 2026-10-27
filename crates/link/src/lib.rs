//! Shareable links: open a rhythm, a generated exercise or a plain metronome
//! from URL query parameters, and build such links.
//!
//! | Parameter | Meaning |
//! |-----------|---------|
//! | `v`       | Link version, see below; [`SharedLink::to_url`] always writes the current one |
//! | `bpm`     | Tempo in quarter notes per minute, 20-300 |
//! | `r`       | Rhythm in the text notation of [`grooph_measure::notation`] |
//! | `ts`      | Time signature, e.g. `7/8`; default for `r`, the generator's meter, or on its own the metronome in that meter (the rhythm stays) |
//! | `sub`     | Generator subdivision: `8`, `16`, `3` (triplets) or `mix` |
//! | `lvl`     | Generator complexity 1-5 |
//! | `bars`    | Generator bars 1-8 |
//! | `space`   | Generator rests 0.0-1.0 |
//! | `seed`    | Generator seed; the same parameters and seed give the same rhythm in the same app version (a generator update may change it) |
//! | `swing`   | Swing in percent, 50 (straight) to 75 (dotted); 66 is a triplet feel |
//! | `su`      | Swung note value: `8` (default) or `16`; needs `swing` |
//! | `click`   | Click bar of the meter of `r` (its first measure) or `ts`: one measure in the text notation, without the time signature (`>` strong, plain medium, `(e)` soft, rest off) |
//! | `poly`    | Polyrhythm `n:m` (n clicks in the time of m beat units) the metronome plays against the click bar of `ts`, its counter bar built as the app's picker does (see [`grooph_measure::counter`]); needs `ts` without `r` or the generator |
//! | `counter` | The counter rhythm written out like `click`, for one whose clicks were edited; otherwise as `poly`. Not together with `poly` |
//!
//! Any of `sub`, `lvl`, `bars`, `space` or `seed` selects the generator;
//! unknown parameters are ignored. [`SharedLink::to_url`] is the inverse of
//! [`parse_query`].
//!
//! ## Versions
//!
//! - Version 1 (links without `v`): `bpm` counted the meter's beat unit, so
//!   in x/8 eighths and in x/16 sixteenths per minute.
//! - Version 2 (`v=2`, [`LINK_VERSION`]): `bpm` always counts quarters.
//!
//! [`parse_query`] converts the tempo of a version 1 link to quarters, so an
//! old 7/8 link at `bpm=140` opens at 70 and plays as fast as it used to. A
//! score mixing beat units is converted by its first measure.

use grooph_measure::counter::{Ratio, counter_bar, counter_ratios, is_ratio_bar, own_ratios};
use grooph_measure::generator::{
    GeneratorSettings, MAX_BARS, MAX_COMPLEXITY, Subdivision, supports_time_signature,
};
use grooph_measure::notation::{format_score, parse_score};
use grooph_measure::swing::{MAX_SWING_PERCENT, MIN_SWING_PERCENT, Swing, SwingUnit};
use grooph_measure::tempo::{MAX_BPM, MIN_BPM};
use grooph_measure::{Measure, Score, TimeSignature};

/// Version [`SharedLink::to_url`] writes as `v`.
pub const LINK_VERSION: u32 = 2;

/// What a link opens. Every part is optional; a link with only `bpm` just
/// sets the tempo.
#[derive(Clone, Debug, Default)]
pub struct SharedLink {
    pub bpm: Option<u32>,
    pub swing: Option<Swing>,
    pub content: Option<LinkContent>,
    /// Click bar of the content's meter (the first measure's meter).
    pub click: Option<Measure>,
    /// Counter rhythm of the metronome's meter.
    pub counter: Option<Measure>,
    /// The ratio `counter` was picked as. [`SharedLink::to_url`] writes it
    /// as `poly` while `counter` is that ratio's bar, else the bar itself.
    pub poly: Option<Ratio>,
}

#[derive(Clone, Debug)]
pub enum LinkContent {
    /// A fixed score (`r`).
    Score(Score),
    /// One measure of beat-unit notes in this meter (`ts` on its own).
    Metronome(TimeSignature),
    /// Generator settings; `seed: None` draws a random rhythm.
    Generator { settings: GeneratorSettings, seed: Option<u64> },
}

/// Parse a URL query string (with or without the leading `?`). Returns
/// `Ok(None)` if it contains none of the link parameters. A query without
/// `v` is a version 1 link (see the [crate docs](crate)).
pub fn parse_query(query: &str) -> Result<Option<SharedLink>, String> {
    parse_query_with_default_version(query, 1)
}

/// [`parse_query`] for queries that count `bpm` in quarters even without
/// `v`, such as links written by hand for the current app.
pub fn parse_query_current(query: &str) -> Result<Option<SharedLink>, String> {
    parse_query_with_default_version(query, LINK_VERSION)
}

fn parse_query_with_default_version(
    query: &str,
    default_version: u32,
) -> Result<Option<SharedLink>, String> {
    let mut version = default_version;
    let mut bpm = None;
    let mut rhythm = None;
    let mut ts = None;
    let mut sub = None;
    let mut lvl = None;
    let mut bars = None;
    let mut space = None;
    let mut seed = None;
    let mut swing_percent = None;
    let mut swing_unit = None;
    let mut click = None;
    let mut counter = None;
    let mut poly = None;

    for pair in query.trim_start_matches('?').split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = percent_decode(value);
        match key {
            "v" => version = parse_in_range("v", &value, 1, LINK_VERSION)?,
            "bpm" => bpm = Some(parse_in_range("bpm", &value, MIN_BPM, MAX_BPM)?),
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
            "swing" => {
                swing_percent =
                    Some(parse_in_range("swing", &value, MIN_SWING_PERCENT, MAX_SWING_PERCENT)?)
            }
            "su" => swing_unit = Some(parse_swing_unit(&value)?),
            "click" => click = Some(value),
            "counter" => counter = Some(value),
            "poly" => poly = Some(value.parse::<Ratio>().map_err(|e| format!("poly: {e}"))?),
            _ => {}
        }
    }

    let swing = match (swing_percent, swing_unit) {
        (Some(percent), unit) => Some(Swing { unit: unit.unwrap_or_default(), percent }),
        (None, Some(_)) => return Err("su needs a swing amount (swing=50-75)".into()),
        (None, None) => None,
    };

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
        if !supports_time_signature(time_signature) {
            return Err("the generator does not support this time signature".into());
        }
        let settings = GeneratorSettings {
            subdivision: sub.unwrap_or(defaults.subdivision),
            complexity: lvl.unwrap_or(defaults.complexity),
            bars: bars.unwrap_or(defaults.bars),
            time_signature,
            space: space.unwrap_or(defaults.space),
            custom_figures_enabled: false,
            ..defaults
        };
        Some(LinkContent::Generator { settings, seed })
    } else {
        ts.map(LinkContent::Metronome)
    };

    let first_ts = content.as_ref().and_then(LinkContent::first_time_signature);
    let click = match click {
        None => None,
        Some(text) => {
            let Some(ts) = first_ts else {
                return Err("click needs a rhythm (r) or a time signature (ts)".into());
            };
            Some(single_measure("click", ts, &text)?)
        }
    };
    let metronome_meter = |name: &str| match content.as_ref() {
        Some(&LinkContent::Metronome(ts)) => Ok(ts),
        _ => Err(format!("{name} needs a time signature (ts) without a rhythm")),
    };
    let counter = match (counter, poly) {
        (None, None) => None,
        (Some(_), Some(_)) => return Err("use either poly or counter, not both".into()),
        (Some(text), None) => Some(single_measure("counter", metronome_meter("counter")?, &text)?),
        (None, Some(ratio)) => Some(poly_bar(metronome_meter("poly")?, ratio)?),
    };

    if version == 1
        && let Some(tempo) = bpm
        && let Some(ts) = content.as_ref().and_then(LinkContent::first_time_signature)
    {
        bpm = Some(quarter_bpm(tempo, ts));
    }

    if bpm.is_none() && swing.is_none() && content.is_none() {
        Ok(None)
    } else {
        Ok(Some(SharedLink { bpm, swing, content, click, counter, poly }))
    }
}

/// The counter bar of `ratio` in `ts`, as the app's polyrhythm picker builds
/// it (any ratio of one's own); the error names the rule and the ratios the
/// picker shows first.
fn poly_bar(ts: TimeSignature, ratio: Ratio) -> Result<Measure, String> {
    if own_ratios(ts).contains(&ratio)
        && let Some(bar) = counter_bar(ts, ratio)
    {
        return Ok(bar);
    }
    let spans: Vec<String> = (1..=ts.beats)
        .filter(|&s| ts.beats.is_multiple_of(s) && own_ratios(ts).iter().any(|r| r.span == s))
        .map(|s| s.to_string())
        .collect();
    let offered: Vec<String> = counter_ratios(ts).iter().map(Ratio::to_string).collect();
    Err(format!(
        "poly {ratio} does not fit {ts}: it takes n:m with m one of {} and at least two clicks once reduced (not 2:4, which is 1:2), such as {}",
        spans.join(", "),
        offered.join(", ")
    ))
}

/// The one measure of `ts` that `text` (without the time signature) writes,
/// for the parameter `name`.
fn single_measure(name: &str, ts: TimeSignature, text: &str) -> Result<Measure, String> {
    let mut bars =
        parse_score(&format!("{ts} {text}")).map_err(|e| format!("{name}: {e}"))?.measures;
    if bars.len() != 1 {
        return Err(format!("{name} must be a single measure"));
    }
    Ok(bars.remove(0))
}

/// Quarter notes per minute for a version 1 tempo of `bpm` beat units of
/// `ts` per minute, kept within the tempo range.
fn quarter_bpm(bpm: u32, ts: TimeSignature) -> u32 {
    let units_per_quarter = (ts.beat_unit as u32 / 4).max(1);
    ((bpm as f64 / units_per_quarter as f64).round() as u32).clamp(MIN_BPM, MAX_BPM)
}

impl LinkContent {
    /// Meter the content starts in.
    fn first_time_signature(&self) -> Option<TimeSignature> {
        match self {
            LinkContent::Score(score) => score.measures.first().map(|m| m.time_signature()),
            LinkContent::Metronome(ts) => Some(*ts),
            LinkContent::Generator { settings, .. } => Some(settings.time_signature),
        }
    }
}

impl SharedLink {
    /// URL that [`parse_query`] reads back as this link, or `None` if the
    /// score cannot be written in the text notation.
    pub fn to_url(&self, base: &str) -> Option<String> {
        let mut params: Vec<(&str, String)> = vec![("v", LINK_VERSION.to_string())];
        if let Some(bpm) = self.bpm {
            params.push(("bpm", bpm.to_string()));
        }
        match &self.content {
            None => {}
            Some(LinkContent::Score(score)) => params.push(("r", format_score(score)?)),
            Some(LinkContent::Metronome(ts)) => params.push(("ts", ts.to_string())),
            Some(LinkContent::Generator { settings, seed }) => {
                if settings.time_signature != TimeSignature::FOUR_FOUR {
                    params.push(("ts", settings.time_signature.to_string()));
                }
                params.push(("sub", subdivision_code(settings.subdivision).to_string()));
                params.push(("lvl", settings.complexity.to_string()));
                params.push(("bars", settings.bars.to_string()));
                if settings.space > 0.0 {
                    params.push(("space", settings.space.to_string()));
                }
                if let Some(seed) = seed {
                    params.push(("seed", seed.to_string()));
                }
            }
        }
        if let Some(swing) = self.swing {
            params.push(("swing", swing.percent.to_string()));
            if swing.unit == SwingUnit::Sixteenths {
                params.push(("su", "16".into()));
            }
        }
        if let Some(bar) = &self.click {
            params.push(("click", format_bar(bar)?));
        }
        if let Some(bar) = &self.counter {
            match self.poly.filter(|&r| is_ratio_bar(bar, r)) {
                Some(ratio) => params.push(("poly", ratio.to_string())),
                None => params.push(("counter", format_bar(bar)?)),
            }
        }
        // The version alone opens nothing.
        if params.len() == 1 {
            return Some(base.to_string());
        }
        let query: Vec<String> =
            params.iter().map(|(k, v)| format!("{k}={}", percent_encode(v))).collect();
        Some(format!("{base}?{}", query.join("&")))
    }
}

/// `bar` in the text notation without its time signature, as `click` and
/// `counter` write it (`>q r:q. (q)`); `None` if a beat has no code.
pub fn format_bar(bar: &Measure) -> Option<String> {
    let text = format_score(&Score::single(bar.clone()))?;
    let ts = bar.time_signature().to_string();
    Some(text.strip_prefix(&ts)?.trim_start().to_string())
}

fn parse_in_range<T>(name: &str, value: &str, min: T, max: T) -> Result<T, String>
where
    T: std::str::FromStr + PartialOrd + std::fmt::Display,
{
    match value.parse::<T>() {
        Ok(v) if v >= min && v <= max => Ok(v),
        _ => Err(format!("{name} must be a number from {min} to {max}, got '{value}'")),
    }
}

fn subdivision_code(subdivision: Subdivision) -> &'static str {
    match subdivision {
        Subdivision::Eighths => "8",
        Subdivision::Sixteenths => "16",
        Subdivision::Triplets => "3",
        Subdivision::Mixed => "mix",
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

fn parse_swing_unit(value: &str) -> Result<SwingUnit, String> {
    match value {
        "8" => Ok(SwingUnit::Eighths),
        "16" => Ok(SwingUnit::Sixteenths),
        _ => Err(format!("su must be 8 or 16, got '{value}'")),
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
                // Exactly two hex digits; anything else (`%+4`, `%4`) stays as it is.
                match (bytes.get(i + 1), bytes.get(i + 2)) {
                    (Some(&hi), Some(&lo)) if hi.is_ascii_hexdigit() && lo.is_ascii_hexdigit() => {
                        out.push(hex_value(hi) << 4 | hex_value(lo));
                        i += 2;
                    }
                    _ => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The value of an ASCII hex digit.
fn hex_value(digit: u8) -> u8 {
    match digit {
        b'0'..=b'9' => digit - b'0',
        b'a'..=b'f' => digit - b'a' + 10,
        _ => digit - b'A' + 10,
    }
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
    use grooph_measure::Beat;
    use grooph_measure::BeatKind::{Note, Rest};
    use grooph_measure::duration::{COMMON_DURATIONS, Duration, NoteValue, TupletSpec};
    use grooph_measure::duration::{e, q};
    use grooph_measure::generator::Rng;

    fn parse(query: &str) -> SharedLink {
        parse_query(query).unwrap().unwrap()
    }

    #[test]
    fn query_without_link_parameters_is_ignored() {
        assert!(parse_query("").unwrap().is_none());
        assert!(parse_query("?utm_source=x").unwrap().is_none());
    }

    #[test]
    fn embed_parameters_leave_the_link_alone() {
        // The MCP App view adds `embed` and `theme` to the tool's link.
        assert!(parse_query("?embed=mcp&theme=dark").unwrap().is_none());
        let link = parse("?v=2&bpm=140&ts=7/8&embed=mcp&theme=light");
        assert_eq!(link.bpm, Some(140));
        assert!(
            matches!(link.content, Some(LinkContent::Metronome(ts)) if ts == TimeSignature::new(7, 8))
        );
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
        assert!(!settings.custom_figures_enabled);
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
        assert!(matches!(link.content, Some(LinkContent::Metronome(TimeSignature::SEVEN_EIGHT))));
    }

    #[test]
    fn swing_link() {
        let link = parse("bpm=120&ts=4/4&swing=66");
        assert_eq!(link.swing, Some(Swing { unit: SwingUnit::Eighths, percent: 66 }));
        let link = parse("swing=60&su=16");
        assert_eq!(link.swing, Some(Swing { unit: SwingUnit::Sixteenths, percent: 60 }));
        assert!(link.content.is_none());
        assert!(parse_query("swing=80").unwrap_err().contains("swing"));
        assert!(parse_query("su=16").unwrap_err().contains("su needs"));
        assert!(parse_query("swing=60&su=4").unwrap_err().contains("su must"));
    }

    #[test]
    fn to_url_round_trips() {
        let generator = SharedLink {
            bpm: Some(80),
            swing: Some(Swing { unit: SwingUnit::Sixteenths, percent: 58 }),
            content: Some(LinkContent::Generator {
                settings: GeneratorSettings {
                    subdivision: Subdivision::Triplets,
                    complexity: 4,
                    bars: 2,
                    time_signature: TimeSignature::THREE_FOUR,
                    space: 0.25,
                    ..GeneratorSettings::default()
                },
                seed: Some(7),
            }),
            click: None,
            counter: None,
            poly: None,
        };
        let url = generator.to_url("https://grooph.app/").unwrap();
        assert_eq!(
            url,
            "https://grooph.app/?v=2&bpm=80&ts=3/4&sub=3&lvl=4&bars=2&space=0.25&seed=7&swing=58&su=16"
        );
        let back = parse(url.split_once('?').unwrap().1);
        assert_eq!(back.bpm, generator.bpm);
        assert_eq!(back.swing, generator.swing);
        let (
            Some(LinkContent::Generator { settings: a, seed: sa }),
            Some(LinkContent::Generator { settings: b, seed: sb }),
        ) = (&back.content, &generator.content)
        else {
            panic!("expected generator settings")
        };
        assert_eq!((a, sa), (b, sb));

        let metronome = SharedLink {
            bpm: Some(140),
            content: Some(LinkContent::Metronome(TimeSignature::SEVEN_EIGHT)),
            ..SharedLink::default()
        };
        assert_eq!(metronome.to_url("/").unwrap(), "/?v=2&bpm=140&ts=7/8");
        let grouped = TimeSignature::SEVEN_EIGHT.with_grouping(2);
        let metronome =
            SharedLink { content: Some(LinkContent::Metronome(grouped)), ..SharedLink::default() };
        let url = metronome.to_url("/").unwrap();
        assert_eq!(url, "/?v=2&ts=3%2B2%2B2/8");
        assert!(
            matches!(parse(&url[2..]).content, Some(LinkContent::Metronome(ts)) if ts == grouped)
        );
        assert_eq!(SharedLink::default().to_url("/").unwrap(), "/");
    }

    #[test]
    fn click_link() {
        let link = parse("v=2&bpm=70&ts=7/8&click=>q+r:q.+(q)");
        let bar = link.click.as_ref().unwrap();
        assert_eq!(bar.time_signature(), TimeSignature::SEVEN_EIGHT);
        assert!(bar.beats()[0].accented && bar.beats()[2].soft);
        assert_eq!(link.to_url("/").unwrap(), "/?v=2&bpm=70&ts=7/8&click=%3Eq+r:q.+%28q%29");
        // The pattern belongs to the first measure of a rhythm.
        let link = parse("r=6/8+q.+q.+|+3/4+q+q+q&click=q.+>q.");
        assert_eq!(link.click.unwrap().time_signature(), TimeSignature::SIX_EIGHT);
        assert!(parse_query("ts=7/8&click=q+q").unwrap_err().contains("click"));
        assert!(parse_query("ts=7/8&click=q+q.+q+|+q+q.+q").unwrap_err().contains("single"));
        assert!(parse_query("bpm=90&click=q+q+q+q").unwrap_err().contains("needs a rhythm"));
    }

    #[test]
    fn counter_link() {
        let link = parse("v=2&bpm=90&ts=4/4&click=>q+q+q+q&counter=t4+r:t4+(t4)+r:t4+(t4)+r:t4");
        let bar = link.counter.as_ref().unwrap();
        assert_eq!(bar.time_signature(), TimeSignature::FOUR_FOUR);
        assert_eq!(bar.beats().len(), 6);
        assert!(bar.beats()[2].soft);
        assert_eq!(
            link.to_url("/").unwrap(),
            "/?v=2&bpm=90&ts=4/4&click=%3Eq+q+q+q&counter=t4+r:t4+%28t4%29+r:t4+%28t4%29+r:t4"
        );
        assert!(parse_query("ts=4/4&counter=t4+t4").unwrap_err().contains("counter"));
        assert!(parse_query("ts=2/4&counter=q+q+|+q+q").unwrap_err().contains("single"));
        // Only the metronome plays a counter rhythm.
        assert!(parse_query("r=q+q+q+q&counter=q+q+q+q").unwrap_err().contains("without a rhythm"));
        assert!(parse_query("bpm=90&counter=q+q+q+q").unwrap_err().contains("needs a time"));
    }

    #[test]
    fn poly_link_builds_the_counter_bar_of_its_ratio() {
        let link = parse("v=2&bpm=90&ts=4/4&poly=3:2");
        assert_eq!(link.poly, Some(Ratio { n: 3, span: 2 }));
        assert_eq!(format_bar(link.counter.as_ref().unwrap()).unwrap(), "t4 t4 t4 t4 t4 t4");
        assert_eq!(link.to_url("/").unwrap(), "/?v=2&bpm=90&ts=4/4&poly=3:2");
        // A ratio with a common factor keeps its name.
        assert_eq!(parse("ts=4/4&poly=4:2").to_url("/").unwrap(), "/?v=2&ts=4/4&poly=4:2");
        let link = parse("ts=5/8&poly=7:5");
        assert_eq!(link.counter.unwrap().beats().len(), 7);
        let err = parse_query("ts=4/4&poly=7:5").unwrap_err();
        assert!(err.contains("does not fit 4/4") && err.contains("3:2"), "{err}");
        assert!(err.contains("m one of 1, 2, 4"), "{err}");
        // Only ratios whose span the error names open, and its examples do.
        for span in 1..=4 {
            for n in 2..=16 {
                let query = format!("ts=4/4&poly={n}:{span}");
                assert!(parse_query(&query).is_err() || span != 3, "{query}");
            }
        }
        for example in ["3:4", "5:4", "7:4", "3:2"] {
            assert!(parse_query(&format!("ts=4/4&poly={example}")).is_ok(), "{example}");
        }
        assert!(parse_query("ts=4/4&poly=three").unwrap_err().contains("poly"));
        assert!(parse_query("ts=4/4&poly=3:2&counter=t2+t2+t2").unwrap_err().contains("either"));
        assert!(parse_query("r=q+q+q+q&poly=3:2").unwrap_err().contains("without a rhythm"));
    }

    /// The MCP tool builds its links in JavaScript (`mcp/link.js`); its test
    /// cases must read back and be written again as they are.
    #[test]
    fn links_of_the_mcp_tool_are_the_apps_own() {
        let cases = include_str!("../../../mcp/link-cases.json");
        let queries: Vec<&str> = cases
            .split("\"query\": \"")
            .skip(1)
            .map(|rest| &rest[..rest.find('"').unwrap()])
            .collect();
        assert!(queries.len() >= 5);
        for query in queries {
            let link = parse_query_current(query).unwrap().unwrap();
            assert_eq!(link.to_url("").unwrap(), format!("?{query}"));
        }
    }

    /// How many beats each tuplet group of `bar` holds, in order.
    fn tuplet_groups(bar: &Measure) -> Vec<usize> {
        let ids: Vec<_> = bar.beats().iter().filter_map(|b| b.tuplet_group_id).collect();
        ids.chunk_by(|a, b| a == b).map(<[_]>::len).collect()
    }

    #[test]
    fn edited_poly_clicks_are_written_out() {
        let mut link = parse("ts=4/4&poly=3:2");
        link.counter.as_mut().unwrap().toggle_soft(1);
        assert!(link.to_url("/").unwrap().contains("&counter=t4+%28t4%29+t4"));
    }

    #[test]
    fn written_out_polyrhythms_are_the_ratios_bars() {
        // The counter values llms.txt used to list open the same as `poly`.
        for (ts, counter, ratio) in [
            ("4/4", "t4+t4+t4+t4+t4+t4", "3:2"),
            ("4/4", "t2+t2+t2", "3:4"),
            ("4/4", "qt4+qt4+qt4+qt4+qt4", "5:4"),
            ("4/4", "spt4+spt4+spt4+spt4+spt4+spt4+spt4", "7:4"),
            ("4/4", "e+e+e+e+e+e+e+e", "2:1"),
            ("2/4", "t4+t4+t4", "3:2"),
            ("3/4", "q.+q.", "2:3"),
            ("3/4", "e.+e.+e.+e.", "4:3"),
            ("6/8", "e.+e.+e.+e.", "2:3"),
            ("6/8", "s.+s.+s.+s.+s.+s.+s.+s.", "4:3"),
            ("5/8", "7:5e+7:5e+7:5e+7:5e+7:5e+7:5e+7:5e", "7:5"),
        ] {
            let written = parse(&format!("ts={ts}&counter={counter}")).counter.unwrap();
            let poly = parse(&format!("ts={ts}&poly={ratio}")).counter.unwrap();
            // `Beat`'s == leaves out the tuplet groups: compare them too.
            assert_eq!(format_bar(&written), format_bar(&poly), "{ratio} in {ts}");
            assert_eq!(tuplet_groups(&written), tuplet_groups(&poly), "{ratio} in {ts}");
        }
    }

    #[test]
    fn counter_link_with_a_tuplet_of_any_ratio() {
        let link = parse("v=2&bpm=90&ts=5/8&counter=7:5e+7:5e+7:5e+7:5e+7:5e+7:5e+7:5e");
        let bar = link.counter.as_ref().unwrap();
        assert_eq!(bar.beats().len(), 7);
        assert_eq!(
            link.to_url("/").unwrap(),
            "/?v=2&bpm=90&ts=5/8&counter=7:5e+7:5e+7:5e+7:5e+7:5e+7:5e+7:5e"
        );
    }

    #[test]
    fn version_1_tempo_counts_the_beat_unit() {
        // Without `v`, bpm counted eighths in x/8 and sixteenths in x/16.
        for (query, bpm) in [
            ("bpm=140&ts=7/8", 70),
            ("bpm=120&ts=11/16&sub=16", 30),
            ("bpm=90&r=6/8+e+e+e+e+e+e", 45),
            ("bpm=90&ts=3/4", 90),
            // The tempo stays within range.
            ("bpm=60&ts=5/16", MIN_BPM),
        ] {
            assert_eq!(parse(query).bpm, Some(bpm), "{query}");
        }
        // Without content there is no meter to convert by.
        assert_eq!(parse("bpm=140").bpm, Some(140));
    }

    #[test]
    fn version_2_tempo_counts_quarters() {
        assert_eq!(parse("v=2&bpm=70&ts=7/8").bpm, Some(70));
        assert_eq!(parse_query_current("bpm=70&ts=7/8").unwrap().unwrap().bpm, Some(70));
        // An explicit v=1 is converted even where v=2 is the default.
        assert_eq!(parse_query_current("v=1&bpm=140&ts=7/8").unwrap().unwrap().bpm, Some(70));
        assert!(parse_query("v=3&bpm=70").unwrap_err().contains("v must"));
    }

    #[test]
    fn written_links_reopen_at_the_same_tempo() {
        let link = SharedLink {
            bpm: Some(70),
            content: Some(LinkContent::Metronome(TimeSignature::SEVEN_EIGHT)),
            ..SharedLink::default()
        };
        let url = link.to_url("/").unwrap();
        assert_eq!(parse(&url[2..]).bpm, Some(70));
    }

    #[test]
    fn tempo_only_link() {
        let link = parse("bpm=60");
        assert_eq!(link.bpm, Some(60));
        assert!(link.content.is_none());
    }

    #[test]
    fn generator_link_accepts_x8_and_x16_meters() {
        for (query, meter) in [
            ("ts=7/8&sub=16&seed=3", "7/8"),
            ("ts=11/16&sub=16", "11/16"),
            ("ts=2%2B2%2B3/8&sub=8", "2+2+3/8"),
        ] {
            let link = parse_query(query).unwrap().unwrap();
            let Some(LinkContent::Generator { settings, .. }) = link.content else {
                panic!("expected generator content");
            };
            assert_eq!(settings.time_signature.to_string(), meter);
        }
    }

    #[test]
    fn invalid_parameters_are_reported() {
        assert!(parse_query("bpm=500").unwrap_err().contains("bpm"));
        assert!(parse_query("lvl=9").unwrap_err().contains("lvl"));
        assert!(parse_query("sub=4").unwrap_err().contains("sub"));
        assert!(parse_query("r=q+q+q").unwrap_err().contains("too short"));
        assert!(parse_query("r=q+q+q+q&seed=1").unwrap_err().contains("not both"));
    }

    #[test]
    fn score_link_round_trips() {
        let score = parse_score("3/4 >q t8 t8 r:t8 e e | e. s q r:q").unwrap();
        let link = SharedLink {
            bpm: Some(96),
            swing: None,
            content: Some(LinkContent::Score(score.clone())),
            click: None,
            counter: None,
            poly: None,
        };
        let url = link.to_url("https://grooph.app/").unwrap();
        assert_eq!(url, "https://grooph.app/?v=2&bpm=96&r=3/4+%3Eq+t8+t8+r:t8+e+e+%7C+e.+s+q+r:q");
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
        assert_eq!(percent_decode("%e2%86%92"), "→");
    }

    #[test]
    fn percent_decoding_takes_only_two_hex_digits() {
        // A sign is not a hex digit: no NUL byte comes out of `%+0`.
        assert_eq!(percent_decode("9%+0"), "9% 0");
        assert_eq!(percent_decode("%-1"), "%-1");
        assert_eq!(percent_decode("%4"), "%4");
        assert_eq!(percent_decode("%4g"), "%4g");
        assert!(parse_query("bpm=9%+0").is_err());
    }

    /// Encode every byte outside the unreserved set as `%XX`, spaces included,
    /// as other tools write query values.
    fn strict_percent_encode(value: &str) -> String {
        value
            .bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    (b as char).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect()
    }

    /// A random score of up to four measures in random meters, filled
    /// beat by beat with random notes and rests.
    fn random_score(rng: &mut Rng) -> Score {
        let meters: Vec<TimeSignature> = TimeSignature::BEAT_UNITS
            .into_iter()
            .flat_map(|unit| {
                TimeSignature::beat_range(unit).unwrap().flat_map(move |beats| {
                    let ts = TimeSignature::new(beats, unit);
                    (0..ts.groupings().len()).map(move |g| ts.with_grouping(g as u8))
                })
            })
            .collect();
        let general =
            [(7, 5, NoteValue::Eighth), (5, 7, NoteValue::Eighth), (9, 8, NoteValue::Sixteenth)]
                .map(|(n, m, base)| Duration::Tuplet(TupletSpec { n, m, base }));
        let durations: Vec<Duration> = COMMON_DURATIONS.into_iter().chain(general).collect();
        let measures = (0..1 + rng.below(4))
            .map(|_| {
                let mut m = Measure::new(meters[rng.below(meters.len())]);
                let (mut idx, mut tries) = (0, 0);
                while idx < m.beats().len() {
                    let d = durations[rng.below(durations.len())];
                    let kind = if rng.chance(0.3) { Rest } else { Note };
                    // Keep the beat there if a few tries do not fit.
                    if m.set_beat(idx, Beat::new(d, kind)).is_err() && tries < 20 {
                        tries += 1;
                        continue;
                    }
                    tries = 0;
                    if kind == Note && rng.chance(0.2) {
                        m.toggle_accent(idx);
                    } else if kind == Note && rng.chance(0.2) {
                        m.toggle_soft(idx);
                    }
                    idx += 1;
                }
                m
            })
            .collect();
        Score { measures }
    }

    fn assert_same_score(a: &Score, b: &Score, context: &str) {
        assert!(a.same_beats(b), "{context}: {:?} became {:?}", format_score(a), format_score(b));
        assert_eq!(format_score(a), format_score(b), "{context}");
    }

    fn run_link_round_trips(seed: u64, scores: usize) {
        let mut rng = Rng::new(seed);
        for i in 0..scores {
            let score = &random_score(&mut rng);
            let context = format!("seed {seed}, score {i}");
            let link = SharedLink {
                bpm: Some(90),
                content: Some(LinkContent::Score(score.clone())),
                ..SharedLink::default()
            };
            let url = link.to_url("https://grooph.app/").expect(&context);
            let query = url.split_once('?').unwrap().1;
            let Some(LinkContent::Score(back)) =
                parse_query_current(query).expect(&context).unwrap().content
            else {
                panic!("{context}: {url} opens no score");
            };
            assert_same_score(score, &back, &format!("{context}, {url}"));

            let text = format_score(score).unwrap();
            let query = format!("v=2&r={}", strict_percent_encode(&text));
            let Some(LinkContent::Score(back)) =
                parse_query_current(&query).expect(&context).unwrap().content
            else {
                panic!("{context}: {query} opens no score");
            };
            assert_same_score(score, &back, &format!("{context}, {query}"));
        }
    }

    #[test]
    fn random_score_links_round_trip() {
        run_link_round_trips(0x9E37_79B9_7F4A_7C15, 500);
    }

    #[test]
    #[ignore = "long run; cargo test -p grooph-link -- --ignored"]
    fn random_score_links_round_trip_long() {
        for seed in 1..=20 {
            run_link_round_trips(seed, 2000);
        }
    }
}
