use grooph_measure::tempo::ScoreTiming;

/// Single-measure tempo view derived from a [`ScoreTiming`].
///
/// Reproduces the semantics of the former `TempoMap` (single-TS). Used by the
/// MIDI/Accuracy pipeline which is still single-measure at `cursor.measure_idx`.
///
/// TODO(midi-multi-measure): once MIDI becomes multi-measure, callers should
/// invoke `ScoreTiming` methods directly instead of going through this bridge.
/// See `accuracy.rs` and `handle_midi_input_events` for the migration notes.
#[derive(Clone, Copy)]
pub(crate) struct LocalTempo {
    pub bpm: u32,
    pub ticks_per_sec: f64,
    pub ticks_per_measure: f64,
}

impl LocalTempo {
    pub(crate) fn from_score_timing(timing: &ScoreTiming, measure_idx: usize) -> Self {
        Self {
            bpm: timing.bpm(),
            ticks_per_sec: timing.ticks_per_sec_in_measure(measure_idx),
            ticks_per_measure: timing.ticks_per_measure(measure_idx) as f64,
        }
    }

    pub(crate) fn valid(&self) -> bool {
        self.ticks_per_sec > 0.0 && self.ticks_per_measure > 0.0
    }
}
