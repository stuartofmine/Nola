# SPDX-License-Identifier: Apache-2.0
"""Readback gate tests, not replacements for production seal validation."""
import copy
import importlib.util
import json
from pathlib import Path
import sqlite3
import unittest

SPEC = importlib.util.spec_from_file_location("ui_terminal_gate",
    Path(__file__).resolve().parents[1] / "verify_mac_ui_terminal.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class TerminalReadbackTests(unittest.TestCase):
    def setUp(self):
        self.db = sqlite3.connect(":memory:")
        self.addCleanup(self.db.close)
        self.db.executescript("""
            CREATE TABLE runs(id TEXT,conversation_id TEXT,assistant_message_id TEXT,status TEXT,
                completed_at_ms INTEGER,kind TEXT);
            CREATE TABLE assistant_run_terminal_seals(run_id TEXT,conversation_id TEXT,seal_json TEXT);
        """)
        self.terminal = {"conversation_id":"conversation", "run_id":"run", "assistant_message_id":"assistant",
            "status":"sealed", "completed_at_ms":7, "result_digest":"a" * 64,
            "result":{"value":{"kind":"answered"}}}
        self.seal = {"identity":{key:self.terminal[key] for key in
            ("conversation_id","run_id","assistant_message_id")},
            **{key:self.terminal[key] for key in ("completed_at_ms","result_digest","result")}}
        self.db.execute("INSERT INTO runs VALUES('run','conversation','assistant','sealed',7,'conversation')")
        self.save_seal()

    def save_seal(self):
        self.db.execute("DELETE FROM assistant_run_terminal_seals")
        self.db.execute("INSERT INTO assistant_run_terminal_seals VALUES('run','conversation',?)",
                        (json.dumps(self.seal),))
        self.db.commit()

    def verify(self, rows=None):
        return MODULE.verify(self.db, rows if rows is not None else
            [{"role":"assistant", "canonicalTerminal":self.terminal}], 1)

    def test_sealed_result_passes_without_legacy_parts_or_trace(self):
        self.assertEqual(self.verify()["canonicalSealedSuccessfulRuns"], 1)

    def test_missing_or_legacy_status_does_not_prove_completion(self):
        self.db.execute("UPDATE runs SET status='completed'")
        self.db.commit()
        with self.assertRaises(ValueError): self.verify()
        self.db.execute("UPDATE runs SET status='sealed'")
        self.db.execute("DELETE FROM assistant_run_terminal_seals")
        self.db.commit()
        with self.assertRaises(ValueError): self.verify()

    def test_report_mismatch_is_rejected(self):
        original = copy.deepcopy(self.terminal)
        for key, value in [("run_id","foreign"), ("assistant_message_id","foreign"),
                           ("result_digest","b" * 64), ("completed_at_ms",8)]:
            self.terminal = {**original,key:value}
            with self.subTest(key=key), self.assertRaises(ValueError): self.verify()

    def test_non_success_seals_remain_distinct_and_fail_success_gate(self):
        for kind in ("incomplete","blocked","failed","cancelled"):
            self.terminal["result"] = {"value":{"kind":kind}}
            self.seal["result"] = self.terminal["result"]
            self.save_seal()
            with self.subTest(kind=kind), self.assertRaises(ValueError): self.verify()

    def test_missing_duplicate_or_unaccounted_run_is_rejected(self):
        with self.assertRaises(ValueError): self.verify([{"role":"assistant"}])
        row = {"role":"assistant", "canonicalTerminal":self.terminal}
        with self.assertRaises(ValueError): self.verify([row,row])
        self.db.execute("INSERT INTO runs VALUES('extra','conversation','extra-answer','running',NULL,'conversation')")
        self.db.commit()
        with self.assertRaises(ValueError): self.verify()


if __name__ == "__main__":
    unittest.main()
