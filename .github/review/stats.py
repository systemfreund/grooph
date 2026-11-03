#!/usr/bin/env python3
"""How the automated code review's findings ended, per reviewer and severity.

Reads, for every PR with a review summary: the hidden state block (final status of each finding), the inline
comment of each finding (severity, found by) and the answers people gave (thread replies, conversation comments
naming the finding). Needs `gh` with read access. Usage: stats.py [--repo o/r] [--json out.json]
"""
import argparse
import base64
import json
import re
import subprocess
from collections import Counter, defaultdict

BOT = "github-actions[bot]"
STATE_RE = re.compile(r"<!-- grooph-review-state:([A-Za-z0-9+/=]+) -->")
FINDING_RE = re.compile(r"<!-- grooph-finding:(F\d+) -->")
FOUND_BY_RE = re.compile(r"<sub>Found by: (.*?)</sub>")
SEVERITY = {"🔴": "high", "🟠": "medium", "🟡": "low"}
TRUSTED = {"OWNER", "MEMBER", "COLLABORATOR"}
# Rough split of withdrawn findings by the synthesis' note.
WRONG_RE = re.compile(r"does not reproduce|not reproduce|already|not a bug|incorrect|wrong|no such|does not (happen|occur)"
                      r"|never|cannot happen|misread|not true|false", re.I)
ACCEPTED_RE = re.compile(r"accept|deliberate|intended|intentional|on purpose|maintainer|by design|agreed to leave", re.I)


def gh(*args):
    out = subprocess.run(["gh", *args], capture_output=True, text=True, check=True).stdout
    return json.loads(out) if out.strip() else None


def paginated(path):
    pages = gh("api", "--paginate", "--slurp", path)
    return [item for page in pages for item in page]


def search_prs(repo):
    pages = gh("api", "--paginate", "--slurp", "-X", "GET", "search/issues",
               "-f", f"q=repo:{repo} is:pr \"grooph-review\" in:comments", "-f", "per_page=100")
    return sorted({i["number"] for p in pages for i in p["items"]})


def classify_withdrawn(note):
    if ACCEPTED_RE.search(note or ""):
        return "accepted"
    if WRONG_RE.search(note or ""):
        return "wrong"
    return "other"


def collect(repo, pr):
    comments = paginated(f"repos/{repo}/issues/{pr}/comments")
    summary = next((c for c in comments if c["user"]["login"] == BOT and STATE_RE.search(c["body"])), None)
    if not summary:
        return []
    state = json.loads(base64.b64decode(STATE_RE.search(summary["body"]).group(1)))
    review_comments = paginated(f"repos/{repo}/pulls/{pr}/comments")
    info = gh("api", f"repos/{repo}/pulls/{pr}")
    inline = {}
    for c in review_comments:
        m = FINDING_RE.search(c["body"])
        if c["user"]["login"] == BOT and m and not c.get("in_reply_to_id"):
            inline.setdefault(m.group(1), c)
    people = [c for c in comments + review_comments if c["user"]["login"] != BOT and c.get("author_association") in TRUSTED]
    out = []
    for f in state["findings"]:
        c = inline.get(f["id"])
        severity = f.get("severity") or (SEVERITY.get(c["body"][:1]) if c else None)
        found_by = f.get("found_by")
        if not found_by and c:
            m = FOUND_BY_RE.search(c["body"])
            found_by = [s.strip() for s in m.group(1).split(",")] if m else []
        thread = c["id"] if c else f.get("comment_id")
        answers = [p for p in people if (thread and p.get("in_reply_to_id") == thread)
                   or re.search(rf"\b{f['id']}\b", p["body"])]
        answered = bool(answers)
        # Answered after the last review: no run has read the answer yet.
        unread = any(p["created_at"] > (state.get("answers_until") or summary["updated_at"]) for p in answers)
        out.append({
            "pr": pr,
            "merged": bool(info.get("merged_at")),
            "id": f["id"],
            "title": f.get("title"),
            "status": f["status"],
            "withdrawn_kind": classify_withdrawn(f.get("resolution")) if f["status"] == "withdrawn" else None,
            "resolution": f.get("resolution"),
            "severity": severity or "unknown",
            # Set by the synthesis since it was asked to; older findings have none.
            "domain": f.get("domain") or "unknown",
            "found_by": found_by or ["unknown"],
            "answered": answered,
            "unread": unread,
        })
    return out


def table(title, groups):
    rows = [f"### {title}", "", "| | total | fixed | withdrawn (wrong) | withdrawn (accepted) | withdrawn (other) | open | open, answered | answer after last run |",
            "|---|---|---|---|---|---|---|---|---|"]
    for key, fs in sorted(groups.items(), key=lambda kv: -len(kv[1])):
        n = Counter(f["status"] for f in fs)
        w = Counter(f["withdrawn_kind"] for f in fs if f["status"] == "withdrawn")
        open_answered = sum(1 for f in fs if f["status"] == "open" and f["answered"])
        open_unread = sum(1 for f in fs if f["status"] == "open" and f["unread"])
        pct = lambda x: f"{x} ({100 * x // len(fs)}%)"
        rows.append(f"| {key} | {len(fs)} | {pct(n['resolved'])} | {w['wrong']} | {w['accepted']} | {w['other']} | "
                    f"{pct(n['open'])} | {open_answered} | {open_unread} |")
    return "\n".join(rows) + "\n"


def lens(reviewer):
    """The lens of a reviewer named "<lens> · <model>"; a plain model name is the general review."""
    return reviewer.split(" · ")[0] if " · " in reviewer else "general"


def off_lens(findings):
    """Findings a lens reported outside its domain, and how many of them its own lens found too."""
    rows = ["### Outside the reporting lens", "",
            "| lens | with domain | outside its domain | of those also found by the domain's lens | outside, fixed |",
            "|---|---|---|---|---|"]
    by_lens = defaultdict(list)
    for f in findings:
        if f["domain"] != "unknown":
            for r in f["found_by"]:
                if lens(r) != "general":
                    by_lens[lens(r)].append(f)
    if not by_lens:
        return "No finding with a domain from a focused lens yet.\n"
    for name, fs in sorted(by_lens.items()):
        outside = [f for f in fs if f["domain"] != name]
        also = [f for f in outside if any(lens(r) == f["domain"] for r in f["found_by"])]
        fixed = sum(f["status"] == "resolved" for f in outside)
        rows.append(f"| {name} | {len(fs)} | {len(outside)} | {len(also)} | {fixed} |")
    return "\n".join(rows) + "\n"


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--repo", default="systemfreund/grooph")
    p.add_argument("--json")
    args = p.parse_args()
    findings = [f for pr in search_prs(args.repo) for f in collect(args.repo, pr)]
    if args.json:
        with open(args.json, "w") as fh:
            json.dump(findings, fh, indent=1, ensure_ascii=False)
    by_reviewer, by_severity, by_domain = defaultdict(list), defaultdict(list), defaultdict(list)
    for f in findings:
        by_severity[f["severity"]].append(f)
        by_domain[f["domain"]].append(f)
        for r in f["found_by"]:
            by_reviewer[r].append(f)
    prs = {f["pr"] for f in findings}
    print(f"# Review findings: {len(findings)} in {len(prs)} PRs\n")
    print(table("All", {"all": findings}))
    print(table("By severity", by_severity))
    print(table("By reviewer (a finding counts for each reviewer that found it)", by_reviewer))
    print(table("By domain (set by the synthesis)", by_domain))
    print(off_lens(findings))
    merged_open = [f for f in findings if f["merged"] and f["status"] == "open"]
    print(f"Merged with open findings: {len({f['pr'] for f in merged_open})} PRs, {len(merged_open)} findings, "
          f"{sum(f['answered'] for f in merged_open)} of them answered by a person, "
          f"{sum(f['unread'] for f in merged_open)} only after the last review run.\n")


if __name__ == "__main__":
    main()
