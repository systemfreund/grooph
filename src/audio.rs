use crate::measure::grid::DEFAULT_GRID;
use crate::measure::{BeatKind, Measure};
use log::{info, log};
use rodio::Source;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MixerVolumes {
    pub downbeat: f32,
    pub primary: f32,
    pub accent: f32,
    pub beat: f32,
}

impl Default for MixerVolumes {
    fn default() -> Self { Self { downbeat: 1.0, primary: 0.0, accent: 1.0, beat: 1.0 } }
}

impl MixerVolumes {
    pub fn new(downbeat: f32, primary: f32, accent: f32, beat: f32) -> Self {
        let result = Self { downbeat, primary, accent, beat };
        result.clamped();
        result
    }

    pub fn clamped(mut self) -> Self {
        self.downbeat = self.downbeat.clamp(0.0, 1.0);
        self.primary = self.primary.clamp(0.0, 1.0);
        self.accent = self.accent.clamp(0.0, 1.0);
        self.beat = self.beat.clamp(0.0, 1.0);
        self
    }
}

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
    // Whether source currently produces absolute silence (no active voices and not playing)
    is_silent: bool,
}

#[derive(Clone)]
struct PlaybackParams {
    bpm: u32,
    ticks_per_beat: u32,
    ticks_per_measure: u32,
    schedule: BTreeMap<u32, SoundType>,
    reset_trigger: usize,
    mixer: MixerVolumes,
    // Controls whether new tones are triggered and cursor advances
    playing: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SoundType {
    Downbeat,
    PrimaryBeat,
    AccentedBeat,
    Beat,
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
            mixer: MixerVolumes::default(),
            playing: false,
        };

        let shared_state = Arc::new(Mutex::new(SharedState {
            params,
            dirty: false,
            playback_tick: 0.0,
            total_ticks: 0,
            is_silent: true,
        }));

        let sink = rodio::Sink::connect_new(stream.mixer());
        let source = MetronomeSource::new(shared_state.clone());
        sink.append(source);
        sink.pause();

        Some(Self { stream, sink, shared_state })
    }

    // Returns true if UI should repaint soon (while playing or while waiting for tail-out)
    pub fn update(&mut self, is_running: bool, bpm: u32, measure: &Measure) -> bool {
        // Playback start: ensure sink runs
        if is_running && self.sink.is_paused() {
            self.sink.play();
        }

        // On stop: do not pause immediately; wait until source reports silence
        if !is_running
            && !self.sink.is_paused()
            && let Ok(state) = self.shared_state.try_lock()
            && state.is_silent
        {
            // Now it's safe to pause without pops
            self.sink.pause();
        }

        let mut state = self.shared_state.lock().unwrap();
        // Check if anything changed to avoid unnecessary dirtying
        let ts = measure.time_signature();
        let ticks_per_beat = DEFAULT_GRID.ticks_per_beat(&ts);
        let ticks_per_measure = DEFAULT_GRID.ticks_per_measure(&ts);

        // Recompute schedule
        let mut schedule = BTreeMap::new();
        schedule.insert(0, SoundType::Downbeat);

        for t in DEFAULT_GRID.primary_boundaries(&ts) {
            schedule.entry(t).or_insert(SoundType::PrimaryBeat);
        }

        let onsets = DEFAULT_GRID.compute_onset_ticks(measure.beats());
        for (i, beat) in measure.beats().iter().enumerate() {
            if beat.kind == BeatKind::Note
                && let Some(&t) = onsets.get(i)
            {
                let sound_type =
                    if beat.accented { SoundType::AccentedBeat } else { SoundType::Beat };

                schedule.insert(t, sound_type);
            }
        }

        // Check differences
        if state.params.bpm != bpm
            || state.params.ticks_per_beat != ticks_per_beat
            || state.params.ticks_per_measure != ticks_per_measure
            || state.params.schedule != schedule
            || state.params.playing != is_running
        {
            state.params.bpm = bpm;
            state.params.ticks_per_beat = ticks_per_beat;
            state.params.ticks_per_measure = ticks_per_measure;
            state.params.schedule = schedule;
            state.params.playing = is_running;
            state.dirty = true;
        }

        // Request repaint while tailing out awaiting pause
        !is_running && !self.sink.is_paused()
    }

    pub fn set_mixer(&mut self, mixer: MixerVolumes) {
        let mut state = self.shared_state.lock().unwrap();
        let m = mixer.clamped();
        if state.params.mixer != m {
            state.params.mixer = m;
            state.dirty = true;
        }
    }

    pub fn stop(&mut self) {
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
    // Polyphonic: multiple short beeps may overlap
    active_beeps: Vec<(f32, SoundType)>,
    samples_processed: usize,
}

impl MetronomeSource {
    // Voice cap used in multiple places
    const MAX_VOICES: usize = 8;

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
            active_beeps: Vec::with_capacity(4),
            samples_processed: 0,
        }
    }

    fn add_triggered_sounds(
        &mut self,
        triggered_sounds: &mut Vec<SoundType>,
        start: u32,
        limit: f64,
    ) {
        let mut k = start;
        while (k as f64) < limit {
            if let Some(&sound) = self.local_params.schedule.get(&k) {
                triggered_sounds.push(sound);
            }
            k += 1;
        }
    }

    fn update_shared_state(&mut self) {
        // Try to publish playback cursor to shared state (periodically, to avoid contention)
        if self.samples_processed.is_multiple_of(1024)
            && let Ok(mut state) = self.shared.try_lock()
        {
            state.playback_tick = self.cursor;
            state.total_ticks = self.local_params.ticks_per_measure;
            // Silent when not playing and no active voices left
            let silent = !self.local_params.playing && self.active_beeps.is_empty();
            state.is_silent = silent;
        }
    }

    fn determine_triggered_sounds(&mut self) -> Vec<SoundType> {
        let bpm = self.local_params.bpm as f64;
        let total_ticks = self.local_params.ticks_per_measure as f64;
        let ticks_per_beat = self.local_params.ticks_per_beat as f64;
        let ticks_per_sample = (bpm / 60.0 * ticks_per_beat) / (self.sample_rate as f64);

        let old_cursor = self.cursor;
        let mut new_cursor = old_cursor + ticks_per_sample;

        // Collect all triggers crossed by this sample advance using [old, new) interval
        let mut triggered_sounds: Vec<SoundType> = Vec::with_capacity(Self::MAX_VOICES);
        if self.local_params.playing {
            if new_cursor >= total_ticks {
                // 1. [old_cursor, total_ticks)
                self.add_triggered_sounds(
                    &mut triggered_sounds,
                    old_cursor.ceil() as u32,
                    total_ticks,
                );

                // Wrap
                new_cursor -= total_ticks;

                // 2. [0.0, new_cursor)
                self.add_triggered_sounds(&mut triggered_sounds, 0, new_cursor);
            } else {
                // [old_cursor, new_cursor)
                self.add_triggered_sounds(
                    &mut triggered_sounds,
                    old_cursor.ceil() as u32,
                    new_cursor,
                );
            }

            self.cursor = new_cursor;
        }
        triggered_sounds
    }

    fn enqueue_triggered_sounds(&mut self) {
        let triggered_sounds = self.determine_triggered_sounds();
        // Enqueue all triggered sounds as new voices (phase = 0)
        if !triggered_sounds.is_empty() {
            for sound in triggered_sounds {
                if self.active_beeps.len() >= Self::MAX_VOICES {
                    // drop the oldest to keep CPU bounded
                    self.active_beeps.remove(0);
                }
                self.active_beeps.push((0.0, sound));
            }
        }
    }

    fn update_local_state(&mut self) {
        self.samples_processed += 1;

        // Periodic check for updates (e.g. every 256 samples ~5ms)
        if self.samples_processed.is_multiple_of(256)
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
    }

    fn synthesize(&mut self) -> f32 {
        // Synthesize a sample by mixing all active voices with envelope
        let dt = 1.0 / (self.sample_rate as f32);
        const DECAY: f32 = 0.05; // ~50 ms
        const ATTACK: f32 = 0.001; // ~1 ms

        if self.active_beeps.is_empty() {
            // If not playing and no active voices, stay silent
            return 0.0;
        }

        // Advance phases and compute sum
        let mut mixed: f32 = 0.0;
        let mut i = 0;
        while i < self.active_beeps.len() {
            let (ref mut phase, sound_type) = self.active_beeps[i];
            *phase += dt;
            let p = *phase;
            if p > DECAY {
                // Remove finished voice
                self.active_beeps.remove(i);
                continue;
            }

            let freq = match sound_type {
                SoundType::Downbeat => 1597.0,
                SoundType::PrimaryBeat => 377.0,
                SoundType::Beat => 610.0,
                SoundType::AccentedBeat => 987.0,
            };
            let gain = match sound_type {
                SoundType::Downbeat => self.local_params.mixer.downbeat,
                SoundType::PrimaryBeat => self.local_params.mixer.primary,
                SoundType::Beat => self.local_params.mixer.beat,
                SoundType::AccentedBeat => self.local_params.mixer.accent,
            };

            let env_attack = (p / ATTACK).min(1.0);
            let env_decay = 1.0 - (p / DECAY);
            let env = env_attack * env_decay;
            let val = (p * freq * 2.0 * std::f32::consts::PI).sin();
            mixed += val * env * 0.6 * gain; // master gain 0.6 to leave headroom

            i += 1;
        }

        // Soft clip to avoid hard clipping when multiple voices overlap
        mixed.tanh()
    }
}

impl Iterator for MetronomeSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        self.update_local_state();
        self.update_shared_state();
        self.enqueue_triggered_sounds();
        Some(self.synthesize())
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
