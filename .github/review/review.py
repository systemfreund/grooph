#!/usr/bin/env python3
"""Glue for the code review workflow (.github/workflows/code-review.yml).

The models never post anything themselves: every reviewer and the synthesis
answer with JSON, and this script turns that into PR comments.

  extract   pull the final JSON answer out of `opencode run --format json` events
  rerun     after a comment: whether the synthesis should run again, and with the reviews of which run
  prepare   gather the inputs of the synthesis step (reviews, prior state and the replies to it, diff since
            the last review)
  publish   post new inline comments, reply to resolved ones, update the summary comment

Standard library only; talks to GitHub through the `gh` CLI (GH_TOKEN in the environment).
Tests: python3 -m unittest discover -s .github/review
"""

import argparse
import base64
import hashlib
import json
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

# publish --dry-run prints what it would post instead of posting it.
DRY_RUN = False
MARKER = "<!-- grooph-review -->"
STATE_RE = re.compile(r"<!-- grooph-review-state:([A-Za-z0-9+/=]+) -->")
FAILURE_RE = re.compile(r"\n*<!-- grooph-review-failure -->.*?<!-- /grooph-review-failure -->", re.S)
FINDING_MARKER = "<!-- grooph-finding:{} -->"
FINDING_MARKER_RE = re.compile(r"<!-- grooph-finding:(F\d+) -->")
FINDING_ID_RE = re.compile(r"\bF\d+\b")
UPDATED_RE = re.compile(r"^> \*\*Updated in `[^`]+`\*\*, see the latest reply\.\n\n")
SEVERITY_ICON = {"high": "🔴", "medium": "🟠", "low": "🟡"}
VERDICT_ICON = {"met": "✅ Met", "partly": "⚠️ Partly met", "not_met": "❌ Not met", "unclear": "❓ Unclear"}
# GitHub rejects comments over 65536 characters.
MAX_COMMENT = 65000
# An answer (review or synthesis) has at least one of these.
ANSWER_KEYS = {"findings", "requirements", "resolved", "withdrawn"}
BOT = "github-actions[bot]"
# Whose replies to the review the synthesis reads: anyone can comment on a public pull request.
TRUSTED = {"OWNER", "MEMBER", "COLLABORATOR"}


def gh(*args, input=None, check=True):
    result = subprocess.run(["gh", *args], input=input, capture_output=True, text=True)
    if check and result.returncode != 0:
        raise RuntimeError(f"gh {' '.join(args)} failed: {result.stderr.strip()}")
    return result


def gh_json(*args, input=None):
    out = gh(*args, input=input).stdout
    return json.loads(out) if out.strip() else None


def gh_paginated(path):
    # --slurp wraps the pages in one array.
    pages = gh_json("api", "--paginate", "--slurp", path)
    return [item for page in pages for item in page]


# ---------------------------------------------------------------- extract


def parse_json_answer(text):
    """The answer object in a model's text, or None.

    Models sometimes quote an example from the instructions next to their answer, fenced or not, so the
    candidate that comes last in the text wins: each code block, and the object in each stretch of text
    between them. An object only counts when it has the keys of an answer.
    """
    def braces(part):
        start, end = part.find("{"), part.rfind("}")
        return [part[start : end + 1]] if start != -1 and end > start else []

    # re.split with a group alternates: text, block, text, block, ..., text.
    parts = re.split(r"```(?:json)?\s*\n(.*?)\n```", text, flags=re.S)
    candidates = []
    for i, part in enumerate(parts):
        candidates += [part] if i % 2 else braces(part)
    candidates = candidates[::-1] + braces(text)
    for candidate in candidates:
        try:
            value = json.loads(candidate)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict) and ANSWER_KEYS & value.keys():
            return value
    return None


def parse_events(lines):
    """The answer in `opencode run --format json` output, or {"error": ...}."""
    texts, errors = [], []
    for line in lines:
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("type") == "text":
            texts.append(event["part"].get("text", ""))
        elif event.get("type") == "error":
            error = event.get("error") or {}
            errors.append((error.get("data") or {}).get("message") or error.get("name") or "unknown error")

    # The final answer is the last text; earlier texts are remarks between tool calls.
    for text in reversed(texts):
        answer = parse_json_answer(text)
        if answer is not None:
            answer.setdefault("findings", [])
            return answer
    return {"error": "; ".join(errors) or ("no JSON in the final answer" if texts else "no answer")}


def cmd_extract(args):
    events = Path(args.events)
    if events.exists():
        answer = parse_events(events.read_text().splitlines())
    else:
        answer = {"error": "OpenCode did not run"}
    result = {**answer, "reviewer": args.reviewer, "model": args.model, "kind": args.kind}
    Path(args.out).write_text(json.dumps(result, indent=2, ensure_ascii=False))
    if "error" in result:
        print(f"::error::{args.reviewer}: {result['error']}")
        return 1
    print(f"{args.reviewer}: {len(result['findings'])} findings")
    return 0


# ---------------------------------------------------------------- prepare


def issue_comments(repo, pr):
    return gh_paginated(f"repos/{repo}/issues/{pr}/comments")


def find_summary_comment(comments):
    for comment in comments:
        if comment["user"]["login"] == BOT and MARKER in comment["body"]:
            return comment
    return None


def read_state(comment):
    if comment is None:
        return None
    match = STATE_RE.search(comment["body"])
    if not match:
        return None
    return json.loads(base64.b64decode(match.group(1)))


def from_people(comment):
    return comment.get("author_association") in TRUSTED and comment["user"]["login"] != BOT


def human_comment(comment):
    return {"author": comment["user"]["login"], "at": comment.get("created_at"), "body": comment["body"]}


def with_replies(findings, review_comments):
    """`findings` with what people answered in their inline threads, under `replies`."""
    replies = {}
    for comment in review_comments:
        # GitHub links every reply to the first comment of its thread.
        if comment.get("in_reply_to_id") and from_people(comment):
            replies.setdefault(comment["in_reply_to_id"], []).append(human_comment(comment))
    return [{**f, "replies": replies[f["comment_id"]]} if f.get("comment_id") in replies else f for f in findings]


def discussion(summary, comments):
    """What people wrote in the conversation of the pull request since the first review."""
    return [
        human_comment(c)
        for c in comments
        if c["id"] != summary["id"] and from_people(c) and c["created_at"] > summary["created_at"]
    ]


def new_answers(summary, state, review_comments, comments):
    """Answers the synthesis has not seen yet: replies in the thread of an open finding, and comments in the
    conversation that name a finding."""
    open_findings = [f for f in state["findings"] if f["status"] == "open"]
    threads = {f["comment_id"] for f in open_findings if f.get("comment_id")}
    ids = {f["id"] for f in open_findings}
    replies = [c for c in review_comments if c.get("in_reply_to_id") in threads]
    # Only ids of open findings: "the F16 case" is no answer.
    named = [c for c in comments if ids & set(FINDING_ID_RE.findall(c["body"]))]
    # Before answers_until existed, the last update of the summary is the closest.
    seen = state.get("answers_until") or summary["updated_at"]
    return [c for c in replies + named if from_people(c) and c["created_at"] > seen]


def cmd_rerun(args):
    """Prints `reuse=<run>` when the synthesis should run again with the reviews of that run, else nothing.

    Checks all answers since the last review, not just the comment that started this run: a newer comment cancels
    a run that is still waiting, and the runs of comments that are no answer must not lose the answers before.
    """
    comments = issue_comments(args.repo, args.pr)
    summary = find_summary_comment(comments)
    state = read_state(summary)
    if not state or state["head_sha"] != args.head:
        reason = "this commit has no review yet; its review reads the answers"
    elif not state.get("reviews_run"):
        reason = "the last review does not say which run has its reviews"
    elif not new_answers(summary, state, gh_paginated(f"repos/{args.repo}/pulls/{args.pr}/comments"), comments):
        reason = "no new answer to a finding"
    else:
        print(f"reuse={state['reviews_run']}")
        return 0
    print(f"::notice::the synthesis does not run again: {reason}", file=sys.stderr)
    return 0


def now():
    """The time in GitHub's format, so it compares with `created_at` as a string."""
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def build_context(pr, head, reviews, expected, prior, diff_since_prior, answers_only=False, answers_until=None):
    answered = {r["reviewer"] for r in reviews}
    failed = [{"reviewer": r["reviewer"], "error": r["error"]} for r in reviews if "error" in r]
    # A reviewer job that failed before its extract step left no file at all.
    failed += [{"reviewer": name, "error": "no output"} for name in expected if name not in answered]
    return {
        "pr": int(pr),
        "head_sha": head,
        "reviewers": [r["reviewer"] for r in reviews if "error" not in r],
        "failed_reviewers": failed,
        "prior_review": prior is not None,
        "diff_since_prior": diff_since_prior,
        "answers_only": answers_only,
        "answers_until": answers_until,
    }


def cmd_prepare(args):
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    reviews = [json.loads(path.read_text()) for path in sorted(Path(args.reviews).glob("*.json"))]
    (out / "reviews.json").write_text(json.dumps(reviews, indent=2, ensure_ascii=False))

    # Every answer written before this moment is in the comments fetched now; a later one is new to the next run.
    fetched_at = now()
    comments = issue_comments(args.repo, args.pr)
    summary = find_summary_comment(comments)
    state = read_state(summary)
    prior = None
    if state:
        open_findings = [f for f in state["findings"] if f["status"] == "open"]
        prior = {
            "reviewed_sha": state["head_sha"],
            "findings": with_replies(open_findings, gh_paginated(f"repos/{args.repo}/pulls/{args.pr}/comments")),
            "discussion": discussion(summary, comments),
        }
        (out / "prior.json").write_text(json.dumps(prior, indent=2, ensure_ascii=False))
        diff = gh(
            "api",
            "-H",
            "Accept: application/vnd.github.diff",
            f"repos/{args.repo}/compare/{state['head_sha']}...{args.head}",
            check=False,
        )
        if diff.returncode == 0:
            (out / "diff-since-prior.patch").write_text(diff.stdout)

    expected = json.loads(args.expected)
    diff_since_prior = (out / "diff-since-prior.patch").exists()
    context = build_context(args.pr, args.head, reviews, expected, prior, diff_since_prior, args.answers_only,
                            fetched_at)
    (out / "context.json").write_text(json.dumps(context, indent=2))
    print(json.dumps(context, indent=2))
    if not context["reviewers"]:
        print("::error::no reviewer answered")
        return 1
    return 0


# ---------------------------------------------------------------- publish


def patch_lines(patch):
    """Right-side line numbers inside one file's diff: inline comments only work there."""
    found = set()
    number = 0
    for row in (patch or "").splitlines():
        hunk = re.match(r"@@ -\d+(?:,\d+)? \+(\d+)", row)
        if hunk:
            number = int(hunk.group(1))
        elif row.startswith("-") or row.startswith("\\"):
            continue
        else:
            found.add(number)
            number += 1
    return found


def diff_lines(repo, pr):
    return {f["filename"]: patch_lines(f.get("patch")) for f in gh_paginated(f"repos/{repo}/pulls/{pr}/files")}


def as_line(value):
    return int(value) if str(value or "").isdigit() else None


def check_synthesis(synthesis, open_ids):
    """Why the synthesis answer has the wrong shape, or None. `open_ids` are the ids it may refer to."""
    for key in ("findings", "resolved", "withdrawn", "discarded", "requirements"):
        items = synthesis.get(key)
        if items is not None and not (isinstance(items, list) and all(isinstance(i, dict) for i in items)):
            return f"`{key}` is not a list of objects"
    for key in ("resolved", "withdrawn"):
        if any(not isinstance(i.get("id"), str) for i in synthesis.get(key) or []):
            return f"an entry in `{key}` has no id"
    for item in synthesis.get("findings") or []:
        if item.get("id") is not None and not isinstance(item["id"], str):
            return f"a finding has the id {item['id']!r}, not a string like \"F1\""
        # A known finding may be listed by its id alone; a new one needs its title.
        if item.get("id") not in open_ids and not (isinstance(item.get("title"), str) and item["title"]):
            return "a new finding has no title"
    out_of_scope = synthesis.get("out_of_scope")
    if out_of_scope is not None and not isinstance(out_of_scope, list):
        return "`out_of_scope` is not a list"
    return None


def content_key(finding):
    """What a reader of the inline thread should see again when it changes: title, severity and text."""
    text = json.dumps([finding.get("title"), finding.get("severity"), finding.get("body")], ensure_ascii=False)
    return hashlib.sha1(text.encode()).hexdigest()[:12]


def merge_state(prior, synthesis, head):
    """Apply the synthesis to the previous state. Returns (state, findings resolved or withdrawn now)."""
    findings = [dict(f) for f in prior["findings"]]
    by_id = {f["id"]: f for f in findings}
    next_id = prior["next_id"]

    # A finding the synthesis still lists stays open, even if it also lists it as resolved.
    still_open = set()
    for item in synthesis.get("findings") or []:
        fields = {k: item.get(k) for k in ("severity", "title", "file", "body")}
        fields["line"] = as_line(item.get("line"))
        fields["found_by"] = item.get("found_by") or []
        f = by_id.get(item.get("id"))
        if f and f["status"] != "open":
            # The synthesis only sees open findings, so a closed id is a mix-up (e.g. copied from the
            # example in SYNTHESIS.md), and the state no longer has the closed finding's text: a new finding.
            f = None
        if f:
            # A field the synthesis left out keeps its previous value.
            f.update({k: v for k, v in fields.items() if v})
            for key in ("resolution", "resolved_in", "replied"):
                f.pop(key, None)
            f["status"] = "open"
        else:
            f = {"id": f"F{next_id}", "status": "open", "comment_id": None, **fields}
            next_id += 1
            findings.append(f)
            by_id[f["id"]] = f
        still_open.add(f["id"])

    newly_resolved = []
    for status in ("resolved", "withdrawn"):
        for item in synthesis.get(status) or []:
            f = by_id.get(item.get("id"))
            if f and f["status"] == "open" and f["id"] not in still_open:
                f.update(status=status, resolution=item.get("note", ""), resolved_in=head)
                if f.get("comment_id"):
                    # Without an inline comment there is no thread to reply in.
                    f["replied"] = False
                newly_resolved.append(f)
    # Prior findings the synthesis did not mention stay open as they were.

    return {"version": 1, "head_sha": head, "next_id": next_id, "findings": findings}, newly_resolved


def encode_state(state, limit=None, closed=True):
    """The hidden state block. `limit` shortens the text of open findings, `closed=False` drops closed ones."""
    # Closed findings only need what the "Resolved earlier" list and the reply to their thread need.
    kept = ("id", "status", "title", "resolution", "resolved_in", "comment_id", "replied")

    def store(f):
        if f["status"] != "open":
            return {k: f.get(k) for k in kept}
        return {**f, "body": shorten(f.get("body"), limit)} if limit else f

    # Closed findings still waiting for their reply stay, or the reply would never be retried.
    keep = [f for f in state["findings"] if closed or f["status"] == "open" or f.get("replied") is False]
    compact = dict(state)
    compact["findings"] = [store(f) for f in keep]
    encoded = base64.b64encode(json.dumps(compact, ensure_ascii=False).encode()).decode()
    return f"<!-- grooph-review-state:{encoded} -->"


def location(finding):
    if not finding.get("file"):
        return ""
    line = f":{finding['line']}" if finding.get("line") else ""
    return f" (`{finding['file']}{line}`)"


def found_by(finding):
    names = finding.get("found_by") or []
    if not names:
        return ""
    return f"<details><summary>Found by {len(names)}</summary>{', '.join(names)}</details>"


def finding_text(finding):
    icon = SEVERITY_ICON.get(finding.get("severity"), "🟡")
    return f"{icon} **{finding['id']} · {finding['title']}**\n\n{finding.get('body') or ''}"


def inline_body(finding):
    parts = [finding_text(finding)]
    if finding.get("found_by"):
        parts.append(f"<sub>Found by: {', '.join(finding['found_by'])}</sub>")
    return "\n\n".join(parts) + "\n" + FINDING_MARKER.format(finding["id"])


def cell(text):
    return str(text or "").replace("|", "\\|").replace("\n", " ")


def shorten(text, limit):
    text = str(text or "")
    return text if len(text) <= limit else text[: limit - 1] + "…"


# Ever shorter forms of the summary: (limit for visible texts, keep closed findings, limit for stored texts).
# The stored texts are only shortened as a last resort: the next run gives them to the synthesis.
SUMMARY_FORMS = [(None, True, None), (600, True, None), (200, True, None), (200, False, None), (60, False, None),
                 (60, False, 2000), (60, False, 300), (60, False, 60)]


def render_summary(synthesis, state, context, head, model, run_url, newly_resolved):
    """The summary comment, shortened step by step until it fits GitHub's size limit."""
    for limit, closed, stored in SUMMARY_FORMS:
        text = render_summary_with(synthesis, state, context, head, model, run_url, newly_resolved, limit, closed, stored)
        if len(text) <= MAX_COMMENT:
            return text
    # Only reachable with hundreds of open findings: fail with the reason rather than a 422 from GitHub.
    raise RuntimeError(f"the summary does not fit in a comment ({len(text)} characters)")


def collapsed(title, lines):
    """A section that starts folded, so the summary takes little room in the pull request."""
    return [f"<details><summary><b>{title}</b></summary>", "", *lines, "", "</details>", ""]


def render_summary_with(synthesis, state, context, head, model, run_url, newly_resolved, limit, closed, stored):
    compact = limit is not None
    body = (lambda text: shorten(text, limit)) if compact else (lambda text: text or "")
    short = head[:7]
    out = [MARKER, "## Code review", ""]
    reviewers = ", ".join(context["reviewers"]) or "none"
    out.append(f"Reviewed `{short}` · reviewers: {reviewers} · synthesis: {model}")
    if context.get("answers_only"):
        out.append("\nMerged again after new answers, with the reviews of this commit.")
    for failed in context["failed_reviewers"]:
        out.append(f"\n⚠️ Reviewer **{failed['reviewer']}** failed: {cell(failed['error'])}")
    out.append("")

    requirements = synthesis.get("requirements") or []
    if requirements or synthesis.get("requirements_source"):
        lines = []
        if synthesis.get("requirements_source"):
            lines += [f"**Requirements source:** {synthesis['requirements_source']}", ""]
        if requirements:
            lines += ["| # | Requirement | Verdict | Where / what is missing |", "|---|---|---|---|"]
        for i, req in enumerate(requirements, 1):
            verdict = VERDICT_ICON.get(req.get("verdict"), cell(req.get("verdict")))
            lines.append(f"| {i} | {cell(req.get('text'))} | {verdict} | {cell(body(req.get('note')))} |")
        met = sum(req.get("verdict") == "met" for req in requirements)
        out += collapsed(f"Requirements ({met}/{len(requirements)} met)" if requirements else "Requirements", lines)

    open_findings = [f for f in state["findings"] if f["status"] == "open"]
    order = {"high": 0, "medium": 1, "low": 2}
    open_findings.sort(key=lambda f: (order.get(f.get("severity"), 3), int(f["id"][1:])))
    if open_findings:
        lines = []
        for f in open_findings:
            icon = SEVERITY_ICON.get(f.get("severity"), "🟡")
            lines.append(f"- {icon} **{f['id']} · {f['title']}**{location(f)}: {body(f.get('body'))}")
            if found_by(f):
                lines.append(f"  {found_by(f)}")
        icons = [SEVERITY_ICON.get(f.get("severity"), "🟡") for f in open_findings]
        counts = " · ".join(f"{icon} {icons.count(icon)}" for icon in dict.fromkeys(icons))
        out += collapsed(f"Findings ({counts})", lines)

    if newly_resolved:
        lines = [f"- ~~{f['id']} · {f['title']}~~ {f['status']}: {body(f.get('resolution'))}" for f in newly_resolved]
        out += collapsed(f"Resolved in <code>{short}</code> ({len(newly_resolved)})", lines)

    resolved_now = {f["id"] for f in newly_resolved}
    earlier = [f for f in state["findings"] if f["status"] != "open" and f["id"] not in resolved_now]
    if earlier and closed:
        out.append(f"<details><summary>Resolved earlier ({len(earlier)})</summary>\n")
        for f in earlier:
            when = (f.get("resolved_in") or "")[:7]
            out.append(f"- ~~{f['id']} · {f['title']}~~ {f['status']} in `{when}`: {f.get('resolution') or ''}")
        out.append("\n</details>\n")

    if synthesis.get("out_of_scope"):
        lines = [f"- {body(item)}" for item in synthesis["out_of_scope"]]
        out += collapsed(f"Out of scope / unrequested changes ({len(lines)})", lines)

    discarded = synthesis.get("discarded") or []
    if discarded and not compact:
        out.append(f"<details><summary>Discarded reviewer findings ({len(discarded)})</summary>\n")
        for d in discarded:
            by = ", ".join(d.get("found_by") or [])
            out.append(f"- **{d.get('title', '')}** ({by}): {d.get('reason', '')}")
        out.append("\n</details>\n")

    if synthesis.get("verdict"):
        out += [f"**Verdict:** {synthesis['verdict']}", ""]
    out.append(f"<sub>[Workflow run]({run_url})</sub>")
    out.append(encode_state(state, stored, closed))
    return "\n".join(out)


def render_failure(previous_body, context, error, head, run_url):
    """The summary after a failed run: the previous review, plus one note that replaces any earlier note."""
    body = FAILURE_RE.sub("", previous_body) if previous_body else f"{MARKER}\n## Code review"
    note = [f"⚠️ The review of `{head[:7]}` failed ({cell(error)}), see the [workflow run]({run_url})."]
    for failed in context.get("failed_reviewers") or []:
        note.append(f"Reviewer **{failed['reviewer']}** failed: {cell(failed['error'])}")
    if previous_body:
        note.append("The findings below are from the previous review.")
    block = "<!-- grooph-review-failure -->\n" + "\n\n".join(note) + "\n<!-- /grooph-review-failure -->"
    # The note goes right under the heading, so it is not mistaken for part of the old review.
    heading = "## Code review"
    if heading in body:
        return body.replace(heading, f"{heading}\n\n{block}", 1)
    return f"{body}\n\n{block}"


def upsert_summary(repo, pr, comment, body):
    """Creates or updates the summary comment; returns the comment."""
    if DRY_RUN:
        print(f"--- summary comment ({'update' if comment else 'new'}) ---\n{body}")
        return comment or {"id": 0, "body": body}
    payload = json.dumps({"body": body})
    if comment:
        return gh_json("api", "-X", "PATCH", f"repos/{repo}/issues/comments/{comment['id']}", "--input", "-", input=payload)
    return gh_json("api", "-X", "POST", f"repos/{repo}/issues/{pr}/comments", "--input", "-", input=payload)


def posted_finding_comments(repo, pr):
    """Inline comments this workflow already posted, by finding id."""
    found = {}
    for comment in gh_paginated(f"repos/{repo}/pulls/{pr}/comments"):
        match = FINDING_MARKER_RE.search(comment["body"])
        if comment["user"]["login"] == BOT and match and not comment.get("in_reply_to_id"):
            found.setdefault(match.group(1), comment)
    return found


def mark_updated(original, head):
    """The original inline comment with a note on top that a newer version is in the thread."""
    return f"> **Updated in `{head[:7]}`**, see the latest reply.\n\n{UPDATED_RE.sub('', original)}"


def thread_updates(state, posted, head):
    """What to say in the threads of findings that already have an inline comment.

    Returns (finding, reply, new body of the original comment or None) for findings resolved or withdrawn
    without a reply yet, and for open findings whose title, severity or text changed since the thread last
    showed them.
    """
    updates = []
    for f in state["findings"]:
        if not f.get("comment_id"):
            continue
        if f["status"] != "open":
            if f.get("replied") is False:
                when = (f.get("resolved_in") or head)[:7]
                updates.append((f, f"{f['status'].capitalize()} in {when}: {f.get('resolution') or ''}", None))
            continue
        original = (posted.get(f["id"]) or {}).get("body")
        if "thread_key" not in f:
            # Posted before thread keys existed: the thread is current if the comment shows this text.
            f["thread_key"] = content_key(f) if original and finding_text(f) in original else None
        if f["thread_key"] != content_key(f):
            reply = f"Still open in `{head[:7]}`, updated:\n\n{finding_text(f)}"
            updates.append((f, reply, mark_updated(original, head) if original else None))
    return updates


def post_inline(repo, pr, head, findings):
    """Posts new inline comments as one review; one by one if GitHub rejects the batch. Returns the failures."""
    def comment(f):
        return {"path": f["file"], "line": f["line"], "side": "RIGHT", "body": inline_body(f)}

    review = {"commit_id": head, "event": "COMMENT", "comments": [comment(f) for f in findings]}
    result = gh("api", "-X", "POST", f"repos/{repo}/pulls/{pr}/reviews", "--input", "-",
                input=json.dumps(review), check=False)
    if result.returncode == 0:
        review_id = json.loads(result.stdout)["id"]
        by_id = {f["id"]: f for f in findings}
        for c in gh_paginated(f"repos/{repo}/pulls/{pr}/reviews/{review_id}/comments"):
            match = FINDING_MARKER_RE.search(c["body"])
            if match and match.group(1) in by_id:
                by_id[match.group(1)]["comment_id"] = c["id"]
        return []

    print(f"::warning::the review with all inline comments was rejected ({result.stderr.strip()}); posting them one by one")
    failed = []
    for f in findings:
        single = gh("api", "-X", "POST", f"repos/{repo}/pulls/{pr}/comments", "--input", "-",
                    input=json.dumps({"commit_id": head, **comment(f)}), check=False)
        if single.returncode == 0:
            f["comment_id"] = json.loads(single.stdout)["id"]
        else:
            failed.append(f"{f['id']} ({f['file']}:{f['line']}): {single.stderr.strip()}")
    return failed


def outdated(args, prior):
    """Whether a newer push or review overtook a run that merges answers to the review of `args.head`."""
    pr = gh_json("api", "-X", "GET", f"repos/{args.repo}/pulls/{args.pr}")
    return pr["head"]["sha"] != args.head or prior.get("head_sha") != args.head


def cmd_publish(args):
    comment = find_summary_comment(issue_comments(args.repo, args.pr))
    prior = read_state(comment) or {"next_id": 1, "findings": []}
    context_path = Path(args.input, "context.json")
    context = json.loads(context_path.read_text()) if context_path.exists() else {"reviewers": [], "failed_reviewers": []}

    if context.get("answers_only") and outdated(args, prior):
        # Comment runs have their own concurrency group, so a push can arrive while one runs; its review wins.
        print(f"::notice::not published: {args.head[:7]} is no longer the head, or was reviewed again meanwhile")
        return 0

    synthesis_path = Path(args.synthesis)
    synthesis = json.loads(synthesis_path.read_text()) if synthesis_path.exists() else {"error": "no output"}
    open_ids = {f["id"] for f in prior["findings"] if f["status"] == "open"}
    problem = synthesis.get("error") or check_synthesis(synthesis, open_ids)
    if problem:
        # Keep the previous findings; only say that this run failed.
        body = render_failure(comment["body"] if comment else None, context, problem, args.head, args.run_url)
        upsert_summary(args.repo, args.pr, comment, body)
        print(f"::error::synthesis failed: {problem}")
        return 1

    state, newly_resolved = merge_state(prior, synthesis, args.head)
    # What a comment needs to start the synthesis again: the reviews to reuse, and which answers it has seen.
    state["reviews_run"] = args.reviews_run
    state["answers_until"] = max(filter(None, [context.get("answers_until"), prior.get("answers_until")]), default=None)

    def summary():
        return render_summary(synthesis, state, context, args.head, args.model, args.run_url, newly_resolved)

    # Store the new state before posting anything: if this job is cancelled halfway, the next run adopts
    # the comments posted under the new ids and retries the replies that are still missing.
    comment = upsert_summary(args.repo, args.pr, comment, summary())

    posted = posted_finding_comments(args.repo, args.pr)
    for f in state["findings"]:
        if not f.get("comment_id") and f["id"] in posted:
            f["comment_id"] = posted[f["id"]]["id"]

    for f, reply, original in thread_updates(state, posted, args.head):
        if DRY_RUN:
            print(f"--- reply to {f['id']} ---\n{reply}")
            continue
        if original:
            edit = f"repos/{args.repo}/pulls/comments/{f['comment_id']}"
            gh("api", "-X", "PATCH", edit, "--input", "-", input=json.dumps({"body": original}), check=False)
        replies = f"repos/{args.repo}/pulls/{args.pr}/comments/{f['comment_id']}/replies"
        result = gh("api", "-X", "POST", replies, "--input", "-", input=json.dumps({"body": reply}), check=False)
        if result.returncode != 0:
            print(f"::warning::reply to {f['id']} failed, retried next run: {result.stderr.strip()}")
        elif f["status"] == "open":
            f["thread_key"] = content_key(f)
        else:
            f["replied"] = True

    in_diff = diff_lines(args.repo, args.pr)
    to_post = [
        f
        for f in state["findings"]
        if f["status"] == "open"
        and not f.get("comment_id")
        and f.get("file")
        and f.get("line") in in_diff.get(f["file"], set())
    ]
    for f in to_post:
        f["thread_key"] = content_key(f)
    failed = []
    if to_post and DRY_RUN:
        for f in to_post:
            print(f"--- inline comment {f['file']}:{f['line']} ---\n{inline_body(f)}")
    elif to_post:
        failed = post_inline(args.repo, args.pr, args.head, to_post)

    upsert_summary(args.repo, args.pr, comment, summary())
    print(f"{len(to_post) - len(failed)} inline comments, {len(newly_resolved)} resolved or withdrawn")
    if failed:
        # The findings are in the summary; the red job says that their inline comments are missing.
        print("::error::inline comments not posted: " + "; ".join(failed))
        return 1
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)

    p = sub.add_parser("extract")
    p.add_argument("--events", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--reviewer", required=True)
    p.add_argument("--model", required=True)
    p.add_argument("--kind", required=True)

    for name in ("rerun", "prepare", "publish"):
        p = sub.add_parser(name)
        p.add_argument("--repo", required=True)
        p.add_argument("--pr", required=True)
        p.add_argument("--head", required=True)
        if name == "prepare":
            p.add_argument("--reviews", required=True)
            p.add_argument("--expected", required=True, help="JSON list of the reviewer names that should answer")
            p.add_argument("--out", required=True)
            p.add_argument("--answers-only", action="store_true", help="the reviews are those of an earlier run")
        elif name == "publish":
            p.add_argument("--input", required=True)
            p.add_argument("--synthesis", required=True)
            p.add_argument("--model", required=True)
            p.add_argument("--run-url", required=True)
            p.add_argument("--reviews-run", required=True, help="the run whose reviews this synthesis merged")
            p.add_argument("--dry-run", action="store_true")

    args = parser.parse_args()
    global DRY_RUN
    DRY_RUN = getattr(args, "dry_run", False)
    commands = {"extract": cmd_extract, "rerun": cmd_rerun, "prepare": cmd_prepare, "publish": cmd_publish}
    return commands[args.command](args)


if __name__ == "__main__":
    sys.exit(main())
