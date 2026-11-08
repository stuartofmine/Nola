# SPDX-License-Identifier: Apache-2.0
"""Readback contract fixtures, not simulated production seal validation."""
import copy
import importlib.util
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest

MODULE_PATH = Path(__file__).resolve().parents[1] / "verify_mac_automation_acceptance.py"
SPEC = importlib.util.spec_from_file_location("mac_automation_audit", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class AutomationReadbackTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        root = Path(self.directory.name)
        self.database = root / "fixture.db"
        artifact = root / "output.json"
        artifact.write_text(json.dumps({"status": "validated", "source": "fixture", "skill": "analyze-data"}))
        binding = {"occurrence_id": "occurrence", "conversation_id": "conversation", "run_id": "run"}
        seal = {"identity": {"conversation_id": "conversation", "run_id": "run", "assistant_message_id": "assistant"},
                "seal_digest": "a" * 64, "result": {"value": {"kind": "action_completed"}}}
        receipt = {"binding": binding, "terminal": seal}
        with sqlite3.connect(self.database) as connection:
            connection.executescript("""
                CREATE TABLE automation_runs(id TEXT, automation_id TEXT, occurrence_key TEXT);
                CREATE TABLE automation_execution_bindings(occurrence_id TEXT, conversation_id TEXT, run_id TEXT, binding_json TEXT);
                CREATE TABLE automation_execution_receipts(occurrence_id TEXT, receipt_json TEXT);
                CREATE TABLE runs(id TEXT, conversation_id TEXT, status TEXT);
                CREATE TABLE assistant_run_terminal_seals(run_id TEXT, conversation_id TEXT, seal_json TEXT);
            """)
            connection.execute("INSERT INTO automation_runs VALUES ('occurrence','automation','key')")
            connection.execute("INSERT INTO automation_execution_bindings VALUES ('occurrence','conversation','run',?)", (json.dumps(binding),))
            connection.execute("INSERT INTO automation_execution_receipts VALUES ('occurrence',?)", (json.dumps(receipt),))
            connection.execute("INSERT INTO runs VALUES ('run','conversation','sealed')")
            connection.execute("INSERT INTO assistant_run_terminal_seals VALUES ('run','conversation',?)", (json.dumps(seal),))
        self.report = {"automation_chain": {
            "matched": True, "source_refreshed": True, "source_consumed": True, "skill_loaded": True,
            "output_written": True, "activity_recorded": True, "autonomous_orchestration_complete": True,
            "occurrence_id": "occurrence", "occurrence_key": "key", "automation_id": "automation",
            "conversation_id": "conversation", "run_id": "run", "assistant_message_id": "assistant",
            "seal_digest": "a" * 64, "canonical_receipt": receipt, "fixture_root": str(root),
            "canonical_database": str(self.database), "output_path": str(artifact)}}

    def test_exact_persisted_receipt_and_artifact_are_required(self):
        self.assertTrue(MODULE.verify(self.report, "fixture")["passed"])
        with self.assertRaises(ValueError):
            MODULE.verify(self.report, "different source")

    def test_other_occurrence_identity_and_changed_seal_fail(self):
        for key in ("occurrence_id", "occurrence_key", "automation_id", "conversation_id", "run_id", "assistant_message_id"):
            changed = copy.deepcopy(self.report)
            changed["automation_chain"][key] = "different"
            with self.subTest(key=key), self.assertRaises(ValueError):
                MODULE.verify(changed, "fixture")
        changed = copy.deepcopy(self.report)
        changed["automation_chain"]["canonical_receipt"]["terminal"]["seal_digest"] = "b" * 64
        with self.assertRaises(ValueError):
            MODULE.verify(changed, "fixture")

    def test_unsealed_run_and_missing_canonical_skill_do_not_pass_from_artifact(self):
        with sqlite3.connect(self.database) as connection:
            connection.execute("UPDATE runs SET status='running'")
        with self.assertRaises(ValueError):
            MODULE.verify(self.report, "fixture")
        # Restore the unrelated failing condition so the missing Skill gate is
        # independently exercised, rather than passing because the Run is open.
        with sqlite3.connect(self.database) as connection:
            connection.execute("UPDATE runs SET status='sealed'")
        self.assertTrue(MODULE.verify(self.report, "fixture")["passed"])
        self.report["automation_chain"]["skill_loaded"] = False
        with self.assertRaises(ValueError):
            MODULE.verify(self.report, "fixture")

    def test_matching_report_cannot_hide_a_different_persisted_receipt(self):
        receipt = copy.deepcopy(self.report["automation_chain"]["canonical_receipt"])
        receipt["binding"]["run_id"] = "another-run"
        with sqlite3.connect(self.database) as connection:
            connection.execute("UPDATE automation_execution_receipts SET receipt_json=?",
                               (json.dumps(receipt),))
        with self.assertRaises(ValueError):
            MODULE.verify(self.report, "fixture")


if __name__ == "__main__":
    unittest.main()
