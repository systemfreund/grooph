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
