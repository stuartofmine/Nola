#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Opt-in CLI -> HTTP fixture -> Store acceptance for ordinary technical prose.

Set NOLA_HARNESS_TEST_BINARY to the freshly built CLI. This is a deterministic
transport fixture, not real-model quality evidence. It never uses user data or
credentials and retains isolated evidence directories for diagnosis.
"""
import http.server
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import threading
import unittest
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from real_e2e_events import committed_journal_parts, load_events


@unittest.skipUnless(os.environ.get("NOLA_HARNESS_TEST_BINARY"), "requires freshly built CLI")
class CliHarnessAnswerContractTests(unittest.TestCase):
    def assert_answer_is_not_resampled(self, prompt, answer):
        root = Path(tempfile.mkdtemp(prefix="nola-answer-contract-"))
        application_home = root / "app-home"
        project = root / "workspace"
        (application_home / ".nola").mkdir(parents=True)
        project.mkdir()
        requests = []

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                requests.append(json.loads(self.rfile.read(int(self.headers["Content-Length"]))))
                chunks = [
                    {"choices": [{"index": 0, "delta": {"role": "assistant", "content": answer},
                                  "finish_reason": None}]},
                    {"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
                     "usage": {"prompt_tokens": 128, "completion_tokens": 32, "total_tokens": 160}},
                ]
                body = ("".join("data: " + json.dumps(chunk) + "\n\n" for chunk in chunks)
                        + "data: [DONE]\n\n").encode()
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        config = {"llm": {"endpoint": f"http://127.0.0.1:{server.server_port}/v1",
                          "api_key": "isolated-fixture", "model": "answer-fixture"},
                  "network_policy": "allow"}
        (application_home / ".nola/config.json").write_text(json.dumps(config))
        env = {key: value for key, value in os.environ.items()
               if not key.startswith(("NOLA_", "OPENAI_", "ANTHROPIC_"))}
        env.update(NOLA_HOME=str(application_home), NOLA_DB_PATH=str(root / "session.db"),
                   NOLA_SKIP_ONBOARDING="1", NOLA_CLI_INTERACTION_MODE="never",
                   NOLA_LLM_MAX_RETRIES="0")
        try:
            with (root / "events.ndjson").open("w") as output:
                result = subprocess.run([
                    str(Path(os.environ["NOLA_HARNESS_TEST_BINARY"]).resolve()),
                    "--project", str(project), "--json", "--yes", "--lang", "en",
                    "--max-iterations", "3", prompt], cwd=project, env=env,
                    stdin=subprocess.DEVNULL, stdout=output, stderr=subprocess.STDOUT,
                    timeout=60, check=False)
            (root / "requests.json").write_text(json.dumps(requests, ensure_ascii=False))
            self.assertEqual(result.returncode, 0, str(root))
            self.assertEqual(len(requests), 1, f"ordinary prose caused extra sampling: {root}")
            with sqlite3.connect(f"file:{root / 'session.db'}?mode=ro", uri=True) as connection:
                seals = [json.loads(row[0]) for row in connection.execute(
                    "SELECT seal_json FROM assistant_run_terminal_seals")]
                runs = connection.execute("SELECT status FROM runs").fetchall()
            self.assertEqual(runs, [("sealed",)], str(root))
            self.assertEqual(len(seals), 1, str(root))
            self.assertEqual(seals[0]["result"]["value"]["kind"], "answered", str(root))
            parsed = load_events(root / "events.ndjson")
            self.assertTrue(any(event.get("type") == "assistant_journal_committed_v1"
                                for event in parsed), str(root))
            parts = committed_journal_parts(parsed, root / "session.db")
            final_bodies = [part["bodies"]["markdown"] for part in parts
                            if part["head"]["metadata"]["kind"] == "text"
                            and part["head"]["metadata"]["data"].get("is_final") is True]
            self.assertEqual(final_bodies, [answer], f"durable answer differs: {root}")
            print(f"answer contract evidence: {root}")
        finally:
            server.shutdown()
            server.server_close()
            worker.join()

    def test_chinese_command_output_explanation_is_not_execution(self):
        self.assert_answer_is_not_resampled(
            "请解释下面命令的示例输出，不要执行命令。",
            "这是示例，不是实际执行结果。\n```bash\nprintf hello\n```\n输出显示 hello。")

    def test_english_command_output_explanation_is_not_execution(self):
        self.assert_answer_is_not_resampled(
            "Explain sample command output; do not run any tools.",
            "This is illustrative command output, not an execution claim.\n```sh\nprintf hello\n```\nhello")
