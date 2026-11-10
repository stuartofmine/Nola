#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Real CLI Web/Service HTTP mutation isolation, without LLM or external MCP.

Build: cargo build -p nola_cli --bin nola -p nola_service --bin nola-service
Run: python3 scripts/test_web_mcp_baseline_e2e.py
"""
import http.cookiejar
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]


def main():
    with tempfile.TemporaryDirectory(prefix="nola-web-mcp-http-") as directory:
        state = Path(directory)
        home = state / "home"
        home.mkdir()
        env = os.environ.copy()
        env.update(HOME=str(home), NOLA_HOME=str(home), NOLA_DB_PATH=str(state / "state.db"),
                   NOLA_APPLICATION_SERVICE_EXECUTABLE=str(ROOT / "target/debug/nola-service"))
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        base = f"http://127.0.0.1:{port}"
        opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))

        def request(path, data=None):
            payload = None if data is None else json.dumps(data).encode()
            req = urllib.request.Request(base + path, data=payload, headers={"Content-Type": "application/json"})
            try:
                response = opener.open(req, timeout=15)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                return response.status, json.load(response)

        def settings():
            status, body = request("/api/agent/settings")
            assert status == 200, (status, body)
            return body

        def baseline(servers):
            return [{"server_id": item["server_id"], "revision": item["revision"]} for item in servers]

        def server(identity):
            return {"server_id": identity, "name": identity, "enabled": False,
                    "transport": "streamable_http", "endpoint": "http://127.0.0.1:9/mcp"}

        with (state / "host.log").open("w") as log:
            process = subprocess.Popen([
                str(ROOT / "target/debug/nola"), "web", "--host", "127.0.0.1", "--port", str(port),
                "--web-root", str(ROOT / "apps/web"),
            ], cwd=state, env=env, stdout=log, stderr=log, start_new_session=True)
            try:
                token = home / ".nola/web/mobile-access-token"
                deadline = time.monotonic() + 20
                while not token.exists():
                    assert process.poll() is None, "Web exited during startup"
                    assert time.monotonic() < deadline, "Web startup timed out"
                    time.sleep(.05)
                status, body = request("/api/web/login", {"token": token.read_text().strip()})
                assert status == 200, (status, body)
                initial = settings()
                assert initial["mcp_servers"] == []
                status, body = request("/api/agent/settings", {"mcp_servers": [server("first")], "mcp_baseline": []})
                assert status == 200, (status, body)
                opened = baseline(body["mcp_servers"])
                status, body = request("/api/agent/settings", {"mcp_servers": [server("second")], "mcp_baseline": []})
                assert status == 200, (status, body)
                assert len(body["mcp_servers"]) == 2
                status, body = request("/api/agent/settings", {"mcp_servers": [], "mcp_baseline": opened})
                assert status == 200, (status, body)
                assert [item["server_id"] for item in body["mcp_servers"]] == ["second"]
                before = settings()
                status, body = request("/api/agent/settings", {"mcp_servers": [server("first")], "mcp_baseline": opened})
                assert status == 409, (status, body)
                assert settings()["mcp_servers"] == before["mcp_servers"]
                status, body = request("/api/agent/settings", {
                    "mcp_servers": [], "mcp_baseline": baseline(before["mcp_servers"]),
                    "user_preferences": "must not be saved",
                })
                assert status == 400 and body["code"] == "mixed_resource_mutation", (status, body)
                after = settings()
                for key in ("mcp_servers", "user_preferences", "uploaded_documents", "skills"):
                    assert after.get(key) == before.get(key), key
                status, body = request("/api/agent/settings", {"mcp_servers": []})
                assert status == 400 and body["code"] == "mcp_baseline_required", (status, body)
                print(json.dumps({"concurrentAdditionPreserved": True, "staleMutation": 409,
                                  "mixedMutation": 400, "missingBaseline": 400,
                                  "otherResourcesUnchanged": True}), flush=True)
            finally:
                try:
                    os.killpg(process.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()


if __name__ == "__main__":
    main()
