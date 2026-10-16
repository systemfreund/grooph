# Project-specific development guidelines

Repository: https://github.com/systemfreund/grooph — remote `origin`, default branch `main`.

Read README.md first.
This document captures build, test, and development practices tailored to this repository. It’s written for experienced
Rust developers and focuses on crate-specific behaviors and APIs that matter when developing or debugging.

## Build and configuration

- Toolchain/edition: Rust edition 2024 on stable. The workspace builds and tests cleanly with the default toolchain
  (no special flags required). Native builds need the ALSA headers (`libasound2-dev` on Debian/Ubuntu).
- Workspace layout (`crates/`, shared versions in the root Cargo.toml under `[workspace.dependencies]`):
    - `measure` (grooph-measure): the music model: durations, beats, measures, scores, time signatures, the
      generator, counting, swing, tempo. No GUI dependencies.
    - `layout` (grooph-layout): turns measures into render plans and pixel layouts.
    - `render` (grooph-render): draws the layouts with egui.
    - `audio` (grooph-audio): click sounds and scheduling (rodio; cpal on WASM).
    - `midi` (grooph-midi): MIDI input.
    - `link` (grooph-link): shareable links (`?v=2&bpm=..&r=..`) and the text notation for scores.
    - `app` (grooph-app): the eframe app, binary + library. WebAssembly code is gated behind
      `cfg(target_arch = "wasm32")`.
    - `site` (grooph-site): static guide pages, written into the web build by a Trunk `post_build` hook.
- Running locally:
    - Native run: `cargo run`
    - Tests: `cargo test --workspace`
    - WASM build (optional): Trunk.toml at the repo root drives web builds. `trunk serve` for dev, or
      `trunk build --release` to produce optimized WASM (release profile uses `opt-level = "z"` and `lto = true`;
      Trunk sets `release = true` and `minify = "always"`). CI pins Trunk (see `.github/workflows/rust.yml`).

## CI

`.github/workflows/rust.yml` runs on every push and pull request to `main`; run the same checks locally before pushing:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings` (unused imports and dead code are errors)
- `cargo build` and `cargo test`
- `trunk build --release` (the web job). To check the WASM target without Trunk:
  `cargo clippy -p grooph-app --target wasm32-unknown-unknown -- -D warnings`

Pushes to `main` deploy the web build to Cloudflare Pages once both jobs pass.

## Testing: how to configure and run

- Unit tests are colocated within modules (e.g., crates/measure/src/lib.rs, crates/layout/src/*). Use standard cargo
  commands:
    - All tests: `cargo test --workspace`
    - One crate: `cargo test -p grooph-measure`
    - Filter by name: `cargo test -p grooph-measure test_triplet_insertions_1`
    - Single fully-qualified test: `cargo test -p grooph-measure tests::append_autofill_to_primary_boundary_simple`
    - Show test output: add `-- --nocapture`

## Writing new tests

- Prefer internal unit tests in the same module as the code under test.
- Common pattern: create a `Measure`, then add beats via helper duration constructors. The measure automatically fills
  gaps to primary boundaries or tuplet completeness when possible.
- Shortcuts for common durations (defined in `crates/measure/src/duration.rs`):
    - Simple: `q()` (quarter), `e()` (eighth), `s()` (sixteenth), `th()` (thirty-second)
    - Triplets: `t8()`, `t16()`, `t32()`
    - Quintuplet: `qt16()` (5 in the time of 4, sixteenth base)
- Constructing beats: `Beat::note(duration)` or `Beat::rest(duration)`
- Constructing measures: `Measure::new(TimeSignature::FOUR_FOUR)` etc. See `time_signature.rs` for variants.

### Important testing gotchas

- `Beat`'s `PartialEq` ignores `tuplet_group_id`. If tuplet grouping matters, compare `tuplet_group_id` explicitly.
- When you insert a tuplet beat (e.g., `t8()`), the remaining tuplets in that group auto-fill as rests to complete the
  group.
- `set_beat` may absorb/fill following beats to keep the measure valid (including auto-filling rests).

Minimal example test (verified)
Place this inside an existing `#[cfg(test)] mod tests { .. }` in `crates/measure/src/lib.rs` or another module of the
measure crate. Imports mirror existing tests so helper functions are visible.

```
#[test]
fn demo_howto_test_measure_with_helpers() {
    use crate::BeatKind::Rest;
    use crate::duration::{e, t8};
    use crate::{Beat, Measure, TimeSignature};

    // 4/4: insert eighth note; auto-fill adds an eighth rest to complete the quarter boundary
    let mut m = Measure::new(TimeSignature::FOUR_FOUR);
    assert!(m.set_beat(0, Beat::note(e())).is_ok());
    let Beat { duration: d1, kind: k1, .. } = m.beats()[1];
    assert_eq!(d1, e());
    assert_eq!(k1, Rest);

    // insert one triplet eighth; remaining two tuplets auto-fill as rests at positions 2 and 3
    assert!(m.set_beat(1, Beat::note(t8())).is_ok());
    let Beat { duration: d2, kind: k2, .. } = m.beats()[2];
    let Beat { duration: d3, kind: k3, .. } = m.beats()[3];
    assert_eq!(d2, t8());
    assert_eq!(k2, Rest);
    assert_eq!(d3, t8());
    assert_eq!(k3, Rest);
}
```

### Guidelines for adding further tests

- Use `Measure::set_beat(idx, Beat::note/rest(..))` to surgically adjust a measure.
- Duration grid utilities: `DEFAULT_GRID.ticks_of(..)` and `DEFAULT_GRID.compute_onset_ticks(..)` are useful for
  low-level assertions.

## Code style and conventions

- Formatting: See `rustfmt.toml` at repo root. Run `cargo fmt` before committing.
- Imports and module layout: mirror the existing test modules for import style (e.g., reusing
  `use crate::duration::{..}`; the measure crate does not depend on layout or render).
- Debug/print helpers: `duration_to_debug_str` and `impl Debug` for `Measure` provide concise readable output for
  debugging failed tests.

## Other notes

- You are already in the right working directory, so no need to `cd` into the repo root.
