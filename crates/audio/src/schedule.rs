use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::{BeatKind, Measure, TimeSignature};
use std::collections::BTreeMap;

use crate::AudioSettings;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SoundType {
    Downbeat,
    PrimaryBeat,
    AccentedBeat,
    Beat,
}

pub(crate) struct SoundProfile {
    pub(crate) freq_mult: f32,
    pub(crate) gain: f32,
}

impl SoundType {
    pub(crate) fn profile(self, settings: &AudioSettings) -> SoundProfile {
        match self {
            SoundType::Downbeat => SoundProfile { freq_mult: 3.375, gain: settings.downbeat },
            SoundType::PrimaryBeat => SoundProfile { freq_mult: 1.0, gain: settings.primary },
            SoundType::AccentedBeat => SoundProfile { freq_mult: 2.25, gain: settings.accent },
            SoundType::Beat => SoundProfile { freq_mult: 1.5, gain: settings.beat },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) struct Schedule {
    map: BTreeMap<u32, Vec<SoundType>>,
}

impl Schedule {
    pub(crate) fn build(measure: &Measure, ts: &TimeSignature) -> Self {
        let mut map: BTreeMap<u32, Vec<SoundType>> = BTreeMap::new();
        map.entry(0).or_default().push(SoundType::Downbeat);

        for t in DEFAULT_GRID.primary_boundaries(ts) {
            map.entry(t).or_default().push(SoundType::PrimaryBeat);
        }

        let onsets = DEFAULT_GRID.compute_onset_ticks(measure.beats());
        for (i, beat) in measure.beats().iter().enumerate() {
            if beat.kind == BeatKind::Note
                && let Some(&t) = onsets.get(i)
            {
                let s = if beat.accented { SoundType::AccentedBeat } else { SoundType::Beat };
                map.entry(t).or_default().push(s);
            }
        }
        Self { map }
    }

    /// Sammelt alle Sounds im halboffenen Tickintervall [start, end).
    /// `end` ist `f64`, weil die Cursor-Position aus Sample-Schritten kommt.
    pub(crate) fn collect_in_range(&self, start: u32, end: f64, out: &mut Vec<SoundType>) {
        let mut k = start;
        while (k as f64) < end {
            if let Some(sounds) = self.map.get(&k) {
                out.extend_from_slice(sounds);
            }
            k += 1;
        }
    }
}
