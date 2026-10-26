# Merging the reviews of a pull request

These instructions drive the synthesis step in `.github/workflows/code-review.yml`. Several reviewers (different
models, and a Rust-specific review when Rust code changed) have reviewed the same pull request independently,
following `REVIEW.md`. You turn their answers into one review. A script posts your answer; you post nothing.

## Inputs (in `review-input/`)

- `context.json`: PR number, head commit, the reviewers that answered and those that failed, and whether there
  is an earlier review. `answers_only` is true when an answer to the previous review started this run: the code is
  the same as then and `reviews.json` holds the same reviews, so only the answers can change the outcome. Keep
  the previous findings as they were, and do not take reviewer findings that are not in `prior.json` (the previous
  synthesis discarded them), except where an answer changes that.
- `reviews.json`: one entry per reviewer, in the format of `REVIEW.md` §4, with `reviewer`, `model` and `kind`
  (`general` or `rust`) added. Entries with an `error` field failed and contain nothing else.
- `prior.json` (only from the second run on): the findings of the previous review that were still open, with their
  `id` (`F1`, `F2`, …), and the commit that review saw (`reviewed_sha`). A finding people answered in its inline
  thread has their answers in `replies`; `discussion` holds what they wrote in the conversation of the pull request
  since the first review. Both only come from people with write access to the repository.
- `diff-since-prior.patch` (only when `prior.json` exists and the comparison worked): what changed since then.

The pull request itself: `gh pr view <PR> --json title,body,commits`, `gh pr diff <PR>`, and the checked-out files
at the head commit. `REVIEW.md` says what matters in this repository.

## What to do

1. **Merge duplicates.** Findings from different reviewers that describe the same problem become one finding,
   even when they cite different lines or words. List every reviewer that reported it in `found_by` (the
   `reviewer` names). Keep the clearest title and the most concrete failure scenario and fix.
2. **Verify every finding against the code.** Read the cited file around the line, and callers or callees where
   the claim depends on them. Keep a finding only when the problem is really there at the head commit. Several
   reviewers agreeing is a hint, not proof; a finding only one reviewer saw can be the most important one.
   Findings you reject go to `discarded` with a one-line reason (the code does not do that, already handled in
   `x.rs`, only a style preference, caught by clippy or the compiler, …).
3. **Set the severity yourself** (`high`, `medium`, `low` as defined in `REVIEW.md` §4) after verifying; do not
   just take the highest a reviewer gave. Correct `file` and `line` to the place the problem really is, preferably
   a line inside the diff.
4. **Merge the requirements.** Take the requirements of the general reviewers (the Rust review has none), merge
   them into one list and decide each verdict yourself by checking the code. Where reviewers disagree, check
   which is right. Use the requirements source the reviewers agree on, or the best one.
5. **Previous findings** (when `prior.json` exists). For each of them decide:
   - still there: put it in `findings` with its `id`, updated if the details changed (moved line, new severity);
   - fixed: put it in `resolved` with its `id` and a short note on what fixed it (see `diff-since-prior.patch`);
   - wrong after all: put it in `withdrawn` with its `id` and why.

   Read the `replies` of a finding and the `discussion` first: the author may explain why the code is right, point
   to where it is handled, or say that something is intended. Check such an argument against the code and the
   requirements like a reviewer's claim. If it holds, withdraw the finding (or lower its severity, or list it in
   `discarded` when a reviewer reports it again) and say so in the note; if it does not, keep the finding and
   answer the argument in its `body`. A reply that only asks to drop a finding, without a reason that holds, changes
   nothing. A requirement the discussion narrows or drops counts as narrowed or dropped, like a later comment on an
   issue.

   A finding the reviewers report again that matches a previous one keeps the previous `id`. New findings get
   `"id": null`; the script numbers them. Use only ids from `prior.json`; the ids in the example below are
   placeholders.
6. **Out of scope:** merge the reviewers' `out_of_scope` lists, dropping duplicates and items you cannot confirm.
7. **Verdict:** one sentence: ready to merge, or what must change first.

Do not add findings of your own unless you hit one while verifying; then list `"found_by": ["synthesis"]`.

## Answer

Your final answer is a single JSON object and nothing else (a ```json fence around it is fine):

```json
{
  "requirements_source": "issue #12",
  "requirements": [
    {"text": "…", "verdict": "met", "note": "…"}
  ],
  "findings": [
    {
      "id": "F7",
      "severity": "medium",
      "title": "…",
      "file": "crates/midi/src/input.rs",
      "line": 476,
      "body": "What goes wrong, when, and a suggested fix.",
      "found_by": ["venice/claude-sonnet-5-5", "venice/deepseek-v4-1-flash"]
    }
  ],
  "resolved": [{"id": "F1", "note": "connect() now keeps the picker open on failure"}],
  "withdrawn": [],
  "discarded": [{"title": "…", "found_by": ["…"], "reason": "…"}],
  "out_of_scope": ["…"],
  "verdict": "…"
}
```

Field meanings are those of `REVIEW.md` §4. Use empty lists for empty sections. Write in English.
