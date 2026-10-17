//! Web input: `getUserMedia` into an AudioWorklet that forwards sample
//! blocks to the page, where the detector runs.

use crate::{MicStatus, OnsetDetector, Shared, is_builtin_ios_mic, now_seconds};
use js_sys::{Array, Float32Array, Object, Reflect};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    AudioContext, AudioContextState, AudioNode, AudioWorkletNode, AudioWorkletNodeOptions, Blob,
    BlobPropertyBag, MediaDeviceInfo, MediaDeviceKind, MediaDevices, MediaStream,
    MediaStreamConstraints, MediaStreamTrack, MessageEvent, Url,
};

const PROCESSOR: &str = "grooph-mic";

/// Gathers the 128-frame render quanta into blocks of 1024 frames, so the
/// page gets a few dozen messages a second, each with its first frame.
const WORKLET: &str = r#"
class GroophMic extends AudioWorkletProcessor {
  constructor() {
    super();
    this.block = new Float32Array(1024);
    this.filled = 0;
    this.start = 0;
  }
  process(inputs) {
    const ch = inputs[0] && inputs[0][0];
    if (!ch) return true;
    if (this.filled + ch.length > this.block.length) this.flush();
    if (this.filled === 0) this.start = currentFrame;
    this.block.set(ch, this.filled);
    this.filled += ch.length;
    if (this.filled === this.block.length) this.flush();
    return true;
  }
  flush() {
    const samples = this.block.slice(0, this.filled);
    this.port.postMessage({ frame: this.start, samples }, [samples.buffer]);
    this.filled = 0;
  }
}
registerProcessor('grooph-mic', GroophMic);
"#;

/// Gesture events that may resume a suspended audio context.
const GESTURES: [&str; 3] = ["pointerdown", "touchend", "keydown"];

type MessageHandler = Closure<dyn FnMut(MessageEvent)>;

#[derive(Default)]
struct Inner {
    closed: Cell<bool>,
    ctx: RefCell<Option<AudioContext>>,
    stream: RefCell<Option<MediaStream>>,
    nodes: RefCell<Vec<AudioNode>>,
    on_message: RefCell<Option<MessageHandler>>,
    on_gesture: RefCell<Option<Closure<dyn FnMut()>>>,
}

impl Inner {
    fn close(&self) {
        self.closed.set(true);
        if let Some(stream) = self.stream.borrow_mut().take() {
            stop(&stream);
        }
        for node in self.nodes.borrow_mut().drain(..) {
            let _ = node.disconnect();
        }
        if let Some(ctx) = self.ctx.borrow_mut().take() {
            let _ = ctx.close();
        }
        if let Some(on_gesture) = self.on_gesture.borrow_mut().take()
            && let Some(window) = web_sys::window()
        {
            for event in GESTURES {
                let _ = window.remove_event_listener_with_callback(
                    event,
                    on_gesture.as_ref().unchecked_ref(),
                );
            }
        }
        self.on_message.borrow_mut().take();
    }
}

pub(crate) struct Backend {
    inner: Rc<Inner>,
}

impl Backend {
    pub(crate) fn open(shared: Arc<Shared>) -> Self {
        let inner = Rc::new(Inner::default());
        let task = inner.clone();
        spawn_local(async move {
            if let Err(err) = start(shared.clone(), &task).await {
                let message = describe(&err);
                log::warn!("Microphone unavailable: {message}");
                task.close();
                shared.set_status(MicStatus::Failed(message));
            }
        });
        Self { inner }
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.inner.close();
    }
}

async fn start(shared: Arc<Shared>, inner: &Inner) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("No window")?;
    let media = window.navigator().media_devices()?;
    let mut stream = user_media(&media, None).await?;
    if inner.closed.get() {
        stop(&stream);
        return Ok(());
    }
    // With a headset connected, iOS records from the headset, whose
    // microphone filters out hits; switch to the built-in one. Labels are
    // only readable once the microphone is allowed.
    let inputs = audio_inputs(&media).await?;
    if let Some(wanted) = inputs.iter().find(|d| is_builtin_ios_mic(&d.label()))
        && wanted.label() != track_label(&stream)
    {
        stop(&stream);
        stream = user_media(&media, Some(&wanted.device_id())).await?;
    }
    if inner.closed.get() {
        stop(&stream);
        return Ok(());
    }
    *inner.stream.borrow_mut() = Some(stream.clone());

    // Created after the microphone is live: a capturing page may start audio
    // without a gesture (iOS). Should it still be suspended, the next tap
    // resumes it.
    let ctx = AudioContext::new()?;
    *inner.ctx.borrow_mut() = Some(ctx.clone());
    let _ = ctx.resume();
    let resume_ctx = ctx.clone();
    let on_gesture = Closure::<dyn FnMut()>::new(move || {
        if resume_ctx.state() != AudioContextState::Running {
            let _ = resume_ctx.resume();
        }
    });
    for event in GESTURES {
        window.add_event_listener_with_callback(event, on_gesture.as_ref().unchecked_ref())?;
    }
    *inner.on_gesture.borrow_mut() = Some(on_gesture);

    let props = BlobPropertyBag::new();
    props.set_type("application/javascript");
    let blob = Blob::new_with_str_sequence_and_options(&Array::of1(&WORKLET.into()), &props)?;
    let url = Url::create_object_url_with_blob(&blob)?;
    let added = JsFuture::from(ctx.audio_worklet()?.add_module(&url)?).await;
    let _ = Url::revoke_object_url(&url);
    added?;
    if inner.closed.get() {
        return Ok(());
    }

    let options = AudioWorkletNodeOptions::new();
    options.set_number_of_inputs(1);
    options.set_number_of_outputs(0);
    let node = AudioWorkletNode::new_with_options(&ctx, PROCESSOR, &options)?;
    let source = ctx.create_media_stream_source(&stream)?;
    source.connect_with_audio_node(&node)?;

    let sample_rate = ctx.sample_rate();
    let mut detector = OnsetDetector::new(sample_rate as u32, shared.threshold_db());
    let mut clock = ContextClock::default();
    let mut hits = Vec::new();
    let handler = shared.clone();
    let clock_ctx = ctx.clone();
    let on_message = MessageHandler::new(move |event: MessageEvent| {
        let data = event.data();
        let frame = Reflect::get(&data, &"frame".into()).ok().and_then(|f| f.as_f64());
        let samples = Reflect::get(&data, &"samples".into())
            .ok()
            .and_then(|s| s.dyn_into::<Float32Array>().ok());
        let (Some(frame), Some(samples)) = (frame, samples) else {
            return;
        };
        let start = clock.map(&clock_ctx, frame / sample_rate as f64);
        detector.set_threshold_db(handler.threshold_db());
        hits.clear();
        let peak = detector.process(samples.to_vec(), start, &mut hits);
        handler.push(&hits, peak);
    });
    node.port()?.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    *inner.on_message.borrow_mut() = Some(on_message);
    inner.nodes.borrow_mut().extend([source.unchecked_into(), node.unchecked_into()]);
    shared.set_status(MicStatus::Running(track_label(&stream)));
    Ok(())
}

/// Maps audio context time to the `performance.now()` timeline. The offset
/// between the two is read whenever a block arrives; `currentTime` lags by
/// up to a render quantum and messages may arrive late, which both only make
/// a reading larger, so the smallest recent one is kept.
#[derive(Default)]
struct ContextClock {
    offset: Option<f64>,
}

impl ContextClock {
    /// How far the kept offset may drift up per block (s): follows the slow
    /// drift between the two clocks.
    const DRIFT: f64 = 1e-5;
    /// A reading this far above the kept offset means the audio clock jumped
    /// (a glitch or a stall); start over from it.
    const JUMP: f64 = 0.05;

    fn map(&mut self, ctx: &AudioContext, context_time: f64) -> f64 {
        let reading = now_seconds() - ctx.current_time();
        let offset = match self.offset {
            Some(kept) if reading - kept < Self::JUMP => reading.min(kept + Self::DRIFT),
            _ => reading,
        };
        self.offset = Some(offset);
        context_time + offset
    }
}

/// The microphone without the browser's voice processing, which would smear
/// or suppress the transients.
async fn user_media(media: &MediaDevices, device_id: Option<&str>) -> Result<MediaStream, JsValue> {
    let audio = Object::new();
    for key in ["echoCancellation", "noiseSuppression", "autoGainControl"] {
        Reflect::set(&audio, &key.into(), &false.into())?;
    }
    if let Some(id) = device_id {
        let exact = Object::new();
        Reflect::set(&exact, &"exact".into(), &id.into())?;
        Reflect::set(&audio, &"deviceId".into(), &exact)?;
    }
    let constraints = MediaStreamConstraints::new();
    constraints.set_audio(&audio);
    let stream = JsFuture::from(media.get_user_media_with_constraints(&constraints)?).await?;
    Ok(stream.unchecked_into())
}

async fn audio_inputs(media: &MediaDevices) -> Result<Vec<MediaDeviceInfo>, JsValue> {
    let list: Array = JsFuture::from(media.enumerate_devices()?).await?.unchecked_into();
    Ok(list
        .iter()
        .map(|d| d.unchecked_into::<MediaDeviceInfo>())
        .filter(|d| d.kind() == MediaDeviceKind::Audioinput)
        .collect())
}

fn tracks(stream: &MediaStream) -> Vec<MediaStreamTrack> {
    stream.get_audio_tracks().iter().map(|t| t.unchecked_into()).collect()
}

fn track_label(stream: &MediaStream) -> String {
    tracks(stream).first().map(|t| t.label()).unwrap_or_default()
}

fn stop(stream: &MediaStream) {
    tracks(stream).iter().for_each(|t| t.stop());
}

fn describe(err: &JsValue) -> String {
    Reflect::get(err, &"message".into())
        .ok()
        .and_then(|m| m.as_string())
        .or_else(|| err.as_string())
        .unwrap_or_else(|| format!("{err:?}"))
}
