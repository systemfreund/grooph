# Grooph

Short description: Grooph is a rhythm and meter editor with playback. The app
shows a single measure, allows notes/rests/tuplets (triplets), plays a metronome
track, and runs native as well as WebAssembly (eframe/egui).

<img width="945" height="2048" alt="image" src="https://github.com/user-attachments/assets/208b524b-b48e-4410-8b41-dfb3f060c8e7" />

This README is meant for LLM coding agents and describes the key concepts,
files, and invariants so changes can be made with confidence.

## Project map (workspace)

- `crates/app`: GUI, app state (`Grooph`), panels, tool palette, input, persistence.
- `crates/measure`: Domain model for measures/beats/duration, editing logic, counting.
- `crates/layout`: Pixel layout from the model (note positions, beam/tuplet plans).
- `crates/render`: Draws the layout with egui (notes, beams, tuplets, cursor).
- `crates/audio`: Metronome synth (rodio), scheduling based on the measure.
- `crates/midi`: MIDI input abstraction (midir).
- `crates/mic`: Microphone hit detection (cpal natively, `getUserMedia` + AudioWorklet on the web).
- `crates/link`: Shareable links (URL query parameters <-> score/generator/metronome).
- `crates/site`: Static guide pages for grooph.app, generated from Markdown at build time.
- `crates/android`: Android entry point (`android_main`, NativeActivity) around `crates/app`.

Important root files:
- `Cargo.toml` (workspace + dependencies, edition 2024)
- `Trunk.toml` (WASM build/serve)
- `rustfmt.toml` (formatting)

## Data model and invariants

- Core object: `grooph_measure::Measure` with `Vec<Beat>` and `TimeSignature`.
- `Beat` has `duration`, `kind` (Note/Rest), `accented`, `soft` (a ghost note,
  head in parentheses; never with `accented`), `tuplet_group_id`.
- `Duration` is `Simple`, `Dotted`, or `Tuplet(TupletSpec { n, m, base })`.
- `Measure::set_beat` guarantees a valid measure length and fills gaps.
- `DEFAULT_GRID` defines the valid duration grid and tick calculations.
- Tuplets:
  - A tuplet beat automatically creates the remaining beats of the group.
  - Groups are tracked via `tuplet_group_id` and `tuplet_anchors`.
  - `set_beat` can absorb/fill following beats to keep the measure consistent.
- Generator (`grooph_measure::generator`): builds random measures for
  sight-reading from per-beat cells. `GeneratorSettings` = subdivision,
  complexity (1-5, caps the cell level), bars, time signature (x/4, x/8,
  x/16), space (extra whole-beat rests). In x/8 and x/16 the generator beats
  are the meter's beat groups (`beat_groups`, from `default_groups_for`:
  6/8 = 3+3, 7/8 = 2+3+2, the same for x/16). Two-eighth groups use the
  quarter-beat cells, three-eighth groups their own `Compound*` cells, x/16
  groups `SixteenthPair` / `SixteenthTriple`. Triplets/mixed fall back to
  8ths/16ths in x/8, x/16 is always 16ths (`effective_subdivision`). Measures are written via `set_beat`, so
  they obey the same invariants as edited ones. Seed `Rng` for reproducibility.
- Note: `Beat` `PartialEq` ignores `tuplet_group_id`. Compare the field explicitly
  if grouping matters.

## Rendering pipeline

1. `grooph_layout::pixel_layout::build_measure_layout` computes `MeasureLayout`
   (note/beam/tuplet positions) from `Measure` and `LayoutOpts`.
2. `grooph_render::measure::draw_measure` renders the result with egui.
3. The font `Bravura.otf` (SMuFL) lives in `crates/app/assets/fonts`.

## Audio, playback, counting

- `grooph_audio::Audio` builds a playback schedule from the measure
  (Downbeat/Primary/Accent/Beat) and uses `DEFAULT_GRID`.
- The playback cursor is smoothed in the UI, audio offset for latency is optional.
- Counting overlay comes from `grooph_measure::counting`.
- Swing (`grooph_measure::swing::Swing`, global, main menu next to BPM): 50%
  (off) to 75% (dotted), swung unit 8ths or 16ths. Notation stays straight;
  `ScoreTiming::with_swing` carries it. `performed_global_tick` moves notes and
  ghost notes in the schedule and is the reference for accuracy hits/misses;
  `written_global_tick` maps the audio position back so the cursor reaches a
  swung note when it sounds. Pairs start at the downbeat, tuplet onsets and a
  trailing incomplete pair stay straight. Not stored with library patterns.

## Text notation and shareable links

- `grooph_measure::notation`: `parse_score` / `format_score` for a compact
  text notation (`4/4 q e e >s s s s t8 t8 r:t8 | q. e r:q q`). Duration
  codes match the helpers in `duration.rs`; `r:` = rest, `>` = accent, `(e)` =
  soft note, `|` =
  bar line, a leading `N/D` sets the time signature. Parsing goes through
  `set_beat`; each measure must be filled exactly, errors name the measure
  and token. Formatting returns `None` for durations without a code.
- `grooph_link` (`crates/link`): query parameters `bpm`, `r`, `ts`, `swing`,
  `su` and the generator's `sub`, `lvl`, `bars`, `space`, `seed` (seeded
  `Rng`, so a link reproduces the same exercise). `parse_query` reads them,
  `SharedLink::to_url` writes them. The app reads the link once at startup
  (`Grooph::open_link_from_url`), then removes it from the address bar. A
  `ts` link on its own opens the Metronome tab with that meter (and its
  `click` bar) and leaves the rhythm alone. A link that replaces the score
  goes through `PendingLoad::Link` and the
  unsaved-changes dialog. "Copy link" in the library panel builds a link via
  `grooph_link::share_url`.
- The link format is documented for AI agents in
  `crates/app/assets/seo/llms.txt`; keep it in sync.

## Guide pages (crates/site)

- Markdown under `crates/site/content/` becomes static pages on grooph.app
  (`content/learn/x.md` -> `/learn/x/`). Front matter: `title`,
  `description`, `section` (Metronome, Sight-reading, Learn, Features),
  optional `heading` and `order`.
- Fenced blocks: ` ```rhythm bpm=90 swing=66 | Caption ` (text notation,
  rendered as a grid with counting syllables and a play link), ` ```open |
  Label ` (link query, rendered as a button), ` ```pages ` (list of a
  section). `## FAQ` with `### Question` headings becomes FAQPage data.
- Runs as a Trunk `post_build` hook (see `Trunk.toml`), so `trunk build`
  writes the pages, `site.css`, `sitemap.xml` and `llms.txt` (base file plus
  a list of guides) into `dist/`. By hand: `cargo run -p grooph-site -- dist`.
- `cargo test -p grooph-site` builds all content and checks every rhythm and
  every link (app links through `grooph_link::parse_query`).

## Generator / endless mode

- UI: the Practice tab of the layout (`crates/app/src/compact.rs`). The
  settings card opens a picker per setting (`crates/app/src/pickers.rs`);
  changing a setting regenerates the score (undoable), 🎲 rolls a new rhythm
  with the same settings. Space and the grouping picker live in the Level
  picker, presets (load, save, update, rename, delete) in the Preset picker.
- App glue: `crates/app/src/generator.rs`. Endless mode replaces a measure once
  both the visible cursor and the audio cursor have left it; the replacement
  keeps the measure's time signature so loop timing stays stable. Needs at
  least 2 bars.
- Measure widths (`staff_layout::min_measure_width`) are sized for at least a
  measure full of sixteenths, so up to that density they depend only on the
  meter: editing or swapping measures (endless mode) never reflows the staff.
  Denser measures (32nds, big tuplets) grow with their beat count.
- Groupings: every one-beat figure has a stable `GroupingId` (bit index in
  `GroupingSet`; never renumber, it is persisted). The catalog order follows
  the reference app's "Custom Groupings" screen. A complexity level is a preset
  subset (`complexity_groupings`); `custom_groupings(_enabled)` in
  `GeneratorSettings` replaces it with a hand-picked subset (drawn uniformly;
  an empty selection falls back to complexity). Picker UI: "Groupings" in the
  generator panel, tiles via `tool_palette::notation_button`.
- Ghost notes (`grooph_measure::ghost::ghost_onsets`): quiet hits on every free
  slot of the generator subdivision (per beat: 8ths=2, 16ths=4, triplets=3;
  mixed = triplets in tuplet beats, else 16ths). Other beat units get a straight
  8th or 16th grid (triplets → 8ths, mixed → 16ths). Scheduled as `SoundType::Ghost`,
  volume via the mixer's "Ghost notes" slider (`AudioSettings::ghost`).
- Count-in (`PlaybackOptions::count_in`): `TickSource` plays one bar of
  clicks on every beat unit of the first measure (`SoundType::CountIn` on the
  downbeat and primary beats, the quieter `CountInSoft` in between, so 7/8
  counts in all seven eighths) before the score; `Audio::playback_position` is negative meanwhile. The UI
  holds the cursor at 0 (`CountInState`) and re-anchors the accuracy session
  when the count-in ends.

- Presets (`crates/app/src/presets.rs`, UI and apply/snapshot in
  `preset_panel.rs`): `PresetSettings` = `GeneratorSettings` + BPM, swing, ghost
  notes, count-in, endless, gap click, tempo ramp and both voice sounds. Built-in
  presets are code (`builtin_presets`, addressed as `PresetId::BuiltIn(index)`),
  never stored; user presets live in `PresetLibrary` in `PersistedState`.
  `active_preset` is not persisted; at startup it is the first preset equal to
  the current settings. Applying a preset regenerates the score.

## Sounds

- Two voices (`crates/audio/src/sounds.rs`): the rhythm (`Beat`, `AccentedBeat`,
  `Ghost`) uses `AudioSettings::rhythm_sound`, the metronome (`Downbeat`,
  `PrimaryBeat`, `CountIn`) `AudioSettings::metronome_sound`. `SoundType::patch`
  turns a hit into a `Patch`: the configurable `Click` (waveform, frequency, noise)
  or an embedded WAV sample, both with the mixer gain of that sound type.
- Samples live in `crates/audio/samples/` and are listed in `SAMPLES`
  (`include_bytes!`, decoded once by the small decoder in `wav.rs`).
  `MetronomeSound::Claves` (default) plays `claves3.wav` on the downbeat and
  `claves4.wav` on the other primary beats and the count-in;
  `MetronomeSound::Xylophone` plays `xylo-high.wav` and `xylo-low.wav` the same way.
  The counter rhythm (`CounterSound`) plays a sample 1.5× faster when the
  metronome uses the same sound, so the two layers stay apart.
  `RhythmSound::Snare` (default) plays its own recording per dynamic:
  `snare-normal.wav` for notes, `snare-accent.wav` for accents and
  `snare-ghost.wav` for ghost notes. A ghost sample is recorded quietly, so its
  gain is divided by the default ghost volume (the default plays it as recorded).
  Samples are stored as 16-bit mono WAV, trimmed after the decay, to keep the
  WASM build small.
  Samples keep their own license (claves: altemark on freesound.org, CC BY-NC; xylophone: Brsjak on
  freesound.org, CC BY 4.0),
  see `crates/audio/samples/CREDITS.md`; add a row there with every new sample.
- `VoiceMixer` (`voices.rs`) plays samples resampled to the device rate.
- Mixer (`mixer_panel.rs`): a "Metronome" group (`downbeat`, `primary` as
  "Medium", `soft`, `count_in`) and a "Rhythm" group (`beat` as "Notes", `accent`,
  `ghost`), each with a mute (`metronome_muted` / `rhythm_muted`, applied in
  `SoundType::profile`). The metronome mute leaves the count-in audible.
  Sounds with zero gain are not triggered at all. Clicks between the beats
  (e.g. every eighth of 7/8) come from the click bar, not from the mixer.
- Click bars (`grooph_measure::click`): what the metronome clicks in a meter
  is a measure, the meter's click bar. Each note is a click, its `ClickLevel`
  from the note: accented = strong, plain = medium, soft = soft, a rest = no
  click; tuplets and dotted notes work as in any rhythm. The default bar
  (`default_click_bar`) has one note per beat group, as long as the group,
  the first accented; `subdivided_click_bar` splits every beat unit into 1 to
  4 clicks (group starts keep their level, the rest are soft). `ClickBars`
  (in `AudioConfig`, persisted) keeps the bars that differ from the default,
  keyed by the whole `TimeSignature` including its grouping, and applies to
  all playback in that meter. `Schedule::build` plays strong as `Downbeat`,
  medium as `PrimaryBeat`, soft as `SoftBeat` (mixer "Soft"), at the swung
  position of each note. The count-in and calibration
  ignore the click bars. Link parameter `click` is the click bar of the
  link's first meter in text notation without the time signature (e.g.
  `>q r:q. (q)`); shared links carry it when it is not the default.

## Metronome trainer

- `grooph_audio::{GapClick, TempoRamp}` (`crates/audio/src/trainer.rs`), carried in
  `PlaybackOptions`. Both depend only on the bar count since the score started
  (`TickSource::bars`, across loops), so nothing rebuilds the schedule.
- Gap click: the audio thread decides per bar (`GapDecider`, random mode seeded via
  `Audio::set_seed`) and filters triggers (`SoundType::muted_by`). The count-in is
  never muted. `Audio::bar_state` publishes bar + muted flag; the measure panel
  dims the muted bar.
- Tempo ramp: `TempoRamp::bpm_for_bar` from the BPM setting; the audio thread
  scales ticks per sample, the UI (`metronome_trainer.rs`) uses the same tempo in
  `score_timing()` via `PlaybackState::ramp_bpm` and re-anchors the accuracy
  session on every change.

- Metronome tab (`crates/app/src/metronome_view.rs`): its own meter
  (`MetronomeState::meter`, persisted, picked in the tab's card), separate
  from the generator's. Opening the tab puts the rhythm (score, cursor, undo,
  dirty flag) aside in `MetronomeState` and edits the meter's click bar in its
  place, so the staff, tool palette and undo work on it; `sync_click_bar`
  writes it back to `ClickBars` every frame, and closing the tab brings the
  rhythm back (persistence saves the rhythm, not the bar). The audio engine
  gets one silent bar of the meter (`metronome_score`), so only the clicks
  sound; ghost notes, endless mode and play along are off there. Below the
  staff a ring shows one segment per note or rest of the bar, as long as it,
  thickness by level; a tap cycles strong, medium, soft, off (undoable). The
  "Clicks" picker offers the ready-made bars (`click_fills`), "Edit clicks"
  shows the tool palette. A meter picked there without a click bar of its
  own starts with `meter_click_bar`: beats in x/4, every eighth or
  sixteenth in x/8 and x/16 (stored as the meter's click bar). A meter picked
  for the rhythm (Practice's meter picker, Edit's time signature dialog)
  becomes the metronome's too (`follow_meter`), with the click bar it
  already has; the metronome's meter never changes the rhythm. The staff gets a share of the free height
  (`METRONOME_STAFF_SHARE`), the ring the rest; the options sheet folds away
  like Practice's (shared `options_collapsed`). While playing, the click and beat being heard light
  up and the middle shows the bar number. Switching into or out of the tab
  stops playback.

## Play along (accuracy)

- Hit sources (`crates/app/src/play_along.rs`): MIDI NoteOns, microphone hits
  and taps (tap key, default `J`, or a press on the staff while playing). All
  hits are stamped on one clock (`crates/app/src/clock.rs`); MIDI and
  microphone timestamps are moved onto it by their age. On the web taps use the DOM event `timeStamp` (`web.rs`), natively
  the frame time. Each source has its own input offset (`PlayAlongSettings`).
- While tap capture is active, staff presses are hits, not edits.
- Microphone (`crates/mic`): a hit starts where the first sample crosses the
  threshold; 40 ms after it are ignored. On the web an AudioWorklet forwards
  1024-frame blocks to the page, whose context time is mapped onto
  `performance.now()`; natively cpal's capture timestamp is used. The
  microphone is open only while a run with it plays, a microphone calibration
  runs or the settings show its level meter, because phones change audio
  routing while recording. On iOS the built-in microphone is chosen over a
  headset's, which filters out hits.
- The session (`accuracy::AccuracyTracker`) runs while MIDI is connected or
  after the first tap or microphone hit of a run. Stopping keeps a `RunSummary` (hits, perfect/
  good/off windows, misses, extra hits, mean, spread) for the "Your run" window
  and the run's marks, drawn on the staff until the next run or an edit.
- Grading (`accuracy::session::HitWindows`): perfect ≤ 25 ms, good ≤ 60 ms,
  narrowed to 1/5 and 2/5 of the gap to the closest neighbouring onset (as
  heard) for fast notes. Markers are coloured by grade.
- Calibration (`crates/app/src/calibration.rs`): stashes the score and playback
  options, plays 4 quarters at 100 BPM, takes the median of the hits after a
  warm-up as the source's offset and restores the stash on stop. Persistence
  reads the stash while a calibration runs.

## Layout

- One layout for every window size, made for touch first
  (`crates/app/src/compact.rs`). On windows wider than 720 pt the controls stay
  a centered column; the staff and the tool palette use the full width.
- Tabs: Metronome (see below), Practice (generator card with
  Meter/Notes/Level/Bars/BPM, options sheet, big "new rhythm" + play
  buttons), Edit (tool palette + edit bar),
  More (Library, Mixer, Settings, Help reuse their panels). The tab is derived
  from `Mode` (`compact_tab`), so modes set elsewhere land on the right tab;
  the Metronome tab is open while `metronome_mode()`, in `Mode::Playback`
  (ring) or `Mode::Edit` (tool palette).
- Settings open in a `Modal` picker (`settings_picker`); generator changes go
  through `finish_generator_change`.
- The staff's em is capped so one row fits the panel's height. The row's
  height is measured from its laid-out notes (`StaffLayout::room`,
  `RowRoom::height`).
- Windows narrower than `NARROW_MAX_WIDTH` (600 pt, `UiShell::narrow_screen`)
  get tighter staff spacing, so glyphs are bigger on phones.
- While a tempo ramp runs, the card shows the tempo being played ("BPM now").
- Share (right of play, and in the edit bar) builds a score link with tempo
  and swing; on the web it opens the system share sheet
  (`platform::open_share_sheet`, Web Share API), else copies the link. In the
  Metronome tab, share (right of play) builds a `ts` link with tempo, swing
  and the click bar instead.
- Keyboard shortcuts are ignored while a text field has the focus.


- Tool registry: `crates/app/src/tools.rs` (ToolKind, Modifier, shortcuts).
- Keyboard handling: `crates/app/src/keyboard_input.rs`.
- Measure operations: `grooph_measure::editing::{Modification, set_tuplet, ...}`.

If you add new tools/shortcuts, remember:
- Tool registry + palette
- Keyboard input
- UI help text

## Build and test

- Native: `cargo run`
- Tests: `cargo test`
- WASM (optional): `trunk serve` or `trunk build --release`
- Android: `crates/android/build-android.sh` builds `target/android/grooph.aab`
  (the App Bundle for Google Play) and `target/android/grooph.apk` (the same
  app to install directly), each for arm64 phones and the x86_64 emulator.
  It needs `cargo install cargo-ndk`, the `aarch64-linux-android` and
  `x86_64-linux-android` Rust targets, `ANDROID_HOME` (with
  `platforms;android-36`), `ANDROID_NDK_HOME`, Java and bundletool; the
  script lists its variables (keystore, version code). The manifest is
  `crates/android/AndroidManifest.xml`, the launcher icon (an adaptive icon
  with a themed layer) is in `crates/android/res`, the 512 px Play Store
  icon in `crates/android/store`. CI builds both on every push (job
  `android`, artifact `grooph-android`).
  - Signing: CI signs with the upload key when the repository has the
    secrets `ANDROID_UPLOAD_KEYSTORE_BASE64` (the keystore, `base64 -w0`),
    `ANDROID_UPLOAD_KEYSTORE_PASSWORD` and `ANDROID_UPLOAD_KEY_ALIAS` (the
    key's password must match the keystore's), else with a throwaway key.
    Create the key once with
    `keytool -genkeypair -keystore upload.keystore -alias upload -keyalg RSA -keysize 2048 -validity 10000`
    and keep a copy outside GitHub: Play only accepts uploads signed with it.
  - The workspace eframe leaves out accesskit (NativeActivity cannot use it);
    `crates/app` adds it back on every other target. Settings are stored in
    the app's internal storage (`persistence_path`). Android renders with
    glow (OpenGL ES): wgpu cannot create a surface in the emulator.
  - Calls into Java go through JNI in `crates/android/src/activity.rs`:
    NativeActivity draws under the status and navigation bars, so
    `insets.rs` reads their size and hands it to egui as safe-area insets;
    the share sheet and the microphone permission reach the app through
    `grooph_app::Host`. The back button arrives as `Key::BrowserBack`
    (the manifest opts out of Android 16's predictive back, which would
    stop sending it) and goes to `Grooph::back`, which closes what is open;
    with nothing open the app moves to the background. eframe's save when
    the window goes away is the only sign of going to the background, so
    the save checks whether the window is hidden and calls `Grooph::hidden`
    (stops playback, audio and microphone). NativeActivity's own request
    for the on-screen keyboard does nothing on current Android versions, so
    the wrapper opens it through the InputMethodManager while egui wants
    text input (logged as `Keyboard open`) and follows the insets (which
    include the keyboard) closely while it slides. Not handled yet: opening
    shared links in the app (Android intents).

## Test notes (measure)

- Tests live in the relevant modules (e.g. `crates/measure/src/...`).
- Helpers for duration in `crates/measure/src/duration.rs`:
  `q()`, `e()`, `s()`, `th()`, `t8()`, `t16()`, `t32()`, `qt16()`.
- Recommended: `Measure::set_beat` + `Beat::note/rest(...)` for targeted changes.
- Low-level checks: `DEFAULT_GRID.ticks_of(...)` and `compute_onset_ticks(...)`.
- Tuplet tests: note auto-fill and `tuplet_group_id` comparisons.

## Change navigation (quick)

- UI/State/UX: `crates/app/src/lib.rs` and panels (`*_panel.rs`).
- Model/rules: `crates/measure/src/lib.rs`, `editing.rs`, `fill.rs`.
- Layout/geometry: `crates/layout/src/pixel_layout.rs`, `render_plan.rs`.
- Drawing: `crates/render/src/measure.rs`, `beat.rs`.
- Audio: `crates/audio/src/lib.rs`.
- MIDI: `crates/midi/src/input.rs`.
- Microphone: `crates/mic/src/` (detector, `native.rs`, `web.rs`).

## Notes for LLM agents

- Model changes can affect layout, audio, and rendering.
  Search for `DEFAULT_GRID` usage and tick logic.
- Web-specific logic is gated by `cfg(target_arch = "wasm32")`.
- Keep measure validation in mind: `set_beat`/`set_tuplet` must not leave
  unfillable measure lengths.
