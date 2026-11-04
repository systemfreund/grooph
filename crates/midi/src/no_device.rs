//! Natively without the `device` feature: no MIDI backend. These stand in
//! for the few midir types `MidiInput` uses, so the crate builds without
//! midir, which needs the audio system's headers natively (ALSA on Linux).
//! [`MidiInput::new`](crate::MidiInput::new) fails, so none of the other
//! methods ever runs: the types have no values.

use crate::{Error, Result};

pub(crate) enum MidirInput {}

pub(crate) enum Ignore {
    None,
}

#[derive(Clone)]
pub(crate) enum MidiInputPort {}

pub(crate) struct MidiInputConnection<T>(Never, std::marker::PhantomData<T>);

pub(crate) enum ConnectError {}

enum Never {}

impl MidirInput {
    pub(crate) fn new(_client_name: &str) -> Result<Self> {
        Err(Error::MidiError("built without a MIDI backend (feature `device`)".into()))
    }

    pub(crate) fn ignore(&mut self, _flags: Ignore) {
        match *self {}
    }

    pub(crate) fn ports(&self) -> Vec<MidiInputPort> {
        match *self {}
    }

    pub(crate) fn port_name(&self, port: &MidiInputPort) -> Result<String> {
        match *port {}
    }

    pub(crate) fn connect<F, T>(
        self,
        port: &MidiInputPort,
        _port_name: &str,
        _callback: F,
        _data: T,
    ) -> std::result::Result<MidiInputConnection<T>, ConnectError>
    where
        F: FnMut(u64, &[u8], &mut T) + Send + 'static,
    {
        match *port {}
    }
}

impl MidiInputPort {
    pub(crate) fn id(&self) -> String {
        match *self {}
    }
}

impl<T> MidiInputConnection<T> {
    pub(crate) fn close(self) -> (MidirInput, T) {
        match self.0 {}
    }
}

impl ConnectError {
    pub(crate) fn into_inner(self) -> MidirInput {
        match self {}
    }
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {}
    }
}
