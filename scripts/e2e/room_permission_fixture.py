# SPDX-License-Identifier: Apache-2.0
"""Loopback-only scripted provider; permissions and file execution remain real."""
import json
import hashlib
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class RoomPermissionFixture:
    def __init__(self, target, replacement):
        self.requests = []
        original_digest = hashlib.sha256(target.read_bytes()).hexdigest()
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                owner.requests.append(body)
                messages = body.get("messages", [])
                is_room = any(str(target) in str(m.get("content", "")) for m in messages if m.get("role") == "user")
                tools = {t["function"]["name"] for t in body.get("tools", [])}
                wrote = any(m.get("role") == "tool" and m.get("tool_call_id") == "fixture-write" for m in messages)
                if is_room and wrote:
                    # Keep a physical attempt in flight while the PTY requests
                    # cancellation; do not invent a successful terminal event.
                    time.sleep(2)
                if is_room and not wrote:
                    if "write_file" in tools:
                        name, call_id, args = "write_file", "fixture-write", {"path": str(target), "content": replacement, "expected_sha256": original_digest}
                    else:
                        assert "tool_search" in tools
                        name, call_id, args = "tool_search", "fixture-discover", {"query": "select:write_file"}
                    delta = {"tool_calls": [{"index": 0, "id": call_id, "type": "function", "function": {"name": name, "arguments": json.dumps(args)}}]}
                    finish = "tool_calls"
                else:
                    delta, finish = {"content": "Fixture provider response."}, "stop"
                events = [{"choices": [{"index": 0, "delta": delta, "finish_reason": None}]},
                          {"choices": [{"index": 0, "delta": {}, "finish_reason": finish}], "usage": {"prompt_tokens": 10, "completion_tokens": 3, "total_tokens": 13}}]
                data = ("".join("data: " + json.dumps(event) + "\n\n" for event in events) + "data: [DONE]\n\n").encode()
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                try:
                    self.wfile.write(data)
                except (BrokenPipeError, ConnectionResetError):
                    pass  # Expected when the real Host cancels this request.

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.configuration = {"llm": {"endpoint": f"http://127.0.0.1:{self.server.server_port}/v1/chat/completions",
            "model": "permission-fixture", "api_key": "synthetic-not-a-secret", "provider": "openai-compatible",
            "executionCapabilities": {"toolCalls": True, "imageInput": False, "reasoning": False, "samplingParameters": False}}}

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()
