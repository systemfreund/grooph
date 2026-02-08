//! MIDI input abstraction for receiving MIDI clock and other messages.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

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

#[cfg(not(target_arch = "wasm32"))]
type ClockInstant = Instant;

#[cfg(target_arch = "wasm32")]
type ClockInstant = f64;

#[inline]
fn clock_now() -> ClockInstant {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Instant::now()
    }

    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() / 1000.0
    }
}

#[inline]
fn clock_interval_seconds(now: ClockInstant, last: ClockInstant) -> f32 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        now.duration_since(last).as_secs_f32()
    }

    #[cfg(target_arch = "wasm32")]
    {
        (now - last) as f32
    }
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
        #[cfg(target_arch = "wasm32")]
        {
            let mut queue = match self.inner.try_lock() {
                Ok(queue) => queue,
                Err(_) => return Vec::new(),
            };
            return queue.drain(..).collect();
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut queue = self.inner.lock().unwrap();
            queue.drain(..).collect()
        }
    }

    /// Clear all queued events
    pub fn clear(&self) {
        #[cfg(target_arch = "wasm32")]
        {
            if let Ok(mut queue) = self.inner.try_lock() {
                queue.clear();
            }
            return;
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut queue = self.inner.lock().unwrap();
            queue.clear();
        }
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
            #[cfg(target_arch = "wasm32")]
            {
                if let Ok(mut queue) = self.inner.try_lock() {
                    queue.push_back(event);
                }
                return;
            }

            #[cfg(not(target_arch = "wasm32"))]
            {
                let mut queue = self.inner.lock().unwrap();
                queue.push_back(event);
            }
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
    pub fn bpm(&self) -> f32 {
        #[cfg(target_arch = "wasm32")]
        {
            return self.inner.try_lock().map(|state| state.bpm).unwrap_or(120.0);
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            self.inner.lock().unwrap().bpm
        }
    }

    /// Check if MIDI clock is running (received start/continue, not stopped)
    pub fn is_running(&self) -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            return self.inner.try_lock().map(|state| state.running).unwrap_or(false);
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            self.inner.lock().unwrap().running
        }
    }

    /// Process a MIDI message and update clock state
    pub fn process_message(&self, message: &[u8]) {
        if message.is_empty() {
            return;
        }

        #[cfg(target_arch = "wasm32")]
        let mut state = match self.inner.try_lock() {
            Ok(state) => state,
            Err(_) => return,
        };

        #[cfg(not(target_arch = "wasm32"))]
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
        #[cfg(target_arch = "wasm32")]
        {
            if let Ok(mut state) = self.inner.try_lock() {
                state.bpm = 120.0;
                state.running = false;
                state.last_clock_time = None;
                state.clock_count = 0;
                state.accumulated_interval = 0.0;
                state.interval_count = 0;
            }
            return;
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut state = self.inner.lock().unwrap();
            state.bpm = 120.0;
            state.running = false;
            state.last_clock_time = None;
            state.clock_count = 0;
            state.accumulated_interval = 0.0;
            state.interval_count = 0;
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native_input::NativeMidiInput;

#[cfg(target_arch = "wasm32")]
pub use web_input::WebMidiInput;

use crate::{MidiNote, MidiVelocity};

#[cfg(not(target_arch = "wasm32"))]
mod native_input {
    use super::*;
    use crate::{Error, Result};
    use midir::{Ignore, MidiInput as MidirInput, MidiInputConnection, MidiInputPort};

    /// Native MIDI input using midir for receiving MIDI clock and note events
    pub struct NativeMidiInput {
        midi_in: Option<MidirInput>,
        connection: Option<MidiInputConnection<()>>,
        ports: Vec<MidiInputPort>,
        clock_state: MidiClockState,
        event_queue: MidiInputEventQueue,
    }

    impl NativeMidiInput {
        /// Create a new native MIDI input
        pub fn new() -> Result<Self> {
            let mut midi_in = MidirInput::new("Chord Joystick Input")?;
            // Don't ignore timing messages - we need them for clock sync
            midi_in.ignore(Ignore::None);
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
        pub fn available_ports(&self) -> Result<Vec<String>> {
            let midi_in = self.midi_in.as_ref().ok_or(Error::NotConnected)?;

            Ok(self.ports.iter().filter_map(|port| midi_in.port_name(port).ok()).collect())
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

            let connection = midi_in.connect(
                &port,
                "chord-joystick-input",
                move |_timestamp, message, _| {
                    clock_state.process_message(message);
                    event_queue.process_message(message);
                },
                (),
            )?;

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
            }
            Ok(())
        }
    }

    impl Drop for NativeMidiInput {
        fn drop(&mut self) { let _ = self.disconnect(); }
    }
}

#[cfg(target_arch = "wasm32")]
mod web_input {
    use super::*;
    use crate::{Error, Result};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{MidiAccess, MidiInput as WebSysMidiInput, MidiInputMap, MidiMessageEvent};

    /// Web MIDI input using Web MIDI API
    pub struct WebMidiInput {
        midi_access: Option<MidiAccess>,
        input: Option<WebSysMidiInput>,
        port_ids: Vec<String>,
        clock_state: MidiClockState,
        event_queue: MidiInputEventQueue,
        message_handler: Option<Closure<dyn FnMut(MidiMessageEvent)>>,
    }

    impl WebMidiInput {
        /// Create a new web MIDI input
        pub async fn new() -> Result<Self> {
            let window =
                web_sys::window().ok_or_else(|| Error::MidiError("No window".to_string()))?;
            let navigator = window.navigator();

            let midi_promise = navigator
                .request_midi_access()
                .map_err(|e| Error::MidiError(format!("Failed to request MIDI access: {:?}", e)))?;

            let midi_access = JsFuture::from(midi_promise)
                .await
                .map_err(|e| Error::MidiError(format!("Failed to get MIDI access: {:?}", e)))?
                .dyn_into::<MidiAccess>()
                .map_err(|e| Error::MidiError(format!("Failed to cast to MidiAccess: {:?}", e)))?;

            let inputs = midi_access.inputs();
            let port_ids = Self::collect_port_ids(&inputs);

            Ok(Self {
                midi_access: Some(midi_access),
                input: None,
                port_ids,
                clock_state: MidiClockState::new(),
                event_queue: MidiInputEventQueue::new(),
                message_handler: None,
            })
        }

        fn get_inputs(&self) -> Result<MidiInputMap> {
            self.midi_access.as_ref().ok_or(Error::NotConnected).map(|access| access.inputs())
        }

        fn collect_port_ids(inputs: &MidiInputMap) -> Vec<String> {
            use std::cell::RefCell;
            use std::rc::Rc;

            let port_ids: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
            let port_ids_clone = port_ids.clone();

            let closure = Closure::wrap(Box::new(
                move |_value: wasm_bindgen::JsValue, key: wasm_bindgen::JsValue| {
                    if let Some(id) = key.as_string() {
                        port_ids_clone.borrow_mut().push(id);
                    }
                },
            )
                as Box<dyn FnMut(wasm_bindgen::JsValue, wasm_bindgen::JsValue)>);

            let _ = inputs.for_each(closure.as_ref().unchecked_ref());

            drop(closure);

            port_ids.borrow().clone()
        }

        /// Get available MIDI input ports
        pub fn available_ports(&mut self) -> Result<Vec<String>> {
            let inputs = self.get_inputs()?;
            self.port_ids = Self::collect_port_ids(&inputs);
            let mut names = Vec::new();

            for id in &self.port_ids {
                if let Some(input) = inputs.get(id) {
                    let name = input.name().unwrap_or_else(|| "Unknown".to_string());
                    names.push(name);
                }
            }

            Ok(names)
        }

        /// Connect to a MIDI input port by index
        pub fn connect(&mut self, port_index: usize) -> Result<()> {
            if self.input.is_some() {
                self.disconnect()?;
            }

            let inputs = self.get_inputs()?;

            let port_id = self.port_ids.get(port_index).ok_or(Error::NoDevicesAvailable)?;

            let input = inputs.get(port_id).ok_or(Error::NoDevicesAvailable)?;

            let _ = input.open();

            let clock_state = self.clock_state.clone();
            let event_queue = self.event_queue.clone();

            let closure = Closure::wrap(Box::new(move |event: MidiMessageEvent| {
                if let Ok(data) = event.data() {
                    if !data.is_empty() {
                        clock_state.process_message(&data);
                        event_queue.process_message(&data);
                    }
                }
            }) as Box<dyn FnMut(MidiMessageEvent)>);

            input.set_onmidimessage(Some(closure.as_ref().unchecked_ref()));
            self.message_handler = Some(closure);
            self.input = Some(input);
            Ok(())
        }

        /// Disconnect from the current MIDI input port
        pub fn disconnect(&mut self) -> Result<()> {
            if let Some(input) = self.input.take() {
                input.set_onmidimessage(None);
                let _ = input.close();
                self.clock_state.reset();
                self.event_queue.clear();
            }
            self.message_handler = None;
            Ok(())
        }

        /// Check if connected to a MIDI input port
        pub fn is_connected(&self) -> bool { self.input.is_some() }

        /// Get the shared clock state for reading BPM
        pub fn clock_state(&self) -> &MidiClockState { &self.clock_state }

        /// Drain queued MIDI note events
        pub fn drain_events(&self) -> Vec<MidiInputEvent> { self.event_queue.drain() }

        /// Refresh the list of available ports
        pub fn refresh_ports(&mut self) -> Result<()> {
            let inputs = self.get_inputs()?;
            self.port_ids = Self::collect_port_ids(&inputs);
            Ok(())
        }
    }

    impl Drop for WebMidiInput {
        fn drop(&mut self) { let _ = self.disconnect(); }
    }
}
