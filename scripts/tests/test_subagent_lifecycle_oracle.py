# SPDX-License-Identifier: Apache-2.0
import copy
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from test_real_cli_subagent_lifecycle import verify_cancelled_receipt, verify_foreign, verify_parent_scope, resume_steps


class SubagentLifecycleOracleTests(unittest.TestCase):
    def test_completion_metadata_is_optional_but_external_effects_are_not(self):
        verify_parent_scope([])
        verify_parent_scope([{"payload": {"tool_name": "requirements_update"}}])
        for name in ["bash", "write_file", "edit_file", "send_agent_message", "unknown_tool"]:
            with self.assertRaises(AssertionError):
                verify_parent_scope([{"payload": {"tool_name": name}}])

    def test_resume_requires_complete_contiguous_recorded_prefix(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "fixture.db").touch()
            (root / "workspace").mkdir()
            first = root / "turn-1"
            first.mkdir()
            with self.assertRaises(AssertionError):
                resume_steps(root)
            (first / "events.ndjson").touch()
            (first / "provider.endpoint").touch()
            resume_steps(root)
            third = root / "turn-3"
            third.mkdir()
            (third / "events.ndjson").touch()
            (third / "provider.endpoint").touch()
            with self.assertRaises(AssertionError):
                resume_steps(root)

    def test_sealed_is_not_itself_cancellation(self):
        receipt = {"journal_result": {"seal": {"result": {"value": {
            "kind": "cancelled", "cancellation_id": "task-user-input-cancel:task", "unknown_effect_refs": []}}}}}
        verify_cancelled_receipt(receipt, "task")
        for field, value in [("kind", "answered"), ("cancellation_id", "other"), ("unknown_effect_refs", ["unconfirmed"])]:
            invalid = copy.deepcopy(receipt)
            invalid["journal_result"]["seal"]["result"]["value"][field] = value
            with self.assertRaises(AssertionError):
                verify_cancelled_receipt(invalid, "task")

    def test_foreign_owner_requires_both_actual_denials(self):
        calls = [{"status": "failed", "payload": {"tool_name": name, "call_id": name, "arguments": {"task_id": "task"}}}
                 for name in ["read_task", "stop_task"]]
        outputs = {name: {"contract": "nola.tool_result.v1", "data": None,
                         "presentation": {"text": "Unknown task: task", "summary": "Unknown task: task"},
                         "execution": None, "full_output_ref": None, "memory_usage": None}
                   for name in ["read_task", "stop_task"]}
        verify_foreign(calls, "task", outputs)
        with self.assertRaises(AssertionError):
            verify_foreign(calls[:1], "task", outputs)
        wrong = copy.deepcopy(calls)
        wrong[0]["status"] = "succeeded"
        with self.assertRaises(AssertionError):
            verify_foreign(wrong, "task", outputs)
        outputs["read_task"]["presentation"]["text"] = "Connection lost"
        with self.assertRaises(AssertionError):
            verify_foreign(calls, "task", outputs)

    def test_foreign_owner_rejects_old_or_malformed_envelope(self):
        calls = [{"status": "failed", "payload": {"tool_name": name, "call_id": name, "arguments": {"task_id": "task"}}}
                 for name in ["read_task", "stop_task"]]
        valid = {"contract": "nola.tool_result.v1", "data": None,
                 "presentation": {"text": "Unknown task: task", "summary": "Unknown task: task"},
                 "execution": None, "full_output_ref": None, "memory_usage": None}
        for invalid in [{"error": True, "text": "Unknown task: task"},
                        dict(valid, contract="nola.tool_result.v2"),
                        {key: value for key, value in valid.items() if key != "execution"},
                        dict(valid, presentation={"text": "Unknown task: task"}),
                        dict(valid, data={"error": "Unknown task: task"}),
                        dict(valid, presentation={"text": "Unknown task: other", "summary": "failed"})]:
            with self.assertRaises(AssertionError):
                verify_foreign(calls, "task", {"read_task": invalid, "stop_task": valid})


class CurrentModelSelectionTests(unittest.TestCase):
    def test_selects_only_the_declared_connection_and_environment(self):
        from test_real_cli_subagent_lifecycle import selected_route
        config = {"llm": {"default_model": {"connection": "chosen", "model": "m"},
            "connections": [{"id": "chosen", "protocol": "chat_completions", "endpoint": "http://127.0.0.1/v1",
                "api_key_env": "FIXTURE_KEY", "models": [{"id": "m", "capabilities": {"native_tool_calls": False}}]}]}}
        route = selected_route(config, {"FIXTURE_KEY": "test-only"})
        self.assertEqual(route["capabilities"], {"native_tool_calls": False})
        with self.assertRaises(ValueError):
            selected_route(config, {})
        with self.assertRaises(KeyError):
            selected_route({"llm": {"endpoint": "old", "model": "old"}}, {})


class ConcurrentObservationTests(unittest.TestCase):
    def test_requires_both_running_tasks_and_no_completion_marker(self):
        import sqlite3
        from test_real_cli_subagent_lifecycle import observe_running_children
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            database = root / "test.db"
            prefixes = ["child-0", "child-1"]
            self.assertIsNone(observe_running_children(database, root, prefixes, 2))
            with sqlite3.connect(database) as db:
                db.execute("CREATE TABLE durable_tasks(id TEXT,child_session_id TEXT,status TEXT,cancel_requested INT,kind TEXT)")
                db.execute("INSERT INTO durable_tasks VALUES ('a','ca','running',0,'agent')")
            for prefix in prefixes:
                (root / f"{prefix}-started.txt").write_text("started")
            self.assertIsNone(observe_running_children(database, root, prefixes, 2))
            with sqlite3.connect(database) as db:
                db.execute("INSERT INTO durable_tasks VALUES ('b','cb','running',0,'agent')")
            self.assertEqual(len(observe_running_children(database, root, prefixes, 2)["tasks"]), 2)
            with sqlite3.connect(database) as db:
                db.execute("UPDATE durable_tasks SET cancel_requested=1 WHERE id='a'")
            self.assertIsNone(observe_running_children(database, root, prefixes, 2))
            with sqlite3.connect(database) as db:
                db.execute("UPDATE durable_tasks SET cancel_requested=0")
            (root / "child-0-finished.txt").write_text("finished")
            with self.assertRaises(AssertionError):
                observe_running_children(database, root, prefixes, 2)


if __name__ == "__main__":
    unittest.main()
