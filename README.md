# Grooph

Short description: Grooph is a rhythm and meter editor with playback. The app
shows a single measure, allows notes/rests/tuplets (triplets), plays a metronome
track, and runs native as well as WebAssembly (eframe/egui).

This README is meant for LLM coding agents and describes the key concepts,
files, and invariants so changes can be made with confidence.

## Project map (workspace)

- `crates/app`: GUI, app state (`Grooph`), panels, tool palette, input, persistence.
- `crates/measure`: Domain model for measures/beats/duration, editing logic, counting.
- `crates/layout`: Pixel layout from the model (note positions, beam/tuplet plans).
- `crates/render`: Draws the layout with egui (notes, beams, tuplets, cursor).
- `crates/audio`: Metronome synth (rodio), scheduling based on the measure.
- `crates/midi`: MIDI input abstraction (midir).

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
  complexity (1-5, caps the cell level), bars, time signature (x/4 only),
  space (extra whole-beat rests). Measures are written via `set_beat`, so
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

## Generator / endless mode

- Panel: `crates/app/src/generator_panel.rs` (🎲 in the main menu). Changing a
  setting regenerates the score (undoable); "Edit" switches to the editor.
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
  mixed = triplets in tuplet beats, else 16ths). Scheduled as `SoundType::Ghost`,
  volume via the mixer's "Ghost" slider (`AudioSettings::ghost`).
- Count-in (`PlaybackOptions::count_in`): `TickSource` plays one bar of
  `SoundType::CountIn` clicks (downbeat + primary beats of the first measure)
  before the score; `Audio::playback_position` is negative meanwhile. The UI
  holds the cursor at 0 (`CountInState`) and re-anchors MIDI accuracy when the
  count-in ends.

## Input/tools

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
