use grooph_measure::BeatKind;
use grooph_measure::Score;
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
    AccentedBeat,
    Beat,
    /// Quiet subdivision hit where no note is written.
    Ghost,
    /// Click of the count-in bar before playback starts.
    CountIn,
}

pub(crate) struct SoundProfile {
    pub(crate) freq_mult: f32,
    pub(crate) gain: f32,
}

impl SoundType {
    /// Whether a muted bar with `mute` silences this sound.
    pub(crate) fn muted_by(self, mute: GapMute) -> bool {
        match self {
            SoundType::CountIn => false,
            SoundType::Downbeat | SoundType::PrimaryBeat | SoundType::Pulse => true,
            SoundType::AccentedBeat | SoundType::Beat | SoundType::Ghost => {
                mute == GapMute::Everything
            }
        }
    }

    pub(crate) fn profile(self, settings: &AudioSettings) -> SoundProfile {
        let muted = match self {
            SoundType::Downbeat | SoundType::PrimaryBeat | SoundType::Pulse => {
                settings.metronome_muted
            }
            SoundType::AccentedBeat | SoundType::Beat | SoundType::Ghost => settings.rhythm_muted,
            SoundType::CountIn => false,
        };
        let profile = match self {
            SoundType::Downbeat => SoundProfile { freq_mult: 3.375, gain: settings.downbeat },
            SoundType::PrimaryBeat => SoundProfile { freq_mult: 1.0, gain: settings.primary },
            SoundType::Pulse => SoundProfile { freq_mult: 1.0, gain: settings.pulse },
            SoundType::AccentedBeat => SoundProfile { freq_mult: 2.25, gain: settings.accent },
            SoundType::Beat => SoundProfile { freq_mult: 1.5, gain: settings.beat },
            SoundType::Ghost => SoundProfile { freq_mult: 1.5, gain: settings.ghost },
            SoundType::CountIn => SoundProfile { freq_mult: 2.25, gain: settings.count_in },
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
    /// [`ScoreTiming::performed_global_tick`]).
    pub(crate) fn build(
        score: &Score,
        timing: &ScoreTiming,
        ghost_notes: Option<Subdivision>,
    ) -> Self {
        let mut map: BTreeMap<u64, Vec<SoundType>> = BTreeMap::new();

        for (idx, measure) in score.measures.iter().enumerate() {
            let ts = measure.time_signature();
            let start = timing.measure_start_tick(idx);

            map.entry(start).or_default().push(SoundType::Downbeat);

            let primaries = DEFAULT_GRID.primary_boundaries(&ts);
            for &t in &primaries {
                map.entry(start + t as u64).or_default().push(SoundType::PrimaryBeat);
            }
            let unit = DEFAULT_GRID.ticks_per_beat(&ts);
            for t in (unit..DEFAULT_GRID.ticks_per_measure(&ts)).step_by(unit.max(1) as usize) {
                if !primaries.contains(&t) {
                    map.entry(start + t as u64).or_default().push(SoundType::Pulse);
                }
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

    #[test]
    fn pulse_fills_the_beat_units_between_primary_beats() {
        let pulses = |ts: TimeSignature| {
            let score = Score { measures: vec![Measure::new(ts)] };
            let timing = ScoreTiming::from_score(&score, 120);
            let s = Schedule::build(&score, &timing, None);
            let count = |t: SoundType| s.map.values().flatten().filter(|&&x| x == t).count();
            (count(SoundType::PrimaryBeat), count(SoundType::Pulse))
        };
        // 7/8 (2+3+2): downbeat, 2 primary beats and 4 pulses = all 7 eighths.
        assert_eq!(pulses(TimeSignature { beats: 7, beat_unit: 8 }), (2, 4));
        assert_eq!(pulses(TimeSignature::SIX_EIGHT), (1, 4));
        // In x/4 every beat unit is already a primary beat.
        assert_eq!(pulses(TimeSignature::FOUR_FOUR), (3, 0));
    }

    #[test]
    fn single_measure_score_has_downbeat_at_zero() {
        let score = score_of_quarters(&[TimeSignature::FOUR_FOUR]);
        let timing = ScoreTiming::from_score(&score, 120);
        let s = Schedule::build(&score, &timing, None);
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
        let s = Schedule::build(&score, &timing, None);
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
        let s = Schedule::build(&score, &timing, None);

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
        assert_eq!(ghosts(&Schedule::build(&score, &timing, None)), 0);
        let with = Schedule::build(&score, &timing, Some(Subdivision::Sixteenths));
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
        let s = Schedule::build(&score, &timing, Some(Subdivision::Sixteenths));

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
        let s = Schedule::build(&score, &timing, None);
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
