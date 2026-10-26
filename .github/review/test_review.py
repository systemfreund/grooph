"""Tests for review.py. Run with: python3 -m unittest discover -s .github/review"""

import json
import tempfile
import types
import unittest
from pathlib import Path
from unittest import mock

import review


def finding(id, status="open", **fields):
    return {"id": id, "status": status, "comment_id": None, "severity": "low", "title": id, "file": "a.rs",
            "line": 1, "body": "b", "found_by": [], **fields}


def merged(prior_findings, synthesis, next_id=None):
    prior = {"next_id": next_id or len(prior_findings) + 1, "findings": prior_findings}
    return review.merge_state(prior, synthesis, "head123")


class ParseEvents(unittest.TestCase):
    def test_takes_the_json_of_the_last_text(self):
        lines = [
            json.dumps({"type": "text", "part": {"text": 'Let me look at {"not": "this"}'}}),
            json.dumps({"type": "tool_use", "part": {}}),
            json.dumps({"type": "text", "part": {"text": 'Done.\n```json\n{"findings": [{"title": "x"}]}\n```'}}),
        ]
        self.assertEqual(review.parse_events(lines), {"findings": [{"title": "x"}]})

    def test_adds_missing_findings(self):
        lines = [json.dumps({"type": "text", "part": {"text": '{"requirements": []}'}})]
        self.assertEqual(review.parse_events(lines), {"requirements": [], "findings": []})

    def test_reports_the_server_error(self):
        lines = [json.dumps({"type": "error", "error": {"name": "UnknownError", "data": {"message": "boom"}}})]
        self.assertEqual(review.parse_events(lines), {"error": "boom"})

    def test_prefers_the_last_block_over_a_quoted_example(self):
        text = 'The format is:\n```json\n{"findings": [{"title": "example"}]}\n```\nMy answer:\n```json\n{"findings": []}\n```'
        lines = [json.dumps({"type": "text", "part": {"text": text}})]
        self.assertEqual(review.parse_events(lines), {"findings": []})

    def test_prefers_an_unfenced_answer_over_a_fenced_example(self):
        text = 'The format is:\n```json\n{"findings": [{"title": "example"}]}\n```\n{"findings": [], "requirements": []}'
        lines = [json.dumps({"type": "text", "part": {"text": text}})]
        self.assertEqual(review.parse_events(lines), {"findings": [], "requirements": []})

    def test_prefers_a_fenced_answer_after_an_unfenced_example(self):
        text = 'The format is {"findings": []}, so:\n```json\n{"findings": [{"title": "real"}]}\n```'
        lines = [json.dumps({"type": "text", "part": {"text": text}})]
        self.assertEqual(review.parse_events(lines), {"findings": [{"title": "real"}]})

    def test_an_object_without_answer_keys_is_no_answer(self):
        lines = [json.dumps({"type": "text", "part": {"text": "Nothing to report {}"}})]
        self.assertEqual(review.parse_events(lines), {"error": "no JSON in the final answer"})

    def test_reports_text_without_json(self):
        lines = [json.dumps({"type": "text", "part": {"text": "I could not finish."}})]
        self.assertEqual(review.parse_events(lines), {"error": "no JSON in the final answer"})

    def test_reports_no_answer(self):
        self.assertEqual(review.parse_events([]), {"error": "no answer"})


class PatchLines(unittest.TestCase):
    def test_counts_context_and_added_lines_but_not_removed_ones(self):
        patch = "\n".join([
            "@@ -10,3 +10,4 @@ fn x()",
            " context",      # 10
            "-removed",
            "+added",        # 11
            "+added",        # 12
            " context",      # 13
            "@@ -40,1 +41,1 @@",
            "-old",
            "+new",          # 41
            "\\ No newline at end of file",
        ])
        self.assertEqual(review.patch_lines(patch), {10, 11, 12, 13, 41})

    def test_no_patch(self):
        self.assertEqual(review.patch_lines(None), set())


class MergeState(unittest.TestCase):
    def test_numbers_new_findings_after_the_prior_ones(self):
        state, resolved = merged([finding("F1"), finding("F2")], {"findings": [{"id": None, "title": "n", "line": "7"}]})
        self.assertEqual([f["id"] for f in state["findings"]], ["F1", "F2", "F3"])
        self.assertEqual(state["findings"][2]["line"], 7)
        self.assertEqual(state["next_id"], 4)
        self.assertEqual(resolved, [])

    def test_resolves_and_withdraws_prior_findings(self):
        state, resolved = merged(
            [finding("F1", comment_id=5), finding("F2")],
            {"resolved": [{"id": "F1", "note": "fixed"}], "withdrawn": [{"id": "F2", "note": "wrong"}]},
        )
        self.assertEqual([(f["id"], f["status"]) for f in resolved], [("F1", "resolved"), ("F2", "withdrawn")])
        self.assertEqual(state["findings"][0]["resolution"], "fixed")
        self.assertEqual(state["findings"][0]["resolved_in"], "head123")
        self.assertEqual(state["findings"][0]["comment_id"], 5)

    def test_unmentioned_prior_findings_stay_open(self):
        state, _ = merged([finding("F1")], {"findings": []})
        self.assertEqual(state["findings"][0]["status"], "open")

    def test_a_finding_listed_as_open_and_resolved_stays_open_under_its_id(self):
        state, resolved = merged(
            [finding("F1", comment_id=5)],
            {"findings": [{"id": "F1", "title": "still", "line": 3}], "resolved": [{"id": "F1", "note": "?"}]},
        )
        self.assertEqual(resolved, [])
        self.assertEqual(len(state["findings"]), 1)
        self.assertEqual(state["findings"][0]["status"], "open")
        self.assertEqual(state["findings"][0]["comment_id"], 5)

    def test_the_id_of_a_closed_finding_starts_a_new_one(self):
        # Closed findings are stored without their text, so they cannot come back as they were.
        closed = {"id": "F1", "status": "resolved", "title": "old", "resolution": "fixed", "comment_id": 5}
        state, _ = merged([closed], {"findings": [{"id": "F1", "title": "back"}]})
        self.assertEqual([(f["id"], f["status"]) for f in state["findings"]], [("F1", "resolved"), ("F2", "open")])

    def test_fields_left_out_keep_their_value(self):
        state, _ = merged([finding("F1", body="text", line=4)], {"findings": [{"id": "F1", "title": "new title"}]})
        self.assertEqual(state["findings"][0]["body"], "text")
        self.assertEqual(state["findings"][0]["line"], 4)
        self.assertEqual(state["findings"][0]["title"], "new title")

    def test_a_resolved_finding_waits_for_its_reply(self):
        state, _ = merged([finding("F1", comment_id=5)], {"resolved": [{"id": "F1", "note": "n"}]})
        self.assertIs(state["findings"][0]["replied"], False)

    def test_a_resolved_finding_without_inline_comment_waits_for_nothing(self):
        state, _ = merged([finding("F1")], {"resolved": [{"id": "F1", "note": "n"}]})
        self.assertNotIn("replied", state["findings"][0])

    def test_keeps_the_prior_reviewers_when_none_are_given(self):
        state, _ = merged([finding("F1", found_by=["m1"])], {"findings": [{"id": "F1", "title": "t"}]})
        self.assertEqual(state["findings"][0]["found_by"], ["m1"])

    def test_does_not_change_the_prior_state(self):
        prior = [finding("F1")]
        merged(prior, {"resolved": [{"id": "F1", "note": "x"}]})
        self.assertEqual(prior[0]["status"], "open")


class Summary(unittest.TestCase):
    context = {"reviewers": ["m1"], "failed_reviewers": [{"reviewer": "m2", "error": "no output"}]}

    def render(self, state, synthesis=None, resolved=()):
        return review.render_summary(synthesis or {}, state, self.context, "abcdef123", "s", "http://run", list(resolved))

    def test_state_round_trip(self):
        state, _ = merged([], {"findings": [{"title": "t", "body": "b", "line": 2}]})
        body = self.render(state)
        self.assertEqual(review.read_state({"body": body}), state)
        self.assertIn("⚠️ Reviewer **m2** failed", body)

    def test_closed_findings_are_stored_without_their_text(self):
        state, resolved = merged([finding("F1", body="long text")], {"resolved": [{"id": "F1", "note": "n"}]})
        stored = review.read_state({"body": self.render(state, resolved=resolved)})
        self.assertNotIn("body", stored["findings"][0])
        self.assertEqual(stored["findings"][0]["resolution"], "n")

    def test_shortens_the_visible_text_before_the_stored_one(self):
        findings = [{"title": f"t{i}", "body": "x" * 3000, "line": i} for i in range(12)]
        state, _ = merged([], {"findings": findings})
        body = self.render(state)
        self.assertLess(len(body), review.MAX_COMMENT)
        self.assertNotIn("x" * 3000, body.split("<!-- grooph-review-state")[0])
        # The stored text stays whole: the next synthesis reads it.
        stored = review.read_state({"body": body})["findings"]
        self.assertEqual([f["body"] for f in stored], ["x" * 3000] * 12)

    def test_shortens_stored_texts_as_a_last_resort(self):
        findings = [{"title": f"t{i}", "body": "x" * 3000, "line": i} for i in range(30)]
        state, _ = merged([], {"findings": findings})
        body = self.render(state)
        self.assertLess(len(body), review.MAX_COMMENT)
        self.assertEqual(len(review.read_state({"body": body})["findings"]), 30)

    def test_drops_closed_findings_before_shortening_stored_texts(self):
        closed = [finding(f"F{i}", status="resolved", title="t" * 300, resolution="r" * 300) for i in range(1, 200)]
        opened = [finding(f"F{i}", body="x" * 1000) for i in range(200, 230)]
        state = {"findings": closed + opened, "next_id": 230, "head_sha": "h"}
        body = self.render(state)
        self.assertLess(len(body), review.MAX_COMMENT)
        stored = review.read_state({"body": body})
        self.assertEqual([f["id"] for f in stored["findings"]], [f["id"] for f in opened])
        self.assertEqual(stored["findings"][0]["body"], "x" * 1000)
        self.assertEqual(stored["next_id"], 230)

    def test_escapes_table_cells(self):
        body = self.render({"findings": [], "next_id": 1}, {"requirements": [{"text": "a | b", "verdict": "met"}]})
        self.assertIn("a \\| b", body)


class Failure(unittest.TestCase):
    context = {"failed_reviewers": [{"reviewer": "m1", "error": "boom"}]}

    def test_names_the_failed_reviewers(self):
        body = review.render_failure(None, self.context, "no output", "abcdef1", "http://run")
        self.assertIn("Reviewer **m1** failed: boom", body)
        self.assertTrue(body.startswith(review.MARKER))

    def test_replaces_the_previous_note_and_keeps_the_state(self):
        state = review.encode_state({"findings": [], "next_id": 3})
        previous = f"{review.MARKER}\n## Code review\n\nold review\n{state}"
        once = review.render_failure(previous, self.context, "first", "aaaaaaa", "http://run")
        twice = review.render_failure(once, self.context, "second", "bbbbbbb", "http://run")
        self.assertNotIn("first", twice)
        self.assertEqual(twice.count("grooph-review-failure -->"), 2)  # one opening, one closing marker
        self.assertIn("old review", twice)
        self.assertEqual(review.read_state({"body": twice})["next_id"], 3)


    def test_the_summary_keeps_closed_findings_that_wait_for_their_reply(self):
        closed = [finding(f"F{i}", status="resolved", title="t" * 300, resolution="r" * 300, replied=True)
                  for i in range(1, 200)]
        waiting = finding("F200", status="resolved", resolution="fixed", comment_id=5, replied=False)
        opened = [finding(f"F{i}", body="x" * 1000) for i in range(201, 231)]
        state = {"findings": closed + [waiting] + opened, "next_id": 231, "head_sha": "h"}
        body = review.render_summary({}, state, {"reviewers": [], "failed_reviewers": []}, "h", "s", "u", [])
        stored = review.read_state({"body": body})
        self.assertIn("F200", [f["id"] for f in stored["findings"]])
        self.assertNotIn("F1", [f["id"] for f in stored["findings"]])


class CheckSynthesis(unittest.TestCase):
    def check(self, synthesis):
        return review.check_synthesis(synthesis, {"F1"})

    def test_accepts_a_normal_answer(self):
        self.assertIsNone(self.check({"findings": [{"title": "t"}], "resolved": [{"id": "F1"}]}))

    def test_a_known_finding_may_be_listed_by_its_id_alone(self):
        self.assertIsNone(self.check({"findings": [{"id": "F1", "line": 9}]}))

    def test_rejects_the_wrong_shape(self):
        self.assertIsNotNone(self.check({"findings": {"F1": {}}}))
        self.assertIsNotNone(self.check({"findings": ["text"]}))
        self.assertIsNotNone(self.check({"findings": [{"body": "no title"}]}))
        self.assertIsNotNone(self.check({"findings": [{"id": "F9", "line": 9}]}))  # unknown id, no title
        self.assertIsNotNone(self.check({"findings": [{"id": 1, "title": "t"}]}))
        self.assertIsNotNone(self.check({"resolved": [{"note": "no id"}]}))
        self.assertIsNotNone(self.check({"out_of_scope": "text"}))


class ThreadUpdates(unittest.TestCase):
    def posted(self, f):
        return {f["id"]: {"id": f["comment_id"], "body": review.inline_body(f)}}

    def test_nothing_to_say_when_the_thread_is_current(self):
        f = finding("F1", comment_id=5)
        f["thread_key"] = review.content_key(f)
        self.assertEqual(review.thread_updates({"findings": [f]}, self.posted(f), "head123"), [])

    def test_a_changed_finding_gets_a_reply_and_a_note_on_the_original(self):
        f = finding("F1", comment_id=5)
        posted = self.posted(f)
        f["thread_key"] = review.content_key(f)
        f.update(title="sharper title", severity="medium")
        [(target, reply, original)] = review.thread_updates({"findings": [f]}, posted, "head123")
        self.assertIs(target, f)
        self.assertIn("Still open in `head123`", reply)
        self.assertIn("🟠 **F1 · sharper title**", reply)
        self.assertTrue(original.startswith("> **Updated in `head123`**"))
        # A second update replaces the note instead of stacking it.
        again = review.mark_updated(original, "fffffff")
        self.assertEqual(again.count("Updated in"), 1)

    def test_moved_lines_alone_are_no_change(self):
        f = finding("F1", comment_id=5)
        f["thread_key"] = review.content_key(f)
        f["line"] = 99
        self.assertEqual(review.thread_updates({"findings": [f]}, self.posted(f), "head123"), [])

    def test_comments_from_before_thread_keys(self):
        f = finding("F1", comment_id=5)
        posted = self.posted(f)
        self.assertEqual(review.thread_updates({"findings": [dict(f)]}, posted, "h"), [])
        changed = dict(f, body="new text")
        self.assertEqual(len(review.thread_updates({"findings": [changed]}, posted, "h")), 1)

    def test_closed_findings_get_one_reply(self):
        waiting = finding("F1", status="resolved", comment_id=5, resolution="fixed", replied=False)
        done = finding("F2", status="resolved", comment_id=6, resolution="fixed", replied=True)
        before_flags = finding("F3", status="resolved", comment_id=7, resolution="fixed")
        updates = review.thread_updates({"findings": [waiting, done, before_flags]}, {}, "head123")
        self.assertEqual([(f["id"], reply) for f, reply, _ in updates], [("F1", "Resolved in head123: fixed")])


class FakeGitHub:
    """The parts of the GitHub API that review.py uses, in memory. `fail` holds what GitHub should reject:
    "review" (a batch of inline comments), "path:line" (a single inline comment) or "reply"."""

    def __init__(self, changed_lines=range(1, 51)):
        self.issue_comments, self.review_comments, self.fail, self.calls = [], [], set(), []
        self.files = [{"filename": "a.rs", "patch": f"@@ -1,0 +1,{len(changed_lines)} @@\n" + "+x\n" * len(changed_lines)}]
        self.next_id, self.reviews = 1000, 0

    def new_id(self):
        self.next_id += 1
        return self.next_id

    def ok(self, value):
        return types.SimpleNamespace(returncode=0, stdout=json.dumps(value), stderr="")

    def rejected(self):
        return types.SimpleNamespace(returncode=1, stdout="", stderr="HTTP 422")

    def gh(self, *args, input=None, check=True):
        result = self.call(args, json.loads(input) if input else None)
        if check and result.returncode != 0:
            raise RuntimeError(result.stderr)
        return result

    def call(self, args, data):
        if "-H" in args:  # the compare diff
            return self.rejected()
        method, path = args[2], args[3].split("/", 3)[3]  # drop "repos/o/r/"
        self.calls.append((method, path))
        if method == "POST" and path == "issues/5/comments":
            comment = {"id": self.new_id(), "body": data["body"], "user": {"login": review.BOT}}
            self.issue_comments.append(comment)
            return self.ok(comment)
        if method == "PATCH" and path.startswith("issues/comments/"):
            comment = next(c for c in self.issue_comments if c["id"] == int(path.rsplit("/", 1)[1]))
            comment["body"] = data["body"]
            return self.ok(comment)
        if method == "POST" and path == "pulls/5/reviews":
            if "review" in self.fail:
                return self.rejected()
            self.reviews += 1
            for c in data["comments"]:
                self.add_review_comment(c, review=self.reviews)
            return self.ok({"id": self.reviews})
        if method == "POST" and path == "pulls/5/comments":
            if f"{data['path']}:{data['line']}" in self.fail:
                return self.rejected()
            return self.ok(self.add_review_comment(data))
        if method == "POST" and path.endswith("/replies"):
            if "reply" in self.fail:
                return self.rejected()
            parent = int(path.split("/")[-2])
            return self.ok(self.add_review_comment({"body": data["body"]}, in_reply_to_id=parent))
        if method == "PATCH" and path.startswith("pulls/comments/"):  # not under the PR number
            comment = next(c for c in self.review_comments if c["id"] == int(path.rsplit("/", 1)[1]))
            comment["body"] = data["body"]
            return self.ok(comment)
        raise AssertionError(f"unexpected call {method} {path}")

    def add_review_comment(self, data, **extra):
        comment = {"id": self.new_id(), "user": {"login": review.BOT}, "in_reply_to_id": None, **data, **extra}
        self.review_comments.append(comment)
        return comment

    def paginated(self, path):
        path = path.split("/", 3)[3]
        if path == "issues/5/comments":
            return list(self.issue_comments)
        if path == "pulls/5/comments":
            return list(self.review_comments)
        if path == "pulls/5/files":
            return self.files
        if path.startswith("pulls/5/reviews/"):
            number = int(path.split("/")[3])
            return [c for c in self.review_comments if c.get("review") == number]
        raise AssertionError(f"unexpected list {path}")

    def replies_to(self, comment_id):
        return [c["body"] for c in self.review_comments if c["in_reply_to_id"] == comment_id]

    def state(self):
        return review.read_state(self.issue_comments[0])


class Publish(unittest.TestCase):
    def setUp(self):
        self.github = FakeGitHub()
        self.dir = Path(tempfile.mkdtemp())
        (self.dir / "context.json").write_text(json.dumps({"reviewers": ["m1"], "failed_reviewers": []}))
        for name, fake in (("gh", self.github.gh), ("gh_paginated", self.github.paginated), ("print", lambda *a: None)):
            patcher = mock.patch.object(review, name, fake, create=True)
            patcher.start()
            self.addCleanup(patcher.stop)
        self.addCleanup(mock.patch.object(review, "DRY_RUN", False).stop)

    def publish(self, head, synthesis):
        (self.dir / "synthesis.json").write_text(json.dumps(synthesis))
        args = types.SimpleNamespace(repo="o/r", pr="5", head=head, input=str(self.dir),
                                     synthesis=str(self.dir / "synthesis.json"), model="s", run_url="http://run")
        return review.cmd_publish(args)

    def by_id(self):
        return {f["id"]: f for f in self.github.state()["findings"]}

    def test_first_run_posts_one_review_and_the_summary(self):
        self.assertEqual(self.publish("aaaaaaa", {"findings": [
            {"title": "in diff", "severity": "high", "file": "a.rs", "line": 3, "body": "b"},
            {"title": "outside", "severity": "low", "file": "b.rs", "line": 3, "body": "b"},
        ]}), 0)
        self.assertEqual(self.github.reviews, 1)
        findings = self.by_id()
        self.assertIsNotNone(findings["F1"]["comment_id"])
        self.assertIsNone(findings["F2"]["comment_id"])
        self.assertEqual(len(self.github.issue_comments), 1)
        self.assertIn("🔴 **F1 · in diff**", self.github.issue_comments[0]["body"])

    def test_a_rejected_batch_is_posted_one_by_one_and_the_job_fails_for_the_rest(self):
        self.github.fail |= {"review", "a.rs:4"}
        self.assertEqual(self.publish("aaaaaaa", {"findings": [
            {"title": "ok", "file": "a.rs", "line": 3, "body": "b"},
            {"title": "rejected", "file": "a.rs", "line": 4, "body": "b"},
        ]}), 1)
        findings = self.by_id()
        self.assertIsNotNone(findings["F1"]["comment_id"])
        self.assertIsNone(findings["F2"]["comment_id"])
        # Both stay in the summary.
        self.assertIn("F2 · rejected", self.github.issue_comments[0]["body"])

    def test_a_failed_reply_is_retried_once_on_the_next_run(self):
        self.publish("aaaaaaa", {"findings": [{"title": "t", "file": "a.rs", "line": 3, "body": "b"}]})
        comment_id = self.by_id()["F1"]["comment_id"]
        self.github.fail.add("reply")
        self.publish("bbbbbbb", {"resolved": [{"id": "F1", "note": "fixed"}]})
        self.assertEqual(self.github.replies_to(comment_id), [])
        self.assertIs(self.by_id()["F1"]["replied"], False)
        self.github.fail.clear()
        self.publish("ccccccc", {})
        self.publish("ddddddd", {})
        self.assertEqual(self.github.replies_to(comment_id), ["Resolved in bbbbbbb: fixed"])
        self.assertIs(self.by_id()["F1"]["replied"], True)

    def test_a_changed_finding_updates_its_thread_once(self):
        first = {"title": "t", "severity": "low", "file": "a.rs", "line": 3, "body": "b"}
        self.assertEqual(self.publish("aaaaaaa", {"findings": [first]}), 0)
        comment_id = self.by_id()["F1"]["comment_id"]
        changed = {"id": "F1", "title": "sharper", "severity": "medium", "line": 7}
        self.assertEqual(self.publish("bbbbbbb", {"findings": [changed]}), 0)
        self.assertEqual(self.publish("ccccccc", {"findings": [{"id": "F1", "line": 9}]}), 0)  # only the line moves
        self.assertEqual(self.by_id()["F1"]["line"], 9)
        self.assertEqual(self.by_id()["F1"]["body"], "b")
        [reply] = self.github.replies_to(comment_id)
        self.assertIn("Still open in `bbbbbbb`", reply)
        self.assertIn("🟠 **F1 · sharper**", reply)
        original = next(c for c in self.github.review_comments if c["id"] == comment_id)["body"]
        self.assertTrue(original.startswith("> **Updated in `bbbbbbb`**"))
        self.assertEqual(self.github.reviews, 1)

    def test_a_cancelled_run_does_not_post_twice(self):
        self.publish("aaaaaaa", {})
        before = self.github.issue_comments[0]["body"]
        real_upsert = review.upsert_summary
        calls = []

        def cancelled_after_posting(repo, pr, comment, body):
            calls.append(body)
            if len(calls) == 2:
                raise KeyboardInterrupt  # cancelled before the final update
            return real_upsert(repo, pr, comment, body)

        new = {"findings": [{"title": "t", "file": "a.rs", "line": 3, "body": "b"}]}
        with mock.patch.object(review, "upsert_summary", cancelled_after_posting):
            with self.assertRaises(KeyboardInterrupt):
                self.publish("bbbbbbb", new)
        self.assertNotEqual(self.github.issue_comments[0]["body"], before)
        self.assertIsNone(self.by_id()["F1"]["comment_id"])
        self.publish("ccccccc", {"findings": [{"id": "F1", "title": "t", "file": "a.rs", "line": 3, "body": "b"}]})
        self.assertEqual(self.github.reviews, 1)
        self.assertIsNotNone(self.by_id()["F1"]["comment_id"])

    def test_a_malformed_synthesis_keeps_the_previous_review(self):
        self.publish("aaaaaaa", {"findings": [{"title": "t", "file": "a.rs", "line": 3, "body": "b"}]})
        self.assertEqual(self.publish("bbbbbbb", {"findings": {"F1": {}}}), 1)
        body = self.github.issue_comments[0]["body"]
        self.assertIn("The review of `bbbbbbb` failed (`findings` is not a list of objects)", body)
        self.assertIn("F1 · t", body)
        self.assertEqual(self.by_id()["F1"]["status"], "open")


class Prepare(unittest.TestCase):
    def test_gives_the_synthesis_the_open_findings_and_the_failed_reviewers(self):
        github = FakeGitHub()
        state = {"version": 1, "head_sha": "old", "next_id": 3, "findings": [
            finding("F1"), {"id": "F2", "status": "resolved", "title": "t", "comment_id": None}]}
        github.issue_comments.append({"id": 1, "user": {"login": review.BOT},
                                      "body": f"{review.MARKER}\n{review.encode_state(state)}"})
        reviews, out = Path(tempfile.mkdtemp()), Path(tempfile.mkdtemp())
        (reviews / "0.json").write_text(json.dumps({"reviewer": "m1", "findings": []}))
        args = types.SimpleNamespace(repo="o/r", pr="5", head="new", reviews=str(reviews), out=str(out),
                                     expected=json.dumps(["m1", "m2"]))
        with (mock.patch.object(review, "gh", github.gh), mock.patch.object(review, "gh_paginated", github.paginated),
              mock.patch.object(review, "print", lambda *a: None, create=True)):
            self.assertEqual(review.cmd_prepare(args), 0)
        prior = json.loads((out / "prior.json").read_text())
        self.assertEqual([f["id"] for f in prior["findings"]], ["F1"])
        context = json.loads((out / "context.json").read_text())
        self.assertEqual(context["failed_reviewers"], [{"reviewer": "m2", "error": "no output"}])
        self.assertFalse(context["diff_since_prior"])


class Context(unittest.TestCase):
    def test_reviewers_without_output_count_as_failed(self):
        reviews = [{"reviewer": "m1", "findings": []}, {"reviewer": "m2", "error": "boom"}]
        context = review.build_context("5", "h", reviews, ["m1", "m2", "m3"], None, False)
        self.assertEqual(context["reviewers"], ["m1"])
        self.assertEqual(
            context["failed_reviewers"],
            [{"reviewer": "m2", "error": "boom"}, {"reviewer": "m3", "error": "no output"}],
        )


if __name__ == "__main__":
    unittest.main()
