#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Opt-in real CLI -> local HTTP fixture -> OS signal -> durable seal test.

NOLA_SIGNAL_TEST_BINARY=/absolute/path/to/nola python3 -m unittest \
    scripts.tests.test_cli_signal_cancellation
No production configuration, credentials, provider or database is used.
"""
import http.server
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import tempfile
import threading
import unittest


@unittest.skipUnless(os.environ.get("NOLA_SIGNAL_TEST_BINARY"), "requires explicitly selected CLI binary")
class CliSignalCancellationTests(unittest.TestCase):
    def run_signal(self, requested_signal):
        root = Path(tempfile.mkdtemp(prefix="nola-signal-acceptance-"))
        home = root / "home"
        project = root / "workspace"
        (home / ".nola").mkdir(parents=True)
        project.mkdir()
        admitted = threading.Event()
        release = threading.Event()

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                self.rfile.read(int(self.headers.get("Content-Length", "0")))
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.end_headers()
                self.wfile.write(b': waiting for controlled cancellation\n\n')
                self.wfile.flush()
                admitted.set()
                release.wait(40)

        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        config = {"llm": {"endpoint": f"http://127.0.0.1:{server.server_port}/v1",
                          "api_key": "isolated-fixture", "model": "signal-fixture"},
                  "network_policy": "allow"}
        (home / ".nola/config.json").write_text(json.dumps(config))
        # Do not inherit provider credentials, routing or native host mode.
        env = {key: value for key, value in os.environ.items()
               if not key.startswith(("NOLA_", "OPENAI_", "ANTHROPIC_"))}
        env.update(NOLA_HOME=str(home), NOLA_DB_PATH=str(root / "session.db"),
                   NOLA_SKIP_ONBOARDING="1", NOLA_CLI_INTERACTION_MODE="never")
        process = None
        try:
            with (root / "events.ndjson").open("w") as output:
                process = subprocess.Popen([
                    str(Path(os.environ["NOLA_SIGNAL_TEST_BINARY"]).resolve()),
                    "--project", str(project), "--json", "--yes", "--lang", "en",
                    "--max-iterations", "2", "Reply briefly without using tools."],
                    cwd=project, env=env, stdin=subprocess.DEVNULL,
                    stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
                self.assertTrue(admitted.wait(30), f"provider not reached: {root}")
                process.send_signal(requested_signal)
                process.wait(timeout=15)
            connection = sqlite3.connect(f"file:{root / 'session.db'}?mode=ro", uri=True)
            try:
                runs = connection.execute("SELECT id,status FROM runs").fetchall()
                seals = [json.loads(row[0]) for row in connection.execute(
                    "SELECT seal_json FROM assistant_run_terminal_seals")]
            finally:
                connection.close()
            (root / "result.json").write_text(json.dumps({"signal": requested_signal,
                "returncode": process.returncode, "runs": runs, "seals": seals}, indent=2))
            self.assertEqual(len(runs), 1, str(root))
            if requested_signal == signal.SIGKILL:
                self.assertEqual(process.returncode, -signal.SIGKILL, str(root))
                self.assertEqual(seals, [], str(root))
            else:
                self.assertGreaterEqual(process.returncode, 0, str(root))
                # Store lifecycle is sealed; cancellation is the typed result,
                # not a second competing Run status vocabulary.
                self.assertEqual(runs[0][1], "sealed", str(root))
                self.assertEqual(len(seals), 1, str(root))
                self.assertEqual(seals[0]["result"]["run_id"], runs[0][0], str(root))
                self.assertEqual(seals[0]["result"]["value"]["kind"], "cancelled", str(root))
                self.assertTrue(seals[0]["result"]["value"]["cancellation_id"], str(root))
            print(f"signal acceptance {requested_signal}: {root}")
        finally:
            if process is not None and process.poll() is None:
                process.kill()
                process.wait()
            release.set()
            server.shutdown()
            server.server_close()
            worker.join()

    def test_sigint_commits_cancelled(self):
        self.run_signal(signal.SIGINT)

    def test_sigterm_commits_cancelled(self):
        self.run_signal(signal.SIGTERM)

    def test_sigkill_does_not_fabricate_terminal_seal(self):
        self.run_signal(signal.SIGKILL)
