# Project-specific development guidelines

Repository: https://github.com/systemfreund/grooph — remote `origin`, default branch `main`.

Read README.md first.
This document captures build, test, and development practices tailored to this repository. It’s written for experienced
Rust developers and focuses on crate-specific behaviors and APIs that matter when developing or debugging.

## Build and configuration

- Toolchain/edition: Rust edition 2024. The crate builds and tests cleanly with the default toolchain (no special flags
  required).
- Crate layout: Binary + library (unit tests run under the lib target). GUI/frontend uses eframe; WebAssembly support is
  gated behind cfg(target_arch = "wasm32").
- Dependencies: eframe, egui_extras (svg), rodio (audio), either, log; env_logger for native only. WASM adds
  wasm-bindgen, wasm-bindgen-futures, web-sys, cpal, getrandom. See Cargo.toml for versions.
- Running locally:
    - Native run: `cargo run`
    - Tests: `cargo test`
    - WASM build (optional): the repo includes Trunk.toml for web builds. Typical commands (if you have trunk
      installed): `trunk serve` for dev, or `trunk build --release` to produce optimized WASM (release profile uses
      `opt-level = "z"` and `lto = true`; Trunk sets `release = true` and `minify = "always"`).

## Testing: how to configure and run

- Unit tests are colocated within modules (e.g., src/measure.rs, src/layout/*). Use standard cargo commands:
    - All tests: `cargo test`
    - Filter by module/name: `cargo test measure::tests` or `cargo test test_triplet_insertions_1`
    - Single fully-qualified test: `cargo test measure::tests::append_autofill_to_primary_boundary_simple`
    - Show test output: add `-- --nocapture`

## Writing new tests

- Prefer internal unit tests in the same module as the code under test.
- Common pattern: create a `Measure`, then add beats via helper duration constructors. The measure automatically fills
  gaps to primary boundaries or tuplet completeness when possible.
- Shortcuts for common durations (defined in `src/measure/duration.rs`):
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
Place this inside an existing `#[cfg(test)] mod tests { .. }` in `src/measure.rs` or another internal module. Imports
mirror existing tests so helper functions are visible.

```
#[test]
fn demo_howto_test_measure_with_helpers() {
    use crate::measure::duration::{e, t8};
    use crate::measure::{Beat, Measure, TimeSignature};
    use crate::measure::BeatKind::Rest;

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
  `use crate::measure::duration::{..}` and avoiding cross-layer layout imports inside measure tests).
- Debug/print helpers: `duration_to_debug_str` and `impl Debug` for `Measure` provide concise readable output for
  debugging failed tests.

## Other notes

- You are already in the right working directory, so no need to `cd` into the repo root.
