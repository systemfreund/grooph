"""Tests for review.py. Run with: python3 -m unittest discover -s .github/review"""

import json
import unittest

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

    def test_reopens_an_earlier_resolved_finding_under_its_id(self):
        state, _ = merged(
            [finding("F1", status="resolved", resolution="fixed", resolved_in="old")],
            {"findings": [{"id": "F1", "title": "back"}]},
        )
        self.assertEqual(len(state["findings"]), 1)
        self.assertEqual(state["findings"][0]["status"], "open")
        self.assertNotIn("resolution", state["findings"][0])

    def test_fields_left_out_keep_their_value(self):
        state, _ = merged([finding("F1", body="text", line=4)], {"findings": [{"id": "F1", "title": "new title"}]})
        self.assertEqual(state["findings"][0]["body"], "text")
        self.assertEqual(state["findings"][0]["line"], 4)
        self.assertEqual(state["findings"][0]["title"], "new title")

    def test_a_resolved_finding_waits_for_its_reply(self):
        state, _ = merged([finding("F1", comment_id=5)], {"resolved": [{"id": "F1", "note": "n"}]})
        self.assertIs(state["findings"][0]["replied"], False)

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


class CheckSynthesis(unittest.TestCase):
    def test_accepts_a_normal_answer(self):
        self.assertIsNone(review.check_synthesis({"findings": [{"title": "t"}], "resolved": [{"id": "F1"}]}))

    def test_rejects_the_wrong_shape(self):
        self.assertIsNotNone(review.check_synthesis({"findings": {"F1": {}}}))
        self.assertIsNotNone(review.check_synthesis({"findings": ["text"]}))
        self.assertIsNotNone(review.check_synthesis({"findings": [{"body": "no title"}]}))
        self.assertIsNotNone(review.check_synthesis({"resolved": [{"note": "no id"}]}))
        self.assertIsNotNone(review.check_synthesis({"out_of_scope": "text"}))


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
