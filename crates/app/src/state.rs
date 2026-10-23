use crate::accuracy::AccuracyState;
use crate::compact::CompactState;
use crate::generator::GeneratorState;
use crate::library::PatternLibrary;
use crate::metronome_view::MetronomeState;
use crate::pickers::Picker;
use crate::platform::PlatformRuntime;
use crate::play_along::PlayAlongState;
use crate::presets::{PresetId, PresetLibrary};
use crate::undo::UndoHistory;
use crate::{CountingSettings, Host, Mode, TransportState};
use eframe::egui::TextStyle;
use eframe::egui::emath::{ease_in_ease_out, lerp};
use eframe::epaint::FontId;
use grooph_audio::{Audio, AudioSettings, GapClick, TempoRamp};
use grooph_layout::staff_layout::Voice;
use grooph_link::SharedLink;
use grooph_measure::click::ClickBars;
use grooph_measure::counter::CounterBars;
use grooph_measure::swing::Swing;
use grooph_measure::tap_tempo::TapTempo;
use grooph_measure::{Cursor, Measure, Score};
use grooph_mic::MicInput;
use grooph_midi::MidiInput;
use std::collections::HashMap;

pub(crate) struct AudioConfig {
    pub(crate) settings: AudioSettings,
    pub(crate) offset: f32,
    pub(crate) latency_enabled: bool,
    /// Play one bar of clicks before playback starts.
    pub(crate) count_in: bool,
    /// Swing feel of the playback (notation stays straight).
    pub(crate) swing: Swing,
    /// Metronome trainer: muted bars.
    pub(crate) gap: GapClick,
    /// Metronome trainer: stepwise tempo change from `bpm`.
    pub(crate) ramp: TempoRamp,
    /// What the metronome clicks in each meter.
    pub(crate) click: ClickBars,
    /// The counter rhythm of each meter that has one.
    pub(crate) counter: CounterBars,
}

impl AudioConfig {
    /// The latency offset in seconds, 0 while switched off.
    pub(crate) fn latency_s(&self) -> f64 {
        if self.latency_enabled { self.offset as f64 } else { 0.0 }
    }
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            settings: AudioSettings::default(),
            offset: 0.0,
            latency_enabled: true,
            count_in: false,
            swing: Swing::default(),
            gap: GapClick::default(),
            ramp: TempoRamp::default(),
            click: ClickBars::default(),
            counter: CounterBars::default(),
        }
    }
}

pub(crate) struct LayoutSettings {
    pub(crate) width_cap_factor: f32,
    pub(crate) accent_below: bool,
    pub(crate) proportional_spacing: bool,
    pub(crate) stem_length_factor: f32,
    /// The counter shares the click bar's line, below it, instead of having
    /// a line of its own.
    pub(crate) counter_on_one_line: bool,
}

impl Default for LayoutSettings {
    fn default() -> Self {
        Self {
            width_cap_factor: 0.1,
            accent_below: false,
            proportional_spacing: true,
            stem_length_factor: 0.9,
            counter_on_one_line: true,
        }
    }
}

/// Whether the audio engine is playing the count-in bar, as seen by the UI.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum CountInState {
    /// No count-in: the score is playing.
    #[default]
    Off,
    /// Playback started with count-in enabled; waiting for the audio engine to
    /// report the count-in (negative position). `since` is the UI time.
    Waiting { since: f64 },
    /// Audio reports the count-in bar.
    Active,
}

/// Give up waiting for the audio engine to report a count-in after this long
/// (no audio device, engine failed to start).
pub(crate) const COUNT_IN_WAIT_TIMEOUT_S: f64 = 0.5;

#[derive(Default)]
pub(crate) struct PlaybackState {
    /// Cursor position in **global ticks** across the entire score loop.
    pub(crate) smooth_tick: f64,
    /// Like `smooth_tick`, but not held during the count-in (which it
    /// wraps into the loop), for the metronome ring.
    pub(crate) heard_tick: f64,
    pub(crate) last_update: Option<f64>,
    pub(crate) flash_intensity: f32,
    /// Last (measure_idx, primary_beat_in_measure) that fired the flash. Tracks
    /// both axes so the flash fires once per primary beat in every measure, not
    /// only when the beat number changes.
    pub(crate) last_primary_beat: Option<(usize, u32)>,
    pub(crate) count_in: CountInState,
    /// Tempo the ramp reached in the bar being played; `None` without a ramp.
    pub(crate) ramp_bpm: Option<u32>,
    /// The gap click silences the bar being played.
    pub(crate) bar_muted: bool,
    /// Vertical scroll that follows the playback cursor from row to row.
    pub(crate) scroll: ScrollTween,
}

impl PlaybackState {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    /// The score hasn't started yet (count-in pending or playing).
    pub(crate) fn counting_in(&self) -> bool {
        self.count_in != CountInState::Off
    }
}

/// How long the score takes to scroll one row further while playing; a
/// longer jump (back to the top on a loop) takes up to [`SCROLL_TWEEN_MAX_S`].
const SCROLL_TWEEN_MIN_S: f64 = 0.35;
const SCROLL_TWEEN_MAX_S: f64 = 0.6;
/// Scroll speed (points per second) past which a jump takes longer than the
/// minimum.
const SCROLL_TWEEN_POINTS_PER_S: f32 = 1000.0;

/// Eases the scroll offset toward the playing row instead of jumping there,
/// so the rows glide up when the cursor wraps to the next one.
#[derive(Default)]
pub(crate) struct ScrollTween {
    /// `None` until playback first scrolls: the first offset is taken as is.
    state: Option<TweenState>,
}

#[derive(Clone, Copy)]
struct TweenState {
    from: f32,
    to: f32,
    start: f64,
    duration: f64,
}

impl TweenState {
    fn value_at(&self, now: f64) -> f32 {
        if self.duration <= 0.0 {
            return self.to;
        }
        let t = ((now - self.start) / self.duration).clamp(0.0, 1.0) as f32;
        lerp(self.from..=self.to, ease_in_ease_out(t))
    }
}

impl ScrollTween {
    /// The offset to show at `now` while heading for `target`. A new target
    /// starts a fresh tween from wherever the scroll is right now.
    pub(crate) fn update(&mut self, target: f32, now: f64) -> f32 {
        let Some(state) = &mut self.state else {
            self.state = Some(TweenState { from: target, to: target, start: now, duration: 0.0 });
            return target;
        };
        if (state.to - target).abs() > 0.5 {
            let from = state.value_at(now);
            let duration = f64::from((target - from).abs() / SCROLL_TWEEN_POINTS_PER_S)
                .clamp(SCROLL_TWEEN_MIN_S, SCROLL_TWEEN_MAX_S);
            *state = TweenState { from, to: target, start: now, duration };
        }
        state.value_at(now)
    }
}

#[derive(Default)]
pub(crate) struct MidiState {
    pub(crate) input: Option<MidiInput>,
    pub(crate) available_ports: Vec<String>,
    pub(crate) selected_port_id: Option<String>,
}

/// Microphone hit input. Open only while it is needed: in use, the
/// microphone changes how phones route and play audio.
#[derive(Default)]
pub(crate) struct MicState {
    pub(crate) input: Option<MicInput>,
    /// The settings show the level meter: keep listening until then (clock
    /// seconds).
    pub(crate) monitor_until: f64,
    /// Level shown by the meter (dBFS); falls back slowly after a peak.
    pub(crate) level_db: f32,
    /// The host was asked for microphone access since the microphone was
    /// last wanted, so it is not asked again until then.
    pub(crate) permission_asked: bool,
}

/// Score editing surface: the document, where the cursor sits, undo history,
/// and pre-built button thumbnails. Mutated by tools and the time-signature
/// dialog; read by the rendering pipeline.
pub(crate) struct EditorState {
    pub(crate) score: Score,
    pub(crate) cursor: Cursor,
    pub(crate) history: UndoHistory,
    /// Prebuilt measures for note/rest/tuplet tool buttons to avoid per-frame
    /// reconstruction.
    pub(crate) button_measures: HashMap<&'static str, Measure>,
    /// User-saved, named patterns (score + tempo). Persisted across sessions.
    pub(crate) library: PatternLibrary,
    /// Id of the saved pattern the working score was loaded from, if any. Drives
    /// "save" (overwrite) vs "save as" (new entry) and the active-row highlight.
    /// `None` means the working score is unsaved / not tied to any entry.
    pub(crate) active_pattern_id: Option<u64>,
    /// Whether the working score/tempo has been edited since the last save or
    /// load. Used to warn before discarding changes when loading another pattern.
    pub(crate) dirty: bool,
    /// Rhythm generator settings and endless-mode state.
    pub(crate) generator: GeneratorState,
    /// User-saved generator presets. Persisted across sessions.
    pub(crate) presets: PresetLibrary,
    /// Preset last loaded or saved; "Save" overwrites it when it is a user
    /// preset. Not persisted.
    pub(crate) active_preset: Option<PresetId>,
    /// The metronome tab's meter, and the rhythm it put aside while it edits
    /// the click bar in the score's place.
    pub(crate) metronome: MetronomeState,
}

/// Realtime playback subsystem: transport, tempo, audio engine, MIDI input,
/// microphone and tap input, and the accuracy tracker that ties hits to score onsets.
pub(crate) struct PlaybackController {
    pub(crate) transport_state: TransportState,
    pub(crate) bpm: u32,
    pub(crate) audio: Option<Audio>,
    pub(crate) audio_cfg: AudioConfig,
    pub(crate) playback: PlaybackState,
    pub(crate) accuracy: AccuracyState,
    pub(crate) midi: MidiState,
    pub(crate) mic: MicState,
    pub(crate) play_along: PlayAlongState,
}

/// UI shell state: which panel is visible, font configuration, layout knobs,
/// counting overlay settings, and the platform abstraction for visibility /
/// wake-lock.
pub(crate) struct UiShell {
    pub(crate) mode: Mode,
    pub(crate) music_font_id: FontId,
    pub(crate) font_bump: f32,
    pub(crate) baseline_dark: Option<Vec<(TextStyle, f32)>>,
    pub(crate) baseline_light: Option<Vec<(TextStyle, f32)>>,
    pub(crate) layout: LayoutSettings,
    pub(crate) counting: CountingSettings,
    /// Whether the playback panel flashes on every primary beat. Off by default.
    pub(crate) flash_enabled: bool,
    pub(crate) platform: PlatformRuntime,
    /// Transient text buffer for the "save current pattern" name input. Not
    /// persisted.
    pub(crate) save_name_buffer: String,
    /// Name input for "save preset as". Not persisted.
    pub(crate) preset_name_buffer: String,
    /// User preset whose row in the preset picker is being renamed. Not persisted.
    pub(crate) renaming_preset: Option<u64>,
    /// Load awaiting the "discard unsaved changes?" confirmation. `Some` shows
    /// the confirm dialog. Not persisted.
    pub(crate) pending_load: Option<PendingLoad>,
    /// Why a shared link could not be opened; `Some` shows an error dialog.
    pub(crate) link_error: Option<String>,
    /// UI time of the last "copy link" click, for a short confirmation.
    pub(crate) link_copied_at: Option<f64>,
    /// The window is phone-sized: the staff packs its notes tighter. Set
    /// every frame.
    pub(crate) narrow_screen: bool,
    /// Em the staff was last drawn at, so the metronome ring's hand is as
    /// wide as the playback cursor.
    pub(crate) staff_em: f32,
    /// Edit cursor the staff last scrolled into view. The staff follows the
    /// cursor only when it moves, so the user can scroll away from it.
    pub(crate) followed_cursor: Option<(Voice, Cursor)>,
    pub(crate) compact: CompactState,
    /// Setting whose picker dialog is open.
    pub(crate) picker: Option<Picker>,
    /// Taps of the tap-tempo buttons (tempo picker).
    pub(crate) tap_tempo: TapTempo,
    /// Share sheet and permissions of the app around this one (Android).
    pub(crate) host: Option<Box<dyn Host>>,
}

/// Something that replaces the working score once unsaved changes are dealt with.
pub(crate) enum PendingLoad {
    Pattern(u64),
    Link(Box<SharedLink>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_tween_takes_the_first_offset_as_is() {
        let mut tween = ScrollTween::default();
        assert_eq!(tween.update(120.0, 0.0), 120.0);
        assert_eq!(tween.update(120.0, 1.0), 120.0);
    }

    #[test]
    fn scroll_tween_glides_to_the_next_row() {
        let mut tween = ScrollTween::default();
        tween.update(0.0, 0.0);
        // The next row starts the tween where the scroll is now.
        assert_eq!(tween.update(100.0, 1.0), 0.0);
        let mid = tween.update(100.0, 1.0 + SCROLL_TWEEN_MIN_S / 2.0);
        assert!((mid - 50.0).abs() < 0.01, "halfway at {mid}");
        assert_eq!(tween.update(100.0, 1.0 + SCROLL_TWEEN_MIN_S), 100.0);
        assert_eq!(tween.update(100.0, 5.0), 100.0);
    }

    #[test]
    fn scroll_tween_retargets_from_where_it_is() {
        let mut tween = ScrollTween::default();
        tween.update(0.0, 0.0);
        tween.update(100.0, 0.0);
        let mid = tween.update(100.0, SCROLL_TWEEN_MIN_S / 2.0);
        // Heading back mid-way continues from the current offset, no jump.
        assert_eq!(tween.update(0.0, SCROLL_TWEEN_MIN_S / 2.0), mid);
    }

    #[test]
    fn scroll_tween_takes_longer_for_a_long_jump() {
        let mut tween = ScrollTween::default();
        tween.update(2000.0, 0.0);
        tween.update(0.0, 0.0);
        assert!(tween.update(0.0, SCROLL_TWEEN_MIN_S) > 0.0);
        assert_eq!(tween.update(0.0, SCROLL_TWEEN_MAX_S), 0.0);
    }
}
