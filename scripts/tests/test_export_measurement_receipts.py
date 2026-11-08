# SPDX-License-Identifier: Apache-2.0
import importlib.util
import pathlib
import sqlite3
import unittest

spec = importlib.util.spec_from_file_location("export_measurement_receipts",
    pathlib.Path(__file__).resolve().parents[1] / "export_measurement_receipts.py")
exporter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(exporter)


class ReceiptExportTests(unittest.TestCase):
    def test_canonical_run_attempt_retry_and_delivery_chain_is_exported_without_payloads(self):
        with sqlite3.connect(":memory:") as db:
            db.executescript("""
                CREATE TABLE command_receipts(client_id TEXT,request_id TEXT,status TEXT,command_type TEXT,command_json TEXT,result_json TEXT);
                CREATE TABLE runs(id TEXT,conversation_id TEXT,kind TEXT);
                CREATE TABLE run_attempts(id TEXT,run_id TEXT,effect_id TEXT,predecessor_id TEXT,state TEXT);
                CREATE TABLE effect_outbox(effect_id TEXT,run_id TEXT,effect_kind TEXT);
                CREATE TABLE run_attempt_deliveries(attempt_id TEXT,request_id TEXT);
                CREATE TABLE host_event_inbox(run_id TEXT,effect_id TEXT,request_id TEXT,payload_json TEXT);
                INSERT INTO runs VALUES('run','session','conversation'),('other','session','conversation');
                INSERT INTO command_receipts VALUES('client','request','committed','begin_assistant_turn',
                    '{"input":{"conversation_id":"session","user_text":"SECRET"}}','{"begin":{"run_id":"run"}}');
                INSERT INTO run_attempts VALUES('first','run','effect',NULL,'failed'),('retry','run','effect','first','succeeded'),
                    ('summary','run','compact',NULL,'succeeded'),('foreign','other','foreign',NULL,'succeeded');
                INSERT INTO effect_outbox VALUES('effect','run','model_stream'),('compact','run','compact'),('foreign','other','model_stream');
                INSERT INTO run_attempt_deliveries VALUES('retry','delivery2'),('summary','delivery3');
                INSERT INTO host_event_inbox VALUES('run','effect','delivery1','{"attempt_id":"first"}');
            """)
            value = exporter.export_receipts(db, {"build_digest":"a"*64,"samples":[{
                "client_id":"client","request_id":"request","command":exporter.command_identity("client","request")}]})
            self.assertEqual(value["receipts"][0]["run_binding"], "opened_run")
            self.assertEqual(len(value["attempts"]), 3)
            self.assertEqual(sum(r["purpose"] == "compaction" for r in value["attempts"]), 1)
            self.assertEqual(sum(r["predecessor"] is not None for r in value["attempts"]), 1)
            self.assertTrue(any(r["attempt"] == exporter.identity("first") and
                r["request"] == exporter.identity("delivery1") for r in value["attempts"]))
            self.assertNotIn("SECRET", str(value))
            self.assertNotIn(exporter.identity("other"), str(value))

    def test_snapshot_keeps_missing_and_rejected_without_exporting_payloads(self):
        with sqlite3.connect(":memory:") as db:
            db.execute("CREATE TABLE command_receipts(client_id TEXT, request_id TEXT, status TEXT, command_type TEXT, command_json TEXT, result_json TEXT)")
            db.executemany("INSERT INTO command_receipts VALUES ('client',?,?,'other',?,NULL)",
                [("a", "committed", '{"secret":true}'), ("b", "rejected", '{}'), ("c", "accepted", '{}')])
            db.commit()
            samples = [{"client_id": "client", "request_id": key,
                "command": exporter.command_identity("client", key)} for key in "abcd"]
            value = exporter.export_receipts(db, {"build_digest": "a" * 64, "samples": samples})
            self.assertEqual([r["status"] for r in value["receipts"]],
                ["committed", "rejected", "recovery_required", "absent"])
            self.assertNotIn("secret", str(value))
            self.assertNotIn("client", str(value))
            self.assertFalse(db.in_transaction)

    def test_duplicate_sample_and_tuple_mismatch_fail_closed(self):
        with sqlite3.connect(":memory:") as db:
            db.execute("CREATE TABLE command_receipts(client_id TEXT, request_id TEXT, status TEXT, command_type TEXT, command_json TEXT, result_json TEXT)")
            sample = {"client_id": "a", "request_id": "bc", "command": exporter.command_identity("a", "bc")}
            for samples in ([sample, sample], [{**sample, "request_id": "different"}]):
                with self.assertRaises(ValueError):
                    exporter.export_receipts(db, {"build_digest": "a" * 64, "samples": samples})
                self.assertFalse(db.in_transaction)

    def test_tuple_hash_is_length_framed(self):
        self.assertNotEqual(exporter.command_identity("a", "bc"), exporter.command_identity("ab", "c"))


if __name__ == "__main__":
    unittest.main()
