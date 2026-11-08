# SPDX-License-Identifier: Apache-2.0
import importlib.util
import hashlib
import json
import gzip
from pathlib import Path
import sqlite3
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    'exporter', Path(__file__).resolve().parents[2] / 'evals/export_nola_evidence.py')
exporter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(exporter)


class EvidenceTests(unittest.TestCase):
    def test_media_archives_bind_content_without_claiming_provider_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            store = root / 'db.outputs'
            store.mkdir()
            content = b'\x89PNG\r\nfixture'
            digest = hashlib.sha256(content).hexdigest()
            media = store / f'media-{digest}.bin'
            media.write_bytes(content)
            event = json.loads(self.tool_event(store / 'missing-text', 7))
            event['trace']['payload']['result']['artifacts'] = [
                {'uri': media.as_uri(), 'mime_type': 'image/png', 'operation': 'observe'},
                {'uri': (store / 'absent.bin').as_uri(), 'mime_type': 'application/pdf'}]
            # HostProgress -> AgentTraceContext -> JSON emitter uses summary,
            # not title, and the envelope owns the durable event sequence.
            prepared = {'type': 'agent_trace', 'sequence': 8, 'run_id': 'run',
                'trace': {'kind': 'tool.progress', 'sequence': 3,
                    'payload': {'summary': 'tool_media.prepared', 'metadata': {
                        'request_id': 'request', 'effect_id': 'effect',
                        'artifacts': [{'snapshot_uri': media.as_uri(), 'sha256': digest,
                                      'tool_call_id': 'call-7', 'status': 'prepared'}]}}}}
            log = root / 'events'
            log.write_text(json.dumps(event) + '\n' + json.dumps(prepared) + '\n')
            archive = exporter.materialize_tool_outputs(log, store, root / 'tool-outputs')
            first, missing = archive['media_references']
            self.assertTrue(first['media_binding_verified'])
            self.assertEqual(first['call_id'], 'call-7')
            self.assertEqual((root / first['archived_path']).read_bytes(), content)
            self.assertEqual(missing['status'], 'missing')
            self.assertFalse(archive['all_media_references_materialized'])
            preparation = archive['media_request_preparations'][0]
            self.assertEqual(preparation['request_id'], 'request')
            self.assertEqual(preparation['sequence'], 8)
            self.assertEqual(preparation['trace_sequence'], 3)
            self.assertFalse(preparation['model_receipt_proven'])

    @staticmethod
    def tool_event(path, sequence=1):
        return json.dumps({'type': 'agent_trace', 'sequence': sequence,
            'event_id': f'event-{sequence}', 'run_id': 'run',
            'trace': {'kind': 'tool.completed', 'event_id': f'event-{sequence}',
            'run_id': 'run', 'sequence': sequence, 'payload': {'call_id': f'call-{sequence}',
            'tool_name': 'bash', 'result': {'full_output_ref': str(path),
                                         'output_preview': 'short preview'}}}})

    def test_archives_complete_bytes_and_preserves_each_call_association(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            store = root / 'trial.db.outputs'
            store.mkdir()
            output = store / 'large.txt'
            content = b'whole output\n' * 10000
            output.write_bytes(content)
            log = root / 'events.gz'
            with gzip.open(log, 'wt') as stream:
                stream.write(self.tool_event(output) + '\n' + self.tool_event(output, 2) + '\n')
            archive = exporter.materialize_tool_outputs(log, store, root / 'tool-outputs')
            self.assertTrue(archive['all_references_materialized'])
            self.assertEqual(len(archive['references']), 2)
            first, second = archive['references']
            self.assertEqual(first['source_sha256'], hashlib.sha256(content).hexdigest())
            self.assertEqual((root / first['archived_path']).read_bytes(), content)
            self.assertEqual(first['archived_path'], second['archived_path'])
            self.assertEqual(second['call_id'], 'call-2')
            self.assertEqual(second['event_log_line'], 2)

    def test_missing_outside_and_symlink_are_explicit_not_preview_fallback(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            store = root / 'db.outputs'
            store.mkdir()
            outside = root / 'credentials'
            outside.write_text('SECRET')
            link = store / 'link'
            link.symlink_to(outside)
            log = root / 'events'
            log.write_text('\n'.join(self.tool_event(path, i) for i, path in enumerate(
                [store / 'missing', outside, link, Path('relative')])) + '\nnot-json\n')
            archive = exporter.materialize_tool_outputs(log, store, root / 'tool-outputs')
            self.assertFalse(archive['all_references_materialized'])
            self.assertEqual([item['status'] for item in archive['references']],
                             ['missing'] + ['rejected_outside_output_store'] * 3)
            self.assertEqual(archive['invalid_json_lines'], 1)
            self.assertFalse((root / 'tool-outputs').exists())

    def test_redaction_has_separate_original_and_archived_hashes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            store = root / 'db.outputs'
            store.mkdir()
            output = store / 'stdout'
            original = b'key=private-key Authorization: Bearer another-token\nresult=42'
            output.write_bytes(original)
            config = root / 'config.json'
            config.write_text(json.dumps({'llm': {'api_key': 'private-key'}, 'model': 'keep'}))
            log = root / 'events'
            log.write_text(self.tool_event(output) + '\n')
            archive = exporter.materialize_tool_outputs(log, store, root / 'tool-outputs',
                                                       exporter.credential_values(config))
            item = archive['references'][0]
            data = (root / item['archived_path']).read_bytes()
            self.assertNotIn(b'private-key', data)
            self.assertNotIn(b'another-token', data)
            self.assertIn(b'result=42', data)
            self.assertTrue(item['redacted'])
            self.assertNotEqual(item['source_sha256'], item['archived_sha256'])
            self.assertEqual(output.read_bytes(), original)

    def test_missing_event_log_does_not_claim_complete(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = exporter.materialize_tool_outputs(root / 'missing', root, root / 'archive')
            self.assertEqual(archive['event_log_status'], 'missing')
            self.assertFalse(archive['all_references_materialized'])

    def test_v4_envelope_sequence_and_parent_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            store = root / 'db.outputs'
            real = store / 'real'
            real.mkdir(parents=True)
            (real / 'tool.txt').write_text('output')
            (store / 'alias').symlink_to(real, target_is_directory=True)
            event = json.loads(self.tool_event(store / 'alias/tool.txt', 128))
            # v4 stores both fields. If they diverge retain the raw trace value
            # but correlate by the NDJSON envelope's sequence.
            event['trace']['sequence'] = 127
            log = root / 'events'
            log.write_text(json.dumps(event) + '\n')
            item = exporter.materialize_tool_outputs(log, store, root / 'archive')['references'][0]
            self.assertEqual(item['sequence'], 128)
            self.assertEqual(item['trace_sequence'], 127)
            self.assertEqual(item['status'], 'rejected_symlink_path')

    def test_bad_or_missing_credentials_preserve_inventory_without_copying_bodies(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            store = root / 'db.outputs'
            store.mkdir()
            output = store / 'secret.txt'
            output.write_text('unknown secret')
            log = root / 'events'
            log.write_text(self.tool_event(output) + '\n')
            malformed = root / 'config.json'
            malformed.write_text('{invalid')
            for config in [malformed, root / 'missing.json']:
                manifest = exporter.archive_with_credentials(log, store, root / 'archive', config)
                self.assertEqual(manifest['credential_status'], 'unavailable')
                self.assertEqual(manifest['references'][0]['status'], 'not_archived_credentials_unavailable')
                self.assertFalse(manifest['all_references_materialized'])
                self.assertFalse((root / 'archive').exists())

    def test_missing_database_is_not_created(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'missing.db'
            with self.assertRaises(sqlite3.OperationalError):
                exporter.collect(path)
            self.assertFalse(path.exists())

    def test_export_is_allowlisted_and_does_not_claim_verified_completion(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'trial.db'
            with sqlite3.connect(path) as db:
                db.executescript('''
                    CREATE TABLE runs(id,conversation_id,status,created_at_ms,completed_at_ms);
                    CREATE TABLE run_attempts(id,run_id,operation_id,state,record_json);
                    CREATE TABLE durable_tasks(id,kind,status,owner_session_id,parent_task_id,child_session_id,agent_type,created_at_ms,started_at_ms,ended_at_ms);
                    CREATE TABLE run_events(run_id,sequence,event_kind,created_at_ms,payload_json);
                    CREATE TABLE messages(id,conversation_id,run_id,role,status,ordinal);
                    CREATE TABLE message_parts(id,message_id,kind,ordinal,state,content_text);
                    CREATE TABLE message_part_body_references(part_id,field,reference_json);
                    CREATE TABLE assistant_run_terminal_seals(run_id,conversation_id,seal_json,seal_digest);
                    CREATE TABLE canonical_stream_heads(part_id,head_json);
                    CREATE TABLE canonical_stream_manifests(part_id,root,previous_root,operation,head_json);
                    CREATE TABLE canonical_stream_chunks(part_id,generation,chunk_index,offset_bytes,text,manifest_root);
                    INSERT INTO canonical_stream_heads VALUES ('part','{"generation":1}');
                    INSERT INTO canonical_stream_manifests VALUES ('part','old',NULL,'append','{"generation":0}');
                    INSERT INTO canonical_stream_manifests VALUES ('part','new','old','replace','{"generation":1}');
                    INSERT INTO canonical_stream_chunks VALUES ('part',1,0,0,'replacement','new');
                    INSERT INTO canonical_stream_chunks VALUES ('part',0,0,0,'superseded','old');
                    CREATE TABLE run_invocation_payloads(config_json);
                    INSERT INTO run_invocation_payloads VALUES ('SECRET_FIXTURE');
                    INSERT INTO runs VALUES ('run','conversation','running',1,NULL);
                    INSERT INTO message_parts VALUES ('part','message','text',0,'complete','answer');
                ''')
                db.executescript('CREATE TABLE store_metadata(key PRIMARY KEY,value);')
                db.executescript((Path(__file__).resolve().parents[2] /
                    'core/nola-store/src/assistant_journal_schema.sql').read_text())
                db.execute('INSERT INTO assistant_journal_chunks VALUES (?,?)',
                           ('chunk', '完整输出'.encode('utf-8')))
            evidence = exporter.collect(path)
            self.assertNotIn('SECRET_FIXTURE', str(evidence))
            self.assertEqual(evidence['parts'][0]['content_text'], 'answer')
            self.assertEqual(evidence['seals'], [])
            self.assertIsNone(evidence['runs'][0]['completed_at_ms'])
            self.assertFalse(evidence['seal_digests_verified'])
            self.assertEqual(evidence['schema_version'], 1)
            self.assertEqual(bytes.fromhex(evidence['journal_chunks'][0]['bytes_hex']).decode('utf-8'), '完整输出')
            self.assertFalse(evidence['stream_bodies_materialized'])
            self.assertEqual(evidence['stream_heads'][0]['head_json'], '{"generation":1}')
            self.assertEqual(
                [(row['generation'], row['text'], row['manifest_root'])
                 for row in evidence['stream_chunks']],
                [(0, 'superseded', 'old'), (1, 'replacement', 'new')])
            self.assertEqual(len(evidence['stream_manifests']), 2)
            with sqlite3.connect(path) as db:
                self.assertEqual(db.execute('SELECT COUNT(*) FROM canonical_stream_chunks').fetchone()[0], 2)


if __name__ == '__main__':
    unittest.main()
