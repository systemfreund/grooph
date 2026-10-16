//! Click patterns: how loud the metronome clicks each beat group of a meter.
//!
//! A [`ClickPattern`] holds one [`ClickLevel`] per group of
//! [`TimeSignature::groups`] (7/8 as 2+3+2 has three). The default is
//! [`ClickLevel::Strong`] on the first group and [`ClickLevel::Medium`] on
//! the others, which is the downbeat and primary beats as they always
//! sounded. [`ClickPatterns`] keeps the patterns that differ from it, one per
//! meter, so every measure in that meter clicks the same way.
//!
//! The text form is one letter per group: `S` strong, `M` medium, `L` soft
//! (low), `-` off. 7/8 as 2+3+2 with the middle group soft is `SLM`.

use crate::TimeSignature;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::str::FromStr;

/// How loud the click on one beat group sounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClickLevel {
    /// The downbeat click.
    Strong,
    /// The click on the other primary beats.
    Medium,
    /// A quieter click.
    Soft,
    /// No click.
    Off,
}

impl ClickLevel {
    pub const ALL: [ClickLevel; 4] =
        [ClickLevel::Strong, ClickLevel::Medium, ClickLevel::Soft, ClickLevel::Off];

    /// Letter of the text form.
    pub const fn code(self) -> char {
        match self {
            ClickLevel::Strong => 'S',
            ClickLevel::Medium => 'M',
            ClickLevel::Soft => 'L',
            ClickLevel::Off => '-',
        }
    }

    pub fn from_code(c: char) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.code() == c)
    }

    /// The next level when tapping through them: strong, medium, soft, off.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&l| l == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// One [`ClickLevel`] per beat group of a meter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClickPattern(pub Vec<ClickLevel>);

impl ClickPattern {
    /// Strong on the first group, medium on the others.
    pub fn default_for(ts: TimeSignature) -> Self {
        let groups = ts.groups().len();
        Self(
            (0..groups)
                .map(|i| if i == 0 { ClickLevel::Strong } else { ClickLevel::Medium })
                .collect(),
        )
    }

    /// Whether the pattern has one level per group of `ts`.
    pub fn fits(&self, ts: TimeSignature) -> bool {
        self.0.len() == ts.groups().len()
    }

    pub fn levels(&self) -> &[ClickLevel] {
        &self.0
    }
}

impl Display for ClickPattern {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.iter().try_for_each(|l| write!(f, "{}", l.code()))
    }
}

impl FromStr for ClickPattern {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.chars()
            .map(|c| {
                ClickLevel::from_code(c).ok_or_else(|| {
                    format!("'{c}' is not a click level (S strong, M medium, L soft, - off)")
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Self)
    }
}

/// The click pattern of every meter: the default unless one was set.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClickPatterns {
    /// Patterns that differ from the default, at most one per meter.
    /// `TimeSignature` has no ordering or hash, and a JSON map needs string
    /// keys, so this is a list.
    custom: Vec<(TimeSignature, ClickPattern)>,
}

impl ClickPatterns {
    /// The pattern `ts` clicks. A stored pattern that no longer fits (its
    /// meter has a different number of groups) falls back to the default.
    pub fn for_meter(&self, ts: TimeSignature) -> ClickPattern {
        self.custom
            .iter()
            .find(|(t, p)| *t == ts && p.fits(ts))
            .map_or_else(|| ClickPattern::default_for(ts), |(_, p)| p.clone())
    }

    /// Set the pattern of `ts`. Returns `false` and changes nothing if it
    /// does not fit the meter; the default pattern is not stored.
    pub fn set(&mut self, ts: TimeSignature, pattern: ClickPattern) -> bool {
        if !pattern.fits(ts) {
            return false;
        }
        self.custom.retain(|(t, _)| *t != ts);
        if pattern != ClickPattern::default_for(ts) {
            self.custom.push((ts, pattern));
        }
        true
    }

    /// The pattern of `ts` if it is not the default.
    pub fn custom_for(&self, ts: TimeSignature) -> Option<ClickPattern> {
        Some(self.for_meter(ts)).filter(|p| *p != ClickPattern::default_for(ts))
    }
}

#[cfg(test)]
mod tests {
    use super::ClickLevel::{Medium, Off, Soft, Strong};
    use super::*;

    #[test]
    fn default_is_strong_then_medium_per_group() {
        assert_eq!(
            ClickPattern::default_for(TimeSignature::FOUR_FOUR).levels(),
            [Strong, Medium, Medium, Medium]
        );
        assert_eq!(ClickPattern::default_for(TimeSignature::SIX_EIGHT).levels(), [Strong, Medium]);
        assert_eq!(ClickPattern::default_for(TimeSignature::SEVEN_EIGHT).to_string(), "SMM");
    }

    #[test]
    fn text_form_round_trips() {
        let p: ClickPattern = "SL-M".parse().unwrap();
        assert_eq!(p.levels(), [Strong, Soft, Off, Medium]);
        assert_eq!(p.to_string(), "SL-M");
        assert!("SX".parse::<ClickPattern>().is_err());
    }

    #[test]
    fn next_cycles_through_all_levels() {
        assert_eq!(Strong.next(), Medium);
        assert_eq!(Medium.next(), Soft);
        assert_eq!(Soft.next(), Off);
        assert_eq!(Off.next(), Strong);
    }

    #[test]
    fn patterns_are_kept_per_meter() {
        let mut patterns = ClickPatterns::default();
        let seven = TimeSignature::SEVEN_EIGHT;
        assert!(patterns.set(seven, "S-L".parse().unwrap()));
        assert_eq!(patterns.for_meter(seven).to_string(), "S-L");
        // Another grouping of 7/8 is another meter.
        assert_eq!(patterns.for_meter(seven.with_grouping(1)).to_string(), "SMM");
        assert_eq!(patterns.for_meter(TimeSignature::FOUR_FOUR).to_string(), "SMMM");
    }

    #[test]
    fn patterns_that_do_not_fit_are_refused() {
        let mut patterns = ClickPatterns::default();
        assert!(!patterns.set(TimeSignature::SIX_EIGHT, "SMM".parse().unwrap()));
        assert_eq!(patterns, ClickPatterns::default());
    }

    #[test]
    fn setting_the_default_forgets_the_custom_pattern() {
        let mut patterns = ClickPatterns::default();
        let ts = TimeSignature::FOUR_FOUR;
        patterns.set(ts, "SLML".parse().unwrap());
        assert!(patterns.custom_for(ts).is_some());
        patterns.set(ts, ClickPattern::default_for(ts));
        assert_eq!(patterns, ClickPatterns::default());
        assert_eq!(patterns.custom_for(ts), None);
    }
}
