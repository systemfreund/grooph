use crate::measure::grid::DEFAULT_GRID;
use crate::measure::{BeatKind, Measure, TimeSignature};
use log::{debug, error, info, log, trace};
use rodio::Source;
use std::collections::BTreeMap;
use std::fmt::{Debug, Formatter, Write};
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
    stream: Option<rodio::OutputStream>,
    sink: Option<rodio::Sink>,
    shared_state: Arc<Mutex<PlaybackState>>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum PlayerState {
    Playing,
    Stopped,
}

struct PlaybackState {
    params: PlaybackParams,
    is_dirty: bool,
    // live playback cursor in ticks within current measure, and measure length in ticks
    playback_tick: f64,
    // Controls whether new tones are triggered and cursor advances
    playing_state: PlayerState,
    // Whether source currently produces absolute silence (no active voices and not playing)
    is_silent: bool,
}

impl Debug for PlaybackState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "PlaybackState {{ is_dirty: {}, playing_state: {:?}, playback_tick: {}}}",
            self.is_dirty, self.playing_state, self.playback_tick
        ))
    }
}

#[derive(Debug, Clone)]
struct PlaybackParams {
    bpm: u32,
    ticks_per_beat: u32,
    ticks_per_measure: u32,
    schedule: BTreeMap<u32, SoundType>,
    mixer: MixerVolumes,
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
        let params = PlaybackParams {
            bpm,
            ticks_per_beat: 0,
            ticks_per_measure: 0,
            schedule: BTreeMap::new(),
            mixer: MixerVolumes::default(),
        };

        let shared_state = Arc::new(Mutex::new(PlaybackState {
            params,
            is_dirty: false,
            playback_tick: 0.0,
            playing_state: PlayerState::Stopped,
            is_silent: true,
        }));

        Some(Self { stream: None, sink: None, shared_state })
    }

    // Returns true if UI should repaint soon (while playing or while waiting for tail-out)
    pub fn update(&mut self, player_state: &PlayerState, bpm: u32, measure: &Measure) -> bool {
        // Check if anything changed to avoid unnecessary dirtying
        let ts = measure.time_signature();
        let ticks_per_beat = DEFAULT_GRID.ticks_per_beat(&ts);
        let ticks_per_measure = DEFAULT_GRID.ticks_per_measure(&ts);

        // Recompute schedule
        let schedule = Self::compute_schedule(measure, &ts);

        // Check differences
        if let Ok(mut shared_state) = self.shared_state.lock()
            && (shared_state.params.bpm != bpm
                || shared_state.params.ticks_per_beat != ticks_per_beat
                || shared_state.params.ticks_per_measure != ticks_per_measure
                || shared_state.params.schedule != schedule
                || shared_state.playing_state != *player_state)
        {
            shared_state.params.bpm = bpm;
            shared_state.params.ticks_per_beat = ticks_per_beat;
            shared_state.params.ticks_per_measure = ticks_per_measure;
            shared_state.params.schedule = schedule;
            shared_state.playing_state = player_state.clone();
            shared_state.is_dirty = true;

            debug!("Shared state changed: {:?}", shared_state);
        }

        // On pause/stop: do not pause sink immediately; wait until source reports silence
        if *player_state != PlayerState::Playing
            && self.sink.is_some()
            && let Ok(mut shared_state) = self.shared_state.try_lock()
            && shared_state.is_silent
            && !shared_state.is_dirty
        {
            // Now it's safe to pause without pops
            debug!("Pausing sink. {:?}", shared_state);
            self.sink = None;
            shared_state.playback_tick = 0.0;
        }

        // Playback start: ensure sink runs
        if *player_state == PlayerState::Playing && self.sink.is_none() {
            self.start_sink();
        }

        // Request repaint while tailing out awaiting pause
        *player_state != PlayerState::Playing && self.sink.is_some()
    }

    fn start_sink(&mut self) {
        debug!("Starting sink.");

        let stream = match rodio::OutputStreamBuilder::open_default_stream() {
            Ok(s) => Some(s),
            Err(e) => {
                error!("Failed to init audio: {}", e);
                None
            }
        };

        self.stream = stream;

        if let Some(stream) = &self.stream {
            let sink = rodio::Sink::connect_new(stream.mixer());
            sink.append(MetronomeSource::new(self.shared_state.clone()));
            sink.play();
            self.sink = Some(sink);
        }
    }

    fn compute_schedule(measure: &Measure, ts: &TimeSignature) -> BTreeMap<u32, SoundType> {
        let mut schedule = BTreeMap::new();
        schedule.insert(0, SoundType::Downbeat);

        for t in DEFAULT_GRID.primary_boundaries(ts) {
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
        schedule
    }

    pub fn set_mixer(&mut self, mixer: MixerVolumes) {
        let mut shared_state = self.shared_state.lock().unwrap();
        let m = mixer.clamped();
        if shared_state.params.mixer != m {
            shared_state.params.mixer = m;
            shared_state.is_dirty = true;
        }
    }

    pub fn playback_position(&self) -> Option<(f64, u32)> {
        // Non-blocking try to avoid UI stalls; fall back to None if busy
        if let Ok(shared_state) = self.shared_state.try_lock() {
            Some((shared_state.playback_tick, shared_state.params.ticks_per_measure))
        } else {
            None
        }
    }
}

struct MetronomeSource {
    shared_state: Arc<Mutex<PlaybackState>>,
    local_params: PlaybackParams,
    player_state: PlayerState,

    // Local state: cursor in ticks within current measure
    cursor: f64,
    sample_rate: u32,
    // Polyphonic: multiple short beeps may overlap
    active_beeps: Vec<(f32, SoundType)>,
    samples_processed: usize,
}

impl MetronomeSource {
    // Voice cap used in multiple places
    const MAX_VOICES: usize = 8;

    fn new(shared: Arc<Mutex<PlaybackState>>) -> Self {
        let (local_params, is_playing) = {
            let state = shared.lock().unwrap();
            (state.params.clone(), state.playing_state.clone())
        };

        Self {
            shared_state: shared,
            local_params,
            player_state: is_playing,
            cursor: 0.0,
            sample_rate: device_sample_rate(),
            active_beeps: Vec::with_capacity(Self::MAX_VOICES),
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
            && let Ok(mut shared_state) = self.shared_state.try_lock()
        {
            shared_state.playback_tick = self.cursor;
            shared_state.is_silent = self.active_beeps.is_empty();

            trace!(
                "Updating shared state. playback_tick={:?} is_silent={}",
                shared_state.playback_tick, shared_state.is_silent
            );
        }
    }

    fn update_local_state(&mut self) {
        // Periodic check for updates (~20ms)
        if self.samples_processed.is_multiple_of(1024)
            && let Ok(mut shared_state) = self.shared_state.try_lock()
            && shared_state.is_dirty
        {
            debug!(
                "samples={} cursor={}. Updating local state from {:?}",
                self.samples_processed, self.cursor, shared_state
            );

            if self.player_state != shared_state.playing_state
                && shared_state.playing_state == PlayerState::Playing
            {
                info!("Starting playback.")
            }

            self.player_state = shared_state.playing_state.clone();
            self.local_params = shared_state.params.clone();
            shared_state.is_dirty = false;
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
        if new_cursor >= total_ticks {
            // 1. [old_cursor, total_ticks)
            self.add_triggered_sounds(&mut triggered_sounds, old_cursor.ceil() as u32, total_ticks);

            // Wrap
            new_cursor -= total_ticks;

            // 2. [0.0, new_cursor)
            self.add_triggered_sounds(&mut triggered_sounds, 0, new_cursor);
        } else {
            // [old_cursor, new_cursor)
            self.add_triggered_sounds(&mut triggered_sounds, old_cursor.ceil() as u32, new_cursor);
        }

        self.cursor = new_cursor;
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

    fn synthesize(&mut self) -> f32 {
        // Synthesize a sample by mixing all active voices with envelope
        let dt = 1.0 / (self.sample_rate as f32);
        const DECAY: f32 = 0.025; // ~50 ms
        const ATTACK: f32 = 0.0005; // ~1 ms

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
        if self.player_state == PlayerState::Playing {
            self.enqueue_triggered_sounds()
        } else if self.samples_processed.is_multiple_of(8192) {
            debug!(
                "Metronome state={:?}, samples processed: {}",
                self.player_state, self.samples_processed
            )
        }

        // debug!("cursor={} state={:?}", self.cursor, self.player_state);

        self.samples_processed += 1;
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

fn device_sample_rate() -> u32 {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    let host = rodio::cpal::default_host();
    if let Some(dev) = host.default_output_device()
        && let Ok(cfg) = dev.default_output_config()
    {
        debug!("Using default output device sample rate: {}", cfg.sample_rate().0);
        return cfg.sample_rate().0;
    }
    // fallback
    info!("Failed to get default output device config, falling back to default sample rate");
    48000
}
