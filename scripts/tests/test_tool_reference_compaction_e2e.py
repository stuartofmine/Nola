# SPDX-License-Identifier: Apache-2.0
import importlib.util
import json
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("reference_acceptance", Path(__file__).parents[1] / "run_tool_reference_compaction_e2e.py")
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class ReferenceAcceptanceTests(unittest.TestCase):
    def test_manual_maintenance_requires_new_completed_run_and_retained_source(self):
        original = {"type": "compaction", "id": "old:summary", "source_item_ids": ["tool-result"],
                    "source_fingerprint": "a" * 64, "original_token_count": 100, "summary": "old facts"}
        current = {"type": "compaction", "id": "new:summary", "source_item_ids": ["later-message"],
                   "source_fingerprint": "b" * 64, "original_token_count": 100, "summary": "later facts"}
        checkpoint = {"state": {"session_id": "session", "execution": {"active_turn": {
            "turn_id": "new", "status": "completed"}}, "conversation": {"model_context": [original, current]}}}
        event = {"reason": "manual", "replaced_item_count": 1, "source_item_count": 1,
                 "compaction_item_id": "new:summary", "source_fingerprint": "b" * 64,
                 "original_token_count": 100}
        check = lambda: runner.require_maintenance_compaction(checkpoint, "session", "new", event, {"tool-result"})
        self.assertEqual(check()["maintenance_run_id"], "new")
        for status in ["running", "failed"]:
            checkpoint["state"]["execution"]["active_turn"]["status"] = status
            with self.assertRaises(AssertionError):
                check()
        checkpoint["state"]["execution"]["active_turn"]["status"] = "completed"
        event["replaced_item_count"] = 0
        with self.assertRaises(AssertionError):
            check()
        event["replaced_item_count"] = 1
        event["compaction_item_id"] = "old:summary"
        with self.assertRaises(AssertionError):
            check()
        event["compaction_item_id"] = "new:summary"
        checkpoint["state"]["conversation"]["model_context"] = [current]
        with self.assertRaises(AssertionError):
            check()

    def page(self, offset, content, more, total):
        return {"type": "tool_output_slice", "path": "/fixture/ref", "json_pointer": None,
                "offset": offset, "next_offset": offset + len(content.encode()),
                "bytes_returned": len(content.encode()), "size_bytes": total,
                "has_more": more, "content": content}

    def test_reconstructs_exact_utf8_pages_and_deduplicates_projection(self):
        first = self.page(0, "原文", True, 9)
        second = self.page(6, "abc", False, 9)
        raw, pages = runner.reconstruct_slices([{"result": json.dumps(first)}, first, second], "/fixture/ref")
        self.assertEqual(raw, "原文abc".encode())
        self.assertEqual(pages, 2)

    def test_does_not_accept_gaps_or_guessed_completion(self):
        with self.assertRaises(AssertionError):
            runner.reconstruct_slices([self.page(0, "abc", True, 9), self.page(6, "xyz", False, 9)], "/fixture/ref")
        with self.assertRaises(AssertionError):
            runner.reconstruct_slices([self.page(0, "abc", False, 9)], "/fixture/ref")

    def test_old_complete_pages_cannot_satisfy_incomplete_cold_run(self):
        old = {"id": "old", "result": self.page(0, "abcdef", False, 6)}
        cold = {"id": "cold", "result": self.page(0, "abc", True, 6)}
        with self.assertRaises(AssertionError):
            runner.cold_result_slices([old, cold], {"old"}, "/fixture/ref")
        with self.assertRaises(AssertionError):
            runner.cold_result_slices([old], {"old"}, "/fixture/ref")
        last = {"id": "cold2", "result": self.page(3, "def", False, 6)}
        self.assertEqual(runner.cold_result_slices([old, cold, last], {"old"}, "/fixture/ref")[0], b"abcdef")

    def test_fixture_must_be_complete_not_only_marker(self):
        fixture = b"BEGIN-unique\nfull-random-payload\nEND-unique\n"
        self.assertTrue(runner.contains_fixture(json.dumps({"stdout": fixture.decode()}).encode(), fixture))
        self.assertFalse(runner.contains_fixture(b"BEGIN-unique", fixture))

    def test_compaction_must_cover_exact_tool_source_not_merely_exist(self):
        semantic = {"retained_item_ids": ["original-tool-result"], "original_token_count": 2000,
                    "compacted_token_count": 120, "source_fingerprint": "a" * 64}
        self.assertEqual(runner.require_compacted_source(semantic, {"original-tool-result"})["source_item_count"], 1)
        for invalid in [{**semantic, "retained_item_ids": ["unrelated-message"]},
                        {**semantic, "original_token_count": 0},
                        {**semantic, "compacted_token_count": 0},
                        {**semantic, "compacted_token_count": 2000}]:
            with self.assertRaises(AssertionError):
                runner.require_compacted_source(invalid, {"original-tool-result"})


if __name__ == "__main__":
    unittest.main()
