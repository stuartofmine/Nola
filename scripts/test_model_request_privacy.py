#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Inspect the HTTP body Nola actually sends, using a local receiver and synthetic PII."""
import argparse
import json
import sqlite3
import subprocess
import tempfile
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from threading import Thread

from test_real_cli_long_tasks import apply_config, fixture

EMAIL = "private.fixture@example.org"
PHONE = "13800138000"
CREDENTIAL = "synthetic-contact-credential"
PROMPT = f"Email {EMAIL}, phone {PHONE}. NOLA_HOST_TOKEN={CREDENTIAL} Reply with OK."
MODEL = "privacy-fixture"


def events(protocol):
    if protocol == "responses":
        return [
            {"type": "response.output_text.delta", "item_id": "msg_1", "output_index": 0, "content_index": 0, "delta": "OK"},
            {"type": "response.completed", "response": {"id": "resp_1", "object": "response", "model": MODEL, "status": "completed", "output": [
                {"type": "message", "id": "msg_1", "role": "assistant", "content": [{"type": "output_text", "text": "OK"}]}]}},
        ]
    if protocol == "anthropic":
        return [
            {"type": "message_start", "message": {"role": "assistant", "usage": {"input_tokens": 2}}},
            {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}},
            {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "OK"}},
            {"type": "content_block_stop", "index": 0},
            {"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 1}},
            {"type": "message_stop"},
        ]
    return [{"choices": [{"index": 0, "delta": {"content": "OK"}, "finish_reason": "stop"}]}]


def verify(binary, protocol, expected_path):
    requests = []
    tool_turn = False
    requested_file = False

    class Receiver(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            requests.append((self.path, body))
            nonlocal requested_file
            reply = events(protocol)
            if tool_turn and not requested_file:
                requested_file = True
                call = {"index": 0, "id": "call_privacy_read", "type": "function", "function": {
                    "name": "read_file", "arguments": json.dumps({"path": "privacy-contact.txt"})}}
                reply = [{"choices": [{"index": 0, "delta": {"tool_calls": [call]}, "finish_reason": "tool_calls"}]}]
            frames = [f"data: {json.dumps(event)}\n\n" for event in reply]
            if protocol == "chat_completions":
                frames.append("data: [DONE]\n\n")
            data = "".join(frames).encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

    with ThreadingHTTPServer(("127.0.0.1", 0), Receiver) as server, tempfile.TemporaryDirectory() as tmp:
        worker = Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            root = Path(tmp)
            project, env, config = fixture(root, MODEL, f"http://127.0.0.1:{server.server_port}/v1")
            settings = json.loads(config.read_text())
            settings["llm"]["connections"][0]["protocol"] = protocol
            config.write_text(json.dumps(settings))
            apply_config(binary, project, env, config, root)
            def cli(*args):
                result = subprocess.run([binary, *args], env=env, cwd=project, capture_output=True, text=True, timeout=45)
                assert result.returncode == 0, (protocol, result.stderr[-1000:], result.stdout[-750:])
                return result.stdout
            for prompt in [PROMPT, "Repeat your last answer."]:
                previous = len(requests)
                output = cli(prompt, "--session", "privacy-fixture", "--no-tools", "--no-skills", "--json")
                traces = [json.loads(line).get("trace", {}) for line in output.splitlines() if line.startswith("{")]
                assert any(t.get("kind") == "run.completed" and t.get("status") == "succeeded" for t in traces), protocol
                assert len(requests) > previous, protocol
                for path, request in requests[previous:]:
                    wire = json.dumps(request, ensure_ascii=False)
                    assert path == expected_path and request["model"] == MODEL, protocol
                    assert all(value not in wire for value in (EMAIL, PHONE, CREDENTIAL)), protocol
                    assert all(tag in wire for tag in ("[EMAIL]", "[PHONE]", "[CREDENTIAL]")), protocol
            history = cli("session", "show", "privacy-fixture")
            assert EMAIL in history and PHONE in history, (protocol, "original message missing from history")
            # The session list/detail preview can truncate a message. Read its full canonical part.
            with sqlite3.connect(env["NOLA_DB_PATH"]) as connection:
                originals = connection.execute("""
                    SELECT p.content_text, p.content_json FROM message_parts p JOIN messages m ON p.message_id = m.id
                    WHERE m.conversation_id = ? AND m.role = 'user' AND p.kind = 'text'
                """, ("privacy-fixture",)).fetchall()
            assert any(PROMPT in (text or "") or PROMPT in (content or "") for text, content in originals), protocol
            print(f"PASS {protocol}: first request, replayed history and original saved message")
            if protocol == "chat_completions":
                tool_turn = True
                contact = project / "privacy-contact.txt"
                contact.write_text("Contact tool.person@example.org. Test marker: copper-leaf.")
                previous = len(requests)
                cli("Read privacy-contact.txt with read_file.", "--session", "privacy-tool",
                    "--no-skills", "--allow-dir", str(project), "--json")
                messages = [message for _, request in requests[previous:] for message in request["messages"]
                            if message.get("role") == "tool" and message.get("tool_call_id", "").split("::")[-1] == "call_privacy_read"]
                assert any("copper-leaf" in str(message["content"]) and "[EMAIL]" in str(message["content"])
                           for message in messages), "Missing actual read_file result"
                assert all("tool.person@example.org" not in json.dumps(request) for _, request in requests[previous:])
                assert "tool.person@example.org" in contact.read_text()
                print("PASS live read_file: result redacted on next model request, original file preserved")
        finally:
            server.shutdown()
            worker.join(timeout=5)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path(__file__).resolve().parents[1] / "target/debug/nola")
    args = parser.parse_args()
    for kind, endpoint in [("chat_completions", "/v1/chat/completions"), ("responses", "/v1/responses"), ("anthropic", "/v1/messages")]:
        verify(args.binary.resolve(), kind, endpoint)
