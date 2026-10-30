//! What sounds when in a measure: the beats of all its voices, merged.
//!
//! Playback, the count labels, the playback cursor and the evaluation of
//! a play-along all follow the measure as it sounds, not one of its voices.
//! A rhythm on one line has one voice and one head per note, so its onsets
//! are its beats'.

use crate::grid::DEFAULT_GRID;
use crate::{Beat, BeatIdx, Head, Measure, VoiceIdx};

/// One head sounding in a measure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeadOnset {
    /// Where it starts, in ticks from the start of the measure.
    pub tick: u32,
    pub voice: VoiceIdx,
    pub beat_idx: BeatIdx,
    pub head: Head,
}

/// A point in time where a beat of some voice starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Onset {
    /// Ticks from the start of the measure.
    pub tick: u32,
    /// Whether a voice plays a note there; `false` where all are rests.
    pub is_note: bool,
    /// The beat that stands for the onset: the note there of the lowest
    /// numbered voice that plays one, or where none plays, the rest of the
    /// first voice. Its head is where the onset is drawn.
    pub voice: VoiceIdx,
    pub beat_idx: BeatIdx,
}

impl Measure {
    /// Every head of every voice, by tick, then by voice and from the foot
    /// up.
    pub fn head_onsets(&self) -> Vec<HeadOnset> {
        let mut out = Vec::new();
        for (voice, bar) in self.voices().iter().enumerate() {
            let ticks = DEFAULT_GRID.compute_onset_ticks(bar.beats());
            for (beat_idx, (beat, &tick)) in bar.beats().iter().zip(&ticks).enumerate() {
                out.extend(beat.heads().iter().map(|&head| HeadOnset {
                    tick,
                    voice,
                    beat_idx,
                    head,
                }));
            }
        }
        // Stable: within a tick the voices and heads keep their order.
        out.sort_by_key(|o| o.tick);
        out
    }

    /// The ticks where a beat of any voice starts, each once and in order.
    pub fn onsets(&self) -> Vec<Onset> {
        let voices: Vec<&[Beat]> = self.voices().iter().map(|v| &v.beats()[..]).collect();
        merged(&voices)
    }
}

/// The onsets of `voices` played together, as [`Measure::onsets`]: each
/// tick where a beat of one of them starts, once and in order. The layout
/// spaces voices by these, so the drawing and the sound agree on them.
pub fn merged(voices: &[&[Beat]]) -> Vec<Onset> {
    let mut out: Vec<Onset> = Vec::new();
    for (voice, beats) in voices.iter().enumerate() {
        let ticks = DEFAULT_GRID.compute_onset_ticks(beats);
        for (beat_idx, (beat, &tick)) in beats.iter().zip(&ticks).enumerate() {
            out.push(Onset { tick, is_note: beat.is_note(), voice, beat_idx });
        }
    }
    // Stable: at a tick the notes come first, then by voice.
    out.sort_by_key(|o| (o.tick, !o.is_note));
    out.dedup_by_key(|o| o.tick);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notation::parse_score;
    use crate::{Instrument, Technique};

    fn measure(s: &str) -> Measure {
        parse_score(s).unwrap().measures.remove(0)
    }

    #[test]
    fn one_line_onsets_are_its_beats() {
        let m = measure("4/4 q e r:e >q (q)");
        let ticks = DEFAULT_GRID.compute_onset_ticks(m.beats());
        let onsets = m.onsets();
        assert_eq!(onsets.iter().map(|o| o.tick).collect::<Vec<_>>(), ticks);
        assert_eq!(
            onsets.iter().map(|o| o.is_note).collect::<Vec<_>>(),
            m.beats().iter().map(|b| b.is_note()).collect::<Vec<_>>()
        );
        let heads = m.head_onsets();
        assert_eq!(heads.len(), 4);
        assert!(heads.iter().all(|h| h.voice == 0 && h.head.instrument() == Instrument::Rhythm));
        assert_eq!(heads.iter().map(|h| h.beat_idx).collect::<Vec<_>>(), [0, 1, 3, 4]);
        assert!(heads[2].head.accented() && heads[3].head.soft());
    }

    #[test]
    fn voices_merge_by_tick() {
        // Hands in eighths, the kick on 1 and 3 with rests between.
        let m = measure("4/4 H:e S-H:e H:e H:e H:e S-H:e Ho:e Hc:e ; K:q r:q K:q r:q");
        let onsets = m.onsets();
        let eighth = DEFAULT_GRID.ticks_per_measure(&m.time_signature()) / 8;
        assert_eq!(onsets.len(), 8);
        assert!(onsets.iter().enumerate().all(|(i, o)| o.tick == i as u32 * eighth && o.is_note));

        let heads = m.head_onsets();
        let at = |tick: u32| -> Vec<(VoiceIdx, Instrument, Technique)> {
            heads
                .iter()
                .filter(|h| h.tick == tick)
                .map(|h| (h.voice, h.head.instrument(), h.head.technique()))
                .collect()
        };
        assert_eq!(
            at(0),
            [(0, Instrument::HiHat, Technique::Normal), (1, Instrument::Kick, Technique::Normal)]
        );
        assert_eq!(
            at(eighth),
            [(0, Instrument::Snare, Technique::Normal), (0, Instrument::HiHat, Technique::Normal)]
        );
        assert_eq!(at(6 * eighth), [(0, Instrument::HiHat, Technique::Open)]);
        assert!(heads.windows(2).all(|w| w[0].tick <= w[1].tick));
    }

    #[test]
    fn a_tick_is_a_note_if_any_voice_plays() {
        let m = measure("2/4 r:q H:q ; K:q r:q");
        let q = m.onsets()[1].tick;
        // The kick stands for the first onset, the hi-hat for the second.
        assert_eq!(
            m.onsets(),
            [
                Onset { tick: 0, is_note: true, voice: 1, beat_idx: 0 },
                Onset { tick: q, is_note: true, voice: 0, beat_idx: 1 },
            ]
        );
        let rests = measure("2/4 r:q H:q ; r:e r:e r:q");
        let notes: Vec<(bool, VoiceIdx)> =
            rests.onsets().iter().map(|o| (o.is_note, o.voice)).collect();
        // Rests only: the first voice's rest stands for the onset.
        assert_eq!(notes, [(false, 0), (false, 1), (true, 0)]);
    }
}
