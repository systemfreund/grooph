//! App side of the metronome trainer: follows the bar the audio engine plays
//! to apply the tempo ramp to the UI timing and to show muted bars, and
//! which of the silent bars' *Mute* values each page offers.

use crate::documents::Document;
use crate::score_change::ScoreChange;
use crate::{Grooph, TransportState};
use grooph_audio::{GapMute, RampUnit, TempoRamp};

impl Grooph {
    /// Track the ramp tempo and gap state of the bar being played. A tempo
    /// change re-anchors the accuracy session, whose timeline assumes one tempo.
    pub(crate) fn update_metronome_trainer(&mut self) {
        let playing = self.playback_ctl.transport_state == TransportState::Playing;
        let bar = match self.playback_ctl.audio.as_mut() {
            Some(audio) if playing => audio.bar_state(),
            _ => None,
        };
        let ramp = self.played_ramp();
        let bpm = self.played_bpm();
        let ramp_bpm = (playing && ramp.enabled).then(|| match bar {
            // The loop's length as the engine counts it for that bar.
            Some(b) => ramp.bpm_for_bar(bpm, b.bar, b.loop_len),
            None => bpm,
        });
        self.playback_ctl.playback.bar_muted = bar.is_some_and(|b| b.muted);
        self.playback_ctl.playback.bar_mute = self.played_mute();
        if self.playback_ctl.playback.ramp_bpm != ramp_bpm {
            self.playback_ctl.playback.ramp_bpm = ramp_bpm;
            self.score_changed(ScoreChange::Timing);
        }
    }
}

impl Grooph {
    /// The *Mute* values the silent bars of `doc`'s page offer: Practice and
    /// the score tab all clicks or clicks and notes, the metronome with a
    /// polyrhythm all clicks or one of its layers, and without one no
    /// choice (all clicks).
    pub(crate) fn offered_mutes_of(&self, doc: Document) -> &'static [GapMute] {
        match doc {
            Document::Metronome if self.counter_bar().is_some() => {
                &[GapMute::Metronome, GapMute::ClickBar, GapMute::Counter]
            }
            Document::Metronome => &[],
            Document::Practice | Document::Score => &[GapMute::Metronome, GapMute::Everything],
        }
    }

    /// [`Self::offered_mutes_of`] the page shown.
    pub(crate) fn offered_mutes(&self) -> &'static [GapMute] {
        self.offered_mutes_of(self.document())
    }

    /// What a silent bar of `doc`'s page silences: its saved *Mute* value
    /// if the page offers it, else all clicks. The saved value stays.
    pub(crate) fn played_mute_of(&self, doc: Document) -> GapMute {
        let mute = self.playback_ctl.settings.get(doc).gap_click.mute;
        if self.offered_mutes_of(doc).contains(&mute) { mute } else { GapMute::Metronome }
    }

    /// [`Self::played_mute_of`] the page shown.
    pub(crate) fn played_mute(&self) -> GapMute {
        self.played_mute_of(self.document())
    }

    /// The tempo ramp as played: endless mode never loops, so there a ramp
    /// counted in loops counts in bars (see [`Self::endless_runs`]).
    pub(crate) fn played_ramp(&self) -> TempoRamp {
        if self.calibrating() {
            return TempoRamp::default();
        }
        let mut ramp = self.tab_playback().tempo_ramp;
        if self.endless_runs() {
            ramp.unit = RampUnit::Bars;
        }
        ramp
    }
}
