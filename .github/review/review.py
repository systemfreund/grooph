#!/usr/bin/env python3
"""Glue for the code review workflow (.github/workflows/code-review.yml).

The models never post anything themselves: every reviewer and the synthesis
answer with JSON, and this script turns that into PR comments.

  extract   pull the final JSON answer out of `opencode run --format json` events
  prepare   gather the inputs of the synthesis step (reviews, prior state, diff since the last review)
  publish   post new inline comments, reply to resolved ones, update the summary comment

Standard library only; talks to GitHub through the `gh` CLI (GH_TOKEN in the environment).
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
FINDING_MARKER = "<!-- grooph-finding:{} -->"
FINDING_MARKER_RE = re.compile(r"<!-- grooph-finding:(F\d+) -->")
SEVERITY_ICON = {"high": "🔴", "medium": "🟠", "low": "🟡"}
VERDICT_ICON = {"met": "✅ Met", "partly": "⚠️ Partly met", "not_met": "❌ Not met", "unclear": "❓ Unclear"}


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


def cmd_extract(args):
    texts, errors = [], []
    for line in Path(args.events).read_text().splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("type") == "text":
            texts.append(event["part"].get("text", ""))
        elif event.get("type") == "error":
            error = event.get("error") or {}
            errors.append((error.get("data") or {}).get("message") or error.get("name") or "unknown error")

    answer = None
    # The final answer is the last text; earlier texts are remarks between tool calls.
    for text in reversed(texts):
        answer = parse_json_answer(text)
        if answer is not None:
            break

    meta = {"reviewer": args.reviewer, "model": args.model, "kind": args.kind}
    if answer is None:
        reason = "; ".join(errors) or ("no JSON in the final answer" if texts else "no answer")
        result = {**meta, "error": reason}
    else:
        answer.setdefault("findings", [])
        result = {**answer, **meta}
    Path(args.out).write_text(json.dumps(result, indent=2, ensure_ascii=False))
    if "error" in result:
        print(f"::error::{args.reviewer}: {result['error']}")
        return 1
    print(f"{args.reviewer}: {len(result['findings'])} findings")
    return 0


# ---------------------------------------------------------------- prepare


def find_summary_comment(repo, pr):
    comments = gh_paginated(f"repos/{repo}/issues/{pr}/comments")
    for comment in comments:
        if comment["user"]["login"] == "github-actions[bot]" and MARKER in comment["body"]:
            return comment
    return None


def read_state(comment):
    if comment is None:
        return None
    match = STATE_RE.search(comment["body"])
    if not match:
        return None
    return json.loads(base64.b64decode(match.group(1)))


def cmd_prepare(args):
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    reviews = []
    for path in sorted(Path(args.reviews).glob("*.json")):
        reviews.append(json.loads(path.read_text()))
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

    context = {
        "pr": int(args.pr),
        "head_sha": args.head,
        "reviewers": [r["reviewer"] for r in reviews if "error" not in r],
        "failed_reviewers": [{"reviewer": r["reviewer"], "error": r["error"]} for r in reviews if "error" in r],
        "prior_review": prior is not None,
        "diff_since_prior": (out / "diff-since-prior.patch").exists(),
    }
    (out / "context.json").write_text(json.dumps(context, indent=2))
    print(json.dumps(context, indent=2))
    if not context["reviewers"]:
        print("::error::no reviewer answered")
        return 1
    return 0


# ---------------------------------------------------------------- publish


def diff_lines(repo, pr):
    """Right-side line numbers inside the PR diff, per file: inline comments only work there."""
    lines = {}
    for file in gh_paginated(f"repos/{repo}/pulls/{pr}/files"):
        found = lines.setdefault(file["filename"], set())
        number = 0
        for row in (file.get("patch") or "").splitlines():
            hunk = re.match(r"@@ -\d+(?:,\d+)? \+(\d+)", row)
            if hunk:
                number = int(hunk.group(1))
            elif row.startswith("-"):
                continue
            elif not row.startswith("\\"):
                found.add(number)
                number += 1
    return lines


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
    parts = [f"{icon} **{finding['id']} · {finding['title']}**", "", finding["body"]]
    if finding.get("found_by"):
        parts += ["", f"<sub>Found by: {', '.join(finding['found_by'])}</sub>"]
    parts.append(FINDING_MARKER.format(finding["id"]))
    return "\n".join(parts)


def cell(text):
    return str(text or "").replace("|", "\\|").replace("\n", " ")


def render_summary(synthesis, state, context, args, newly_resolved):
    short = args.head[:7]
    out = [MARKER, "## Code review", ""]
    reviewers = ", ".join(context["reviewers"]) or "none"
    out.append(f"Reviewed `{short}` · reviewers: {reviewers} · synthesis: {args.model}")
    for failed in context["failed_reviewers"]:
        out.append(f"\n⚠️ Reviewer **{failed['reviewer']}** failed: {cell(failed['error'])}")
    out.append("")

    if synthesis.get("requirements_source"):
        out += [f"**Requirements source:** {synthesis['requirements_source']}", ""]
    if synthesis.get("requirements"):
        out += ["### Requirements", "| # | Requirement | Verdict | Where / what is missing |", "|---|---|---|---|"]
        for i, req in enumerate(synthesis["requirements"], 1):
            verdict = VERDICT_ICON.get(req.get("verdict"), cell(req.get("verdict")))
            out.append(f"| {i} | {cell(req.get('text'))} | {verdict} | {cell(req.get('note'))} |")
        out.append("")

    open_findings = [f for f in state["findings"] if f["status"] == "open"]
    order = {"high": 0, "medium": 1, "low": 2}
    open_findings.sort(key=lambda f: (order.get(f["severity"], 3), int(f["id"][1:])))
    if open_findings:
        out.append("### Findings")
        for f in open_findings:
            icon = SEVERITY_ICON.get(f["severity"], "🟡")
            out.append(f"- {icon} **{f['id']} · {f['title']}**{location(f)}: {f['body']}")
            if found_by(f):
                out.append(f"  {found_by(f)}")
        out.append("")

    if newly_resolved:
        out.append(f"### Resolved in `{short}`")
        for f in newly_resolved:
            label = "resolved" if f["status"] == "resolved" else "withdrawn"
            out.append(f"- ~~{f['id']} · {f['title']}~~ {label}: {f.get('resolution', '')}")
        out.append("")

    earlier = [f for f in state["findings"] if f["status"] != "open" and f not in newly_resolved]
    if earlier:
        out.append(f"<details><summary>Resolved earlier ({len(earlier)})</summary>\n")
        for f in earlier:
            out.append(f"- ~~{f['id']} · {f['title']}~~ {f['status']} in `{f.get('resolved_in', '')[:7]}`: {f.get('resolution', '')}")
        out.append("\n</details>\n")

    if synthesis.get("out_of_scope"):
        out.append("### Out of scope / unrequested changes")
        out += [f"- {item}" for item in synthesis["out_of_scope"]]
        out.append("")

    discarded = synthesis.get("discarded") or []
    if discarded:
        out.append(f"<details><summary>Discarded reviewer findings ({len(discarded)})</summary>\n")
        for d in discarded:
            by = ", ".join(d.get("found_by") or [])
            out.append(f"- **{d.get('title', '')}** ({by}): {d.get('reason', '')}")
        out.append("\n</details>\n")

    if synthesis.get("verdict"):
        out += [f"**Verdict:** {synthesis['verdict']}", ""]
    out.append(f"<sub>[Workflow run]({args.run_url})</sub>")
    encoded = base64.b64encode(json.dumps(state, ensure_ascii=False).encode()).decode()
    out.append(f"<!-- grooph-review-state:{encoded} -->")
    return "\n".join(out)


def upsert_summary(repo, pr, comment, body):
    if DRY_RUN:
        print(f"--- summary comment ({'update' if comment else 'new'}) ---\n{body}")
        return
    payload = json.dumps({"body": body})
    if comment:
        gh("api", "-X", "PATCH", f"repos/{repo}/issues/comments/{comment['id']}", "--input", "-", input=payload)
    else:
        gh("api", "-X", "POST", f"repos/{repo}/issues/{pr}/comments", "--input", "-", input=payload)


def cmd_publish(args):
    comment = find_summary_comment(args.repo, args.pr)
    prior = read_state(comment) or {"next_id": 1, "findings": []}
    context = json.loads(Path(args.input, "context.json").read_text())

    synthesis_path = Path(args.synthesis)
    synthesis = json.loads(synthesis_path.read_text()) if synthesis_path.exists() else {"error": "no output"}
    if "error" in synthesis:
        # Keep the previous findings; only say that this run failed.
        body = (comment["body"] if comment else f"{MARKER}\n## Code review\n") + (
            f"\n\n⚠️ The review of `{args.head[:7]}` failed ({cell(synthesis['error'])}), "
            f"see the [workflow run]({args.run_url})."
        )
        upsert_summary(args.repo, args.pr, comment, body)
        print(f"::error::synthesis failed: {synthesis['error']}")
        return 1

    by_id = {f["id"]: f for f in prior["findings"]}
    next_id = prior["next_id"]
    newly_resolved = []

    for kind, status in (("resolved", "resolved"), ("withdrawn", "withdrawn")):
        for item in synthesis.get(kind) or []:
            f = by_id.get(item.get("id"))
            if f and f["status"] == "open":
                f.update(status=status, resolution=item.get("note", ""), resolved_in=args.head)
                newly_resolved.append(f)

    for item in synthesis.get("findings") or []:
        f = by_id.get(item.get("id"))
        fields = {k: item.get(k) for k in ("severity", "title", "file", "body")}
        fields["line"] = int(item["line"]) if str(item.get("line") or "").isdigit() else None
        fields["found_by"] = item.get("found_by") or (f or {}).get("found_by") or []
        if f and f["status"] == "open":
            f.update(fields)
        else:
            f = {"id": f"F{next_id}", "status": "open", "comment_id": None, **fields}
            next_id += 1
            prior["findings"].append(f)
            by_id[f["id"]] = f
    # Prior findings the synthesis did not mention stay open as they were.

    state = {"version": 1, "head_sha": args.head, "next_id": next_id, "findings": prior["findings"]}

    for f in newly_resolved:
        if f.get("comment_id") and not DRY_RUN:
            label = "Resolved" if f["status"] == "resolved" else "Withdrawn"
            reply = json.dumps({"body": f"{label} in {args.head[:7]}: {f['resolution']}"})
            gh(
                "api",
                "-X",
                "POST",
                f"repos/{args.repo}/pulls/{args.pr}/comments/{f['comment_id']}/replies",
                "--input",
                "-",
                input=reply,
                check=False,
            )

    in_diff = diff_lines(args.repo, args.pr)
    to_post = [
        f
        for f in state["findings"]
        if f["status"] == "open"
        and not f.get("comment_id")
        and f.get("file")
        and f.get("line")
        and f["line"] in in_diff.get(f["file"], set())
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
            review_id = json.loads(result.stdout)["id"]
            for posted in gh_paginated(f"repos/{args.repo}/pulls/{args.pr}/reviews/{review_id}/comments"):
                match = FINDING_MARKER_RE.search(posted["body"])
                if match and match.group(1) in by_id:
                    by_id[match.group(1)]["comment_id"] = posted["id"]
        else:
            print(f"::warning::inline comments not posted: {result.stderr.strip()}")

    upsert_summary(args.repo, args.pr, comment, render_summary(synthesis, state, context, args, newly_resolved))
    print(f"{len(to_post)} inline comments, {len(newly_resolved)} resolved")
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
