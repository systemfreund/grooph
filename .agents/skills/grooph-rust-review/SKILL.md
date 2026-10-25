---
name: grooph-rust-review
description: Rust-specific review of a grooph pull request in CI. Runs the beagle review-rust skill with the adjustments this repository's review job needs (no cargo, no git, diff from gh). Use only when the review workflow asks for it.
---

# Rust review of a grooph pull request

Run the review described in `.agents/skills/review-rust/SKILL.md` on the pull request named in the prompt.
Read that file and the skills it links (they sit next to it under `.agents/skills/`) with the Read tool.
Follow it with the adjustments below; where they conflict, these win.

## What is different here

This runs in a CI job that has no Rust toolchain and no git history, and the shell only allows a few `gh`
commands. Do not try `cargo`, `git` or `grep` in the shell: they are denied.

- **Step 1 (changed files):** use `gh pr diff <PR> --name-only` and keep the `.rs` paths. For the diff itself use
  `gh pr diff <PR>`. Wherever review-rust runs `git diff $(git merge-base HEAD main)..HEAD`, use `gh pr diff <PR>`
  instead. If no `.rs` file changed, answer "No Rust files in scope" and stop.
- **Step 2 (edition):** the workspace uses edition 2024; check `Cargo.toml` with the Read tool.
- **Step 3 (clippy / cargo check) and Post-Fix Verification:** skip both. CI (`.github/workflows/rust.yml`) runs
  `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`, the tests and the web build on every
  pull request. For the compiler/linter gate, state "covered by CI (rust.yml)" and do not report anything clippy
  or the compiler would flag.
- **Step 4 (detect technologies):** do the same checks with the built-in grep/glob/read tools on the checked-out
  files and on the `gh pr diff` output instead of shell pipelines. Only the skills present under
  `.agents/skills/` apply; this project uses serde, thiserror and wasm-bindgen, not tokio, axum or sqlx.
- **Step 7 (review):** take the sequential path.
- **Project context:** `AGENTS.md` describes the workspace and its conventions; `REVIEW.md` §3 lists the
  project-specific risk areas (shared links, audio timing, native vs. WebAssembly). Treat both as the project
  conventions the verification protocol asks about.

## Output

- **Inline comments:** for each Critical, Major or Minor finding on a line inside the diff, post an inline comment
  with the `gh api` command from `REVIEW.md` §4. Start each with `**[Rust review · <MODEL>]**` and the severity.
- **Summary:** your final answer is the summary comment; the workflow posts it, so do not post it yourself. Use the
  Output Format of review-rust, but start with the heading `## Rust review (<MODEL>)` instead of
  `## Review Summary`, and leave out empty sections. Write in English.
- Do not approve or request changes, and do not edit files.
