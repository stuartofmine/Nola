#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Actual CLI SIGKILL + PTY recovery against a controlled streaming provider.

NOLA_HARNESS_TEST_BINARY=/absolute/nola python3 -m unittest discover \
    -s scripts/tests -p test_cli_crash_recovery.py
This is fault injection, not real-model quality evidence. All data is isolated.
"""
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
from canonical_run_audit import audit_runs
from e2e.real_overwrite_permission import isolated_environment, scrub_case_credentials


@unittest.skipUnless(os.environ.get("NOLA_HARNESS_TEST_BINARY"), "requires an explicit CLI binary")
class CliCrashRecoveryTests(unittest.TestCase):
    def public_text_parts(self, db):
        parts = []
        heads = [(owner, json.loads(raw)) for owner, raw in db.execute("SELECT owner,head_json FROM assistant_journal_parts")]
        for owner, head in sorted(heads, key=lambda item: int(item[1]["order_key"]["position"])):
            if head["metadata"]["kind"] != "text" or head["metadata"]["data"].get("visibility") != "public":
                continue
            for body in head["bodies"]:
                if body["field"] != "markdown":
                    continue
                remaining = int(body["chunk_count"])
                content_root = body["content_root"]
                chunks, seen = [], set()
                while remaining:
                    self.assertNotIn(content_root, seen, "cyclic journal manifest")
                    seen.add(content_root)
                    row = db.execute("SELECT previous_root,chunk_digest FROM assistant_journal_manifests WHERE root=? AND owner=? AND part_id=?",
                                     (content_root, owner, head["part_id"])).fetchone()
                    self.assertIsNotNone(row)
                    content_root, chunk = row
                    if chunk:
                        data, = db.execute("SELECT bytes FROM assistant_journal_chunks WHERE digest=?", (chunk,)).fetchone()
                        chunks.append(bytes(data))
                        remaining -= 1
                text = b"".join(reversed(chunks)).decode()
                self.assertEqual(len(text.encode()), int(body["utf8_bytes"]))
                parts.append((bool(head["metadata"]["data"].get("is_final")), text))
        return parts

    def test_partial_stream_hard_crash_recovers_same_turn_without_duplicate_text(self):
        root = Path(tempfile.mkdtemp(prefix="nola-hard-crash-"))
        root.chmod(0o700)
        project = root / "workspace"
        project.mkdir()
        database = root / "nola.db"
        env = isolated_environment(root, "cautious")
        env.update(NOLA_LLM_MAX_RETRIES="0", TERM="xterm-256color")
        binary = Path(os.environ["NOLA_HARNESS_TEST_BINARY"]).resolve()
        session = "crash-recovery"
        prefix, answer = "CRASH_PARTIAL_", "CRASH_PARTIAL_recovered exactly once."
        release = threading.Event()
        requests = []
        lock = threading.Lock()
        report = {"passed": False, "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}
        process = child = None

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                with lock:
                    requests.append(body)
                    number = len(requests)
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.end_headers()
                def send(delta, finish=None):
                    value = {"choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}
                    self.wfile.write(("data: " + json.dumps(value) + "\n\n").encode())
                    self.wfile.flush()
                try:
                    if number == 1:
                        send({"role": "assistant", "content": prefix})
                        release.wait(90)
                    else:
                        send({"role": "assistant", "content": answer})
                        send({}, "stop")
                        self.wfile.write(b"data: [DONE]\n\n")
                        self.wfile.flush()
                except (BrokenPipeError, ConnectionResetError):
                    pass

        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        server_thread = threading.Thread(target=server.serve_forever, daemon=True)
        server_thread.start()
        endpoint = f"http://127.0.0.1:{server.server_port}/v1"
        print(f"Hard crash evidence: {root}", flush=True)
        try:
            with (root / "before.ndjson").open("w") as output:
                process = subprocess.Popen([str(binary), "Reply briefly without using tools.",
                    "--session", session, "--project", str(project), "--json", "--lang", "en",
                    "--endpoint", endpoint, "--model", "crash-fixture", "--api-key", "",
                    "--no-tools", "--max-iterations", "3"], cwd=project, env=env,
                    stdin=subprocess.DEVNULL, stdout=output, stderr=subprocess.STDOUT)
                deadline = time.monotonic() + 30
                while True:
                    self.assertIsNone(process.poll(), f"initial Host exited: {root}")
                    if database.exists():
                        with closing(sqlite3.connect(database.as_uri() + "?mode=ro", uri=True)) as db:
                            db.execute("BEGIN")
                            if db.execute("SELECT 1 FROM sqlite_master WHERE name='assistant_journal_parts'").fetchone():
                                if "".join(text for _, text in self.public_text_parts(db)) == prefix:
                                    run_id, = db.execute("SELECT id FROM runs").fetchone()
                                    break
                    self.assertLess(time.monotonic(), deadline, f"streaming prefix was not persisted: {root}")
                    time.sleep(0.05)
                report.update(partial_persisted=True, run_id=run_id)
                process.kill()
                self.assertEqual(process.wait(timeout=10), -signal.SIGKILL)
            release.set()
            with closing(sqlite3.connect(database.as_uri() + "?mode=ro", uri=True)) as db:
                self.assertEqual(db.execute("SELECT count(*) FROM assistant_run_terminal_seals").fetchone()[0], 0)
            env["NOLA_CLI_INTERACTION_MODE"] = "always"
            with (root / "recovered-terminal.log").open("w") as log:
                child = pexpect.spawn(str(binary), ["--session", session, "--project", str(project), "--lang", "en"],
                    env=env, encoding="utf-8", timeout=45, dimensions=(24, 100))
                child.logfile = log
                child.expect("Type a message")
                deadline = time.monotonic() + 45
                while True:
                    with closing(sqlite3.connect(database.as_uri() + "?mode=ro", uri=True)) as db:
                        if db.execute("SELECT status FROM runs WHERE id=?", (run_id,)).fetchone() == ("sealed",):
                            break
                    self.assertLess(time.monotonic(), deadline, f"recovery did not seal the original Run: {root}")
                    try:
                        child.read_nonblocking(65536, timeout=0.05)
                    except pexpect.TIMEOUT:
                        pass
                child.send("/exit\r")
                child.expect(pexpect.EOF)
                child.close()
                self.assertEqual(child.exitstatus, 0, str(root))
            with closing(sqlite3.connect(database.as_uri() + "?mode=ro", uri=True)) as db:
                db.execute("BEGIN")
                self.assertEqual(audit_runs(db, session), {"runCount": 1, "outcomes": ["answered"]}, str(root))
                self.assertEqual(db.execute("SELECT id FROM runs").fetchall(), [(run_id,)])
                self.assertEqual(db.execute("SELECT role,count(*) FROM messages GROUP BY role ORDER BY role").fetchall(),
                                 [("assistant", 1), ("user", 1)])
                parts = self.public_text_parts(db)
                final = [text for is_final, text in parts if is_final]
                public_text = [text for _, text in parts]
                # Recovery may retain a cleared stable Part identity. Empty
                # bodies are not visible duplicate answers; nonempty fragments
                # must compose to exactly the provider's full answer.
                self.assertEqual("".join(final), answer, str(root))
                self.assertEqual("".join(public_text), answer, f"partial replay left duplicate visible text: {root}")
            self.assertFalse((root / "home/.nola/config.json").exists(), "recovery must not create a replacement model config")
            self.assertEqual(len(requests), 2, f"unexpected additional model sampling: {root}")
            self.assertEqual(requests[0]["messages"], requests[1]["messages"], "recovery changed the frozen prompt")
            self.assertEqual(requests[0].get("tools"), requests[1].get("tools"), "recovery changed tool exposure")
            self.assertEqual(requests[0]["model"], requests[1]["model"], "recovery changed the selected model")
            report.update(passed=True, same_run=True, final_text=final, request_count=len(requests))
        except Exception as error:
            report["failure_type"] = type(error).__name__
            raise
        finally:
            cleanup_errors = []
            def cleanup(name, action):
                try:
                    action()
                except Exception as error:
                    cleanup_errors.append({"stage": name, "type": type(error).__name__})
            if process is not None and process.poll() is None:
                cleanup("initial_host_kill", process.kill)
                cleanup("initial_host_wait", lambda: process.wait(timeout=10))
            if child is not None and child.isalive():
                cleanup("recovered_host_kill", lambda: child.kill(signal.SIGKILL))
                cleanup("recovered_host_wait", lambda: child.close(force=True))
            release.set()
            cleanup("server_shutdown", server.shutdown)
            cleanup("server_close", server.server_close)
            cleanup("server_join", lambda: server_thread.join(timeout=5))
            cleanup("fixture_credentials", lambda: scrub_case_credentials(root))
            report["cleanup_errors"] = cleanup_errors
            if cleanup_errors:
                report["passed"] = False
            (root / "requests.json").write_text(json.dumps(requests))
            (root / "result.json").write_text(json.dumps(report, indent=2))
            self.assertFalse(cleanup_errors, f"cleanup failed: {root}")
