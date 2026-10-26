# Project-specific development guidelines

Repository: https://github.com/systemfreund/grooph — remote `origin`, default branch `main`.

Read README.md first.
This document captures build, test, and development practices tailored to this repository. It’s written for experienced
Rust developers and focuses on crate-specific behaviors and APIs that matter when developing or debugging.

## Build and configuration

- Toolchain/edition: Rust edition 2024 on stable, at least the `rust-version` in the root Cargo.toml. Lints are
  shared through `[workspace.lints]` there; every crate opts in with `[lints] workspace = true`. The workspace builds
  and tests cleanly with the default toolchain (no special flags required). Native builds need the ALSA headers
  (`libasound2-dev` on Debian/Ubuntu).
- Workspace layout (`crates/`, shared versions in the root Cargo.toml under `[workspace.dependencies]`):
    - `measure` (grooph-measure): the music model: durations, beats, measures, scores, time signatures, the
      generator, counting, swing, tempo. No GUI dependencies.
    - `layout` (grooph-layout): turns measures into render plans and pixel layouts.
    - `render` (grooph-render): draws the layouts with egui.
    - `audio` (grooph-audio): click sounds and scheduling (rodio; cpal on WASM).
    - `midi` (grooph-midi): MIDI input.
    - `mic` (grooph-mic): microphone hit detection (cpal natively; `getUserMedia` and an AudioWorklet on the web).
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
- `python3 -m unittest discover -s .github/review` (the scripts of the code review workflow)
- `trunk build --release` (the web job). To check the WASM target without Trunk:
  `cargo clippy -p grooph-app --target wasm32-unknown-unknown -- -D warnings`

Pushes to `main` deploy the web build to Cloudflare Pages (grooph.app) once both jobs pass. Pull requests from
branches of this repository deploy to their own preview (`<branch>.grooph.pages.dev`) and get the link as a comment;
pull requests from forks are not deployed.

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

## Pull requests

Every pull request gets an automated review (`.github/workflows/code-review.yml`, guided by `REVIEW.md`) that
checks the change against its requirements; pull requests that change `.rs` files also get a Rust-specific review
(`.agents/skills/grooph-rust-review`). Several models review independently, and a synthesis step
(`.github/review/SYNTHESIS.md`) merges them into one summary comment plus inline comments. When opening a pull
request:

- If it implements an issue, write `Closes #<n>` in the description.
- If there is no issue (e.g. it comes out of a coding session), the description is the requirements. List what the
  change should do as concrete, checkable points, including behavior that must stay the same, and note anything
  deliberately left out.
- Drafts and pull requests from forks are not reviewed; marking a draft ready starts the review.

### Working through the review

If you opened the pull request, see it through the review:

1. **Wait for the review.** It takes several minutes: `gh run list --workflow code-review.yml --branch <branch>`,
   then `gh run watch <id>`. The result is one comment by `github-actions[bot]` starting with
   `<!-- grooph-review -->` (`gh pr view <n> --json comments`): a requirements table, the findings `F1`, `F2`, …
   with severity, and a verdict. Findings on lines of the diff also get an inline comment
   (`gh api repos/<owner>/<repo>/pulls/<n>/comments`). Never edit or delete the bot's comments: the summary
   carries the review's state in a hidden block.
2. **Fix what is right.** Every push is reviewed in full, and the summary marks the fixed findings as resolved and
   replies in their threads. A newer push cancels the running review, so push the fixes for one round together.
3. **Answer what is wrong.** When a finding is mistaken or the behavior is intended, say why: reply in its inline
   thread (`gh api -X POST repos/<owner>/<repo>/pulls/<n>/comments/<comment id>/replies -f body=…`), or comment
   on the pull request naming the finding (`F3: …`), which is the only way for findings without an inline comment.
   Give a reason that can be checked: where the case is handled, which requirement allows it, what the reviewer
   misread. An answer alone (no push needed) runs the synthesis again a few minutes later with the same reviews;
   it withdraws the finding if the reason holds against the code, and otherwise keeps it and answers the reason.
   "Please drop this" changes nothing. Only answers from accounts with write access count, and only comments that
   reply in a finding's thread or name a finding start a run, so put several answers into one comment when you
   can.
4. **Change the requirements in the open.** If the scope changes, update the description and say so in a comment
   naming the affected findings; the synthesis reads the description and the comments.
5. **Done** when the verdict says the pull request is ready, no high or medium finding is open, and CI is green.
   Fix or answer the low ones too. Merging is for the user to decide unless they said otherwise.

If the summary says the review failed (`⚠️ The review of … failed`), it still shows the previous findings; start a
new review with `gh workflow run code-review.yml -f pr=<n>`, or `gh run rerun <id> --failed`.

## Screenshots

To show a UI change, screenshot the real app. Two ways work in a headless Linux container:

**Web build + Playwright (supports clicks).** Playwright and Chromium are preinstalled (`require('playwright')`).

```
rustup target add wasm32-unknown-unknown
wget -qO- https://github.com/trunk-rs/trunk/releases/download/v0.21.14/trunk-x86_64-unknown-linux-gnu.tar.gz \
  | tar -xzf- -C ~/.cargo/bin                     # version pinned in .github/workflows/rust.yml
trunk build --release --dist /tmp/web             # ~3 min; one dist dir per variant (before/after)
(cd /tmp/web && python3 -m http.server 8123 &)
node shot.js http://localhost:8123/ out.png 390 844 344,812   # phone size, then tap at (344,812)
```

`shot.js`:

```js
const { chromium } = require('playwright');
const [url, out, w, h, ...taps] = process.argv.slice(2);
(async () => {
  const b = await chromium.launch();
  // deviceScaleFactor must be 1: at 2, egui lays out wrong and clicks miss.
  const p = await b.newPage({ viewport: { width: +w, height: +h }, deviceScaleFactor: 1 });
  await p.goto(url);
  await p.waitForTimeout(5000); // let the WASM start
  for (const t of taps) {
    const [x, y] = t.split(',').map(Number);
    await p.mouse.click(x, y);
    await p.waitForTimeout(700);
  }
  await p.screenshot({ path: out });
  await b.close();
})();
```

- The page is a single canvas, so there are no selectors: tap by pixel. Take a screenshot, read the PNG to find the
  next coordinates, and replay the taps from the start (each run is a fresh browser with empty storage).
- Typical sizes: 390×844 (phone), 1280×800 (desktop). At 390 px the bottom tabs sit at y≈812, with More at x≈344.
- A shared link (`?v=2&bpm=..&r=..`, see the `link` crate) opens a specific score or setting without taps.

**Native build under Xvfb (no clicks).** Needs `apt-get update && apt-get install -y libasound2-dev
libxkbcommon-x11-0 libgl1-mesa-dri libegl1` (`xvfb-run` and ImageMagick's `import` are usually present). Reach the
screen by temporarily changing the default state in code (e.g. the start tab or a collapsed panel), `cargo build`,
copy `target/debug/grooph-app` away, then revert. The window opens at 800×600 (`crates/app/src/main.rs`) and is
clipped to the Xvfb screen, so a 420-wide screen gives a 420×600 phone-like view (crop the black rest); change
`with_inner_size` temporarily for other sizes. Use a fresh `HOME` so saved state does not interfere:

```
HOME=/tmp/home xvfb-run -a -s "-screen 0 420x860x24" \
  bash -c "./grooph-app & sleep 15; import -window root out.png; kill %1"
```

For before/after images, build both variants, crop and place them side by side with Python/PIL, and attach the
image in the conversation; screenshots are not committed to the repository.

## Code style and conventions

- Formatting: See `rustfmt.toml` at repo root. Run `cargo fmt` before committing.
- Imports and module layout: mirror the existing test modules for import style (e.g., reusing
  `use crate::duration::{..}`; the measure crate does not depend on layout or render).
- Debug/print helpers: `duration_to_debug_str` and `impl Debug` for `Measure` provide concise readable output for
  debugging failed tests.

## Other notes

- You are already in the right working directory, so no need to `cd` into the repo root.
