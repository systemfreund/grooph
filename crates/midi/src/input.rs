//! MIDI input abstraction for receiving MIDI clock and other messages.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use midir::{Ignore, MidiInput as MidirInput, MidiInputConnection, MidiInputPort};

use crate::source::MidiMessageSource;
use crate::{Error, MidiNote, MidiVelocity, Result};

/// MIDI clock message byte
const MIDI_CLOCK: u8 = 0xF8;
/// MIDI start message byte
const MIDI_START: u8 = 0xFA;
/// MIDI stop message byte
const MIDI_STOP: u8 = 0xFC;
/// MIDI continue message byte
const MIDI_CONTINUE: u8 = 0xFB;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) type ClockInstant = Instant;

#[cfg(target_arch = "wasm32")]
pub(crate) type ClockInstant = f64;

#[inline]
pub(crate) fn clock_now() -> ClockInstant {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Instant::now()
    }

    #[cfg(target_arch = "wasm32")]
    {
        web_sys::js_sys::Date::now() / 1000.0
    }
}

#[inline]
pub(crate) fn clock_interval_seconds(now: ClockInstant, last: ClockInstant) -> f32 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        now.duration_since(last).as_secs_f32()
    }

    #[cfg(target_arch = "wasm32")]
    {
        (now - last) as f32
    }
}

#[inline]
pub(crate) fn clock_elapsed_seconds(now: ClockInstant, origin: ClockInstant) -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        now.duration_since(origin).as_secs_f64()
    }

    #[cfg(target_arch = "wasm32")]
    {
        (now - origin) as f64
    }
}

/// MIDI input events for note handling
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MidiInputEvent {
    /// Note On event (channel, note, velocity)
    NoteOn { channel: u8, note: MidiNote, velocity: MidiVelocity, timestamp: f64 },
    /// Note Off event (channel, note, velocity)
    NoteOff { channel: u8, note: MidiNote, velocity: MidiVelocity, timestamp: f64 },
    /// Control Change event (channel, controller, value)
    ControlChange { channel: u8, controller: u8, value: u8, timestamp: f64 },
}

/// Shared queue for MIDI input events
#[derive(Clone)]
pub struct MidiInputEventQueue {
    inner: Arc<Mutex<VecDeque<MidiInputEvent>>>,
}

impl MidiInputEventQueue {
    /// Create a new MIDI input event queue
    pub fn new() -> Self { Self { inner: Arc::new(Mutex::new(VecDeque::new())) } }

    /// Drain all queued events
    pub fn drain(&self) -> Vec<MidiInputEvent> {
        let mut queue = self.inner.lock().unwrap();
        queue.drain(..).collect()
    }

    /// Clear all queued events
    pub fn clear(&self) {
        let mut queue = self.inner.lock().unwrap();
        queue.clear();
    }

    /// Process a MIDI message and queue note events
    pub fn process_message(&self, message: &[u8], timestamp: f64) -> bool {
        if message.len() < 3 {
            return false;
        }

        let status = message[0];
        let data1 = message[1];
        let data2 = message[2];

        let channel = status & 0x0F;

        let event = match status & 0xF0 {
            0x80 => {
                Some(MidiInputEvent::NoteOff { channel, note: data1, velocity: data2, timestamp })
            }
            0x90 => {
                if data2 == 0 {
                    Some(MidiInputEvent::NoteOff {
                        channel,
                        note: data1,
                        velocity: data2,
                        timestamp,
                    })
                } else {
                    Some(MidiInputEvent::NoteOn {
                        channel,
                        note: data1,
                        velocity: data2,
                        timestamp,
                    })
                }
            }
            0xB0 => Some(MidiInputEvent::ControlChange {
                channel,
                controller: data1,
                value: data2,
                timestamp,
            }),
            _ => None,
        };

        if let Some(event) = event {
            let mut queue = self.inner.lock().unwrap();
            queue.push_back(event);
            return true;
        }
        false
    }
}

impl Default for MidiInputEventQueue {
    fn default() -> Self { Self::new() }
}

/// Tuning parameters for [`ClockSync`].
///
/// Values are exposed as named fields rather than embedded as literals so that
/// different MIDI hosts (with looser jitter envelopes or different reporting
/// rates) can be supported without touching the smoothing logic.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClockSyncConfig {
    /// Number of MIDI clock pulses that make up one quarter note (MIDI standard: 24 PPQN).
    pub clocks_per_beat: u32,
    /// BPM value used at startup and after [`ClockSync::reset`].
    pub bpm_init: f32,
    /// Lower bound for an accepted inter-clock interval (seconds). Below this
    /// the pulse is treated as a duplicate/jitter and ignored.
    pub interval_min_s: f32,
    /// Upper bound for an accepted inter-clock interval (seconds). Above this
    /// the pulse is treated as a stall (host paused, system stutter) and ignored.
    pub interval_max_s: f32,
    /// Lower BPM bound for an accepted freshly computed BPM. Used to drop
    /// nonsense values (e.g. host sent intervals that briefly imply 5 BPM).
    pub bpm_min: f32,
    /// Upper BPM bound for an accepted freshly computed BPM.
    pub bpm_max: f32,
    /// Weight of a newly computed BPM in the EMA smoothing
    /// (`bpm = bpm * (1 - smoothing) + new * smoothing`). Range `0.0..=1.0`.
    pub smoothing: f32,
}

impl ClockSyncConfig {
    pub const DEFAULT: Self = Self {
        clocks_per_beat: 24,
        bpm_init: 120.0,
        interval_min_s: 0.005,
        interval_max_s: 1.0,
        bpm_min: 20.0,
        bpm_max: 300.0,
        smoothing: 0.3,
    };
}

impl Default for ClockSyncConfig {
    fn default() -> Self { Self::DEFAULT }
}

/// Pure BPM-smoothing state machine driven by MIDI clock pulses.
///
/// Holds no synchronisation primitives — it is wrapped by [`MidiClockState`]
/// when used across the audio/UI thread boundary. The split makes the
/// smoothing logic unit-testable with synthetic intervals (see
/// [`ClockSync::observe_interval`]).
#[derive(Clone, Debug)]
pub struct ClockSync {
    cfg: ClockSyncConfig,
    bpm: f32,
    last_clock_time: Option<ClockInstant>,
    accumulated_interval: f32,
    interval_count: u32,
    clock_count: u32,
}

impl Default for ClockSync {
    fn default() -> Self { Self::new() }
}

impl ClockSync {
    pub fn new() -> Self { Self::with_config(ClockSyncConfig::DEFAULT) }

    pub fn with_config(cfg: ClockSyncConfig) -> Self {
        Self {
            bpm: cfg.bpm_init,
            cfg,
            last_clock_time: None,
            accumulated_interval: 0.0,
            interval_count: 0,
            clock_count: 0,
        }
    }

    pub fn bpm(&self) -> f32 { self.bpm }

    pub fn config(&self) -> &ClockSyncConfig { &self.cfg }

    /// Reset only the interval accumulators (called on MIDI Start so that the
    /// next pulse establishes a fresh time reference). BPM is left intact.
    pub fn reset_intervals(&mut self) {
        self.last_clock_time = None;
        self.accumulated_interval = 0.0;
        self.interval_count = 0;
        self.clock_count = 0;
    }

    /// Full reset including BPM back to [`ClockSyncConfig::bpm_init`].
    pub fn reset(&mut self) {
        self.bpm = self.cfg.bpm_init;
        self.reset_intervals();
    }

    /// Record a clock pulse arriving at `now`. The first call only seeds the
    /// timestamp; subsequent calls derive an interval and forward it to
    /// [`Self::observe_interval`].
    pub fn on_clock(&mut self, now: ClockInstant) -> Option<f32> {
        let out = if let Some(last) = self.last_clock_time {
            self.observe_interval(clock_interval_seconds(now, last))
        } else {
            None
        };
        self.last_clock_time = Some(now);
        self.clock_count += 1;
        out
    }

    /// Feed a precomputed inter-pulse interval (seconds). Returns
    /// `Some(new_bpm)` if this pulse completed a `clocks_per_beat` batch
    /// and the resulting BPM passed the sanity gates.
    pub fn observe_interval(&mut self, interval_s: f32) -> Option<f32> {
        if interval_s <= self.cfg.interval_min_s || interval_s >= self.cfg.interval_max_s {
            return None;
        }
        self.accumulated_interval += interval_s;
        self.interval_count += 1;
        if self.interval_count < self.cfg.clocks_per_beat {
            return None;
        }
        let avg_interval = self.accumulated_interval / self.interval_count as f32;
        let new_bpm = 60.0 / (avg_interval * self.cfg.clocks_per_beat as f32);
        self.accumulated_interval = 0.0;
        self.interval_count = 0;
        if new_bpm <= self.cfg.bpm_min || new_bpm >= self.cfg.bpm_max {
            return None;
        }
        let a = self.cfg.smoothing;
        self.bpm = self.bpm * (1.0 - a) + new_bpm * a;
        Some(self.bpm)
    }
}

/// Shared state for MIDI clock synchronization.
///
/// Thin `Arc<Mutex<…>>` wrapper around a [`ClockSync`] plus a `running` flag,
/// so the audio thread can publish updates and the UI thread can read BPM.
#[derive(Clone)]
pub struct MidiClockState {
    inner: Arc<Mutex<MidiClockStateInner>>,
}

struct MidiClockStateInner {
    sync: ClockSync,
    /// Whether MIDI clock is currently running (received start/continue).
    running: bool,
}

impl Default for MidiClockState {
    fn default() -> Self { Self::new() }
}

impl MidiClockState {
    /// Create a new MIDI clock state with the default smoothing config.
    pub fn new() -> Self { Self::with_config(ClockSyncConfig::DEFAULT) }

    /// Create a new MIDI clock state with a custom smoothing config.
    pub fn with_config(cfg: ClockSyncConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(MidiClockStateInner {
                sync: ClockSync::with_config(cfg),
                running: false,
            })),
        }
    }

    /// Get the current BPM calculated from MIDI clock.
    pub fn bpm(&self) -> f32 { self.inner.lock().unwrap().sync.bpm() }

    /// Check if MIDI clock is running (received start/continue, not stopped).
    pub fn is_running(&self) -> bool { self.inner.lock().unwrap().running }

    /// Process a MIDI realtime message and update clock state.
    pub fn process_message(&self, message: &[u8]) {
        let Some(&status) = message.first() else {
            return;
        };

        let mut state = self.inner.lock().unwrap();
        match status {
            MIDI_CLOCK => {
                state.sync.on_clock(clock_now());
            }
            MIDI_START => {
                state.running = true;
                state.sync.reset_intervals();
            }
            MIDI_CONTINUE => {
                state.running = true;
            }
            MIDI_STOP => {
                state.running = false;
            }
            _ => {}
        }
    }

    /// Reset the clock state (BPM back to default, accumulators cleared).
    pub fn reset(&self) {
        let mut state = self.inner.lock().unwrap();
        state.sync.reset();
        state.running = false;
    }
}

const MIDI_CLIENT_NAME: &str = "grooph-midi-in";

/// MIDI input using midir for receiving MIDI clock and note events (native and web).
pub struct MidiInput {
    midi_in: Option<MidirInput>,
    connection: Option<MidiInputConnection<()>>,
    ports: Vec<MidiInputPort>,
    clock_state: MidiClockState,
    event_queue: MidiInputEventQueue,
    event_notifier: Option<Arc<dyn Fn() + Send + Sync>>,
    clock_origin: ClockInstant,
}

impl MidiInput {
    fn new_midir_input() -> Result<MidirInput> {
        let mut midi_in = MidirInput::new(MIDI_CLIENT_NAME)?;
        // Don't ignore timing messages - we need them for clock sync
        midi_in.ignore(Ignore::None);
        Ok(midi_in)
    }

    /// Create a new MIDI input
    pub fn new() -> Result<Self> {
        let midi_in = Self::new_midir_input()?;
        let ports = midi_in.ports();

        Ok(Self {
            midi_in: Some(midi_in),
            connection: None,
            ports,
            clock_state: MidiClockState::new(),
            event_queue: MidiInputEventQueue::new(),
            event_notifier: None,
            clock_origin: clock_now(),
        })
    }

    /// Get available MIDI input ports
    pub fn available_ports(&mut self) -> Result<Vec<String>> {
        if let Some(ref midi_in) = self.midi_in {
            let ports = midi_in.ports();
            let names = ports.iter().filter_map(|port| midi_in.port_name(port).ok()).collect();
            self.ports = ports;
            return Ok(names);
        }

        if self.connection.is_some() {
            let midi_in = Self::new_midir_input()?;
            let ports = midi_in.ports();
            let names = ports.iter().filter_map(|port| midi_in.port_name(port).ok()).collect();
            self.ports = ports;
            return Ok(names);
        }

        Err(Error::NotConnected)
    }

    /// Connect to a MIDI input port by index
    pub fn connect(&mut self, port_index: usize) -> Result<()> {
        // Disconnect if already connected
        if self.connection.is_some() {
            self.disconnect()?;
        }

        let midi_in = self.midi_in.take().ok_or(Error::NotConnected)?;

        let port = self.ports.get(port_index).ok_or(Error::NoDevicesAvailable)?.clone();

        let clock_state = self.clock_state.clone();
        let event_queue = self.event_queue.clone();
        let event_notifier = self.event_notifier.clone();
        let clock_origin = self.clock_origin;

        let connection = match midi_in.connect(
            &port,
            "grooph-input",
            move |_timestamp, message, _| {
                clock_state.process_message(message);
                let event_timestamp = clock_elapsed_seconds(clock_now(), clock_origin);
                if event_queue.process_message(message, event_timestamp)
                    && let Some(ref notify) = event_notifier
                {
                    notify();
                }
            },
            (),
        ) {
            Ok(connection) => connection,
            Err(err) => {
                let message = err.to_string();
                let midi_in = err.into_inner();
                self.midi_in = Some(midi_in);
                return Err(Error::ConnectionFailed(message));
            }
        };

        self.connection = Some(connection);
        Ok(())
    }

    /// Disconnect from the current MIDI input port
    pub fn disconnect(&mut self) -> Result<()> {
        if let Some(connection) = self.connection.take() {
            let (midi_in, _) = connection.close();
            self.midi_in = Some(midi_in);
            self.clock_state.reset();
            self.event_queue.clear();
        }
        Ok(())
    }

    /// Check if connected to a MIDI input port
    pub fn is_connected(&self) -> bool { self.connection.is_some() }

    /// Get the shared clock state for reading BPM
    pub fn clock_state(&self) -> &MidiClockState { &self.clock_state }

    /// Drain queued MIDI note events
    pub fn drain_events(&self) -> Vec<MidiInputEvent> { self.event_queue.drain() }

    /// Current time in seconds relative to the MIDI input clock origin.
    pub fn now_seconds(&self) -> f64 { clock_elapsed_seconds(clock_now(), self.clock_origin) }

    /// Set a callback to be invoked when note/control events are enqueued.
    pub fn set_event_notifier(&mut self, notifier: Option<Arc<dyn Fn() + Send + Sync>>) {
        self.event_notifier = notifier;
    }

    /// Refresh the list of available ports
    pub fn refresh_ports(&mut self) -> Result<()> {
        if let Some(ref midi_in) = self.midi_in {
            self.ports = midi_in.ports();
            return Ok(());
        }

        if self.connection.is_some() {
            let midi_in = Self::new_midir_input()?;
            self.ports = midi_in.ports();
            return Ok(());
        }

        Err(Error::NotConnected)
    }

    /// Get the stable port identifier for a port index.
    pub fn port_id(&self, port_index: usize) -> Option<String> {
        self.ports.get(port_index).map(|port| port.id())
    }

    /// Find a port index by its stable identifier.
    pub fn find_port_index_by_id(&self, id: &str) -> Option<usize> {
        self.ports.iter().position(|port| port.id() == id)
    }
}

impl Drop for MidiInput {
    fn drop(&mut self) { let _ = self.disconnect(); }
}

impl MidiMessageSource for MidiInput {
    fn drain_events(&self) -> Vec<MidiInputEvent> { self.event_queue.drain() }

    fn now_seconds(&self) -> f64 { Self::now_seconds(self) }

    fn is_connected(&self) -> bool { Self::is_connected(self) }

    fn clock_state(&self) -> &MidiClockState { Self::clock_state(self) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn interval_for_bpm(bpm: f32, cpb: u32) -> f32 { 60.0 / (bpm * cpb as f32) }

    #[test]
    fn constant_60_bpm_converges() {
        let mut cs = ClockSync::new();
        let interval = interval_for_bpm(60.0, cs.config().clocks_per_beat);
        // First batch shifts BPM by `smoothing` fraction toward 60.
        for _ in 0..cs.config().clocks_per_beat {
            cs.observe_interval(interval);
        }
        let expected_after_one = 120.0 * 0.7 + 60.0 * 0.3;
        assert!(
            (cs.bpm() - expected_after_one).abs() < 0.05,
            "after one beat: {} vs expected {}",
            cs.bpm(),
            expected_after_one
        );
        // After many batches the BPM should converge close to 60.
        for _ in 0..(50 * cs.config().clocks_per_beat) {
            cs.observe_interval(interval);
        }
        assert!((cs.bpm() - 60.0).abs() < 0.5, "did not converge: bpm={}", cs.bpm());
    }

    #[test]
    fn outlier_intervals_are_rejected() {
        let mut cs = ClockSync::new();
        let too_short = cs.config().interval_min_s * 0.5;
        let too_long = cs.config().interval_max_s * 2.0;
        for _ in 0..(2 * cs.config().clocks_per_beat) {
            cs.observe_interval(too_short);
            cs.observe_interval(too_long);
        }
        assert_eq!(cs.bpm(), cs.config().bpm_init, "outliers should not affect BPM");
    }

    #[test]
    fn computed_bpm_above_bpm_max_is_dropped() {
        let mut cs = ClockSync::new();
        // Interval is within the outlier gate but yields a computed BPM well
        // above bpm_max (300) — should leave bpm untouched.
        let interval = 0.006_f32; // ≈ 417 BPM at 24 PPQN
        for _ in 0..cs.config().clocks_per_beat {
            cs.observe_interval(interval);
        }
        assert_eq!(cs.bpm(), 120.0);
    }

    #[test]
    fn partial_batch_does_not_update_bpm() {
        let mut cs = ClockSync::new();
        let interval = interval_for_bpm(60.0, cs.config().clocks_per_beat);
        // Feed cpb - 1 intervals: no full batch yet.
        for _ in 0..(cs.config().clocks_per_beat - 1) {
            assert_eq!(cs.observe_interval(interval), None);
        }
        assert_eq!(cs.bpm(), 120.0);
        // The cpb-th interval completes a batch and returns the new BPM.
        let updated = cs.observe_interval(interval).expect("batch completes");
        assert!((updated - (120.0 * 0.7 + 60.0 * 0.3)).abs() < 0.05);
    }

    #[test]
    fn reset_intervals_keeps_bpm_but_clears_accumulators() {
        let mut cs = ClockSync::new();
        let interval = interval_for_bpm(60.0, cs.config().clocks_per_beat);
        for _ in 0..cs.config().clocks_per_beat {
            cs.observe_interval(interval);
        }
        let after_first_batch = cs.bpm();
        cs.reset_intervals();
        assert_eq!(cs.bpm(), after_first_batch, "BPM survives reset_intervals");
        // After reset, partial batch should not immediately update.
        for _ in 0..(cs.config().clocks_per_beat - 1) {
            cs.observe_interval(interval);
        }
        assert_eq!(cs.bpm(), after_first_batch);
    }

    #[test]
    fn reset_restores_default_bpm() {
        let mut cs = ClockSync::new();
        let interval = interval_for_bpm(60.0, cs.config().clocks_per_beat);
        for _ in 0..cs.config().clocks_per_beat {
            cs.observe_interval(interval);
        }
        assert_ne!(cs.bpm(), 120.0);
        cs.reset();
        assert_eq!(cs.bpm(), 120.0);
    }

    #[test]
    fn midi_clock_state_start_then_stop_flips_running() {
        let state = MidiClockState::new();
        assert!(!state.is_running());
        state.process_message(&[MIDI_START]);
        assert!(state.is_running());
        state.process_message(&[MIDI_STOP]);
        assert!(!state.is_running());
        state.process_message(&[MIDI_CONTINUE]);
        assert!(state.is_running());
    }

    #[test]
    fn midi_clock_state_empty_message_is_noop() {
        let state = MidiClockState::new();
        state.process_message(&[]);
        assert!(!state.is_running());
        assert_eq!(state.bpm(), 120.0);
    }

    /// A minimal in-memory [`MidiMessageSource`] for tests. Demonstrates that
    /// the trait surface is enough to drive an app-layer event loop without
    /// touching `midir`.
    struct MockMidiSource {
        events: RefCell<Vec<MidiInputEvent>>,
        clock_state: MidiClockState,
        now: f64,
        connected: bool,
    }

    impl MockMidiSource {
        fn new() -> Self {
            Self {
                events: RefCell::new(Vec::new()),
                clock_state: MidiClockState::new(),
                now: 0.0,
                connected: true,
            }
        }

        fn enqueue(&self, ev: MidiInputEvent) { self.events.borrow_mut().push(ev); }
    }

    impl MidiMessageSource for MockMidiSource {
        fn drain_events(&self) -> Vec<MidiInputEvent> {
            std::mem::take(&mut *self.events.borrow_mut())
        }
        fn now_seconds(&self) -> f64 { self.now }
        fn is_connected(&self) -> bool { self.connected }
        fn clock_state(&self) -> &MidiClockState { &self.clock_state }
    }

    #[test]
    fn mock_source_can_be_consumed_through_trait() {
        let mock = MockMidiSource::new();
        mock.enqueue(MidiInputEvent::NoteOn {
            channel: 0,
            note: 60,
            velocity: 100,
            timestamp: 0.0,
        });

        fn count_note_ons(src: &dyn MidiMessageSource) -> usize {
            src.drain_events()
                .into_iter()
                .filter(|e| matches!(e, MidiInputEvent::NoteOn { .. }))
                .count()
        }

        assert_eq!(count_note_ons(&mock), 1);
        // Drain leaves the source empty.
        assert_eq!(count_note_ons(&mock), 0);
    }

    #[test]
    fn input_event_queue_routes_note_on_off_and_cc() {
        let q = MidiInputEventQueue::new();
        // Note On
        assert!(q.process_message(&[0x90, 60, 100], 1.0));
        // Note On with velocity 0 = Note Off
        assert!(q.process_message(&[0x90, 60, 0], 2.0));
        // Note Off
        assert!(q.process_message(&[0x80, 60, 64], 3.0));
        // Control Change
        assert!(q.process_message(&[0xB0, 7, 127], 4.0));
        // Unsupported status
        assert!(!q.process_message(&[0xC0, 0, 0], 5.0));
        let events = q.drain();
        assert_eq!(events.len(), 4);
        assert!(matches!(events[0], MidiInputEvent::NoteOn { note: 60, velocity: 100, .. }));
        assert!(matches!(events[1], MidiInputEvent::NoteOff { note: 60, velocity: 0, .. }));
        assert!(matches!(events[2], MidiInputEvent::NoteOff { note: 60, velocity: 64, .. }));
        assert!(matches!(
            events[3],
            MidiInputEvent::ControlChange { controller: 7, value: 127, .. }
        ));
    }
}
