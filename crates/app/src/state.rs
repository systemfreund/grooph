use grooph_audio::AudioSettings;
use grooph_midi::MidiInput;

pub(crate) struct AudioConfig {
    pub(crate) settings: AudioSettings,
    pub(crate) offset: f32,
    pub(crate) latency_enabled: bool,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self { settings: AudioSettings::default(), offset: 0.0, latency_enabled: true }
    }
}

pub(crate) struct LayoutSettings {
    pub(crate) width_cap_factor: f32,
    pub(crate) accent_below: bool,
    pub(crate) proportional_spacing: bool,
    pub(crate) stem_length_factor: f32,
    pub(crate) debug_bbox: bool,
}

impl Default for LayoutSettings {
    fn default() -> Self {
        Self {
            width_cap_factor: 0.1,
            accent_below: false,
            proportional_spacing: true,
            stem_length_factor: 0.9,
            debug_bbox: false,
        }
    }
}

#[derive(Default)]
pub(crate) struct PlaybackState {
    pub(crate) smooth_tick: f64,
    pub(crate) last_update: Option<f64>,
    pub(crate) flash_intensity: f32,
    pub(crate) last_primary_beat: Option<u32>,
}

impl PlaybackState {
    pub(crate) fn reset(&mut self) { *self = Self::default(); }
}

#[derive(Default)]
pub(crate) struct MidiState {
    pub(crate) input: Option<MidiInput>,
    pub(crate) available_ports: Vec<String>,
    pub(crate) selected_port_id: Option<String>,
}
