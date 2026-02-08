//! MIDI input abstraction for receiving MIDI clock and other messages.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use midir::{Ignore, MidiInput as MidirInput, MidiInputConnection, MidiInputPort};

use crate::{Error, MidiNote, MidiVelocity, Result};

/// MIDI clock message byte
const MIDI_CLOCK: u8 = 0xF8;
/// MIDI start message byte
const MIDI_START: u8 = 0xFA;
/// MIDI stop message byte
const MIDI_STOP: u8 = 0xFC;
/// MIDI continue message byte
const MIDI_CONTINUE: u8 = 0xFB;

/// Number of clock messages per beat (MIDI standard: 24 PPQN)
const CLOCKS_PER_BEAT: u32 = 24;

type ClockInstant = Instant;

#[inline]
fn clock_now() -> ClockInstant {
    Instant::now()
}

#[inline]
fn clock_interval_seconds(now: ClockInstant, last: ClockInstant) -> f32 {
    now.duration_since(last).as_secs_f32()
}

/// MIDI input events for note handling
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiInputEvent {
    /// Note On event (channel, note, velocity)
    NoteOn { channel: u8, note: MidiNote, velocity: MidiVelocity },
    /// Note Off event (channel, note, velocity)
    NoteOff { channel: u8, note: MidiNote, velocity: MidiVelocity },
    /// Control Change event (channel, controller, value)
    ControlChange { channel: u8, controller: u8, value: u8 },
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
    pub fn process_message(&self, message: &[u8]) {
        if message.len() < 3 {
            return;
        }

        let status = message[0];
        let data1 = message[1];
        let data2 = message[2];

        let channel = status & 0x0F;

        let event = match status & 0xF0 {
            0x80 => Some(MidiInputEvent::NoteOff { channel, note: data1, velocity: data2 }),
            0x90 => {
                if data2 == 0 {
                    Some(MidiInputEvent::NoteOff { channel, note: data1, velocity: data2 })
                } else {
                    Some(MidiInputEvent::NoteOn { channel, note: data1, velocity: data2 })
                }
            }
            0xB0 => {
                Some(MidiInputEvent::ControlChange { channel, controller: data1, value: data2 })
            }
            _ => None,
        };

        if let Some(event) = event {
            let mut queue = self.inner.lock().unwrap();
            queue.push_back(event);
        }
    }
}

impl Default for MidiInputEventQueue {
    fn default() -> Self { Self::new() }
}

/// Shared state for MIDI clock synchronization
#[derive(Clone)]
pub struct MidiClockState {
    inner: Arc<Mutex<MidiClockStateInner>>,
}

struct MidiClockStateInner {
    /// Calculated BPM from MIDI clock
    bpm: f32,
    /// Whether MIDI clock is currently running (received start/continue)
    running: bool,
    /// Last clock message timestamp
    last_clock_time: Option<ClockInstant>,
    /// Clock message counter for averaging
    clock_count: u32,
    /// Accumulated time between clocks for BPM calculation
    accumulated_interval: f32,
    /// Number of intervals accumulated
    interval_count: u32,
}

impl Default for MidiClockState {
    fn default() -> Self { Self::new() }
}

impl MidiClockState {
    /// Create a new MIDI clock state
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(MidiClockStateInner {
                bpm: 120.0,
                running: false,
                last_clock_time: None,
                clock_count: 0,
                accumulated_interval: 0.0,
                interval_count: 0,
            })),
        }
    }

    /// Get the current BPM calculated from MIDI clock
    pub fn bpm(&self) -> f32 { self.inner.lock().unwrap().bpm }

    /// Check if MIDI clock is running (received start/continue, not stopped)
    pub fn is_running(&self) -> bool { self.inner.lock().unwrap().running }

    /// Process a MIDI message and update clock state
    pub fn process_message(&self, message: &[u8]) {
        if message.is_empty() {
            return;
        }

        let mut state = self.inner.lock().unwrap();

        match message[0] {
            MIDI_CLOCK => {
                let now = clock_now();

                if let Some(last_time) = state.last_clock_time {
                    let interval = clock_interval_seconds(now, last_time);

                    // Accumulate intervals for averaging (ignore outliers)
                    if interval > 0.005 && interval < 1.0 {
                        state.accumulated_interval += interval;
                        state.interval_count += 1;

                        // Calculate BPM every 24 clocks (one beat)
                        if state.interval_count >= CLOCKS_PER_BEAT {
                            let avg_interval =
                                state.accumulated_interval / state.interval_count as f32;
                            // BPM = 60 / (interval * clocks_per_beat)
                            let new_bpm = 60.0 / (avg_interval * CLOCKS_PER_BEAT as f32);

                            // Smooth the BPM value to avoid jitter
                            if new_bpm > 20.0 && new_bpm < 300.0 {
                                state.bpm = state.bpm * 0.7 + new_bpm * 0.3;
                            }

                            // Reset accumulators
                            state.accumulated_interval = 0.0;
                            state.interval_count = 0;
                        }
                    }
                }

                state.last_clock_time = Some(now);
                state.clock_count += 1;
            }
            MIDI_START => {
                state.running = true;
                state.clock_count = 0;
                state.last_clock_time = None;
                state.accumulated_interval = 0.0;
                state.interval_count = 0;
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

    /// Reset the clock state
    pub fn reset(&self) {
        let mut state = self.inner.lock().unwrap();
        state.bpm = 120.0;
        state.running = false;
        state.last_clock_time = None;
        state.clock_count = 0;
        state.accumulated_interval = 0.0;
        state.interval_count = 0;
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

        let connection = match midi_in.connect(
            &port,
            "grooph-input",
            move |_timestamp, message, _| {
                clock_state.process_message(message);
                event_queue.process_message(message);
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
