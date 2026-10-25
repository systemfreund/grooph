//! Native input through cpal.

use crate::{MicStatus, OnsetDetector, Shared, now_seconds};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, InputCallbackInfo, SampleFormat, SizedSample, StreamConfig};
use std::sync::Arc;

pub(crate) struct Backend {
    _stream: cpal::Stream,
}

impl Backend {
    pub(crate) fn open(shared: Arc<Shared>) -> Result<Self, String> {
        let chosen = cpal::default_host().default_input_device().ok_or("No microphone found")?;
        let name = chosen.name().unwrap_or_else(|_| "Microphone".into());
        let supported = chosen.default_input_config().map_err(|e| e.to_string())?;
        let config: StreamConfig = supported.config();
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&chosen, &config, shared.clone()),
            SampleFormat::I16 => build::<i16>(&chosen, &config, shared.clone()),
            SampleFormat::U16 => build::<u16>(&chosen, &config, shared.clone()),
            SampleFormat::I32 => build::<i32>(&chosen, &config, shared.clone()),
            other => return Err(format!("Unsupported sample format {other}")),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        shared.set_status(MicStatus::Running(name));
        Ok(Self { _stream: stream })
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels.max(1) as usize;
    let mut detector = OnsetDetector::new(config.sample_rate.0, shared.threshold_db());
    let mut hits = Vec::new();
    let mut peak = 0f32;
    let errors = shared.clone();
    device
        .build_input_stream(
            config,
            move |data: &[T], info: &InputCallbackInfo| {
                let now = now_seconds();
                // The block was captured this long before the callback runs.
                let ts = info.timestamp();
                let delay = ts.callback.duration_since(&ts.capture).unwrap_or_default();
                detector.set_threshold_db(shared.threshold_db());
                let first_channel = data.chunks(channels).map(|frame| frame[0].to_sample::<f32>());
                peak =
                    peak.max(detector.process(first_channel, now - delay.as_secs_f64(), &mut hits));
                shared.push(&mut hits, &mut peak);
            },
            move |err| errors.set_status(MicStatus::Failed(err.to_string())),
            None,
        )
        .map_err(|e| e.to_string())
}
