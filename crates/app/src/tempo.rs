use grooph_measure::TimeSignature;
use grooph_measure::grid::DEFAULT_GRID;

#[derive(Clone, Copy)]
pub(crate) struct TempoMap {
    pub bpm: u32,
    pub ticks_per_beat: f64,
    pub ticks_per_sec: f64,
    pub ticks_per_measure: f64,
}

impl TempoMap {
    pub(crate) fn new(bpm: u32, ts: &TimeSignature) -> Self {
        let ticks_per_beat = DEFAULT_GRID.ticks_per_beat(ts) as f64;
        let ticks_per_measure = DEFAULT_GRID.ticks_per_measure(ts) as f64;
        let ticks_per_sec = (bpm as f64 / 60.0) * ticks_per_beat;
        Self { bpm, ticks_per_beat, ticks_per_sec, ticks_per_measure }
    }

    pub(crate) fn valid(&self) -> bool { self.ticks_per_sec > 0.0 && self.ticks_per_measure > 0.0 }
}
