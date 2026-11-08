# SPDX-License-Identifier: Apache-2.0
"""Accounting regressions for the real-model acceptance evidence reader."""
import json
import sqlite3
import unittest

from test_real_code_mode import model_cost_metrics


class ModelCostMetricsTests(unittest.TestCase):
    def setUp(self):
        self.db = sqlite3.connect(":memory:")
        self.addCleanup(self.db.close)
        self.db.executescript("""
            CREATE TABLE effect_outbox_payloads(effect_id TEXT, payload_json TEXT);
            CREATE TABLE host_event_inbox(effect_id TEXT, payload_json TEXT,
                received_at_ms INTEGER, sequence INTEGER);
        """)

    def event(self, kind, usage, sequence):
        self.db.execute("INSERT INTO host_event_inbox VALUES(?,?,?,?)",
                        ("model-1", json.dumps({"payload": {"type": kind, "usage": usage}}),
                         1000, sequence))

    def test_progress_usage_is_not_counted_twice(self):
        self.db.execute("INSERT INTO effect_outbox_payloads VALUES(?,?)", (
            "model-1", json.dumps({"payload": {"type": "model_stream", "settings": {
                "step_context": {"tool_projection": {"schema_token_estimate": 37}}}}})))
        usage = {"input_tokens": 100, "output_tokens": 10,
                 "cached_input_tokens": 0, "cached_input_tokens_reported": True}
        self.event("tool_progress", usage, 1)
        self.event("model_completed", usage, 2)
        report = model_cost_metrics(self.db)
        self.assertEqual(report["completed_requests"], 1)
        self.assertEqual(report["totals"], {"input_tokens": 100, "output_tokens": 10,
                         "cached_input_tokens": 0, "schema_tokens_estimated": 37})

    def test_unknown_measurements_are_not_zero(self):
        self.event("model_completed", {"cached_input_tokens": 0,
                   "cached_input_tokens_reported": False}, 1)
        self.assertTrue(all(value is None for value in model_cost_metrics(self.db)["totals"].values()))

    def test_no_completed_requests_has_no_measured_totals(self):
        report = model_cost_metrics(self.db)
        self.assertEqual(report["completed_requests"], 0)
        self.assertTrue(all(value is None for value in report["totals"].values()))


if __name__ == "__main__":
    unittest.main()
