# SPDX-License-Identifier: Apache-2.0
import json
import sqlite3
import tempfile
import unittest
from pathlib import Path

from compaction_continuation_evidence import verify


class CompactionEvidenceTests(unittest.TestCase):
    def fixture(self, mutation=None):
        rows = [
            ("run", 1, "compaction_requested", {"source_fingerprint": "source"}),
            ("run", 2, "host_progress", {"title": "compaction_request_metrics", "effect_id": "effect", "turn_id": "turn", "metadata": {
                "accepted_summary": False, "continuation_count": 0, "request_digest": "prefix"}}),
            ("run", 3, "host_progress", {"title": "compaction_request_metrics", "effect_id": "effect", "turn_id": "turn", "metadata": {
                "strategy": "summary_continuation", "accepted_summary": True, "continuation_count": 1,
                "partial_summary_bytes": 458, "request_digest": "prefix"}}),
            ("run", 4, "compaction_completed", {"source_fingerprint": "source", "summary": "summary", "compaction_item_id": "item"}),
        ]
        if mutation:
            mutation(rows)
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "test.db"
            with sqlite3.connect(path) as db:
                db.execute("CREATE TABLE run_events(run_id TEXT, sequence INTEGER, event_kind TEXT, payload_json TEXT)")
                db.executemany("INSERT INTO run_events VALUES(?,?,?,?)", [(run, seq, kind, json.dumps(body)) for run, seq, kind, body in rows])
            return verify(path)

    def test_committed_continuation_does_not_require_trace_table(self):
        self.assertEqual(self.fixture()["partial_summary_bytes"], 458)

    def test_rejects_unrelated_or_uncommitted_evidence(self):
        mutations = [
            lambda rows: rows.pop(),
            lambda rows: rows[2][3].update(effect_id="other"),
            lambda rows: rows[2][3].update(turn_id="other"),
            lambda rows: rows[2][3]["metadata"].update(request_digest="other"),
            lambda rows: rows[2][3]["metadata"].update(partial_summary_bytes=0),
            lambda rows: rows[3][3].update(source_fingerprint="other"),
            lambda rows: rows.__setitem__(3, ("other", *rows[3][1:])),
            lambda rows: rows.insert(3, ("run", 3, "compaction_requested", {"source_fingerprint": "source"})),
        ]
        for mutation in mutations:
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.fixture(mutation)


if __name__ == "__main__":
    unittest.main()
