# SPDX-License-Identifier: Apache-2.0
"""Counterexamples for the independent, read-only compaction evidence audit."""
from contextlib import closing
import hashlib
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from audit_cli_compaction import audit


class CompactionAuditTests(unittest.TestCase):
    def make_fixture(self, root):
        (root / 'case').mkdir()
        source = {'type':'message', 'id':'old', 'role':'user', 'content':'Keep this fact'}
        digest = hashlib.sha256(json.dumps([source], separators=(',', ':')).encode()).hexdigest()
        compacted = {'type':'compaction', 'id':'summary', 'summary':'Retained fact',
            'source_item_ids':['old'], 'source_fingerprint':digest}
        request = {'type':'compaction_requested', 'estimated_tokens':30000,
            'source_fingerprint':digest, 'source_item_count':1}
        result = {'type':'compaction_completed', 'compaction_item_id':'summary',
            'source_fingerprint':digest, 'replaced_item_count':1, 'original_token_count':4000,
            'summary':'Retained fact'}
        rows = []
        with closing(sqlite3.connect(root / 'case/nola.db')) as db, db:
            db.execute('CREATE TABLE run_events(run_id TEXT, sequence INTEGER, payload_json TEXT, created_at_ms INTEGER)')
            db.execute('CREATE TABLE core_checkpoint_bodies(checkpoint_json TEXT)')
            for sequence, tokens, maintenance in [(1,30000,False),(4,31000,True),(5,22000,False)]:
                attempt = {'state':'succeeded', 'usage':{'input_tokens':tokens, 'output_tokens':10,
                    'cached_input_tokens':29000 if maintenance else 0,
                    'cache_usage':{'is_compaction':maintenance}},
                    'started_at_ms':sequence*1000, 'completed_at_ms':sequence*1000+500}
                db.execute('INSERT INTO run_events VALUES(?,?,?,?)', ('r',sequence,json.dumps(attempt),sequence))
                rows.append({'status':200,'toolChoice':'none' if maintenance else 'auto',
                    'toolSchemaSha256':'same-tools','requestSha256':str(sequence),'upstreamRequestSha256':str(sequence),
                    'responseUsage':{'input_tokens':tokens,'output_tokens':10}})
            for sequence, event in [(2,request),(3,result)]:
                db.execute('INSERT INTO run_events VALUES(?,?,?,?)', ('r',sequence,json.dumps(event),sequence))
            # A later Run has a smaller local event sequence. It must not be
            # interleaved with the preceding Run by sequence alone.
            later = {**attempt, 'started_at_ms':9000,'completed_at_ms':9500}
            db.execute('INSERT INTO run_events VALUES(?,?,?,?)', ('cold',1,json.dumps(later),9))
            rows.append(rows[-1])
            checkpoint = {'state':{'conversation':{'transcript':[source,compacted]}}}
            db.execute('INSERT INTO core_checkpoint_bodies VALUES(?)', (json.dumps(checkpoint),))
        (root / 'provider.ndjson').write_text('\n'.join(map(json.dumps,rows)))

    def test_source_lineage_and_run_scoped_accounting(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder); self.make_fixture(root)
            result = audit(root, True)
            self.assertTrue(result['all_sources_verified'])
            self.assertEqual(result['compactions'][0]['business_input_reduction'], 8000)

    def test_changed_canonical_source_is_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder); self.make_fixture(root)
            with closing(sqlite3.connect(root/'case/nola.db')) as db, db:
                checkpoint = json.loads(db.execute('SELECT checkpoint_json FROM core_checkpoint_bodies').fetchone()[0])
                checkpoint['state']['conversation']['transcript'][0]['content'] = 'Changed source'
                db.execute('UPDATE core_checkpoint_bodies SET checkpoint_json=?', (json.dumps(checkpoint),))
            with self.assertRaisesRegex(AssertionError, 'archive fingerprint'):
                audit(root, True)

    def test_summary_cannot_reenable_tools_or_change_the_catalog(self):
        for field, value, message in [('toolChoice','auto','permits tool calls'),
                ('toolSchemaSha256','different','catalog changed')]:
            with self.subTest(field=field), tempfile.TemporaryDirectory() as folder:
                root = Path(folder); self.make_fixture(root)
                path = root/'provider.ndjson'
                rows = list(map(json.loads,path.read_text().splitlines()))
                rows[1][field] = value
                path.write_text('\n'.join(map(json.dumps,rows)))
                with self.assertRaisesRegex(AssertionError, message):
                    audit(root, True)


if __name__ == '__main__':
    unittest.main()
