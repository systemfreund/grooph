Project-specific development guidelines

This document captures build, test, and development practices tailored to this repository. It’s written for experienced
Rust developers and focuses on crate-specific behaviors and APIs that matter when developing or debugging.

Build and configuration

- Toolchain/edition: Rust edition 2024. The crate builds and tests cleanly with the default toolchain (no special flags
  required).
- Crate layout: Binary + library (unit tests run under the lib target). GUI/frontend uses eframe; WebAssembly support is
  gated behind cfg(target_arch = "wasm32").
- Dependencies: eframe, log. See Cargo.toml for versions. No feature flags at this time.
- Running locally:
    - Native run: `cargo run`
    - Tests: `cargo test`
    - WASM build (optional): the repo includes Trunk.toml for web builds. Typical commands (if you have trunk
      installed): `trunk serve` for dev, or `trunk build --release` to produce optimized WASM (release profile sets
      `opt-level = 2`).

Testing: how to configure and run

- Unit tests are colocated within modules (e.g., src/measure.rs, src/layout/*). Use standard cargo commands:
    - All tests: `cargo test`
    - Filter by module/name: `cargo test measure::tests` or `cargo test test_triplet_insertions_1`
    - Single fully-qualified test: `cargo test measure::tests::append_autofill_to_primary_boundary_simple`
    - Show test output: add `-- --nocapture`

Writing new tests in this crate

- Prefer internal unit tests in the same module as the code under test.
- Common pattern: create a `Measure`, then add beats via helper duration constructors. The measure automatically fills
  gaps to primary boundaries or tuplet completeness when possible.
- Shortcuts for common durations (defined in `src/measure/duration.rs`):
    - Simple: `q()` (quarter), `e()` (eighth), `s()` (sixteenth), `th()` (thirty-second)
    - Triplets: `t8()`, `t16()`, `t32()`
    - Quintuplet: `qt16()` (5 in the time of 4, sixteenth base)
- Constructing beats: `Beat::note(duration)` or `Beat::rest(duration)`
- Constructing measures: `Measure::new(TimeSignature::FOUR_FOUR)` etc. See `time_signature.rs` for variants.

Important testing gotchas

- Don’t compare `Beat` instances directly with `==` unless you intend to include all fields in the comparison. Tuplet
  bookkeeping like `tuplet_group_id` can differ even when musically equivalent. Prefer comparing selected fields, e.g.,
  `duration` and `kind`.
- When you insert a tuplet beat (e.g., `t8()`), the remaining tuplets in that group auto-fill as rests to complete the
  group.
- `set_beat_at` may auto-fill up to the next primary boundary based on the current duration grid.

Minimal example test (verified)
Place this inside an existing `#[cfg(test)] mod tests { .. }` in `src/measure.rs` or another internal module. Imports
mirror existing tests so helper functions are visible.

```
#[test]
fn demo_howto_test_measure_with_helpers() {
    use crate::measure::duration::{e, t8};
    use crate::measure::{Beat, TimeSignature};
    use crate::measure::BeatKind::Rest;

    // 4/4: insert eighth note; auto-fill adds an eighth rest to complete the quarter boundary
    let mut m = Measure::new(TimeSignature::FOUR_FOUR);
    assert!(m.set_beat_at(0, Beat::note(e())).is_ok());
    let Beat { duration: d1, kind: k1, .. } = m.beats()[1];
    assert_eq!(d1, e());
    assert_eq!(k1, Rest);

    // insert one triplet eighth; remaining two tuplets auto-fill as rests at positions 2 and 3
    assert!(m.set_beat_at(1, Beat::note(t8())).is_ok());
    let Beat { duration: d2, kind: k2, .. } = m.beats()[2];
    let Beat { duration: d3, kind: k3, .. } = m.beats()[3];
    assert_eq!(d2, t8());
    assert_eq!(k2, Rest);
    assert_eq!(d3, t8());
    assert_eq!(k3, Rest);
}
```

Running just this test:

- If placed under `src/measure.rs`’s test module: `cargo test measure::tests::demo_howto_test_measure_with_helpers`

Guidelines for adding further tests

- Use `Measure::set_beat_at(idx, Beat::note/rest(..))` to surgically adjust a measure.
- Duration grid utilities: `default_duration_set()` and its `grid` can translate durations to integer ticks and compute
  onsets if you need low-level assertions.

Code style and conventions

- Formatting: See `rustfmt.toml` at repo root. Run `cargo fmt` before committing.
- Imports and module layout: mirror the existing test modules for import style (e.g., reusing
  `use crate::measure::duration::{..}` and avoiding cross-layer layout imports inside measure tests).
- Debug/print helpers: `duration_to_debug_str` and `impl Debug` for `Measure` provide concise readable output for
  debugging failed tests.
