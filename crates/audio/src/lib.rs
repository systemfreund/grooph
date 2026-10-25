mod schedule;
mod sounds;
mod tick_source;
mod trainer;
mod voices;
mod wav;

pub use sounds::{CounterSound, MetronomeSound, RhythmSound};
pub use trainer::{GapClick, GapMode, GapMute, RampEnd, RampUnit, TempoRamp};

use crate::tick_source::{CountInPlan, TickSource};
use crate::trainer::GapDecider;
use crate::voices::VoiceMixer;
use grooph_measure::Score;
use grooph_measure::click::ClickBars;
use grooph_measure::counter::CounterBars;
use grooph_measure::generator::Subdivision;
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::swing::Swing;
use grooph_measure::tempo::ScoreTiming;
use log::{debug, error, info, trace};
use rodio::Source;
use rodio::source::Function as RodioFunction;
use schedule::{Schedule, SoundType};
use serde::{Deserialize, Serialize};
use std::fmt::{Debug, Formatter};
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
    pub noise_hpf_hz: f32, // High-pass cutoff for the noise (2-6 kHz)
    pub noise_mix: f32,    // Share [0..1] of noise mixed into the tone
    pub noise_decay: f32,  // Separate decay for the noise only
    /// Volume of ghost notes. Defaulted so older persisted settings still load.
    #[serde(default = "default_ghost_gain")]
    pub ghost: f32,
    /// Sound of notes, accents and ghost notes.
    #[serde(default)]
    pub rhythm_sound: RhythmSound,
    /// Sound of downbeat, primary beats and count-in.
    #[serde(default)]
    pub metronome_sound: MetronomeSound,
    /// Volume of the count-in bar.
    #[serde(default = "default_count_in_gain")]
    pub count_in: f32,
    /// Volume of the beat groups a click pattern sets to soft.
    #[serde(default = "default_soft_gain")]
    pub soft: f32,
    /// Silence downbeat and primary beats (the count-in still sounds).
    #[serde(default)]
    pub metronome_muted: bool,
    /// Silence notes, accents and ghost notes.
    #[serde(default)]
    pub rhythm_muted: bool,
    /// Sound of the counter rhythm.
    #[serde(default)]
    pub counter_sound: CounterSound,
    /// Volume of the counter rhythm's medium clicks.
    #[serde(default = "default_counter_gain")]
    pub counter: f32,
    /// Silence the counter rhythm.
    #[serde(default)]
    pub counter_muted: bool,
}

fn default_ghost_gain() -> f32 {
    0.15
}

fn default_count_in_gain() -> f32 {
    1.0
}

fn default_soft_gain() -> f32 {
    0.15
}

fn default_counter_gain() -> f32 {
    1.0
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            downbeat: 1.0,
            primary: 0.7,
            accent: 1.0,
            beat: 1.0,
            base_frequency: 440.0,
            decay: 0.042,
            waveform: Waveform::Triangle,
            noise_hpf_hz: 4200.0,
            noise_mix: 0.05,
            noise_decay: 0.017,
            ghost: default_ghost_gain(),
            rhythm_sound: RhythmSound::default(),
            metronome_sound: MetronomeSound::default(),
            count_in: default_count_in_gain(),
            soft: default_soft_gain(),
            metronome_muted: false,
            rhythm_muted: false,
            counter_sound: CounterSound::default(),
            counter: default_counter_gain(),
            counter_muted: false,
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
            ghost: default_ghost_gain(),
            rhythm_sound: RhythmSound::default(),
            metronome_sound: MetronomeSound::default(),
            count_in: default_count_in_gain(),
            soft: default_soft_gain(),
            metronome_muted: false,
            rhythm_muted: false,
            counter_sound: CounterSound::default(),
            counter: default_counter_gain(),
            counter_muted: false,
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
        self.ghost = self.ghost.clamp(0.0, 1.0);
        self.count_in = self.count_in.clamp(0.0, 1.0);
        self.soft = self.soft.clamp(0.0, 1.0);
        self.counter = self.counter.clamp(0.0, 1.0);
        self
    }
}

/// Playback extras that are not part of the click sound design.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlaybackOptions {
    /// Play one bar of clicks before the score starts.
    pub count_in: bool,
    /// Fill free slots of this grid with quiet ghost notes.
    pub ghost_notes: Option<Subdivision>,
    /// Swing feel for notes and ghost notes.
    pub swing: Swing,
    /// Muted bars.
    pub gap: GapClick,
    /// Stepwise tempo change, starting at the score tempo.
    pub ramp: TempoRamp,
    /// What the metronome clicks in each meter.
    pub click: ClickBars,
    /// The counter rhythm played against the click bar, per meter.
    pub counter: CounterBars,
}

/// The bar the audio engine is playing, as counted since the score started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarState {
    /// Bars started since the first downbeat (0 = first bar), across loops.
    pub bar: u64,
    /// The gap click silences this bar.
    pub muted: bool,
}

pub struct Audio {
    stream: Option<rodio::OutputStream>,
    sink: Option<rodio::Sink>,
    shared_state: Arc<Mutex<PlaybackState>>,
    options: PlaybackOptions,
    /// The plan and player state last handed to the audio thread; updates
    /// are compared against these without taking the lock.
    sent_plan: Arc<PlaybackPlan>,
    sent_state: PlayerState,
    /// The inputs of the last built plan and that plan, so a frame with the
    /// same score, tempo and options reuses it instead of building it again.
    built: Option<(PlanInputs, Arc<PlaybackPlan>)>,
    /// Plans the audio thread may still hold. They are dropped here, on the
    /// UI thread, once the audio thread has let go of them.
    retired: Vec<Arc<PlaybackPlan>>,
    /// Last bar read from the audio thread, kept while its lock is busy.
    last_bar: Option<BarState>,
    /// Ticks dropped from the front of the score since the last update, see
    /// [`Audio::drop_leading_ticks`].
    pending_shift: f64,
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
    // Bar being played; `None` before the score starts (stopped, count-in).
    bar: Option<BarState>,
    // Ticks to move the cursor back when the new params are picked up: the
    // score lost that much from its front (endless mode).
    shift: f64,
}

impl Debug for PlaybackState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "PlaybackState {{ is_dirty: {}, playing_state: {:?}, playback_tick: {}}}",
            self.is_dirty, self.playing_state, self.playback_tick
        ))
    }
}

/// What the audio thread plays. Cloning it only bumps reference counts, so
/// the audio thread can take it over without allocating.
#[derive(Debug, Clone)]
struct PlaybackParams {
    plan: Arc<PlaybackPlan>,
    audio_settings: AudioSettings,
    /// Seed for random gap bars.
    seed: u64,
}

/// The part of [`PlaybackParams`] that is built from the score. It is never
/// changed in place: the UI builds a new one outside the lock and swaps it in.
#[derive(Debug, PartialEq)]
struct PlaybackPlan {
    bpm: u32,
    timing: ScoreTiming,
    schedule: Schedule,
    count_in: Option<Arc<CountInPlan>>,
    gap: GapClick,
    ramp: TempoRamp,
}

/// Everything a [`PlaybackPlan`] is built from.
struct PlanInputs {
    bpm: u32,
    options: PlaybackOptions,
    score: Score,
}

impl PlanInputs {
    fn matches(&self, bpm: u32, options: &PlaybackOptions, score: &Score) -> bool {
        self.bpm == bpm && self.options == *options && self.score.same_beats(score)
    }
}

impl PlaybackPlan {
    /// Whether anyone besides the holder of `self` still uses the plan or its
    /// count-in bar (the audio thread keeps the count-in during a count-in).
    fn is_shared(self: &Arc<Self>) -> bool {
        Arc::strong_count(self) > 1
            || self.count_in.as_ref().is_some_and(|c| Arc::strong_count(c) > 1)
    }
}

impl Audio {
    pub fn new(bpm: u32) -> Option<Self> {
        let plan = Arc::new(PlaybackPlan {
            bpm,
            timing: ScoreTiming::default(),
            schedule: Schedule::default(),
            count_in: None,
            gap: GapClick::default(),
            ramp: TempoRamp::default(),
        });
        let params = PlaybackParams {
            plan: Arc::clone(&plan),
            audio_settings: AudioSettings::default(),
            seed: 1,
        };

        let shared_state = Arc::new(Mutex::new(PlaybackState {
            params,
            is_dirty: false,
            playback_tick: 0.0,
            playing_state: PlayerState::Stopped,
            is_silent: true,
            bar: None,
            shift: 0.0,
        }));

        Some(Self {
            stream: None,
            sink: None,
            shared_state,
            options: PlaybackOptions::default(),
            sent_plan: plan,
            sent_state: PlayerState::Stopped,
            built: None,
            retired: Vec::new(),
            last_bar: None,
            pending_shift: 0.0,
        })
    }

    // Returns true if UI should repaint soon (while playing or while waiting for tail-out)
    pub fn update(&mut self, player_state: &PlayerState, bpm: u32, score: &Score) -> bool {
        let plan = self.plan(bpm, score);
        self.publish(player_state, plan);
        self.drop_released_plans();

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
            shared_state.bar = None;
        }

        // Playback start: ensure sink runs
        if *player_state == PlayerState::Playing && self.sink.is_none() {
            self.start_sink();
        }

        // Request repaint while tailing out awaiting pause
        *player_state != PlayerState::Playing && self.sink.is_some()
    }

    /// The plan for `score` at `bpm` with the current options. It is built
    /// only when one of them changed since the last call.
    fn plan(&mut self, bpm: u32, score: &Score) -> Arc<PlaybackPlan> {
        if let Some((inputs, plan)) = &self.built
            && inputs.matches(bpm, &self.options, score)
        {
            return Arc::clone(plan);
        }
        let options = &self.options;
        let timing = ScoreTiming::from_score(score, bpm).with_swing(options.swing);
        let schedule = Schedule::build(score, &timing, options.ghost_notes, &options.click)
            .with_counter(score, &timing, &options.counter);
        let count_in = if options.count_in { count_in_plan(score).map(Arc::new) } else { None };
        let plan =
            PlaybackPlan { bpm, timing, schedule, count_in, gap: options.gap, ramp: options.ramp };
        // Inputs that changed back keep the plan the audio thread already has.
        let plan =
            if *self.sent_plan == plan { Arc::clone(&self.sent_plan) } else { Arc::new(plan) };
        let inputs = PlanInputs { bpm, options: options.clone(), score: score.clone() };
        self.built = Some((inputs, Arc::clone(&plan)));
        plan
    }

    /// Hand `plan` and `player_state` to the audio thread if either changed.
    /// The lock is held only to swap the plan in; the old one is retired and
    /// dropped later on this thread, see [`Audio::drop_released_plans`].
    fn publish(&mut self, player_state: &PlayerState, plan: Arc<PlaybackPlan>) {
        let same_plan = Arc::ptr_eq(&plan, &self.sent_plan) || *self.sent_plan == *plan;
        if self.pending_shift <= 0.0 && same_plan && self.sent_state == *player_state {
            return;
        }
        let plan = if same_plan { Arc::clone(&self.sent_plan) } else { plan };

        // A shift must reach the audio thread with the shorter score it was
        // made for, so this update waits for the lock rather than skip it.
        let state = Arc::clone(&self.shared_state);
        let shared =
            if self.pending_shift > 0.0 { state.lock().ok() } else { state.try_lock().ok() };
        let Some(mut shared_state) = shared else {
            return;
        };
        let old = std::mem::replace(&mut shared_state.params.plan, Arc::clone(&plan));
        shared_state.playing_state = player_state.clone();
        shared_state.shift += self.pending_shift;
        // The published cursor moves with the score right away, so the
        // app never reads the old position against the new score.
        shared_state.playback_tick = (shared_state.playback_tick - self.pending_shift).max(0.0);
        shared_state.is_dirty = true;
        debug!("Shared state changed: {:?}", shared_state);
        drop(shared_state);

        self.pending_shift = 0.0;
        self.sent_plan = plan;
        self.sent_state = player_state.clone();
        if !Arc::ptr_eq(&old, &self.sent_plan) {
            self.retired.push(old);
        }
    }

    /// Drop the retired plans the audio thread no longer holds, so their
    /// memory is freed here and never on the audio thread.
    fn drop_released_plans(&mut self) {
        self.retired.retain(PlaybackPlan::is_shared);
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

    /// Seed for randomly muted bars (gap click in random mode).
    pub fn set_seed(&mut self, seed: u64) {
        if let Ok(mut shared_state) = self.shared_state.lock() {
            shared_state.params.seed = seed;
        }
    }

    pub fn set_playback_options(&mut self, options: PlaybackOptions) {
        self.options = options;
    }

    /// The score lost `ticks` from its front (endless mode dropped a played
    /// measure): the next [`Audio::update`] moves the cursor back by as much,
    /// together with the shorter score, so playback goes on without a skip.
    pub fn drop_leading_ticks(&mut self, ticks: u64) {
        self.pending_shift += ticks as f64;
    }

    /// Whether an output stream is running, i.e. `playback_position` advances.
    pub fn is_running(&self) -> bool {
        self.sink.is_some()
    }

    /// Bar the engine is playing; `None` while stopped or counting in. While
    /// the audio thread holds its lock, the last known bar is returned.
    pub fn bar_state(&mut self) -> Option<BarState> {
        if !self.is_running() {
            self.last_bar = None;
        } else if let Ok(shared_state) = self.shared_state.try_lock() {
            self.last_bar = shared_state.bar;
        }
        self.last_bar
    }

    /// Returns `(global_tick, total_loop_ticks)` where `global_tick` is the
    /// current audio cursor over the whole score loop. It is negative while
    /// the count-in bar plays and reaches 0 on the score's first downbeat.
    pub fn playback_position(&self) -> Option<(f64, u64)> {
        // Non-blocking try to avoid UI stalls; fall back to None if busy
        if let Ok(shared_state) = self.shared_state.try_lock() {
            Some((shared_state.playback_tick, shared_state.params.plan.timing.total_loop_ticks()))
        } else {
            None
        }
    }
}

/// Count-in bar for `score`: the first measure's length with a click on the
/// downbeat and each primary beat, and a softer one on every other beat unit
/// (so 7/8 clicks all seven eighths, the group starts louder).
fn count_in_plan(score: &Score) -> Option<CountInPlan> {
    let ts = score.measures.first()?.time_signature();
    let ticks = DEFAULT_GRID.ticks_per_measure(&ts);
    let primaries = DEFAULT_GRID.primary_boundaries(&ts);
    let unit = DEFAULT_GRID.ticks_per_beat(&ts).max(1);
    let clicks = (0..ticks)
        .step_by(unit as usize)
        .map(|t| {
            let start = t == 0 || primaries.contains(&t);
            (t, if start { SoundType::CountIn } else { SoundType::CountInSoft })
        })
        .collect();
    Some(CountInPlan { ticks, clicks })
}

/// Audio source that drives the metronome: a small orchestrator wiring a
/// [`TickSource`] (timing) and a [`VoiceMixer`] (synthesis) together with
/// the shared `PlaybackState` for cross-thread communication with the app.
struct MetronomeSource {
    shared_state: Arc<Mutex<PlaybackState>>,
    local_params: PlaybackParams,
    player_state: PlayerState,

    tick_source: TickSource,
    gap: GapDecider,
    voices: VoiceMixer,
    /// Sounds triggered by the current sample, reused so that rendering a
    /// sample does not allocate.
    triggered: Vec<SoundType>,
    sample_rate: u32,
    samples_processed: usize,
}

impl MetronomeSource {
    fn new(shared: Arc<Mutex<PlaybackState>>) -> Self {
        Self::with_sample_rate(shared, device_sample_rate())
    }

    fn with_sample_rate(shared: Arc<Mutex<PlaybackState>>, sample_rate: u32) -> Self {
        let (local_params, is_playing) = {
            let state = shared.lock().unwrap();
            (state.params.clone(), state.playing_state.clone())
        };

        let seed = local_params.seed;
        let mut tick_source = TickSource::new(sample_rate);
        if is_playing == PlayerState::Playing {
            tick_source.restart(local_params.plan.count_in.as_ref());
        }
        Self {
            shared_state: shared,
            local_params,
            player_state: is_playing,
            tick_source,
            gap: GapDecider::new(seed),
            voices: VoiceMixer::new(sample_rate),
            // Room for many more sounds per sample than can sound at once.
            triggered: Vec::with_capacity(4 * VoiceMixer::MAX_VOICES),
            sample_rate,
            samples_processed: 0,
        }
    }

    fn update_shared_state(&mut self) {
        // Try to publish playback cursor to shared state (periodically, to avoid contention)
        if !self.samples_processed.is_multiple_of(1024) {
            return;
        }
        let bar = self.current_bar();
        if let Ok(mut shared_state) = self.shared_state.try_lock()
            // Not yet in step with new params: a pending shift already moved
            // the published cursor.
            && !shared_state.is_dirty
        {
            shared_state.playback_tick = self.tick_source.cursor();
            shared_state.is_silent = self.voices.is_silent();
            shared_state.bar = bar;

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

            let starting = self.player_state != shared_state.playing_state
                && shared_state.playing_state == PlayerState::Playing;

            self.player_state = shared_state.playing_state.clone();
            // Only reference counts change: the UI keeps the old plan alive
            // and frees it, see `Audio::drop_released_plans`.
            self.local_params = shared_state.params.clone();
            shared_state.is_dirty = false;
            let shift = std::mem::take(&mut shared_state.shift);
            if !starting && shift > 0.0 {
                self.tick_source.shift_back(shift);
            }

            if starting {
                info!("Starting playback.");
                self.tick_source.restart(self.local_params.plan.count_in.as_ref());
                self.gap.reset(self.local_params.seed ^ self.samples_processed as u64);
            }
        }
    }
}

impl MetronomeSource {
    /// Bar being played, with the gap click decision for it.
    fn current_bar(&mut self) -> Option<BarState> {
        if self.player_state != PlayerState::Playing || self.tick_source.in_count_in() {
            return None;
        }
        let bar = self.tick_source.bars();
        Some(BarState { bar, muted: self.gap.muted(&self.local_params.plan.gap, bar) })
    }

    /// Ratio of the ramp tempo of the current bar to the score tempo.
    fn tempo_scale(&self) -> f64 {
        let params = &self.local_params.plan;
        if !params.ramp.enabled || params.bpm == 0 || self.tick_source.in_count_in() {
            return 1.0;
        }
        let bars = params.timing.measure_count();
        params.ramp.bpm_for_bar(params.bpm, self.tick_source.bars(), bars) as f64
            / params.bpm as f64
    }
}

impl Iterator for MetronomeSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        self.update_local_state();
        self.update_shared_state();

        if self.player_state == PlayerState::Playing {
            self.triggered.clear();
            let tempo_scale = self.tempo_scale();
            self.tick_source.advance_one_sample(
                &self.local_params.plan.timing,
                &self.local_params.plan.schedule,
                tempo_scale,
                &mut self.triggered,
            );
            let muted = self.current_bar().is_some_and(|b| b.muted);
            let mute = self.local_params.plan.gap.mute;
            for &sound in &self.triggered {
                let settings = &self.local_params.audio_settings;
                if (muted && sound.muted_by(mute)) || sound.profile(settings).gain <= 0.0 {
                    continue;
                }
                self.voices.trigger(sound.patch(&self.local_params.audio_settings));
            }
        } else if self.samples_processed.is_multiple_of(8192) {
            debug!(
                "Metronome state={:?}, samples processed: {}",
                self.player_state, self.samples_processed
            )
        }

        self.samples_processed += 1;
        Some(self.voices.next_sample())
    }
}

impl Source for MetronomeSource {
    fn current_span_len(&self) -> Option<usize> {
        None // Infinite
    }

    fn channels(&self) -> u16 {
        1
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use grooph_measure::{Measure, TimeSignature};
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    fn count_in_of(ts: TimeSignature) -> Vec<SoundType> {
        let score = Score { measures: vec![Measure::new(ts)] };
        count_in_plan(&score).unwrap().clicks.into_iter().map(|(_, s)| s).collect()
    }

    /// Counts the allocations and deallocations of the current thread while
    /// it is armed, see [`count_allocations`].
    struct CountingAllocator;

    thread_local! {
        static ARMED: Cell<bool> = const { Cell::new(false) };
        static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    }

    fn count_if_armed() {
        let _ = ARMED.try_with(|armed| {
            if armed.get() {
                let _ = ALLOCATIONS.try_with(|n| n.set(n.get() + 1));
            }
        });
    }

    // SAFETY: every method forwards to `System` with the caller's arguments
    // unchanged; counting only touches const thread-locals through `try_with`,
    // so it neither allocates nor reenters the allocator.
    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            count_if_armed();
            // SAFETY: the caller's `layout` is passed on as is.
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            count_if_armed();
            // SAFETY: `ptr` came from `System` with this `layout`, see `alloc`.
            unsafe { System.dealloc(ptr, layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            count_if_armed();
            // SAFETY: as for `dealloc`; `new_size` is the caller's, unchanged.
            unsafe { System.realloc(ptr, layout, new_size) }
        }
    }

    #[global_allocator]
    static ALLOCATOR: CountingAllocator = CountingAllocator;

    /// Allocations and deallocations `f` makes on this thread.
    fn count_allocations(f: impl FnOnce()) -> usize {
        ALLOCATIONS.with(|n| n.set(0));
        ARMED.with(|a| a.set(true));
        f();
        ARMED.with(|a| a.set(false));
        ALLOCATIONS.with(|n| n.get())
    }

    fn score_of(ts: TimeSignature, bars: usize) -> Score {
        Score { measures: (0..bars).map(|_| Measure::new(ts)).collect() }
    }

    /// Everything audible is set to zero, so no voice is started: starting a
    /// voice builds its signal, which is not part of this check.
    fn silent_settings() -> AudioSettings {
        AudioSettings {
            downbeat: 0.0,
            primary: 0.0,
            accent: 0.0,
            beat: 0.0,
            ghost: 0.0,
            count_in: 0.0,
            soft: 0.0,
            counter: 0.0,
            ..AudioSettings::default()
        }
    }

    fn plan_for(audio: &Audio, bpm: u32, score: &Score) -> Arc<PlaybackPlan> {
        let timing = ScoreTiming::from_score(score, bpm);
        let schedule = Schedule::build(score, &timing, None, &audio.options.click);
        let count_in = count_in_plan(score).map(Arc::new);
        Arc::new(PlaybackPlan {
            bpm,
            timing,
            schedule,
            count_in,
            gap: GapClick::default(),
            ramp: TempoRamp::default(),
        })
    }

    #[test]
    fn rendering_does_not_allocate() {
        // The counter sees allocations at all.
        let n = count_allocations(|| drop(std::hint::black_box(Vec::<u8>::with_capacity(8))));
        assert_eq!(n, 2);

        let mut audio = Audio::new(120).unwrap();
        audio.set_audio_settings(silent_settings());
        let score = score_of(TimeSignature::FOUR_FOUR, 2);
        audio.publish(&PlayerState::Playing, plan_for(&audio, 120, &score));
        let mut source = MetronomeSource::with_sample_rate(Arc::clone(&audio.shared_state), 48_000);

        // Pick up the plan, play through the count-in and loop the score.
        let n = count_allocations(|| {
            for _ in 0..(48_000 * 7) {
                source.next();
            }
        });
        assert_eq!(n, 0, "allocations while playing");

        // A new plan while playing: the audio thread takes it over and lets
        // go of the old one without allocating or freeing.
        audio.publish(
            &PlayerState::Playing,
            plan_for(&audio, 90, &score_of(TimeSignature::SIX_EIGHT, 3)),
        );
        audio.drop_leading_ticks(1);
        audio.publish(&PlayerState::Playing, plan_for(&audio, 100, &score));
        let n = count_allocations(|| {
            for _ in 0..(48_000 * 2) {
                source.next();
            }
        });
        assert_eq!(n, 0, "allocations after a plan change");
        assert_eq!(source.local_params.plan.bpm, 100);

        // Stopping and starting again restarts the count-in.
        audio.publish(&PlayerState::Stopped, plan_for(&audio, 100, &score));
        let n = count_allocations(|| {
            for _ in 0..4096 {
                source.next();
            }
        });
        assert_eq!(n, 0, "allocations when stopping");
        audio.publish(&PlayerState::Playing, plan_for(&audio, 100, &score));
        let n = count_allocations(|| {
            for _ in 0..(48_000 * 3) {
                source.next();
            }
        });
        assert_eq!(n, 0, "allocations when starting again");
    }

    #[test]
    fn retired_plans_are_freed_once_the_audio_thread_lets_go() {
        let mut audio = Audio::new(120).unwrap();
        let score = score_of(TimeSignature::FOUR_FOUR, 1);
        audio.publish(&PlayerState::Playing, plan_for(&audio, 120, &score));
        let mut source = MetronomeSource::with_sample_rate(Arc::clone(&audio.shared_state), 48_000);
        source.next();
        let first = Arc::clone(&source.local_params.plan);

        audio.publish(&PlayerState::Playing, plan_for(&audio, 90, &score));
        audio.drop_released_plans();
        // The source has not picked up the new plan yet.
        assert_eq!(audio.retired.len(), 1);
        assert!(Arc::ptr_eq(&audio.retired[0], &first));

        for _ in 0..2048 {
            source.next();
        }
        assert_eq!(source.local_params.plan.bpm, 90);
        drop(first);
        // The count-in that started with the first plan is still playing.
        audio.drop_released_plans();
        assert_eq!(audio.retired.len(), 1);

        for _ in 0..(48_000 * 4) {
            source.next();
        }
        assert!(!source.tick_source.in_count_in());
        audio.drop_released_plans();
        assert!(audio.retired.is_empty());
    }

    #[test]
    fn unchanged_plan_is_not_sent_again() {
        let mut audio = Audio::new(120).unwrap();
        let score = score_of(TimeSignature::FOUR_FOUR, 1);
        audio.publish(&PlayerState::Playing, plan_for(&audio, 120, &score));
        audio.shared_state.lock().unwrap().is_dirty = false;
        audio.publish(&PlayerState::Playing, plan_for(&audio, 120, &score));
        assert!(!audio.shared_state.lock().unwrap().is_dirty);
        audio.publish(&PlayerState::Stopped, plan_for(&audio, 120, &score));
        assert!(audio.shared_state.lock().unwrap().is_dirty);
        // Only the plan from `Audio::new` was replaced.
        assert_eq!(audio.retired.len(), 1);
    }

    #[test]
    fn plan_is_built_only_when_its_inputs_change() {
        let mut audio = Audio::new(120).unwrap();
        let mut score = score_of(TimeSignature::FOUR_FOUR, 2);
        let built = |audio: &Audio| Arc::clone(&audio.built.as_ref().unwrap().1);

        audio.update(&PlayerState::Stopped, 120, &score);
        let first = built(&audio);
        assert!(Arc::ptr_eq(&first, &audio.sent_plan));
        audio.shared_state.lock().unwrap().is_dirty = false;
        // Same inputs, also from a fresh copy of the score: nothing is built
        // or sent.
        audio.update(&PlayerState::Stopped, 120, &score.clone());
        assert!(Arc::ptr_eq(&built(&audio), &first));
        assert!(!audio.shared_state.lock().unwrap().is_dirty);

        // A new tempo builds and sends a new plan.
        audio.update(&PlayerState::Stopped, 90, &score);
        let slower = built(&audio);
        assert!(!Arc::ptr_eq(&slower, &first));
        assert_eq!(audio.sent_plan.bpm, 90);

        // So does an edited beat.
        score.measures[1].toggle_beat_kind(0);
        audio.update(&PlayerState::Stopped, 90, &score);
        let edited = built(&audio);
        assert!(!Arc::ptr_eq(&edited, &slower));
        assert!(Arc::ptr_eq(&edited, &audio.sent_plan));

        // And a changed option.
        audio
            .set_playback_options(PlaybackOptions { count_in: true, ..PlaybackOptions::default() });
        audio.update(&PlayerState::Stopped, 90, &score);
        assert!(!Arc::ptr_eq(&built(&audio), &edited));
        assert!(audio.sent_plan.count_in.is_some());
    }

    #[test]
    fn inputs_changed_back_reuse_the_sent_plan() {
        let mut audio = Audio::new(120).unwrap();
        let score = score_of(TimeSignature::FOUR_FOUR, 1);
        audio.update(&PlayerState::Stopped, 120, &score);
        let sent = Arc::clone(&audio.sent_plan);
        audio.shared_state.lock().unwrap().is_dirty = false;

        // A new tempo is built while the audio thread holds the lock, so it
        // is not sent; then the tempo goes back.
        let state = Arc::clone(&audio.shared_state);
        let guard = state.lock().unwrap();
        audio.update(&PlayerState::Stopped, 90, &score);
        drop(guard);
        assert!(Arc::ptr_eq(&audio.sent_plan, &sent));
        audio.update(&PlayerState::Stopped, 120, &score);
        assert!(Arc::ptr_eq(&audio.built.as_ref().unwrap().1, &sent));
        assert!(!audio.shared_state.lock().unwrap().is_dirty);
    }

    #[test]
    fn count_in_clicks_every_beat_unit() {
        use SoundType::{CountIn as C, CountInSoft as S};
        // 7/8 (2+3+2): all seven eighths, the group starts louder.
        assert_eq!(count_in_of(TimeSignature::new(7, 8)), [C, S, C, S, S, C, S]);
        assert_eq!(count_in_of(TimeSignature::SIX_EIGHT), [C, S, S, C, S, S]);
        assert_eq!(count_in_of(TimeSignature::FOUR_FOUR), [C, C, C, C]);
    }
}
