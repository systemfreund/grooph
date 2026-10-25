# Agent skills

Skills in the [Agent Skills](https://agentskills.io) format. OpenCode and other agents discover them here.

- `grooph-rust-review`: written for this repository. Drives the Rust review job in
  `.github/workflows/code-review.yml`.
- All other skills are copied unchanged from
  [existential-birds/beagle](https://github.com/existential-birds/beagle) (`plugins/beagle-rust/skills`),
  commit `d1a74899fbfec74974d1818e4cac7c3d54d44b65`, under the Apache License 2.0 (`LICENSE-beagle`). Only the
  skills that `review-rust` can load for this codebase are included.

To update them, copy the same directories from a newer beagle commit, review the diff (the skills steer an agent
that can comment on pull requests) and update the commit above. Keep repository-specific changes in
`grooph-rust-review` so the copied files stay identical to upstream.
