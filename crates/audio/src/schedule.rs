use grooph_measure::click::{ClickBars, ClickLevel};
use grooph_measure::counter::CounterBars;
use grooph_measure::generator::Subdivision;
use grooph_measure::ghost::ghost_onsets;
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::onsets::HeadOnset;
use grooph_measure::tempo::ScoreTiming;
use grooph_measure::{Instrument, Score, Technique};
use std::collections::BTreeMap;

use crate::AudioSettings;
use crate::sounds::Level;
use crate::trainer::GapMute;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SoundType {
    Downbeat,
    PrimaryBeat,
    /// Primary beat that the click pattern sets to [`ClickLevel::Soft`].
    SoftBeat,
    AccentedBeat,
    Beat,
    /// Quiet subdivision hit where no note is written.
    Ghost,
    /// Click of the count-in bar before playback starts.
    CountIn,
    /// Count-in click on a beat unit inside a beat group (the eighths
    /// between the group starts of 7/8), quieter than [`SoundType::CountIn`].
    CountInSoft,
    /// Clicks of the counter rhythm (see [`grooph_measure::counter`]), at
    /// the levels of its notes.
    CounterStrong,
    CounterMedium,
    CounterSoft,
    /// A head of a drum kit score, played on the kit.
    Kit(KitHit),
}

/// One head of the drum kit: what is hit, how and how hard.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct KitHit {
    pub(crate) instrument: Instrument,
    pub(crate) technique: Technique,
    pub(crate) level: Level,
}

impl SoundType {
    /// What `onset` plays: the rhythm voice's note, accented or not, for a
    /// head on the one line, the kit for the others.
    fn of_head(onset: &HeadOnset) -> Self {
        let head = onset.head;
        match head.instrument() {
            Instrument::Rhythm if head.accented() => SoundType::AccentedBeat,
            Instrument::Rhythm => SoundType::Beat,
            instrument => {
                let level = if head.accented() {
                    Level::Strong
                } else if head.soft() {
                    Level::Soft
                } else {
                    Level::Normal
                };
                SoundType::Kit(KitHit { instrument, technique: head.technique(), level })
            }
        }
    }
}

/// Volume of [`SoundType::CountInSoft`] relative to the count-in volume.
const COUNT_IN_SOFT_GAIN: f32 = 0.6;
/// Volume of the strong and soft counter clicks relative to the counter
/// volume of the mixer, which is the medium click's.
const COUNTER_STRONG_GAIN: f32 = 1.4;
pub(crate) const COUNTER_SOFT_GAIN: f32 = 0.5;

pub(crate) struct SoundProfile {
    pub(crate) freq_mult: f32,
    pub(crate) gain: f32,
}

impl SoundType {
    /// Whether a muted bar with `mute` silences this sound.
    pub(crate) fn muted_by(self, mute: GapMute) -> bool {
        match self {
            SoundType::CountIn | SoundType::CountInSoft => false,
            SoundType::Downbeat | SoundType::PrimaryBeat | SoundType::SoftBeat => true,
            SoundType::CounterStrong | SoundType::CounterMedium | SoundType::CounterSoft => true,
            SoundType::AccentedBeat | SoundType::Beat | SoundType::Ghost | SoundType::Kit(_) => {
                mute == GapMute::Everything
            }
        }
    }

    pub(crate) fn profile(self, settings: &AudioSettings) -> SoundProfile {
        let muted = match self {
            SoundType::Downbeat
            | SoundType::PrimaryBeat
            | SoundType::SoftBeat
            | SoundType::CounterStrong
            | SoundType::CounterMedium
            | SoundType::CounterSoft => settings.metronome_muted,
            SoundType::AccentedBeat | SoundType::Beat | SoundType::Ghost => settings.rhythm_muted,
            SoundType::CountIn | SoundType::CountInSoft => false,
            SoundType::Kit(_) => settings.kit_muted,
        };
        let profile = match self {
            SoundType::Downbeat => SoundProfile { freq_mult: 3.375, gain: settings.downbeat },
            SoundType::PrimaryBeat => SoundProfile { freq_mult: 1.0, gain: settings.primary },
            SoundType::SoftBeat => SoundProfile { freq_mult: 1.0, gain: settings.soft },
            SoundType::AccentedBeat => SoundProfile { freq_mult: 2.25, gain: settings.accent },
            SoundType::Beat => SoundProfile { freq_mult: 1.5, gain: settings.beat },
            SoundType::Ghost => SoundProfile { freq_mult: 1.5, gain: settings.ghost },
            SoundType::CountIn => SoundProfile { freq_mult: 2.25, gain: settings.count_in },
            SoundType::CountInSoft => {
                SoundProfile { freq_mult: 1.0, gain: COUNT_IN_SOFT_GAIN * settings.count_in }
            }
            SoundType::CounterStrong => SoundProfile {
                freq_mult: 5.0,
                gain: (COUNTER_STRONG_GAIN * settings.counter).min(1.0),
            },
            SoundType::CounterMedium => SoundProfile { freq_mult: 4.0, gain: settings.counter },
            SoundType::CounterSoft => {
                SoundProfile { freq_mult: 4.0, gain: COUNTER_SOFT_GAIN * settings.counter }
            }
            SoundType::Kit(hit) => SoundProfile {
                freq_mult: 1.0,
                gain: hit.level_gain() * settings.kit.of(hit.instrument),
            },
        };
        if muted { SoundProfile { gain: 0.0, ..profile } } else { profile }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) struct Schedule {
    map: BTreeMap<u64, Vec<SoundType>>,
}

impl Schedule {
    /// Build a schedule covering the entire score. Keys are global ticks across
    /// the score loop; per-measure offsets come from `timing.measure_start_tick`.
    /// `ghost_notes` adds [`SoundType::Ghost`] on the free slots of that grid.
    /// Notes and ghost notes sound at their swung position (see
    /// [`ScoreTiming::performed_global_tick`]). `click` holds the click bar
    /// of each meter: its notes click at their level (accented is the
    /// downbeat click, plain the primary beat click, soft the soft click),
    /// swung like the notes.
    pub(crate) fn build(
        score: &Score,
        timing: &ScoreTiming,
        ghost_notes: Option<Subdivision>,
        click: &ClickBars,
    ) -> Self {
        let mut map: BTreeMap<u64, Vec<SoundType>> = BTreeMap::new();

        for (idx, measure) in score.measures.iter().enumerate() {
            let ts = measure.time_signature();

            let bar = click.for_meter(ts);
            let bar_onsets = DEFAULT_GRID.compute_onset_ticks(bar.first_voice().beats());
            for (beat, &t) in bar.first_voice().beats().iter().zip(&bar_onsets) {
                let sound = match ClickLevel::of(beat) {
                    ClickLevel::Strong => SoundType::Downbeat,
                    ClickLevel::Medium => SoundType::PrimaryBeat,
                    ClickLevel::Soft => SoundType::SoftBeat,
                    ClickLevel::Off => continue,
                };
                map.entry(timing.performed_global_tick(idx, t)).or_default().push(sound);
            }

            // The heads of every voice, the one line's as before.
            for onset in measure.head_onsets() {
                map.entry(timing.performed_global_tick(idx, onset.tick))
                    .or_default()
                    .push(SoundType::of_head(&onset));
            }

            if let Some(subdivision) = ghost_notes {
                for t in ghost_onsets(measure, subdivision) {
                    map.entry(timing.performed_global_tick(idx, t))
                        .or_default()
                        .push(SoundType::Ghost);
                }
            }
        }

        Self { map }
    }

    /// Add the clicks of the counter bar of each measure's meter, if it has
    /// one. They sound where they are written: a counter rhythm does not
    /// swing.
    pub(crate) fn with_counter(
        mut self,
        score: &Score,
        timing: &ScoreTiming,
        counter: &CounterBars,
    ) -> Self {
        for (idx, measure) in score.measures.iter().enumerate() {
            let Some(bar) = counter.for_meter(measure.time_signature()) else {
                continue;
            };
            let onsets = DEFAULT_GRID.compute_onset_ticks(bar.first_voice().beats());
            for (beat, &t) in bar.first_voice().beats().iter().zip(&onsets) {
                let sound = match ClickLevel::of(beat) {
                    ClickLevel::Strong => SoundType::CounterStrong,
                    ClickLevel::Medium => SoundType::CounterMedium,
                    ClickLevel::Soft => SoundType::CounterSoft,
                    ClickLevel::Off => continue,
                };
                self.map.entry(timing.to_global_tick(idx, t)).or_default().push(sound);
            }
        }
        self
    }

    /// Collect all sounds in the half-open tick interval `[start, end)`.
    /// `end` is `f64` because the audio cursor advances in sub-tick steps.
    pub(crate) fn collect_in_range(&self, start: u64, end: f64, out: &mut Vec<SoundType>) {
        let mut k = start;
        while (k as f64) < end {
            if let Some(sounds) = self.map.get(&k) {
                out.extend_from_slice(sounds);
            }
            k += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grooph_measure::duration::q;
    use grooph_measure::{Beat, Measure, Score, TimeSignature};

    fn score_of_quarters(ts_list: &[TimeSignature]) -> Score {
        Score::new(
            ts_list
                .iter()
                .map(|ts| {
                    let mut m = Measure::new(*ts);
                    for i in 0..(ts.beats as usize) {
                        m.first_voice_mut().set_beat(i, Beat::note(q())).unwrap();
                    }
                    m
                })
                .collect(),
        )
    }

    #[test]
    fn voice_mutes_silence_their_sounds_but_not_the_count_in() {
        let settings = AudioSettings {
            metronome_muted: true,
            rhythm_muted: true,
            count_in: 0.8,
            ..AudioSettings::default()
        };
        for sound in [
            SoundType::Downbeat,
            SoundType::PrimaryBeat,
            SoundType::SoftBeat,
            SoundType::AccentedBeat,
            SoundType::Beat,
            SoundType::Ghost,
        ] {
            assert_eq!(sound.profile(&settings).gain, 0.0, "{sound:?}");
        }
        assert_eq!(SoundType::CountIn.profile(&settings).gain, 0.8);
        let only_rhythm = AudioSettings { rhythm_muted: true, ..AudioSettings::default() };
        assert_eq!(SoundType::Downbeat.profile(&only_rhythm).gain, only_rhythm.downbeat);
        assert_eq!(SoundType::Beat.profile(&only_rhythm).gain, 0.0);
    }

    /// Metronome sounds of the first bar of a one-measure `ts` score with
    /// `pattern`, as (tick in the bar, sound).
    fn clicks(ts: TimeSignature, bar: &str) -> Vec<(u64, SoundType)> {
        let score = Score::new(vec![Measure::new(ts)]);
        let timing = ScoreTiming::from_score(&score, 120);
        let mut bars = ClickBars::default();
        let bar = grooph_measure::notation::parse_score(&format!("{ts} {bar}")).unwrap();
        bars.set(bar.measures[0].clone());
        let s = Schedule::build(&score, &timing, None, &bars);
        s.map
            .iter()
            .flat_map(|(&t, sounds)| sounds.iter().map(move |&x| (t, x)))
            .filter(|(_, x)| {
                matches!(x, SoundType::Downbeat | SoundType::PrimaryBeat | SoundType::SoftBeat)
            })
            .collect()
    }

    #[test]
    fn default_click_pattern_is_downbeat_and_primary_beats() {
        let eighth = DEFAULT_GRID.ticks_per_beat(&TimeSignature::SEVEN_EIGHT) as u64;
        // 7/8 as 2+3+2.
        assert_eq!(
            clicks(TimeSignature::SEVEN_EIGHT, ">q q. q"),
            [
                (0, SoundType::Downbeat),
                (2 * eighth, SoundType::PrimaryBeat),
                (5 * eighth, SoundType::PrimaryBeat)
            ]
        );
    }

    #[test]
    fn meters_never_set_click_their_beat_unit() {
        // 4/4 then 5/8, no click bar set: quarters, then every eighth.
        let four = TimeSignature::FOUR_FOUR;
        let five = TimeSignature::FIVE_EIGHT;
        let score = Score::new(vec![Measure::new(four), Measure::new(five)]);
        let timing = ScoreTiming::from_score(&score, 120);
        let s = Schedule::build(&score, &timing, None, &ClickBars::default());
        let quarter = DEFAULT_GRID.ticks_per_beat(&four) as u64;
        let eighth = DEFAULT_GRID.ticks_per_beat(&five) as u64;
        let bar_two = 4 * quarter;
        let click_ticks: Vec<u64> = s
            .map
            .iter()
            .filter(|(_, sounds)| {
                sounds.iter().any(|x| {
                    matches!(x, SoundType::Downbeat | SoundType::PrimaryBeat | SoundType::SoftBeat)
                })
            })
            .map(|(&t, _)| t)
            .collect();
        let expected: Vec<u64> =
            (0..4).map(|i| i * quarter).chain((0..5).map(|i| bar_two + i * eighth)).collect();
        assert_eq!(click_ticks, expected);
    }

    #[test]
    fn click_pattern_sets_the_sound_of_each_group() {
        let eighth = DEFAULT_GRID.ticks_per_beat(&TimeSignature::SEVEN_EIGHT) as u64;
        assert_eq!(
            clicks(TimeSignature::SEVEN_EIGHT, "q r:q. >q"),
            [(0, SoundType::PrimaryBeat), (5 * eighth, SoundType::Downbeat)]
        );
        let quarter = DEFAULT_GRID.ticks_per_beat(&TimeSignature::FOUR_FOUR) as u64;
        assert_eq!(
            clicks(TimeSignature::FOUR_FOUR, ">q (q) q (q)"),
            [
                (0, SoundType::Downbeat),
                (quarter, SoundType::SoftBeat),
                (2 * quarter, SoundType::PrimaryBeat),
                (3 * quarter, SoundType::SoftBeat)
            ]
        );
    }

    #[test]
    fn click_bars_play_every_note_and_skip_rests() {
        let quarter = DEFAULT_GRID.ticks_per_beat(&TimeSignature::FOUR_FOUR) as u64;
        let third = quarter / 3;
        // Triplets with the middle one silent, on one beat.
        let c = clicks(TimeSignature::FOUR_FOUR, ">t8 r:t8 (t8) q q q");
        assert_eq!(
            c,
            [
                (0, SoundType::Downbeat),
                (2 * third, SoundType::SoftBeat),
                (quarter, SoundType::PrimaryBeat),
                (2 * quarter, SoundType::PrimaryBeat),
                (3 * quarter, SoundType::PrimaryBeat)
            ]
        );
    }

    #[test]
    fn soft_beats_play_at_their_own_volume() {
        let settings = AudioSettings { soft: 0.3, ..AudioSettings::default() };
        assert_eq!(SoundType::SoftBeat.profile(&settings).gain, 0.3);
        assert!(SoundType::SoftBeat.muted_by(GapMute::Metronome));
    }

    #[test]
    fn single_measure_score_has_downbeat_at_zero() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let s = Schedule::build(&score, &timing, None, &ClickBars::default());
        assert!(s.map.contains_key(&0));
        assert!(s.map[&0].contains(&SoundType::Downbeat));
    }

    #[test]
    fn multi_measure_downbeats_at_measure_starts() {
        let score = score_of_quarters(&[
            TimeSignature::FOUR_FOUR,
            TimeSignature::THREE_FOUR,
            TimeSignature::FOUR_FOUR,
        ]);
        let timing = ScoreTiming::from_score(&score, 120);
        let s = Schedule::build(&score, &timing, None, &ClickBars::default());
        for i in 0..score.len() {
            let start = timing.measure_start_tick(i);
            let sounds = s.map.get(&start).expect("downbeat key");
            assert!(sounds.contains(&SoundType::Downbeat), "no downbeat at idx {i}");
        }
    }

    #[test]
    fn counter_bars_click_unswung_at_their_levels() {
        use grooph_measure::counter::{Ratio, counter_bar};
        use grooph_measure::swing::{Swing, SwingUnit};
        let ts = TimeSignature::FOUR_FOUR;
        let score = Score::new(vec![Measure::new(ts), Measure::new(ts.with_grouping(0))]);
        let swing = Swing { unit: SwingUnit::Eighths, percent: 75 };
        let timing = ScoreTiming::from_score(&score, 120).with_swing(swing);
        let mut counter = CounterBars::default();
        let mut bar = counter_bar(ts, Ratio { n: 3, span: 2 }).unwrap();
        bar.first_voice_mut().set_click_level(1, ClickLevel::Strong);
        bar.first_voice_mut().set_click_level(2, ClickLevel::Off);
        bar.first_voice_mut().set_click_level(3, ClickLevel::Soft);
        counter.set(ts, Some(bar));
        let s = Schedule::build(&score, &timing, None, &ClickBars::default())
            .with_counter(&score, &timing, &counter);
        let counter_clicks: Vec<_> = s
            .map
            .iter()
            .flat_map(|(&t, sounds)| sounds.iter().map(move |&x| (t, x)))
            .filter(|(_, x)| x.is_counter())
            .collect();
        // Quarter triplets: a click every 2/3 of a beat, in both bars, where
        // they are written although the eighths swing.
        let third = DEFAULT_GRID.ticks_per_beat(&ts) as u64 * 2 / 3;
        let bar_ticks = timing.measure_start_tick(1);
        assert_eq!(counter_clicks.len(), 10);
        assert_eq!(counter_clicks[0], (0, SoundType::CounterMedium));
        assert_eq!(counter_clicks[1], (third, SoundType::CounterStrong));
        assert_eq!(counter_clicks[2], (3 * third, SoundType::CounterSoft));
        assert_eq!(counter_clicks[5], (bar_ticks, SoundType::CounterMedium));
    }

    #[test]
    fn counter_clicks_follow_the_metronome_mute_and_muted_bars() {
        let metronome_muted = AudioSettings { metronome_muted: true, ..AudioSettings::default() };
        assert_eq!(SoundType::CounterMedium.profile(&metronome_muted).gain, 0.0);
        let rhythm_muted = AudioSettings { rhythm_muted: true, ..AudioSettings::default() };
        assert!(SoundType::CounterMedium.profile(&rhythm_muted).gain > 0.0);
        assert!(SoundType::CounterSoft.muted_by(GapMute::Metronome));
    }

    #[test]
    fn multi_measure_notes_offset_by_measure_start() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR, TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let s = Schedule::build(&score, &timing, None, &ClickBars::default());

        let m1_start = timing.measure_start_tick(1);
        let onsets = DEFAULT_GRID.compute_onset_ticks(score.measures[1].first_voice().beats());
        for &local in &onsets {
            let key = m1_start + local as u64;
            assert!(s.map.contains_key(&key), "missing key {key} for measure 1 note");
        }
    }

    #[test]
    fn ghost_notes_fill_free_sixteenths_only_when_enabled() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let ghosts =
            |s: &Schedule| s.map.values().flatten().filter(|&&x| x == SoundType::Ghost).count();
        assert_eq!(ghosts(&Schedule::build(&score, &timing, None, &ClickBars::default())), 0);
        let with =
            Schedule::build(&score, &timing, Some(Subdivision::Sixteenths), &ClickBars::default());
        // 16 sixteenth slots minus 4 quarter-note onsets.
        assert_eq!(ghosts(&with), 12);
        assert!(!with.map[&0].contains(&SoundType::Ghost));
    }

    #[test]
    fn swing_delays_offbeat_notes_and_ghosts() {
        use grooph_measure::duration::e;
        use grooph_measure::swing::{Swing, SwingUnit};
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        for i in 0..8 {
            m.first_voice_mut().set_beat(i, Beat::note(e())).unwrap();
        }
        let score = Score::new(vec![m]);
        let swing = Swing { unit: SwingUnit::Eighths, percent: 75 };
        let timing = ScoreTiming::from_score(&score, 120).with_swing(swing);
        let s =
            Schedule::build(&score, &timing, Some(Subdivision::Sixteenths), &ClickBars::default());

        let tpq = DEFAULT_GRID.ticks_of(&q()).unwrap() as u64;
        let tpe = tpq / 2;
        let tps = tpq / 4;
        for beat in 0..4 {
            let on = beat * tpq;
            assert!(s.map[&on].contains(&SoundType::Beat));
            // The "&" moves to the fourth sixteenth; nothing is left straight.
            assert!(s.map[&(on + 3 * tps)].contains(&SoundType::Beat));
            assert!(!s.map.contains_key(&(on + tpe)));
            // Ghost "e" is stretched along with the first half of the pair.
            assert!(s.map[&(on + tps * 3 / 2)].contains(&SoundType::Ghost));
        }
    }

    #[test]
    fn collect_in_range_picks_up_boundary_inclusive_start_exclusive_end() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let s = Schedule::build(&score, &timing, None, &ClickBars::default());
        // Downbeat is at tick 0.
        let mut out = vec![];
        s.collect_in_range(0, 1.0, &mut out);
        assert!(out.contains(&SoundType::Downbeat));
        // Half-open: starting *after* the downbeat should not collect it.
        let mut out2 = vec![];
        s.collect_in_range(1, 2.0, &mut out2);
        assert!(!out2.contains(&SoundType::Downbeat));
    }

    /// The kit hits of a one-measure score, as (tick, instrument,
    /// technique, level).
    fn kit_hits(notation: &str) -> Vec<(u64, Instrument, Technique, Level)> {
        let score = grooph_measure::notation::parse_score(notation).unwrap();
        let timing = ScoreTiming::from_score(&score, 120);
        let s = Schedule::build(&score, &timing, None, &ClickBars::default());
        s.map
            .iter()
            .flat_map(|(&t, sounds)| sounds.iter().map(move |&x| (t, x)))
            .filter_map(|(t, x)| match x {
                SoundType::Kit(h) => Some((t, h.instrument, h.technique, h.level)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_rock_groove_plays_every_head_of_both_voices() {
        use Instrument::{HiHat, Kick, Snare};
        use Technique::{Closed, Normal, Open};
        let hits = kit_hits("4/4 H:e S-H:e H:e >S-H:e H:e (S)-H:e Ho:e Hc:e ; K:q r:q K:q r:q");
        let e = DEFAULT_GRID.ticks_per_measure(&TimeSignature::FOUR_FOUR) as u64 / 8;
        let (n, strong, soft) = (Level::Normal, Level::Strong, Level::Soft);
        assert_eq!(
            hits,
            [
                (0, HiHat, Normal, n),
                (0, Kick, Normal, n),
                (e, Snare, Normal, n),
                (e, HiHat, Normal, n),
                (2 * e, HiHat, Normal, n),
                (3 * e, Snare, Normal, strong),
                (3 * e, HiHat, Normal, n),
                (4 * e, HiHat, Normal, n),
                (4 * e, Kick, Normal, n),
                (5 * e, Snare, Normal, soft),
                (5 * e, HiHat, Normal, n),
                (6 * e, HiHat, Open, n),
                (7 * e, HiHat, Closed, n),
            ]
        );
    }

    #[test]
    fn a_line_score_plays_its_notes_as_before() {
        let score = grooph_measure::notation::parse_score("4/4 q >e (e) r:q q").unwrap();
        let timing = ScoreTiming::from_score(&score, 120);
        let s = Schedule::build(&score, &timing, None, &ClickBars::default());
        let notes: Vec<(u64, SoundType)> = s
            .map
            .iter()
            .flat_map(|(&t, sounds)| sounds.iter().map(move |&x| (t, x)))
            .filter(|(_, x)| x.is_rhythm() || matches!(x, SoundType::Kit(_)))
            .collect();
        let q = DEFAULT_GRID.ticks_per_beat(&TimeSignature::FOUR_FOUR) as u64;
        assert_eq!(
            notes,
            [
                (0, SoundType::Beat),
                (q, SoundType::AccentedBeat),
                // A written ghost note on the line plays as a plain note.
                (q + q / 2, SoundType::Beat),
                (3 * q, SoundType::Beat),
            ]
        );
    }

    #[test]
    fn the_kit_has_its_own_mute_and_volumes() {
        let hit = |instrument, level| {
            SoundType::Kit(KitHit { instrument, technique: Technique::Normal, level })
        };
        let settings = AudioSettings::default();
        let kick = hit(Instrument::Kick, Level::Normal);
        assert!(kick.profile(&settings).gain > 0.0);
        let accent = hit(Instrument::Kick, Level::Strong).profile(&settings).gain;
        let ghost = hit(Instrument::Kick, Level::Soft).profile(&settings).gain;
        assert!(ghost < kick.profile(&settings).gain && kick.profile(&settings).gain < accent);
        // The rhythm's mute leaves the kit; the kit's silences it.
        let rhythm_muted = AudioSettings { rhythm_muted: true, ..settings };
        assert!(kick.profile(&rhythm_muted).gain > 0.0);
        let kit_muted = AudioSettings { kit_muted: true, ..settings };
        assert_eq!(kick.profile(&kit_muted).gain, 0.0);
        assert!(SoundType::Beat.profile(&kit_muted).gain > 0.0);
        let mut quiet_hats = settings;
        quiet_hats.kit.hihat = 0.0;
        assert_eq!(hit(Instrument::HiHat, Level::Normal).profile(&quiet_hats).gain, 0.0);
        assert_eq!(hit(Instrument::HiHatFoot, Level::Normal).profile(&quiet_hats).gain, 0.0);
        assert!(kick.profile(&quiet_hats).gain > 0.0);
        // Muted bars of the gap trainer silence the kit with the notes.
        assert!(kick.muted_by(GapMute::Everything) && !kick.muted_by(GapMute::Metronome));
    }
}
