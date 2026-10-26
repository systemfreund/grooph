#!/usr/bin/env python3
"""Glue for the code review workflow (.github/workflows/code-review.yml).

The models never post anything themselves: every reviewer and the synthesis
answer with JSON, and this script turns that into PR comments.

  extract   pull the final JSON answer out of `opencode run --format json` events
  prepare   gather the inputs of the synthesis step (reviews, prior state, diff since the last review)
  publish   post new inline comments, reply to resolved ones, update the summary comment

Standard library only; talks to GitHub through the `gh` CLI (GH_TOKEN in the environment).
Tests: python3 -m unittest discover -s .github/review
"""

import argparse
import base64
import json
import re
import subprocess
import sys
from pathlib import Path

# publish --dry-run prints what it would post instead of posting it.
DRY_RUN = False
MARKER = "<!-- grooph-review -->"
STATE_RE = re.compile(r"<!-- grooph-review-state:([A-Za-z0-9+/=]+) -->")
FAILURE_RE = re.compile(r"\n*<!-- grooph-review-failure -->.*?<!-- /grooph-review-failure -->", re.S)
FINDING_MARKER = "<!-- grooph-finding:{} -->"
FINDING_MARKER_RE = re.compile(r"<!-- grooph-finding:(F\d+) -->")
SEVERITY_ICON = {"high": "🔴", "medium": "🟠", "low": "🟡"}
VERDICT_ICON = {"met": "✅ Met", "partly": "⚠️ Partly met", "not_met": "❌ Not met", "unclear": "❓ Unclear"}
# GitHub rejects comments over 65536 characters.
MAX_COMMENT = 65000
BOT = "github-actions[bot]"


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
    """The JSON object in a model's answer, which may be wrapped in a code fence or prose."""
    fence = re.search(r"```(?:json)?\s*\n(.*?)\n```", text, re.S)
    candidates = [fence.group(1)] if fence else []
    start, end = text.find("{"), text.rfind("}")
    if start != -1 and end > start:
        candidates.append(text[start : end + 1])
    for candidate in candidates:
        try:
            value = json.loads(candidate)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
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


def find_summary_comment(repo, pr):
    for comment in gh_paginated(f"repos/{repo}/issues/{pr}/comments"):
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


def build_context(pr, head, reviews, expected, prior, diff_since_prior):
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
    }


def cmd_prepare(args):
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    reviews = [json.loads(path.read_text()) for path in sorted(Path(args.reviews).glob("*.json"))]
    (out / "reviews.json").write_text(json.dumps(reviews, indent=2, ensure_ascii=False))

    state = read_state(find_summary_comment(args.repo, args.pr))
    prior = None
    if state:
        prior = {
            "reviewed_sha": state["head_sha"],
            "findings": [f for f in state["findings"] if f["status"] == "open"],
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
    context = build_context(args.pr, args.head, reviews, expected, prior, (out / "diff-since-prior.patch").exists())
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
        f = by_id.get(item.get("id"))
        if f:
            fields["found_by"] = item.get("found_by") or f.get("found_by") or []
            for key in ("resolution", "resolved_in"):
                f.pop(key, None)
            f.update(fields, status="open")
        else:
            fields["found_by"] = item.get("found_by") or []
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
                newly_resolved.append(f)
    # Prior findings the synthesis did not mention stay open as they were.

    return {"version": 1, "head_sha": head, "next_id": next_id, "findings": findings}, newly_resolved


def encode_state(state, limit=None):
    """The hidden state block. `limit` shortens the text of open findings."""
    # Closed findings only need what the "Resolved earlier" list shows.
    kept = ("id", "status", "title", "resolution", "resolved_in", "comment_id")

    def store(f):
        if f["status"] != "open":
            return {k: f.get(k) for k in kept}
        return {**f, "body": shorten(f["body"], limit)} if limit else f

    compact = dict(state)
    compact["findings"] = [store(f) for f in state["findings"]]
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


def inline_body(finding):
    icon = SEVERITY_ICON.get(finding["severity"], "🟡")
    parts = [f"{icon} **{finding['id']} · {finding['title']}**", "", finding["body"] or ""]
    if finding.get("found_by"):
        parts += ["", f"<sub>Found by: {', '.join(finding['found_by'])}</sub>"]
    parts.append(FINDING_MARKER.format(finding["id"]))
    return "\n".join(parts)


def cell(text):
    return str(text or "").replace("|", "\\|").replace("\n", " ")


def shorten(text, limit):
    text = str(text or "")
    return text if len(text) <= limit else text[: limit - 1] + "…"


def render_summary(synthesis, state, context, head, model, run_url, newly_resolved):
    """The summary comment, shortened step by step until it fits GitHub's size limit."""
    for limit in (None, 600, 200, 60):
        text = render_summary_with(synthesis, state, context, head, model, run_url, newly_resolved, limit)
        if len(text) <= MAX_COMMENT:
            break
    return text


def render_summary_with(synthesis, state, context, head, model, run_url, newly_resolved, limit):
    compact = limit is not None
    body = (lambda text: shorten(text, limit)) if compact else (lambda text: text or "")
    short = head[:7]
    out = [MARKER, "## Code review", ""]
    reviewers = ", ".join(context["reviewers"]) or "none"
    out.append(f"Reviewed `{short}` · reviewers: {reviewers} · synthesis: {model}")
    for failed in context["failed_reviewers"]:
        out.append(f"\n⚠️ Reviewer **{failed['reviewer']}** failed: {cell(failed['error'])}")
    out.append("")

    if synthesis.get("requirements_source"):
        out += [f"**Requirements source:** {synthesis['requirements_source']}", ""]
    if synthesis.get("requirements"):
        out += ["### Requirements", "| # | Requirement | Verdict | Where / what is missing |", "|---|---|---|---|"]
        for i, req in enumerate(synthesis["requirements"], 1):
            verdict = VERDICT_ICON.get(req.get("verdict"), cell(req.get("verdict")))
            out.append(f"| {i} | {cell(req.get('text'))} | {verdict} | {cell(body(req.get('note')))} |")
        out.append("")

    open_findings = [f for f in state["findings"] if f["status"] == "open"]
    order = {"high": 0, "medium": 1, "low": 2}
    open_findings.sort(key=lambda f: (order.get(f["severity"], 3), int(f["id"][1:])))
    if open_findings:
        out.append("### Findings")
        for f in open_findings:
            icon = SEVERITY_ICON.get(f["severity"], "🟡")
            out.append(f"- {icon} **{f['id']} · {f['title']}**{location(f)}: {body(f['body'])}")
            if found_by(f):
                out.append(f"  {found_by(f)}")
        out.append("")

    if newly_resolved:
        out.append(f"### Resolved in `{short}`")
        for f in newly_resolved:
            out.append(f"- ~~{f['id']} · {f['title']}~~ {f['status']}: {body(f.get('resolution'))}")
        out.append("")

    resolved_now = {f["id"] for f in newly_resolved}
    earlier = [f for f in state["findings"] if f["status"] != "open" and f["id"] not in resolved_now]
    if earlier and not compact:
        out.append(f"<details><summary>Resolved earlier ({len(earlier)})</summary>\n")
        for f in earlier:
            when = (f.get("resolved_in") or "")[:7]
            out.append(f"- ~~{f['id']} · {f['title']}~~ {f['status']} in `{when}`: {f.get('resolution') or ''}")
        out.append("\n</details>\n")

    if synthesis.get("out_of_scope"):
        out.append("### Out of scope / unrequested changes")
        out += [f"- {body(item)}" for item in synthesis["out_of_scope"]]
        out.append("")

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
    out.append(encode_state(state, limit))
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
            found.setdefault(match.group(1), comment["id"])
    return found


def cmd_publish(args):
    comment = find_summary_comment(args.repo, args.pr)
    prior = read_state(comment) or {"next_id": 1, "findings": []}
    context_path = Path(args.input, "context.json")
    context = json.loads(context_path.read_text()) if context_path.exists() else {"reviewers": [], "failed_reviewers": []}

    synthesis_path = Path(args.synthesis)
    synthesis = json.loads(synthesis_path.read_text()) if synthesis_path.exists() else {"error": "no output"}
    if "error" in synthesis:
        # Keep the previous findings; only say that this run failed.
        body = render_failure(comment["body"] if comment else None, context, synthesis["error"], args.head, args.run_url)
        upsert_summary(args.repo, args.pr, comment, body)
        print(f"::error::synthesis failed: {synthesis['error']}")
        return 1

    state, newly_resolved = merge_state(prior, synthesis, args.head)

    def summary():
        return render_summary(synthesis, state, context, args.head, args.model, args.run_url, newly_resolved)

    # Store the new ids before posting anything: if this job is cancelled after posting, the next run
    # finds the posted comments by their markers instead of posting them again under other ids.
    comment = upsert_summary(args.repo, args.pr, comment, summary())

    posted = {} if DRY_RUN else posted_finding_comments(args.repo, args.pr)
    for f in state["findings"]:
        if not f.get("comment_id") and f["id"] in posted:
            f["comment_id"] = posted[f["id"]]

    for f in newly_resolved:
        if f.get("comment_id") and not DRY_RUN:
            reply = json.dumps({"body": f"{f['status'].capitalize()} in {args.head[:7]}: {f['resolution']}"})
            path = f"repos/{args.repo}/pulls/{args.pr}/comments/{f['comment_id']}/replies"
            gh("api", "-X", "POST", path, "--input", "-", input=reply, check=False)

    in_diff = diff_lines(args.repo, args.pr)
    to_post = [
        f
        for f in state["findings"]
        if f["status"] == "open"
        and not f.get("comment_id")
        and f.get("file")
        and f.get("line") in in_diff.get(f["file"], set())
    ]
    if to_post and DRY_RUN:
        for f in to_post:
            print(f"--- inline comment {f['file']}:{f['line']} ---\n{inline_body(f)}")
    elif to_post:
        review = {
            "commit_id": args.head,
            "event": "COMMENT",
            "comments": [
                {"path": f["file"], "line": f["line"], "side": "RIGHT", "body": inline_body(f)} for f in to_post
            ],
        }
        result = gh(
            "api", "-X", "POST", f"repos/{args.repo}/pulls/{args.pr}/reviews", "--input", "-",
            input=json.dumps(review), check=False,
        )
        if result.returncode == 0:
            by_id = {f["id"]: f for f in state["findings"]}
            review_id = json.loads(result.stdout)["id"]
            for c in gh_paginated(f"repos/{args.repo}/pulls/{args.pr}/reviews/{review_id}/comments"):
                match = FINDING_MARKER_RE.search(c["body"])
                if match and match.group(1) in by_id:
                    by_id[match.group(1)]["comment_id"] = c["id"]
        else:
            print(f"::warning::inline comments not posted: {result.stderr.strip()}")

    upsert_summary(args.repo, args.pr, comment, summary())
    print(f"{len(to_post)} inline comments, {len(newly_resolved)} resolved or withdrawn")
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

    for name in ("prepare", "publish"):
        p = sub.add_parser(name)
        p.add_argument("--repo", required=True)
        p.add_argument("--pr", required=True)
        p.add_argument("--head", required=True)
        if name == "prepare":
            p.add_argument("--reviews", required=True)
            p.add_argument("--expected", required=True, help="JSON list of the reviewer names that should answer")
            p.add_argument("--out", required=True)
        else:
            p.add_argument("--input", required=True)
            p.add_argument("--synthesis", required=True)
            p.add_argument("--model", required=True)
            p.add_argument("--run-url", required=True)
            p.add_argument("--dry-run", action="store_true")

    args = parser.parse_args()
    global DRY_RUN
    DRY_RUN = getattr(args, "dry_run", False)
    return {"extract": cmd_extract, "prepare": cmd_prepare, "publish": cmd_publish}[args.command](args)


if __name__ == "__main__":
    sys.exit(main())
