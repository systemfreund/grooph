use grooph_measure::BeatKind;
use grooph_measure::Score;
use grooph_measure::TimeSignature;
use grooph_measure::click::{ClickLevel, ClickPatterns};
use grooph_measure::generator::Subdivision;
use grooph_measure::ghost::ghost_onsets;
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::tempo::ScoreTiming;
use std::collections::BTreeMap;

use crate::AudioSettings;
use crate::trainer::GapMute;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SoundType {
    Downbeat,
    PrimaryBeat,
    /// Click on a beat unit between the primary beats, e.g. the eighths
    /// inside the 2+2+3 groups of 7/8. Silent unless its volume is raised.
    Pulse,
    /// Primary beat that the click pattern sets to [`ClickLevel::Soft`].
    SoftBeat,
    AccentedBeat,
    Beat,
    /// Quiet subdivision hit where no note is written.
    Ghost,
    /// Click of the count-in bar before playback starts.
    CountIn,
    /// [`SoundType::Pulse`] inside the count-in bar, at the pulse volume
    /// scaled by the count-in volume.
    CountInPulse,
}

pub(crate) struct SoundProfile {
    pub(crate) freq_mult: f32,
    pub(crate) gain: f32,
}

impl SoundType {
    /// Whether a muted bar with `mute` silences this sound.
    pub(crate) fn muted_by(self, mute: GapMute) -> bool {
        match self {
            SoundType::CountIn | SoundType::CountInPulse => false,
            SoundType::Downbeat
            | SoundType::PrimaryBeat
            | SoundType::SoftBeat
            | SoundType::Pulse => true,
            SoundType::AccentedBeat | SoundType::Beat | SoundType::Ghost => {
                mute == GapMute::Everything
            }
        }
    }

    pub(crate) fn profile(self, settings: &AudioSettings) -> SoundProfile {
        let muted = match self {
            SoundType::Downbeat
            | SoundType::PrimaryBeat
            | SoundType::SoftBeat
            | SoundType::Pulse => settings.metronome_muted,
            SoundType::AccentedBeat | SoundType::Beat | SoundType::Ghost => settings.rhythm_muted,
            SoundType::CountIn | SoundType::CountInPulse => false,
        };
        let profile = match self {
            SoundType::Downbeat => SoundProfile { freq_mult: 3.375, gain: settings.downbeat },
            SoundType::PrimaryBeat => SoundProfile { freq_mult: 1.0, gain: settings.primary },
            SoundType::Pulse => SoundProfile { freq_mult: 1.0, gain: settings.pulse },
            SoundType::SoftBeat => SoundProfile { freq_mult: 1.0, gain: settings.soft },
            SoundType::AccentedBeat => SoundProfile { freq_mult: 2.25, gain: settings.accent },
            SoundType::Beat => SoundProfile { freq_mult: 1.5, gain: settings.beat },
            SoundType::Ghost => SoundProfile { freq_mult: 1.5, gain: settings.ghost },
            SoundType::CountIn => SoundProfile { freq_mult: 2.25, gain: settings.count_in },
            SoundType::CountInPulse => {
                SoundProfile { freq_mult: 1.0, gain: settings.pulse * settings.count_in }
            }
        };
        if muted { SoundProfile { gain: 0.0, ..profile } } else { profile }
    }
}

/// Ticks within a bar of `ts` that get a [`SoundType::Pulse`]: every beat
/// unit that is neither the downbeat nor a primary beat.
pub(crate) fn pulse_ticks(ts: &TimeSignature) -> Vec<u32> {
    let primaries = DEFAULT_GRID.primary_boundaries(ts);
    let unit = DEFAULT_GRID.ticks_per_beat(ts);
    (unit..DEFAULT_GRID.ticks_per_measure(ts))
        .step_by(unit.max(1) as usize)
        .filter(|t| !primaries.contains(t))
        .collect()
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
    /// [`ScoreTiming::performed_global_tick`]). `click` sets the sound of
    /// each beat group: strong is the downbeat click, medium the primary
    /// beat click.
    pub(crate) fn build(
        score: &Score,
        timing: &ScoreTiming,
        ghost_notes: Option<Subdivision>,
        click: &ClickPatterns,
    ) -> Self {
        let mut map: BTreeMap<u64, Vec<SoundType>> = BTreeMap::new();

        for (idx, measure) in score.measures.iter().enumerate() {
            let ts = measure.time_signature();
            let start = timing.measure_start_tick(idx);

            let group_starts = std::iter::once(0).chain(DEFAULT_GRID.primary_boundaries(&ts));
            for (t, level) in group_starts.zip(click.for_meter(ts).0) {
                let sound = match level {
                    ClickLevel::Strong => SoundType::Downbeat,
                    ClickLevel::Medium => SoundType::PrimaryBeat,
                    ClickLevel::Soft => SoundType::SoftBeat,
                    ClickLevel::Off => continue,
                };
                map.entry(start + t as u64).or_default().push(sound);
            }
            for t in pulse_ticks(&ts) {
                map.entry(start + t as u64).or_default().push(SoundType::Pulse);
            }

            let onsets = DEFAULT_GRID.compute_onset_ticks(measure.beats());
            for (i, beat) in measure.beats().iter().enumerate() {
                if beat.kind == BeatKind::Note
                    && let Some(&t) = onsets.get(i)
                {
                    let s = if beat.accented { SoundType::AccentedBeat } else { SoundType::Beat };
                    map.entry(timing.performed_global_tick(idx, t)).or_default().push(s);
                }
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
        Score {
            measures: ts_list
                .iter()
                .map(|ts| {
                    let mut m = Measure::new(*ts);
                    for i in 0..(ts.beats as usize) {
                        m.set_beat(i, Beat::note(q())).unwrap();
                    }
                    m
                })
                .collect(),
        }
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
    fn clicks(ts: TimeSignature, pattern: &str) -> Vec<(u64, SoundType)> {
        let score = Score { measures: vec![Measure::new(ts)] };
        let timing = ScoreTiming::from_score(&score, 120);
        let mut patterns = ClickPatterns::default();
        assert!(patterns.set(ts, pattern.parse().unwrap()));
        let s = Schedule::build(&score, &timing, None, &patterns);
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
            clicks(TimeSignature::SEVEN_EIGHT, "SMM"),
            [
                (0, SoundType::Downbeat),
                (2 * eighth, SoundType::PrimaryBeat),
                (5 * eighth, SoundType::PrimaryBeat)
            ]
        );
    }

    #[test]
    fn click_pattern_sets_the_sound_of_each_group() {
        let eighth = DEFAULT_GRID.ticks_per_beat(&TimeSignature::SEVEN_EIGHT) as u64;
        assert_eq!(
            clicks(TimeSignature::SEVEN_EIGHT, "M-S"),
            [(0, SoundType::PrimaryBeat), (5 * eighth, SoundType::Downbeat)]
        );
        let quarter = DEFAULT_GRID.ticks_per_beat(&TimeSignature::FOUR_FOUR) as u64;
        assert_eq!(
            clicks(TimeSignature::FOUR_FOUR, "SLML"),
            [
                (0, SoundType::Downbeat),
                (quarter, SoundType::SoftBeat),
                (2 * quarter, SoundType::PrimaryBeat),
                (3 * quarter, SoundType::SoftBeat)
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
    fn pulse_fills_the_beat_units_between_primary_beats() {
        let pulses = |ts: TimeSignature| {
            let score = Score { measures: vec![Measure::new(ts)] };
            let timing = ScoreTiming::from_score(&score, 120);
            let s = Schedule::build(&score, &timing, None, &ClickPatterns::default());
            let count = |t: SoundType| s.map.values().flatten().filter(|&&x| x == t).count();
            (count(SoundType::PrimaryBeat), count(SoundType::Pulse))
        };
        // 7/8 (2+3+2): downbeat, 2 primary beats and 4 pulses = all 7 eighths.
        assert_eq!(pulses(TimeSignature::new(7, 8)), (2, 4));
        assert_eq!(pulses(TimeSignature::SIX_EIGHT), (1, 4));
        // 5/8 as 2+3 still clicks one primary beat.
        assert_eq!(pulses(TimeSignature::FIVE_EIGHT.with_grouping(1)), (1, 3));
        // In x/4 every beat unit is already a primary beat.
        assert_eq!(pulses(TimeSignature::FOUR_FOUR), (3, 0));
    }

    #[test]
    fn single_measure_score_has_downbeat_at_zero() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let s = Schedule::build(&score, &timing, None, &ClickPatterns::default());
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
        let s = Schedule::build(&score, &timing, None, &ClickPatterns::default());
        for i in 0..score.len() {
            let start = timing.measure_start_tick(i);
            let sounds = s.map.get(&start).expect("downbeat key");
            assert!(sounds.contains(&SoundType::Downbeat), "no downbeat at idx {i}");
        }
    }

    #[test]
    fn multi_measure_notes_offset_by_measure_start() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR, TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let s = Schedule::build(&score, &timing, None, &ClickPatterns::default());

        let m1_start = timing.measure_start_tick(1);
        let onsets = DEFAULT_GRID.compute_onset_ticks(score.measures[1].beats());
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
        assert_eq!(ghosts(&Schedule::build(&score, &timing, None, &ClickPatterns::default())), 0);
        let with = Schedule::build(
            &score,
            &timing,
            Some(Subdivision::Sixteenths),
            &ClickPatterns::default(),
        );
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
            m.set_beat(i, Beat::note(e())).unwrap();
        }
        let score = Score { measures: vec![m] };
        let swing = Swing { unit: SwingUnit::Eighths, percent: 75 };
        let timing = ScoreTiming::from_score(&score, 120).with_swing(swing);
        let s = Schedule::build(
            &score,
            &timing,
            Some(Subdivision::Sixteenths),
            &ClickPatterns::default(),
        );

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
        let s = Schedule::build(&score, &timing, None, &ClickPatterns::default());
        // Downbeat is at tick 0.
        let mut out = vec![];
        s.collect_in_range(0, 1.0, &mut out);
        assert!(out.contains(&SoundType::Downbeat));
        // Half-open: starting *after* the downbeat should not collect it.
        let mut out2 = vec![];
        s.collect_in_range(1, 2.0, &mut out2);
        assert!(!out2.contains(&SoundType::Downbeat));
    }
}
