mod schedule;
mod tick_source;
mod voices;

use crate::tick_source::TickSource;
use crate::voices::VoiceMixer;
use grooph_measure::Score;
use grooph_measure::tempo::ScoreTiming;
use log::{debug, error, info, trace};
use rodio::Source;
use rodio::source::Function as RodioFunction;
use schedule::{Schedule, SoundType};
use serde::{Deserialize, Serialize};
use std::fmt::{Debug, Formatter, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Waveform {
    Sine,
    Triangle,
    Square,
    Sawtooth,
}

impl Waveform {
    pub(crate) fn to_rodio(self) -> RodioFunction {
        match self {
            Waveform::Sine => RodioFunction::Sine,
            Waveform::Triangle => RodioFunction::Triangle,
            Waveform::Square => RodioFunction::Square,
            Waveform::Sawtooth => RodioFunction::Sawtooth,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioSettings {
    pub downbeat: f32,
    pub primary: f32,
    pub accent: f32,
    pub beat: f32,
    pub base_frequency: f32,
    pub decay: f32,
    pub waveform: Waveform,
    pub noise_hpf_hz: f32, // High-Pass-Cutoff für Noise (2-6 kHz)
    pub noise_mix: f32,    // Anteil [0..1], der dem Basissignal beigemischt wird
    pub noise_decay: f32,  // unabhängiger Decay nur für Noise-Anteil
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            downbeat: 1.0,
            primary: 0.0,
            accent: 1.0,
            beat: 1.0,
            base_frequency: 440.0,
            decay: 0.042,
            waveform: Waveform::Triangle,
            noise_hpf_hz: 4200.0,
            noise_mix: 0.05,
            noise_decay: 0.017,
        }
    }
}

impl AudioSettings {
    pub fn new(
        downbeat: f32,
        primary: f32,
        accent: f32,
        beat: f32,
        base_frequency: f32,
        decay: f32,
    ) -> Self {
        let result = Self {
            downbeat,
            primary,
            accent,
            beat,
            base_frequency,
            decay,
            waveform: Waveform::Sine,
            noise_hpf_hz: 4000.0,
            noise_mix: 0.0,
            noise_decay: 0.05,
        };
        result.clamped()
    }

    pub fn clamped(mut self) -> Self {
        self.downbeat = self.downbeat.clamp(0.0, 1.0);
        self.primary = self.primary.clamp(0.0, 1.0);
        self.accent = self.accent.clamp(0.0, 1.0);
        self.beat = self.beat.clamp(0.0, 1.0);
        self.base_frequency = self.base_frequency.clamp(20.0, 3000.0);
        self.decay = self.decay.clamp(0.005, 1.0);
        self.noise_hpf_hz = self.noise_hpf_hz.clamp(2000.0, 8000.0);
        self.noise_mix = self.noise_mix.clamp(0.0, 1.0);
        self.noise_decay = self.noise_decay.clamp(0.005, 1.0);
        self
    }
}

pub struct Audio {
    stream: Option<rodio::OutputStream>,
    sink: Option<rodio::Sink>,
    shared_state: Arc<Mutex<PlaybackState>>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PlayerState {
    Playing,
    Stopped,
}

struct PlaybackState {
    params: PlaybackParams,
    is_dirty: bool,
    // Live playback cursor in global ticks across the whole score loop.
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
    timing: ScoreTiming,
    schedule: Schedule,
    audio_settings: AudioSettings,
}

impl Audio {
    pub fn new(bpm: u32) -> Option<Self> {
        let params = PlaybackParams {
            bpm,
            timing: ScoreTiming::default(),
            schedule: Schedule::default(),
            audio_settings: AudioSettings::default(),
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
    pub fn update(&mut self, player_state: &PlayerState, bpm: u32, score: &Score) -> bool {
        let timing = ScoreTiming::from_score(score, bpm);
        let schedule = Schedule::build(score, &timing);

        // Check differences
        if let Ok(mut shared_state) = self.shared_state.try_lock()
            && (shared_state.params.bpm != bpm
                || shared_state.params.timing != timing
                || shared_state.params.schedule != schedule
                || shared_state.playing_state != *player_state)
        {
            shared_state.params.bpm = bpm;
            shared_state.params.timing = timing;
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

    pub fn set_audio_settings(&mut self, settings: AudioSettings) {
        let mut shared_state = self.shared_state.lock().unwrap();
        let s = settings.clamped();
        if shared_state.params.audio_settings != s {
            shared_state.params.audio_settings = s;
            shared_state.is_dirty = true;
        }
    }

    /// Returns `(global_tick, total_loop_ticks)` where `global_tick` is the
    /// current audio cursor over the whole score loop.
    pub fn playback_position(&self) -> Option<(f64, u64)> {
        // Non-blocking try to avoid UI stalls; fall back to None if busy
        if let Ok(shared_state) = self.shared_state.try_lock() {
            Some((shared_state.playback_tick, shared_state.params.timing.total_loop_ticks()))
        } else {
            None
        }
    }
}

/// Audio source that drives the metronome: a small orchestrator wiring a
/// [`TickSource`] (timing) and a [`VoiceMixer`] (synthesis) together with
/// the shared `PlaybackState` for cross-thread communication with the app.
struct MetronomeSource {
    shared_state: Arc<Mutex<PlaybackState>>,
    local_params: PlaybackParams,
    player_state: PlayerState,

    tick_source: TickSource,
    voices: VoiceMixer,
    sample_rate: u32,
    samples_processed: usize,
}

impl MetronomeSource {
    fn new(shared: Arc<Mutex<PlaybackState>>) -> Self {
        let (local_params, is_playing) = {
            let state = shared.lock().unwrap();
            (state.params.clone(), state.playing_state.clone())
        };

        let sample_rate = device_sample_rate();
        Self {
            shared_state: shared,
            local_params,
            player_state: is_playing,
            tick_source: TickSource::new(sample_rate),
            voices: VoiceMixer::new(sample_rate),
            sample_rate,
            samples_processed: 0,
        }
    }

    fn update_shared_state(&mut self) {
        // Try to publish playback cursor to shared state (periodically, to avoid contention)
        if self.samples_processed.is_multiple_of(1024)
            && let Ok(mut shared_state) = self.shared_state.try_lock()
        {
            shared_state.playback_tick = self.tick_source.cursor();
            shared_state.is_silent = self.voices.is_silent();

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
                self.samples_processed,
                self.tick_source.cursor(),
                shared_state
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
}

impl Iterator for MetronomeSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        self.update_local_state();
        self.update_shared_state();

        if self.player_state == PlayerState::Playing {
            // Same allocation pattern as before — fresh small Vec per sample.
            let mut triggered: Vec<SoundType> = Vec::with_capacity(VoiceMixer::MAX_VOICES);
            self.tick_source.advance_one_sample(
                &self.local_params.timing,
                &self.local_params.schedule,
                &mut triggered,
            );
            for sound in triggered {
                self.voices.trigger(sound, &self.local_params.audio_settings);
            }
        } else if self.samples_processed.is_multiple_of(8192) {
            debug!(
                "Metronome state={:?}, samples processed: {}",
                self.player_state, self.samples_processed
            )
        }

        self.samples_processed += 1;
        Some(self.voices.next_sample(self.local_params.audio_settings.noise_mix))
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
