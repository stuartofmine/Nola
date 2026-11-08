# SPDX-License-Identifier: Apache-2.0
import sys
from pathlib import Path
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parents[1]))
from test_real_cli_unknown_tool_recovery import AuditedFaultProxy, verify_recovery


class UnknownRecoveryTests(unittest.TestCase):
    def test_only_one_name_changes_and_id_arguments_are_preserved(self):
        proxy = AuditedFaultProxy("http://192.0.2.1/v1", Path("/unused"))
        try:
            event = {"choices": [{"delta": {"tool_calls": [{"id": "stable", "index": 0,
                "function": {"name": "read_file", "arguments": '{"path":"资料.txt"}'}}]}}]}
            proxy._rewrite_event(event)
            call = event["choices"][0]["delta"]["tool_calls"][0]
            self.assertEqual(call["function"]["name"], "nola_unknown_tool_probe")
            self.assertEqual(call["id"], "stable")
            self.assertEqual(call["function"]["arguments"], '{"path":"资料.txt"}')
            another = {"choices": [{"message": {"tool_calls": [{"function": {"name": "read_file"}}]}}]}
            proxy._rewrite_event(another)
            self.assertEqual(another["choices"][0]["message"]["tool_calls"][0]["function"]["name"], "read_file")
            self.assertEqual(len(proxy.injections), 1)
        finally:
            proxy._server.server_close()

    @patch("test_real_cli_unknown_tool_recovery.assert_completed_turn")
    def test_injection_or_final_answer_alone_never_proves_recovery(self, _completed):
        events = [{"trace": {"kind": "tool.completed", "status": "failed", "payload": {"tool_name": "nola_unknown_tool_probe"}}}]
        with self.assertRaises((AssertionError, KeyError)):
            verify_recovery(events, [], "secret-marker", [{}])
        read = {"id": "source", "name": "read_file", "output": "secret-marker"}
        with self.assertRaises(AssertionError):
            verify_recovery(events, [read], "secret-marker", [])
        bad = events + [{"trace": {"kind": "tool.started", "payload": {"tool_name": "bash"}}}]
        with self.assertRaises(AssertionError):
            verify_recovery(bad, [read], "secret-marker", [{}])
        # Even a claimed successful read without linked canonical rejection and
        # ordered same-Run execution cannot satisfy the acceptance oracle.
        with self.assertRaises((AssertionError, KeyError)):
            verify_recovery(events, [read], "secret-marker", [{}], "secret-marker")

    def test_fragmented_tool_name_is_one_exact_injection(self):
        import json
        proxy = AuditedFaultProxy("http://192.0.2.1/v1", Path("/unused"))
        try:
            frames = [{"choices": [{"delta": {"tool_calls": [{"id": "stable", "index": 0,
                "function": {"name": "read_", "arguments": "{"}}]}}]},
                {"choices": [{"delta": {"tool_calls": [{"index": 0,
                "function": {"name": "file", "arguments": "}"}}]}}]}]
            body = b"".join(b"data: " + json.dumps(frame).encode() + b"\n\n" for frame in frames)
            changed = proxy._rewrite(body, "text/event-stream")
            self.assertNotIn(b'"name":"file"', changed)
            self.assertEqual(proxy.injections[0]["original_name"], "read_file")
            later = proxy._rewrite(body, "text/event-stream")
            self.assertEqual([json.loads(line[6:]) for line in later.splitlines() if line.startswith(b"data: ")], frames)
        finally:
            proxy._server.server_close()

    @patch("test_real_cli_unknown_tool_recovery.assert_completed_turn")
    def test_requires_ordered_same_run_success_and_final_answer(self, _completed):
        import copy
        events = [ {"trace": {"kind": "tool.completed", "status": status, "run_id": "run", "sequence": seq,
            "payload": {"tool_name": name, "call_id": call}}} for status, seq, name, call in
            [("failed", 1, "nola_unknown_tool_probe", "effect::injected"), ("succeeded", 2, "read_file", "read-call")] ]
        results = [{"id": "rejection", "call_id": "effect::injected", "name": "nola_unknown_tool_probe", "is_error": True,
            "output": {"error": "tool_not_exposed_in_model_step", "tool_execution_skipped": True}},
            {"id": "read", "call_id": "read-call", "name": "read_file", "is_error": False, "output": {"text": "marker"}}]
        injection = [{"call_id": "injected"}]
        self.assertEqual(verify_recovery(events, results, "marker", injection, "marker")["read_result_ids"], ["read"])
        for field, value in [("sequence", 0), ("run_id", "other"), ("status", "failed")]:
            bad = copy.deepcopy(events)
            bad[1]["trace"][field] = value
            with self.assertRaises(AssertionError):
                verify_recovery(bad, results, "marker", injection, "marker")
        with self.assertRaises(AssertionError):
            verify_recovery(events, results, "marker", injection, "not answered")
        results[1]["is_error"] = True
        with self.assertRaises(AssertionError):
            verify_recovery(events, results, "marker", injection, "marker")
