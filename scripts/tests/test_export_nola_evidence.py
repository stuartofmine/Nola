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
                    CREATE TABLE sessions(
                        id TEXT PRIMARY KEY,
                        title TEXT NOT NULL,
                        workspace_root TEXT NOT NULL,
                        model_connection TEXT,
                        model_id TEXT,
                        created_at_ms INTEGER NOT NULL,
                        updated_at_ms INTEGER NOT NULL
                    );
                    CREATE TABLE events(
                        scope TEXT NOT NULL,
                        seq INTEGER NOT NULL,
                        kind TEXT NOT NULL,
                        payload_json TEXT NOT NULL,
                        created_at_ms INTEGER NOT NULL,
                        PRIMARY KEY (scope, seq)
                    );
                    CREATE TABLE memory_items(
                        id TEXT PRIMARY KEY,
                        namespace TEXT NOT NULL,
                        title TEXT NOT NULL,
                        content TEXT NOT NULL,
                        tags_json TEXT NOT NULL,
                        created_at_ms INTEGER NOT NULL,
                        updated_at_ms INTEGER NOT NULL
                    );
                    CREATE TABLE run_invocation_payloads(config_json TEXT);
                    INSERT INTO run_invocation_payloads VALUES ('SECRET_FIXTURE');
                    INSERT INTO sessions VALUES (
                        'session-1', 'Eval Session', '/workspace', 'openai', 'gpt-4o', 100, 200
                    );
                    INSERT INTO events VALUES (
                        'session-1', 1, 'assistant_text', '{"text":"answer"}', 150
                    );
                    INSERT INTO memory_items VALUES (
                        'mem-1', 'default', 'Editor Preference', '完整输出', '["pref"]', 120, 180
                    );
                ''')
            evidence = exporter.collect(path)
            self.assertNotIn('SECRET_FIXTURE', str(evidence))
            self.assertEqual(evidence['schema_version'], 2)
            self.assertEqual(evidence['scope'], 'isolated_evaluation_database_v2')
            self.assertEqual(evidence['sessions'][0]['id'], 'session-1')
            self.assertEqual(evidence['events'][0]['kind'], 'assistant_text')
            self.assertEqual(evidence['events'][0]['payload_json'], '{"text":"answer"}')
            self.assertEqual(evidence['memory_items'][0]['id'], 'mem-1')
            self.assertEqual(evidence['memory_items'][0]['title'], 'Editor Preference')
            with sqlite3.connect(path) as db:
                self.assertEqual(db.execute('SELECT COUNT(*) FROM events').fetchone()[0], 1)

    def test_v2_flat_tool_finished_archives_referenced_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            store = root / 'db.outputs'
            store.mkdir()
            output = store / 'v2-out.txt'
            content = b'v2 tool output\n'
            output.write_bytes(content)
            event = {
                'type': 'tool_finished',
                'call_id': 'call-v2-1',
                'name': 'bash',
                'is_error': False,
                'summary': 'ok',
                'effects': [],
                'full_output_ref': str(output),
            }
            log = root / 'events.ndjson'
            log.write_text(json.dumps(event) + '\n')
            archive = exporter.materialize_tool_outputs(log, store, root / 'tool-outputs')
            self.assertTrue(archive['all_references_materialized'])
            self.assertEqual(archive['tool_events_seen'], 1)
            self.assertEqual(archive['references'][0]['call_id'], 'call-v2-1')
            self.assertEqual(archive['references'][0]['tool_name'], 'bash')
            self.assertEqual((root / archive['references'][0]['archived_path']).read_bytes(), content)


if __name__ == '__main__':
    unittest.main()
