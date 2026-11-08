import copy
import hashlib
import json
import unittest

from test_real_cli_terminal_input import verify_delivery
from test_real_cli_terminal_cancel import execution_metadata


class TerminalInputAcceptanceContractTests(unittest.TestCase):
    def test_recovery_comparison_excludes_only_notification_delivery(self):
        before = {"model_notified":False,"command":"read","terminal":{"host_id":"host"},
                  "_nola_shell_worker_dispatch":{"attempt_id":"one"}}
        after = copy.deepcopy(before)
        after["model_notified"] = True
        self.assertEqual(execution_metadata(json.dumps(before)), execution_metadata(json.dumps(after)))
        after["_nola_shell_worker_dispatch"]["attempt_id"] = "two"
        self.assertNotEqual(execution_metadata(json.dumps(before)), execution_metadata(json.dumps(after)))

    def receipt(self):
        return {"phase": "delivered", "acknowledged_bytes": 6, "size_bytes": 6,
                "content_digest": hashlib.sha256(b"hello\n").hexdigest(),
                "identity": {"task_id": "task-1", "owner_session_id": "terminal-input-acceptance"}}

    def verify(self, receipts, owner="terminal-input-acceptance", status="completed"):
        verify_delivery([("task-1", owner, status, json.dumps({"_nola_terminal_input_receipts": receipts}))], "hello")

    def test_exact_delivery(self):
        self.verify({"request-1": self.receipt()})

    def test_missing_duplicate_or_unknown_receipts_fail(self):
        valid = self.receipt()
        unknown = copy.deepcopy(valid)
        unknown["phase"] = "unknown"
        for receipts in [{}, {"one": valid, "two": valid}, {"one": valid, "two": unknown}]:
            with self.subTest(receipts=receipts), self.assertRaises(AssertionError):
                self.verify(receipts)

    def test_identity_content_and_terminal_state_are_required(self):
        for key, value in [("content_digest", "wrong"), ("acknowledged_bytes", 5), ("size_bytes", 5)]:
            receipt = self.receipt()
            receipt[key] = value
            with self.subTest(key=key), self.assertRaises(AssertionError):
                self.verify({"one": receipt})
        for field in ["task_id", "owner_session_id"]:
            receipt = self.receipt()
            receipt["identity"][field] = "another"
            with self.subTest(field=field), self.assertRaises(AssertionError):
                self.verify({"one": receipt})
        for owner, status in [("other", "completed"), ("terminal-input-acceptance", "failed")]:
            with self.assertRaises(AssertionError):
                self.verify({"one": self.receipt()}, owner=owner, status=status)


if __name__ == "__main__":
    unittest.main()
