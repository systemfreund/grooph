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
- `crates/link`: Shareable links (URL query parameters <-> score/generator/metronome).
- `crates/site`: Static guide pages for grooph.app, generated from Markdown at build time.

Important root files:
- `Cargo.toml` (workspace + dependencies, edition 2024)
- `Trunk.toml` (WASM build/serve)
- `rustfmt.toml` (formatting)

## Data model and invariants

- Core object: `grooph_measure::Measure` with `Vec<Beat>` and `TimeSignature`.
- `Beat` has `duration`, `kind` (Note/Rest), `accented`, `tuplet_group_id`.
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
  codes match the helpers in `duration.rs`; `r:` = rest, `>` = accent, `|` =
  bar line, a leading `N/D` sets the time signature. Parsing goes through
  `set_beat`; each measure must be filled exactly, errors name the measure
  and token. Formatting returns `None` for durations without a code.
- `grooph_link` (`crates/link`): query parameters `bpm`, `r`, `ts`, `swing`,
  `su` and the generator's `sub`, `lvl`, `bars`, `space`, `seed` (seeded
  `Rng`, so a link reproduces the same exercise). `parse_query` reads them,
  `SharedLink::to_url` writes them. The app reads the link once at startup
  (`Grooph::open_link_from_url`), then removes it from the address bar. A link
  that replaces the score goes through `PendingLoad::Link` and the
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

- Panel: `crates/app/src/generator_panel.rs` (🎼 in the main menu; 🎲 next to
  play rolls a new rhythm). Changing a setting regenerates the score
  (undoable); "Edit" switches to the editor. The time signature opens the
  compact layout's picker grid (`compact_picker`), so both layouts offer the
  same meters and groupings.
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
  `SoundType::CountIn` clicks (downbeat + primary beats of the first measure)
  before the score; `Audio::playback_position` is negative meanwhile. The UI
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
  `claves4.wav` on the other primary beats and the count-in.
  `RhythmSound::Snare` (default) plays its own recording per dynamic:
  `snare-normal.wav` for notes, `snare-accent.wav` for accents and
  `snare-ghost.wav` for ghost notes. A ghost sample is recorded quietly, so its
  gain is divided by the default ghost volume (the default plays it as recorded).
  Samples are stored as 16-bit mono WAV, trimmed after the decay, to keep the
  WASM build small.
  Samples keep their own license (claves: altemark on freesound.org, CC BY-NC),
  see `crates/audio/samples/CREDITS.md`; add a row there with every new sample.
- `VoiceMixer` (`voices.rs`) plays samples resampled to the device rate.
- Mixer (`mixer_panel.rs`): a "Metronome" group (`downbeat`, `primary` as
  "Beats", `soft`, `pulse`, `count_in`) and a "Rhythm" group (`beat` as "Notes", `accent`,
  `ghost`), each with a mute (`metronome_muted` / `rhythm_muted`, applied in
  `SoundType::profile`). The metronome mute leaves the count-in audible.
  `pulse` (default 0, off) clicks every beat unit between the primary beats
  (`SoundType::Pulse`, e.g. the eighths inside the groups of 7/8); sounds
  with zero gain are not triggered at all.
- Click patterns (`grooph_measure::click`): one `ClickLevel` (strong, medium,
  soft, off) per beat group of a meter. `ClickPatterns` (in `AudioConfig`,
  persisted) keeps the ones that differ from the default (strong, then
  medium), keyed by the whole `TimeSignature` including its grouping.
  `Schedule::build` plays strong as `Downbeat`, medium as `PrimaryBeat`, soft
  as `SoftBeat` (mixer "Soft"); off groups get no click, their pulse clicks
  stay. The count-in and calibration ignore the patterns. Link parameter
  `click` (`S`, `M`, `L`, `-` per group) sets the pattern of the link's first
  meter; shared links carry it when it is not the default.

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

## Play along (accuracy)

- Hit sources (`crates/app/src/play_along.rs`): MIDI NoteOns and taps (tap key,
  default `J`, or a press on the staff while playing). All hits are stamped on
  one clock (`crates/app/src/clock.rs`); MIDI timestamps are moved onto it by
  their age. On the web taps use the DOM event `timeStamp` (`web.rs`), natively
  the frame time. Each source has its own input offset (`PlayAlongSettings`).
- While tap capture is active, staff presses are hits, not edits.
- The session (`accuracy::AccuracyTracker`) runs while MIDI is connected or
  after the first tap of a run. Stopping keeps a `RunSummary` (hits, perfect/
  good/off windows, misses, extra hits, mean, spread) for the "Your run" window
  and the run's marks, drawn on the staff until the next run or an edit.
- Grading (`accuracy::session::HitWindows`): perfect ≤ 25 ms, good ≤ 60 ms,
  narrowed to 1/5 and 2/5 of the gap to the closest neighbouring onset (as
  heard) for fast notes. Markers are coloured by grade.
- Calibration (`crates/app/src/calibration.rs`): stashes the score and playback
  options, plays 4 quarters at 100 BPM, takes the median of the hits after a
  warm-up as the source's offset and restores the stash on stop. Persistence
  reads the stash while a calibration runs.

## Compact layout (phones)

- `crates/app/src/compact.rs`: windows narrower than `COMPACT_MAX_WIDTH`
  (600 pt) get a phone layout instead of the bottom panels; checked every
  frame (`UiShell::compact_layout`), so the desktop layout is unchanged.
- Tabs: Practice (generator card with Meter/Notes/Level/Bars/BPM, options
  sheet, big "new rhythm" + play buttons), Edit (tool palette + edit bar),
  More (Library, Mixer, Settings, Help reuse their panels). The tab is derived
  from `Mode` (`compact_tab`), so modes set elsewhere land on the right tab.
- Settings open in a `Modal` picker; generator changes go through
  `finish_generator_change`, shared with the desktop generator panel.
- The staff uses tighter spacing and no clef there, so glyphs are bigger.
- Share (right of play, and in the edit bar) builds a score link with tempo
  and swing; on the web it opens the system share sheet
  (`platform::open_share_sheet`, Web Share API), else copies the link.
- Shared with the desktop main menu: share (`share_link`) and tap tempo
  (`record_tap`).


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

## Notes for LLM agents

- Model changes can affect layout, audio, and rendering.
  Search for `DEFAULT_GRID` usage and tick logic.
- Web-specific logic is gated by `cfg(target_arch = "wasm32")`.
- Keep measure validation in mind: `set_beat`/`set_tuplet` must not leave
  unfillable measure lengths.
