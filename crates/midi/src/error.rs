pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("No MIDI output devices available")]
    NoDevicesAvailable,

    #[error("Failed to connect to MIDI output: {0}")]
    ConnectionFailed(String),

    #[error("Failed to send MIDI message: {0}")]
    SendFailed(String),

    #[error("MIDI output not connected")]
    NotConnected,

    #[error("MIDI error: {0}")]
    MidiError(String),
}

#[cfg(not(target_arch = "wasm32"))]
impl From<midir::InitError> for Error {
    fn from(e: midir::InitError) -> Self {
        Error::MidiError(e.to_string())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<midir::SendError> for Error {
    fn from(e: midir::SendError) -> Self {
        Error::SendFailed(e.to_string())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<midir::ConnectError<midir::MidiOutput>> for Error {
    fn from(e: midir::ConnectError<midir::MidiOutput>) -> Self {
        Error::ConnectionFailed(e.to_string())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<midir::ConnectError<midir::MidiInput>> for Error {
    fn from(e: midir::ConnectError<midir::MidiInput>) -> Self {
        Error::ConnectionFailed(e.to_string())
    }
}
