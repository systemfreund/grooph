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
   Read each with `gh issue view <n> --comments`; later comments often refine or narrow the original ask.
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
- **Tests:** tests that assert too little, or compare `Beat`s with `==` where the tuplet group matters
  (`Beat`'s `PartialEq` ignores `tuplet_group_id`).

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
  the Bravura metadata in `layout/src/glyphs` rather than new hand-tuned offsets. Check the compact (phone)
  and desktop views alike.
- **Guide pages (`crates/site`):** when app behavior changes, the guide texts and their example links must
  still describe and open what the app now does.

Only report a weakness you can point to in the code, with a concrete failure scenario. No style nits, no
speculative "consider maybe" remarks. If you are not sure something is a bug, say how sure you are.

## 4. Output

Your **final answer is the summary comment**: the workflow posts it on the pull request, so do not post it
yourself. For findings tied to a specific line, post **inline comments** on the changed lines:

```
gh api repos/<REPO>/pulls/<PR>/comments -f commit_id=<HEAD SHA> -f path=<file> -F line=<line> -f side=RIGHT -f body='…'
```

The head SHA is in the prompt; otherwise get it with `gh pr view <PR> --json headRefOid`. Inline comments only
work on lines inside the diff; mention other findings in the summary only. Do not approve or request changes,
and do not edit files or push commits.

Several models may review the same pull request side by side. Put the model from the prompt (`MODEL`) in the
summary heading as shown below, and start every inline comment with `**[<MODEL>]**`.

Summary comment layout:

```
## Code review (<MODEL>)

**Requirements source:** …

### Requirements
| # | Requirement | Verdict | Where / what is missing |
|---|---|---|---|

### Findings
1. 🔴/🟠/🟡 **Short title** (`file.rs:line`): what goes wrong, when, and a suggested fix.

### Out of scope / unrequested changes
…

**Verdict:** one sentence: ready to merge, or what must change first.
```

Severity: 🔴 breaks a requirement, loses data, panics or breaks existing links; 🟠 wrong in a realistic
edge case; 🟡 weakness worth fixing but not blocking. Leave out empty sections. Write in English.
