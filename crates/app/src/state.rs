use crate::accuracy::AccuracyState;
use crate::generator::GeneratorState;
use crate::guide::{AnchorFlash, Guide};
use crate::library::PatternLibrary;
use crate::library_panel::{LibraryItem, LibraryTab};
use crate::metronome_view::MetronomeState;
use crate::metronome_view::MetronomeVoice;
use crate::persist::StateBackup;
use crate::pickers::Picker;
use crate::platform::PlatformRuntime;
use crate::play_along::PlayAlongState;
use crate::presets::{PresetId, PresetLibrary};
use crate::shell::ShellState;
use crate::undo::UndoHistory;
use crate::{CountingSettings, Host, Mode, TransportState};
use eframe::egui::TextStyle;
use eframe::egui::emath::{ease_in_ease_out, lerp};
use eframe::epaint::FontId;
use grooph_audio::{Audio, AudioSettings, GapClick, TempoRamp};
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
    pub(crate) stem_length_factor: f32,
}

impl Default for LayoutSettings {
    fn default() -> Self {
        Self { width_cap_factor: 0.1, stem_length_factor: 0.9 }
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
    /// The cursor waited for the output to start playing, see
    /// `Audio::is_starting`.
    pub(crate) waited_for_output: bool,
    pub(crate) flash_intensity: f32,
    /// Last (measure_idx, primary_beat_in_measure) playback reached; the flash
    /// fires when it changes. Tracks both axes so the flash fires once per
    /// primary beat in every measure, not only when the beat number changes.
    pub(crate) last_primary_beat: Option<(usize, u32)>,
    /// Bars played since the start, the first being 0, for the guide's
    /// "every second bar" (endless mode renumbers the measures).
    pub(crate) bars_played: u64,
    /// Measure and local tick played in the last frame, to count
    /// `bars_played`.
    pub(crate) guide_bar: Option<(usize, f64)>,
    /// Last guide anchor reached.
    pub(crate) anchor: Option<AnchorFlash>,
    pub(crate) count_in: CountInState,
    /// Tempo the ramp reached in the bar being played; `None` without a ramp.
    pub(crate) ramp_bpm: Option<u32>,
    /// The gap click silences the bar being played.
    pub(crate) bar_muted: bool,
    /// Vertical scroll that follows the playback cursor from row to row.
    pub(crate) scroll: ScrollTween,
    /// Distance from one row of the staff to the next, as last drawn.
    pub(crate) row_pitch: f32,
    /// Rows scrolled out of view above, as last drawn (endless mode drops
    /// them).
    pub(crate) rows_above_view: usize,
    /// The rows above were removed since the last frame: the scroll area
    /// takes the shifted offset right away, or it would show the content
    /// moved up a row for a frame.
    pub(crate) rows_removed: bool,
}

impl PlaybackState {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    /// The score hasn't started yet (count-in pending or playing).
    pub(crate) fn counting_in(&self) -> bool {
        self.count_in != CountInState::Off
    }

    /// Fire the staff flash when playback reaches `beat` (measure, beat in
    /// the measure). Like the guide's anchors: nothing during the count-in,
    /// so the first flash is the score's first downbeat.
    pub(crate) fn reach_flash_beat(&mut self, beat: (usize, u32), enabled: bool) {
        if self.counting_in() || self.last_primary_beat == Some(beat) {
            return;
        }
        self.last_primary_beat = Some(beat);
        if enabled {
            self.flash_intensity = 1.0;
        }
    }

    /// How strongly the staff flash shows now. Dark in a silent bar, like
    /// the anchors: checked when drawn, not when the beat is reached, as
    /// `bar_muted` can trail the cursor into the next bar by a frame.
    pub(crate) fn flash_shown(&self) -> f32 {
        if self.bar_muted { 0.0 } else { self.flash_intensity }
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

    /// The offset at `now`, without a new target.
    pub(crate) fn value_at(&self, now: f64) -> Option<f32> {
        self.state.map(|state| state.value_at(now))
    }

    /// Move the whole tween by `delta`, e.g. when rows above it were removed
    /// and the content moved up by as much: the view stays where it was.
    pub(crate) fn shift(&mut self, delta: f32) {
        if let Some(state) = &mut self.state {
            state.from += delta;
            state.to += delta;
        }
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
    /// Saved state that did not restore completely, kept under a backup key
    /// at the next save.
    pub(crate) state_backup: Option<StateBackup>,
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
    /// Whether the staff flashes on every beat while playing. Off by default.
    pub(crate) flash_enabled: bool,
    /// What the practice staff shows while it plays.
    pub(crate) guide: Guide,
    pub(crate) platform: PlatformRuntime,
    /// User preset whose row in the preset picker is being renamed. Not persisted.
    pub(crate) renaming_preset: Option<u64>,
    /// Saved pattern whose card in the library is being renamed. Not persisted.
    pub(crate) renaming_pattern: Option<u64>,
    /// The name being typed for the card being renamed; it becomes the name
    /// on ✔ or Enter only. Not persisted.
    pub(crate) rename_typed: String,
    /// Load awaiting the "discard unsaved changes?" confirmation. `Some` shows
    /// the confirm dialog. Not persisted.
    pub(crate) pending_load: Option<PendingLoad>,
    /// Why a shared link could not be opened; `Some` shows an error dialog.
    pub(crate) link_error: Option<String>,
    /// UI time of the last "copy link" click, for a short confirmation.
    pub(crate) link_copied_at: Option<f64>,
    /// The library item saved last and the UI time, for "✓ Saved" on its
    /// card. Not persisted.
    pub(crate) library_saved: Option<(LibraryItem, f64)>,
    /// A library card to scroll into view once: the one just saved.
    pub(crate) scroll_to_library_item: Option<LibraryItem>,
    /// The half of the library shown. Not persisted.
    pub(crate) library_tab: LibraryTab,
    /// The window is phone-sized: the staff packs its notes tighter. Set
    /// every frame.
    pub(crate) narrow_screen: bool,
    /// Em the staff was last drawn at, so the metronome ring's hand is as
    /// wide as the playback cursor.
    pub(crate) staff_em: f32,
    /// Edit cursor the staff last scrolled into view. The staff follows the
    /// cursor only when it moves, so the user can scroll away from it.
    pub(crate) followed_cursor: Option<(MetronomeVoice, Cursor)>,
    pub(crate) shell: ShellState,
    /// Setting whose picker dialog is open.
    pub(crate) picker: Option<Picker>,
    /// Picker that opened the one in [`Self::picker`] and comes back when
    /// it closes (Play along's MIDI row opens the MIDI input picker).
    pub(crate) picker_return: Option<Picker>,
    /// Taps of the tap-tempo buttons (tempo picker).
    pub(crate) tap_tempo: TapTempo,
    /// Share sheet and permissions of the app around this one (Android).
    pub(crate) host: Option<Box<dyn Host>>,
    /// `Some` when the app runs embedded in a chat (see [`crate::embed`]).
    pub(crate) embed: Option<crate::embed::Embed>,
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
    fn the_flash_waits_for_the_first_downbeat() {
        let mut playback = PlaybackState { count_in: CountInState::Active, ..Default::default() };
        playback.reach_flash_beat((0, 0), true);
        assert_eq!(playback.flash_intensity, 0.0);
        playback.count_in = CountInState::Off;
        playback.reach_flash_beat((0, 0), true);
        assert_eq!(playback.flash_intensity, 1.0);
    }

    #[test]
    fn the_flash_fires_once_per_beat() {
        let mut playback = PlaybackState::default();
        playback.reach_flash_beat((0, 1), true);
        playback.flash_intensity = 0.5;
        playback.reach_flash_beat((0, 1), true);
        assert_eq!(playback.flash_intensity, 0.5);
        playback.reach_flash_beat((0, 2), true);
        assert_eq!(playback.flash_intensity, 1.0);
    }

    #[test]
    fn a_silent_bar_does_not_flash() {
        let mut playback = PlaybackState { bar_muted: true, ..Default::default() };
        playback.reach_flash_beat((0, 0), true);
        assert_eq!(playback.flash_shown(), 0.0);
        // The mute ends a frame late: the next bar's downbeat still shows.
        playback.reach_flash_beat((1, 0), true);
        playback.bar_muted = false;
        assert_eq!(playback.flash_shown(), 1.0);
    }

    #[test]
    fn scroll_tween_shift_moves_the_glide_along() {
        let mut tween = ScrollTween::default();
        tween.update(0.0, 0.0);
        tween.update(100.0, 0.0);
        let mid = tween.update(100.0, SCROLL_TWEEN_MIN_S / 2.0);
        tween.shift(-40.0);
        // Same target, shifted: the glide goes on, shifted along.
        assert_eq!(tween.update(60.0, SCROLL_TWEEN_MIN_S / 2.0), mid - 40.0);
        assert_eq!(tween.update(60.0, SCROLL_TWEEN_MIN_S), 60.0);
    }

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
