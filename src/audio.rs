use crate::measure::Measure;
use crate::measure::grid::DEFAULT_GRID;
use rodio::Source;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use log::{info, log};

pub struct Audio {
    stream: rodio::OutputStream,
    sink: rodio::Sink,
    shared_state: Arc<Mutex<SharedState>>,
}

struct SharedState {
    params: PlaybackParams,
    dirty: bool,
    // live playback cursor in ticks within current measure, and measure length in ticks
    playback_tick: f64,
    total_ticks: u32,
}

#[derive(Clone)]
struct PlaybackParams {
    bpm: u32,
    ticks_per_beat: u32,
    ticks_per_measure: u32,
    schedule: BTreeMap<u32, SoundType>,
    reset_trigger: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SoundType {
    High,
    Low,
}

impl Audio {
    pub fn new(bpm: u32) -> Option<Self> {
        let stream = match rodio::OutputStreamBuilder::open_default_stream() {
            Ok(s) => Some(s),
            Err(e) => {
                info!("Failed to init audio: {}", e);
                None
            }
        }?;

        let params = PlaybackParams {
            bpm,
            ticks_per_beat: 0,
            ticks_per_measure: 0,
            schedule: BTreeMap::new(),
            reset_trigger: 0,
        };

        let shared_state = Arc::new(Mutex::new(SharedState {
            params,
            dirty: false,
            playback_tick: 0.0,
            total_ticks: 0,
        }));

        let sink = rodio::Sink::connect_new(stream.mixer());
        let source = MetronomeSource::new(shared_state.clone());
        sink.append(source);
        sink.pause();

        Some(Self { stream, sink, shared_state })
    }

    pub fn update(&mut self, is_running: bool, bpm: u32, measure: &Measure) {
        if is_running && self.sink.is_paused() {
            self.sink.play();
        } else if !is_running && !self.sink.is_paused() {
            self.sink.pause();
            // When stopping/pausing, we might want to reset cursor if it's a Stop.
            // But here we only have is_running.
            // We'll leave reset logic for now or add explicit "reset" method if needed.
            // For now, pause just pauses.
        }

        let mut state = self.shared_state.lock().unwrap();
        // Check if anything changed to avoid unnecessary dirtying (optimization)
        let ts = measure.time_signature();
        let ticks_per_beat = DEFAULT_GRID.ticks_per_beat(&ts);
        let ticks_per_measure = DEFAULT_GRID.ticks_per_measure(&ts);

        // Recompute schedule
        let mut schedule = BTreeMap::new();
        schedule.insert(0, SoundType::High);

        for t in DEFAULT_GRID.primary_boundaries(&ts) {
            schedule.entry(t).or_insert(SoundType::Low);
        }

        let onsets = DEFAULT_GRID.compute_onset_ticks(measure.beats());
        for (i, beat) in measure.beats().iter().enumerate() {
            if beat.accented
                && let Some(&t) = onsets.get(i)
            {
                schedule.insert(t, SoundType::High);
            }
        }

        // Check differences
        if state.params.bpm != bpm
            || state.params.ticks_per_beat != ticks_per_beat
            || state.params.ticks_per_measure != ticks_per_measure
            || state.params.schedule != schedule
        {
            state.params.bpm = bpm;
            state.params.ticks_per_beat = ticks_per_beat;
            state.params.ticks_per_measure = ticks_per_measure;
            state.params.schedule = schedule;
            state.dirty = true;
        }
    }

    // Optional: add explicit stop/reset method
    pub fn stop(&mut self) {
        self.sink.pause();
        let mut state = self.shared_state.lock().unwrap();
        state.params.reset_trigger += 1;
        state.dirty = true;
        state.playback_tick = 0.0;
        state.total_ticks = state.params.ticks_per_measure;
    }

    pub fn playback_position(&self) -> Option<(f64, u32)> {
        // Non-blocking try to avoid UI stalls; fall back to None if busy
        if let Ok(state) = self.shared_state.try_lock() {
            let total = if state.total_ticks != 0 {
                state.total_ticks
            } else {
                state.params.ticks_per_measure
            };
            Some((state.playback_tick, total))
        } else {
            None
        }
    }
}

struct MetronomeSource {
    shared: Arc<Mutex<SharedState>>,
    local_params: PlaybackParams,
    cursor: f64,
    sample_rate: u32,
    current_beep: Option<(f32, SoundType)>,
    samples_processed: usize,
}

impl MetronomeSource {
    fn new(shared: Arc<Mutex<SharedState>>) -> Self {
        let local_params = {
            let state = shared.lock().unwrap();
            state.params.clone()
        };

        Self {
            shared,
            local_params,
            cursor: 0.0,
            sample_rate: 44100,
            current_beep: None,
            samples_processed: 0,
        }
    }
}

impl Iterator for MetronomeSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        self.samples_processed += 1;

        // Periodic check for updates (e.g. every 256 samples ~5ms)
        let check_interval = 256;
        let should_check = (self.samples_processed % check_interval) == 0;

        if should_check
            && let Ok(mut state) = self.shared.try_lock()
            && state.dirty
        {
            // Check if reset was triggered
            if state.params.reset_trigger > self.local_params.reset_trigger {
                self.cursor = 0.0;
            }
            self.local_params = state.params.clone();
            state.dirty = false;
        }

        let bpm = self.local_params.bpm as f64;
        let ticks_per_beat = self.local_params.ticks_per_beat as f64;
        let total_ticks = self.local_params.ticks_per_measure as f64;

        if ticks_per_beat == 0.0 {
            return Some(0.0);
        }

        let ticks_per_sample = (bpm / 60.0 * ticks_per_beat) / (self.sample_rate as f64);

        let old_cursor = self.cursor;
        self.cursor += ticks_per_sample;
        let new_cursor = self.cursor;

        let mut triggered_sound = None;

        if new_cursor >= total_ticks {
            self.cursor -= total_ticks;
            // Wrapped
            if self.local_params.schedule.contains_key(&0) {
                triggered_sound = Some(self.local_params.schedule[&0]);
            }
        } else {
            // Check schedule
            // We use BTreeMap range.
            for (&_tick, &sound) in self
                .local_params
                .schedule
                .range((old_cursor.ceil() as u32)..(new_cursor.ceil() as u32))
            {
                triggered_sound = Some(sound);
            }
        }

        // Try to publish playback cursor to shared state (periodically, to avoid contention)
        if (self.samples_processed % 1024) == 0 {
             if let Ok(mut state) = self.shared.try_lock() {
                state.playback_tick = self.cursor;
                state.total_ticks = self.local_params.ticks_per_measure;
            }
        }

        if let Some(sound) = triggered_sound {
            self.current_beep = Some((0.0, sound));
        }

        if let Some((phase, sound_type)) = self.current_beep {
            let freq = match sound_type {
                SoundType::High => 1500.0,
                SoundType::Low => 800.0,
            };
            let decay = 0.05;
            let dt = 1.0 / (self.sample_rate as f32);
            let new_phase = phase + dt;

            return if new_phase > decay {
                self.current_beep = None;
                Some(0.0)
            } else {
                self.current_beep = Some((new_phase, sound_type));
                let val = (new_phase * freq * 2.0 * std::f32::consts::PI).sin();
                let amp = 1.0 - (new_phase / decay);
                Some(val * amp * 0.5)
            };
        }

        Some(0.0)
    }
}

impl Source for MetronomeSource {
    fn current_span_len(&self) -> Option<usize> {
        None // Infinite
    }

    fn channels(&self) -> u16 { 1 }

    fn sample_rate(&self) -> u32 { self.sample_rate }

    fn total_duration(&self) -> Option<Duration> { None }
}
