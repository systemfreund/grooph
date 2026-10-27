//! The output device the metronome plays on. Natively through rodio; on the
//! web through a cpal stream of its own, whose `AudioContext` (which rodio
//! keeps hidden) tells when the browser takes the output away.

use crate::{MetronomeSource, PlaybackState};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct Output {
    /// Samples the source rendered, see [`MetronomeSource::rendered`].
    rendered: Arc<AtomicU64>,
    // Declared before the stream, so it is dropped before the stream it
    // plays on.
    _sink: rodio::Sink,
    _stream: rodio::OutputStream,
}

#[cfg(not(target_arch = "wasm32"))]
impl Output {
    pub(crate) fn open(shared: Arc<Mutex<PlaybackState>>) -> Result<Self, String> {
        let stream =
            rodio::OutputStreamBuilder::open_default_stream().map_err(|e| e.to_string())?;
        let sink = rodio::Sink::connect_new(stream.mixer());
        let source = MetronomeSource::new(shared);
        let rendered = source.rendered();
        sink.append(source);
        sink.play();
        Ok(Self { rendered, _sink: sink, _stream: stream })
    }

    /// Whether the device plays: it asked for the first samples.
    pub(crate) fn started(&self) -> bool {
        self.rendered() > 0
    }

    /// Native devices give no sign of their own; the watchdog in
    /// [`crate::Audio`] notices when they stop.
    pub(crate) fn lost(&self) -> bool {
        false
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) struct Output {
    /// Samples the source rendered, see [`MetronomeSource::rendered`].
    rendered: Arc<AtomicU64>,
    stream: cpal::platform::WebAudioStream,
    /// The context was seen running.
    ran: std::cell::Cell<bool>,
}

#[cfg(target_arch = "wasm32")]
impl Output {
    pub(crate) fn open(shared: Arc<Mutex<PlaybackState>>) -> Result<Self, String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let host = cpal::platform::WebAudioHost::new().map_err(|e| e.to_string())?;
        let device = host.default_output_device().ok_or("no output device")?;
        let config = device.default_output_config().map_err(|e| e.to_string())?;
        let channels = usize::from(config.channels()).max(1);
        let mut source = MetronomeSource::with_sample_rate(shared, config.sample_rate().0);
        let rendered = source.rendered();
        let stream = device
            .build_output_stream(
                &config.config(),
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    for frame in data.chunks_mut(channels) {
                        frame.fill(source.next().unwrap_or(0.0));
                    }
                },
                |e| log::error!("Audio output: {e}"),
                None,
            )
            .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self { rendered, stream, ran: std::cell::Cell::new(false) })
    }

    /// Whether the context plays. It starts suspended, and cpal renders its
    /// first buffers before it runs, so the samples alone do not tell.
    pub(crate) fn started(&self) -> bool {
        let ctx = self.stream.audio_context();
        ctx.state() == web_sys::AudioContextState::Running && ctx.current_time() > 0.0
    }

    /// The context stopped running after it ran: the browser suspended or
    /// interrupted it, as iOS does when the Bluetooth device goes away. It
    /// does not come back without a tap, and cpal never resumes it.
    pub(crate) fn lost(&self) -> bool {
        let running = self.stream.audio_context().state() == web_sys::AudioContextState::Running;
        if running {
            self.ran.set(true);
        }
        !running && self.ran.get()
    }
}

impl Output {
    /// Samples rendered so far; it stops moving when the device stops
    /// asking for more.
    pub(crate) fn rendered(&self) -> u64 {
        self.rendered.load(Ordering::Relaxed)
    }
}
