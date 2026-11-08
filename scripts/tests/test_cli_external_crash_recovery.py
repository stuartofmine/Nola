#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Opt-in real CLI/PTY crash test; controlled provider, not real-model evidence."""
from contextlib import closing
import hashlib
import http.server
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import pexpect

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from e2e.real_overwrite_permission import isolated_environment, scrub_case_credentials
from canonical_run_audit import audit_runs


@unittest.skipUnless(os.environ.get('NOLA_HARNESS_TEST_BINARY'), 'requires an explicit CLI binary')
class ExternalCrashTests(unittest.TestCase):
    def test_started_command_is_not_replayed_or_declared_cancelled(self):
        root = Path(tempfile.mkdtemp(prefix='nola-external-crash-'))
        root.chmod(0o700)
        project = root / 'workspace'
        project.mkdir()
        (project / 'nola.json').write_text(json.dumps({'model_capabilities': {
            'native_tool_calls': True, 'stream_text': True, 'stream_tool_calls': True}}))
        env = isolated_environment(root, 'allow_all')
        env.update(TERM='xterm-256color', NOLA_LLM_MAX_RETRIES='0')
        binary = str(Path(os.environ['NOLA_HARNESS_TEST_BINARY']).resolve())
        database = root / 'nola.db'
        requests = []
        report = {'passed': False, 'binary_sha256': hashlib.sha256(Path(binary).read_bytes()).hexdigest()}
        process = terminal = None
        command = "printf '%s\\n' \"$$\" > child.pid; printf 'started\\n' >> starts.txt; sleep 20; printf 'finished\\n' >> finishes.txt"

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass
            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                requests.append(body)
                self.send_response(200)
                self.send_header('Content-Type', 'text/event-stream')
                self.end_headers()
                names = [tool['function']['name'] for tool in body.get('tools', [])]
                if 'bash' not in names:
                    delta = {'tool_calls': [{'index': 0, 'id': 'discover', 'type': 'function', 'function': {
                        'name': 'tool_search', 'arguments': json.dumps({'query': 'select:bash'})}}]}
                else:
                    delta = {'tool_calls': [{'index': 0, 'id': 'external', 'type': 'function', 'function': {
                        'name': 'bash', 'arguments': json.dumps({'command': command, 'cwd': str(project),
                            'operation_kind': 'execute', 'timeout_sec': 30})}}]}
                for value in [dict(choices=[dict(index=0, delta=delta, finish_reason=None)]),
                              dict(choices=[dict(index=0, delta={}, finish_reason='tool_calls')])]:
                    self.wfile.write(('data: ' + json.dumps(value) + '\n\n').encode())
                self.wfile.write(b'data: [DONE]\n\n')
                self.wfile.flush()

        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        def query(sql):
            with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as db:
                return db.execute(sql).fetchall()
        print(f'External crash evidence: {root}', flush=True)
        try:
            with (root / 'before.ndjson').open('w') as output:
                process = subprocess.Popen([binary, 'Execute the requested fixture command once.', '--session',
                    'external-crash', '--project', str(project), '--json', '--lang', 'en', '--endpoint',
                    f'http://127.0.0.1:{server.server_port}/v1', '--model', 'fixture', '--api-key', '',
                    '--force-tool-set', 'all', '--max-iterations', '3'], env=env, cwd=project,
                    stdin=subprocess.DEVNULL, stdout=output, stderr=subprocess.STDOUT)
                deadline = time.monotonic() + 30
                while not (project / 'starts.txt').exists():
                    self.assertIsNone(process.poll(), str(root))
                    self.assertLess(time.monotonic(), deadline, str(root))
                    time.sleep(.05)
                pid = int((project / 'child.pid').read_text())
                report['child_before'] = subprocess.check_output(
                    ['ps', '-p', str(pid), '-o', 'pid=,ppid=,pgid=,lstart=,stat='], text=True).strip()
                self.assertEqual(int(report['child_before'].split()[1]), process.pid, 'child must belong to this Host')
                run_id, = query('SELECT id FROM runs')[0]
                report['run_id'] = run_id
                process.kill()
                self.assertEqual(process.wait(timeout=10), -signal.SIGKILL)
            self.assertEqual(query('SELECT count(*) FROM assistant_run_terminal_seals'), [(0,)])
            os.kill(pid, 0)  # Observation only; never signal a persisted PID for cleanup.
            report['child_alive_after_host_kill'] = True
            request_count = len(requests)
            env['NOLA_CLI_INTERACTION_MODE'] = 'always'
            with (root / 'terminal.log').open('w') as log:
                terminal = pexpect.spawn(binary, ['--session', 'external-crash', '--project', str(project),
                    '--lang', 'en'], env=env, encoding='utf-8', timeout=30, dimensions=(30, 120))
                terminal.logfile = log
                terminal.expect('The run is still waiting')
                terminal.expect('Context')
                terminal.send('/recover\r')
                terminal.expect('Retry may duplicate effects')
                self.assertEqual(query('SELECT count(*) FROM assistant_run_terminal_seals'), [(0,)])
                unknown_attempts = query("SELECT id FROM effect_delivery_attempts WHERE state='outcome_unknown'")
                self.assertEqual(len(unknown_attempts), 1)
                terminal.send('/recover block retain unconfirmed external effect\r')
                terminal.expect('Unknown effects retained; Run sealed as blocked')
                terminal.send('/exit\r')
                terminal.expect(pexpect.EOF)
                terminal.close()
                self.assertEqual(terminal.exitstatus, 0)
            deadline = time.monotonic() + 35
            while not (project / 'finishes.txt').exists():
                self.assertLess(time.monotonic(), deadline, 'original child did not complete')
                time.sleep(.1)
            self.assertEqual((project / 'starts.txt').read_text(), 'started\n')
            self.assertEqual((project / 'finishes.txt').read_text(), 'finished\n')
            self.assertEqual(len(requests), request_count, 'recovery sampled the provider again')
            with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as db:
                self.assertEqual(audit_runs(db, 'external-crash'), {'runCount': 1, 'outcomes': ['blocked']})
            self.assertEqual(query('SELECT id FROM runs'), [(run_id,)])
            result = json.loads(query('SELECT result_json FROM run_results')[0][0])
            self.assertEqual(query("SELECT id FROM effect_delivery_attempts WHERE state='outcome_unknown'"), unknown_attempts)
            heads = [json.loads(raw) for raw, in query('SELECT head_json FROM assistant_journal_parts')]
            tools = [head['metadata']['data'] for head in heads if head['metadata']['kind'] == 'tool'
                     and head['metadata']['data']['capability_id'] == 'bash']
            self.assertEqual(len(tools), 1)
            self.assertEqual(tools[0]['state'], 'sealed')
            self.assertIn('unknown', tools[0]['summary']['value'])
            report['unknown_attempts'] = unknown_attempts
            report['result'] = result
            report['request_count'] = len(requests)
            self.assertEqual(hashlib.sha256(Path(binary).read_bytes()).hexdigest(), report['binary_sha256'],
                             'binary changed during the scenario')
            report['passed'] = True
        except Exception as error:
            report['failure_type'] = type(error).__name__
            raise
        finally:
            if process is not None and process.poll() is None:
                process.kill()
                process.wait(timeout=10)
            if terminal is not None and terminal.isalive():
                terminal.kill(signal.SIGKILL)
                terminal.close(force=True)
            # The fixture shell is bounded. After losing Child ownership, do
            # not kill its persisted PID (which may have been recycled).
            if (project / 'starts.txt').exists():
                deadline = time.monotonic() + 35
                while not (project / 'finishes.txt').exists() and time.monotonic() < deadline:
                    time.sleep(.1)
                report['fixture_child_finished'] = (project / 'finishes.txt').exists()
                if not report['fixture_child_finished']:
                    report['passed'] = False
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)
            scrub_case_credentials(root)
            (root / 'requests.json').write_text(json.dumps(requests))
            (root / 'result.json').write_text(json.dumps(report, indent=2))
            self.assertTrue(report.get('fixture_child_finished', True), 'bounded fixture child did not finish')
