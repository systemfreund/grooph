# Pull request review guidelines

These instructions drive the automated review in `.github/workflows/code-review.yml`.
Edit this file to change what the review looks at; the workflow does not need to change.

The CI workflow (`rust.yml`) already runs `cargo fmt`, `cargo clippy -D warnings`, the tests and the
web build. Do not repeat what those catch (formatting, lint warnings, compile errors). Spend the review on
what they cannot see: whether the change does what it should, and where it is weak.

## 1. Find the requirements

Work out what this pull request is supposed to achieve, in this order:

1. **Linked issues.** `gh pr view <PR> --json closingIssuesReferences,body,title` lists the issues the PR
   closes. Also follow issue references in the PR body (`#123`, "Fixes", "Closes", "Part of").
   Read each with `gh issue view <n> --json title,body,comments`; later comments often refine or narrow the
   original ask. Do not use `--comments` without `--json`: outside a terminal it prints only the comments, so
   an issue without comments comes back empty. If a `gh` command prints nothing or an error, say so in the
   review instead of treating the issue as missing.
2. **No issue:** many PRs here come straight out of a coding session and have no issue. Then the PR title
   and description are the requirements, supplemented by the commit messages
   (`gh pr view <PR> --json commits`). Treat every behavior the description claims as a requirement.
3. **Nothing usable** (empty or one-line description, no issue): say so at the top of the review, infer
   the most likely intent from the diff, and review against that inferred intent, marking it as inferred.

Always state at the top which source you used: `Requirements source: issue #N`, `PR description`, or
`inferred from the diff`.

## 2. Check the requirements

Turn the requirements into a numbered list of concrete, checkable items. For each, give one verdict:

- ✅ **Met**: name the file/function that implements it.
- ⚠️ **Partly met**: say what is missing.
- ❌ **Not met**: say what is missing.
- ❓ **Unclear**: the requirement is ambiguous, or met only under one reading. Say which reading you assumed.

Also flag the reverse direction: **changes in the diff no requirement asks for** (scope creep, unrelated
refactors, leftover debug code). For PRs without an issue, flag claims in the description that the diff
does not back up.

Check that each requirement with observable behavior has a test, or say why a test is impractical (e.g.
pure UI drawing).

## 3. Look for weaknesses

Read the changed code in context, not just the diff lines. Open callers and callees. Look for:

- **Correctness:** edge cases (empty measures, the first/last beat, tuplet groups, odd time signatures such
  as 5/8 or 7/8, extreme tempos), off-by-one errors, wrong assumptions about ordering or timing.
- **Error handling:** `unwrap()`/`expect()`/indexing that can panic on user input, a shared link or device
  state; errors silently swallowed.
- **Shared links (`crates/link`):** links already out in the world must keep opening the same way.
  Any change to parsing or the `?v=2&…` format needs backward compatibility or a new version, plus a test
  that parses an old link.
- **Audio and timing (`audio`, `mic`, `midi`, clocks):** allocation, locking or blocking on the audio
  path; drift between the click and the display; behavior when no device is present or a device
  disappears.
- **Native vs. WebAssembly:** code that only works on one target; WASM-only code must sit behind
  `cfg(target_arch = "wasm32")`; browser APIs (`getUserMedia`, AudioWorklet) failing or being denied.
- **State and undo:** changes to app state that bypass undo, presets or the library, or that are not
  persisted where similar settings are.
- **Rust specifics:** `unsafe` without a `// SAFETY:` comment (edition 2024 rules apply), needless
  clones or allocations in hot loops (rendering, scheduling), lossy `as` casts on durations and ticks,
  public API changes in library crates that ripple through the workspace.
- **Tests:** tests that assert too little, or strip tuplet marks (`beat::unmarked`) where the tuplet
  group matters.

**Music domain.** grooph is a notation and rhythm tool, so check the music, not just the code:

- **Notation conventions (`measure` grouping/notation, `layout` beam and tuplet plans):** beaming, rest
  grouping and syncopations must follow standard engraving practice (Gould, *Behind Bars*): the beat and
  half-bar stay visible, e.g. eighths beamed in fours only where the meter allows it, rests not hiding the
  middle of a 4/4 bar, an empty bar as one whole rest. A changed rule needs tests in at least one simple
  (2/4, 3/4 or 4/4), one compound (6/8) and one odd meter (5/8 or 7/8), and the PR should show before/after
  examples.
- **Rhythm invariants:** a measure's durations add up to exactly its time signature; tuplet groups are
  complete; durations and onsets are computed in integer ticks on the grid (`grid.rs`), with floats only at
  the edges (pixels, seconds). Click, counting labels, the practice tracker and the notation must use the
  same onsets; flag one of them computing its own timing.
- **Meaning of stored values:** when a change alters what an existing number means (as when BPM switched to
  counting quarters), shared links, user presets, library patterns and the site's example links must still
  open the same way: through a migration or a new link version, with a test using an old value. This goes
  beyond format changes.
- **Layout across voices and modes:** a change to how notes, rests, beams, accents or tuplet marks are placed
  must hold for both stem directions (the lower voice of a two-voice staff, `stem_down`), must keep the row
  room (`RowRoom` in `staff_layout.rs`) large enough for the new ink, and should take glyph positions from
  the Bravura metadata in `layout/src/glyphs` rather than new hand-tuned offsets. Check narrow (phone)
  and wide windows alike.
- **Guide pages (`crates/site`):** when app behavior changes, the guide texts and their example links must
  still describe and open what the app now does.

**Design and maintainability.** Check whether the change leaves the code harder to change than it found it.
`ARCHITECTURE.md` is the reference: its crate dependencies, data model and invariants.

- **Dependency direction:** a new dependency between crates, or a crate reaching into something its layer
  should not know (GUI in `measure`, `audio` or `link`; drawing in `layout`; the model in `midi` or `mic`).
- **Logic in the wrong place:** music rules (durations, onsets, grouping, counting) computed in `app` or a
  panel instead of `measure`; state logic buried in drawing code; a panel doing what a model function should.
- **Cross-cutting concerns:** persistence, undo, error reporting, repaint requests and the native/web split
  each have an existing mechanism. Flag a feature that builds its own instead, or spreads one concern over
  many call sites (`cfg(target_arch = "wasm32")` branches in callers, the same save or undo step repeated by
  hand) where one place would do.
- **Duplication:** a new helper, type or constant that repeats one the workspace already has. Name the
  existing one.
- **One source of truth:** new state that copies or derives from existing state and has to be kept in sync
  by hand; flags that live across updates and only work if things happen in a certain order.
- **Interfaces:** callers that must know internal details or call things in a fixed order; fields or
  functions made `pub` only to reach them from elsewhere; functions whose parameter lists or `match`/`if`
  chains grow with every feature where the data model should carry the case.
- **Debt left behind:** `TODO`s, commented-out code, workarounds without a reason, magic numbers next to an
  existing constant, a structural change that `ARCHITECTURE.md` no longer describes.
- **Testability:** new logic reachable only through the UI or a real device, when it could sit in a plain
  function with a unit test.

Only report a weakness you can point to in the code. For correctness, give a concrete failure scenario. For
design, give a concrete, checkable reason instead: the `ARCHITECTURE.md` rule it breaks, the existing helper
it duplicates, the two places that must now be kept in sync, the call sites a future change will have to
touch. No style nits (naming, formatting, taste), no speculative "consider maybe" remarks. If you are not
sure something is a bug, say how sure you are.

## 4. Output

Do not post anything on the pull request: several reviewers run side by side, and a synthesis step merges
their answers into one review (`.github/review/SYNTHESIS.md`). Your **final answer is a single JSON object**
and nothing else (a ```json fence around it is fine):

```json
{
  "requirements_source": "issue #12",
  "requirements": [
    {"text": "Tapping More opens the MIDI picker", "verdict": "met", "note": "shell.rs: more_menu()"}
  ],
  "findings": [
    {
      "severity": "medium",
      "title": "Failed connect leaves the old port",
      "file": "crates/midi/src/input.rs",
      "line": 476,
      "body": "What goes wrong, when, and a suggested fix.",
      "confidence": "high"
    }
  ],
  "out_of_scope": ["Also changes value_row in the Options sheet; not mentioned in the PR"]
}
```

- `requirements_source`: `issue #N`, `PR description`, or `inferred from the diff` (§1).
- `requirements`: the numbered list from §2, in order. `verdict` is `met`, `partly`, `not_met` or `unclear`;
  `note` names the file/function or says what is missing.
- `findings`: the weaknesses from §3, and requirements that are not met. `severity` is `high` (breaks a
  requirement, loses data, panics or breaks existing links), `medium` (wrong in a realistic edge case) or
  `low` (weakness worth fixing but not blocking). Design findings are `low`, or `medium` when they break a
  rule `ARCHITECTURE.md` states or leave two copies of the same state or logic to keep in sync. `file` is relative to the repository root; `line` is a line
  of that file at the head commit, preferably one inside the diff, or `null` when the finding has no single
  place. `confidence` is `high`, `medium` or `low`: how sure you are that it is a real problem.
- `out_of_scope`: changes no requirement asks for, and claims in the description the diff does not back up.

Use empty lists for empty sections. Write in English.
