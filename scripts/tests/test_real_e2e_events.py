import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from real_e2e_events import committed_journal_parts


class JournalEvidenceTests(unittest.TestCase):
    def test_full_body_and_notification_boundary_not_preview(self):
        with tempfile.TemporaryDirectory() as directory:
            database = Path(directory) / "fixture.db"
            identity = {"run_id": "run", "conversation_id": "conversation",
                        "assistant_message_id": "assistant", "source_store_epoch": "epoch"}
            owner = json.dumps(identity)
            header = {"identity": identity, "sequence": "1", "record_digest": "record"}
            head = {"identity": identity, "part_id": "part", "order_key": {"position": "1"},
                    "metadata": {"kind": "text", "data": {"is_final": True}},
                    "bodies": [{"field": "markdown", "content_generation": "1",
                                "utf8_bytes": "10", "chunk_count": "2", "content_root": "root2"}]}
            with sqlite3.connect(database) as db:
                db.executescript("""
                    CREATE TABLE assistant_journal_records(owner,sequence,header_json);
                    CREATE TABLE assistant_journal_parts(owner,head_json);
                    CREATE TABLE assistant_journal_chunks(digest,bytes);
                    CREATE TABLE assistant_journal_manifests(root,owner,part_id,field,generation,
                        byte_count,chunk_count,previous_root,chunk_digest);
                """)
                db.execute("INSERT INTO assistant_journal_records VALUES(?,?,?)", (owner, 1, json.dumps(header)))
                db.execute("INSERT INTO assistant_journal_parts VALUES(?,?)", (owner, json.dumps(head)))
                db.executemany("INSERT INTO assistant_journal_chunks VALUES(?,?)", [("c1", b"full "), ("c2", b"body!")])
                db.executemany("INSERT INTO assistant_journal_manifests VALUES(?,?,?,?,?,?,?,?,?)", [
                    ("root1", owner, "part", '"markdown"', 1, 5, 1, "empty", "c1"),
                    ("root2", owner, "part", '"markdown"', 1, 10, 2, "root1", "c2")])
            event = {"type": "assistant_journal_committed_v1", "through": {
                "identity": identity, "sequence": "1", "record_digest": "record"}}
            self.assertEqual(committed_journal_parts([event], database)[0]["bodies"]["markdown"], "full body!")
            event["through"]["record_digest"] = "wrong"
            with self.assertRaisesRegex(ValueError, "digest mismatch"):
                committed_journal_parts([event], database)
            event["through"]["record_digest"] = "record"
            event["through"]["identity"] = {**identity, "run_id": "other-run"}
            with self.assertRaisesRegex(ValueError, "owner is missing"):
                committed_journal_parts([event], database)
            event["through"]["identity"] = identity
            with sqlite3.connect(database) as db:
                db.execute("DELETE FROM assistant_journal_chunks WHERE digest='c1'")
            with self.assertRaisesRegex(ValueError, "missing journal body chunk"):
                committed_journal_parts([event], database)


if __name__ == "__main__":
    unittest.main()
