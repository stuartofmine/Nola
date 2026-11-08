"""Offline checks for completion evidence, separate from Git postconditions."""
import unittest
import json
import sqlite3

from test_real_cli_commit_followup import assert_completed_turn, assert_existing_files_unchanged, pending_scope_question


def event(reason, kind="run.completed"):
    return {"trace": {"kind": kind, "payload": {"reason": reason}}}


class CompletionContractTests(unittest.TestCase):
    def pending_fixture(self):
        db = sqlite3.connect(":memory:")
        self.addCleanup(db.close)
        db.executescript("""
            CREATE TABLE runs(id,conversation_id,status,completed_at_ms);
            CREATE TABLE user_input_wait_heads(wait_id,conversation_id,run_id,revision,state,response_schema_digest,source_event_id);
            CREATE TABLE run_events(event_id,run_id,event_kind,payload_json);
        """)
        wait = {"wait_id": "wait", "revision": 1, "response_schema_digest": "digest",
                "state": {"type": "open"}, "question_set": {"questions": [
                    {"id": "selection", "prompt": "Include this pending addition?"}]}}
        db.execute("INSERT INTO runs VALUES('run','session','waiting_input',NULL)")
        db.execute("INSERT INTO user_input_wait_heads VALUES('wait','session','run',1,'open','digest','event')")
        db.execute("INSERT INTO run_events VALUES('event','run','user_input_wait_changed',?)",
                   (json.dumps({"wait": wait}),))
        events = [{"trace": {"kind": "run.started", "run_id": "run"}},
                  {"type": "user_input_wait_changed", "record": {
                      "run_id": "run", "conversation_id": "session", "wait": wait}}]
        return db, events

    def test_committed_open_question_is_wait_not_completion(self):
        db, events = self.pending_fixture()
        self.assertEqual(pending_scope_question(db, events, "session")["wait_id"], "wait")
        with self.assertRaises(AssertionError):
            assert_completed_turn(events)

    def test_prose_or_missing_question_is_not_wait_evidence(self):
        db, _ = self.pending_fixture()
        self.assertIsNone(pending_scope_question(db, [{"text": "Should I commit?"}], "session"))

    def test_pending_question_rejects_stale_owner_terminal_and_changed_content(self):
        for change in ["owner", "closed", "terminal", "content", "missing"]:
            with self.subTest(change=change):
                db, events = self.pending_fixture()
                if change == "owner":
                    db.execute("UPDATE user_input_wait_heads SET conversation_id='other'")
                elif change == "closed":
                    db.execute("UPDATE user_input_wait_heads SET state='resolved'")
                elif change == "terminal":
                    events.append(event("answered"))
                elif change == "content":
                    events[-1]["record"]["wait"]["question_set"]["questions"][0]["prompt"] = "Different"
                else:
                    db.execute("DELETE FROM run_events")
                with self.assertRaises(AssertionError):
                    pending_scope_question(db, events, "session")

    def test_new_review_files_are_allowed(self):
        assert_existing_files_unchanged({"source": b"original"},
                                        {"source": b"original", "notes": b"new"})

    def test_task_created_notes_can_be_updated_on_followup(self):
        task_baseline = {"source": b"original"}
        assert_existing_files_unchanged(task_baseline,
                                        {"source": b"original", "notes": b"first review"})
        assert_existing_files_unchanged(task_baseline,
                                        {"source": b"original", "notes": b"expanded review"})
        with self.assertRaises(AssertionError):
            assert_existing_files_unchanged(task_baseline,
                                            {"source": b"changed", "notes": b"expanded review"})

    def test_existing_files_must_not_be_deleted_or_modified(self):
        for after in [{}, {"source": b"changed"}]:
            with self.assertRaises(AssertionError):
                assert_existing_files_unchanged({"source": b"original"}, after)

    def test_direct_answer_and_optional_assessment_are_accepted(self):
        assert_completed_turn([event("answered")])
        assert_completed_turn([event("action_completed")])

    def test_missing_incomplete_or_conflicting_terminal_is_rejected(self):
        for events in [[], [event("completed"), event("completed")], [event("needs_input")], [event("blocked")],
                       [event("incomplete")], [event("completed")],
                       [event("failed", "run.failed"), event("answered")]]:
            with self.assertRaises(AssertionError):
                assert_completed_turn(events)


if __name__ == "__main__":
    unittest.main()
