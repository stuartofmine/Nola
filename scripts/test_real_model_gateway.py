#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Isolated real LAN gateway acceptance; HTTP evidence, not visual acceptance.

The observing relay forwards to the authorized real LAN model without generating
or changing responses. Comparing its captured bytes proves gateway passthrough.
No real external-client configuration is written.
"""
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
from urllib.error import HTTPError
from urllib.parse import urlsplit
from urllib.request import Request, urlopen

sys.path.insert(0, str(Path(__file__).resolve().parent / "e2e"))
from e2e.real_overwrite_permission import isolated_environment, scrub_case_credentials


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--service", type=Path, required=True)
    parser.add_argument("--config", type=Path, required=True)
    args = parser.parse_args()
    llm = json.loads(args.config.read_text())["llm"]
    upstream = urlsplit(llm["endpoint"])
    assert upstream.hostname == "192.168.31.18" and upstream.port == 8081
    root = Path(tempfile.mkdtemp(prefix="nola-real-gateway-"))
    environment = isolated_environment(root, "cautious")
    environment["NOLA_APPLICATION_SERVICE_EXECUTABLE"] = str(args.service.resolve())
    captures = []

    class Relay(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            assert self.path == "/v1/chat/completions"
            body = self.rfile.read(int(self.headers["Content-Length"]))
            request = json.loads(body)
            assert request["model"] == llm["model"] and request["stream"] is True
            headers = {"Content-Type": "application/json", "Authorization": self.headers["Authorization"]}
            # The relay must receive the upstream account key, not gateway access.
            assert headers["Authorization"] == "Bearer " + llm["api_key"]
            target = llm["endpoint"].rstrip("/") + "/chat/completions"
            with urlopen(Request(target, data=body, headers=headers), timeout=180) as response:
                self.send_response(response.status)
                self.send_header("Content-Type", response.headers["Content-Type"])
                self.send_header("Connection", "close")
                self.end_headers()
                chunks = []
                while chunk := response.read1(8192):
                    chunks.append(chunk)
                    self.wfile.write(chunk)
                    self.wfile.flush()
                captures.append(b"".join(chunks))
                self.close_connection = True

    relay = ThreadingHTTPServer(("127.0.0.1", 0), Relay)
    threading.Thread(target=relay.serve_forever, daemon=True).start()
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    base = f"http://127.0.0.1:{port}"
    fixture = root / "fixture-opencode.json"
    with os.fdopen(os.open(fixture, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as out:
        json.dump({"provider": {"authorized-lan": {"name": "Isolated LAN acceptance",
            "npm": "@ai-sdk/openai-compatible", "options": {
                "baseURL": f"http://127.0.0.1:{relay.server_port}/v1", "apiKey": llm["api_key"]},
            "models": {llm["model"]: {"name": "LAN model"}}}}}, out)

    def http(path, body=None, token=None, expected=200):
        headers = {"Content-Type": "application/json"}
        if token is not None:
            headers["Authorization"] = "Bearer " + token
        request = Request(base + path, data=None if body is None else json.dumps(body).encode(), headers=headers)
        try:
            response = urlopen(request, timeout=240)
        except HTTPError as error:
            response = error
        with response:
            payload = response.read()
            assert response.status == expected, f"{path}: HTTP {response.status}, expected {expected}"
            return payload, response.headers

    def api(path, body=None):
        return json.loads(http(path, body)[0])

    def start():
        errors = (root / "host.stderr").open("ab")
        process = subprocess.Popen([str(args.binary.resolve()), "web", "--host", "127.0.0.1",
            "--port", str(port), "--project", str(root), "--json", "--no-skills"],
            env=environment, stdout=subprocess.DEVNULL, stderr=errors)
        errors.close()
        for _ in range(200):
            assert process.poll() is None, f"Host exited; inspect {root / 'host.stderr'}"
            try:
                api("/api/model-gateway/access")
                return process
            except OSError:
                time.sleep(0.05)
        raise AssertionError("Host startup timed out")

    process = None
    print(f"Isolated gateway evidence: {root}", flush=True)
    try:
        process = start()
        plan = api("/api/model-import/plan", {"source": "open_code", "path": str(fixture),
            "credential_path": None, "include_credentials": True, "allow_environment": False})
        assert len(plan["candidates"]) == 1 and plan["candidates"][0]["issue"] is None
        committed = api("/api/model-import/commit", {"plan_id": plan["plan_id"], "selections": [{
            "candidate_id": plan["candidates"][0]["candidate_id"], "display_name": None,
            "update_existing": False, "enable_models": True}]})
        assert committed[0]["status"] == "imported"
        inventory = api("/api/model-connections")
        assert len(inventory["accounts"]) == 1 and len(inventory["connections"]) == 1
        issued, headers = http("/api/model-gateway/access", {"expected_revision": 0, "revoke": False})
        access = json.loads(issued)
        token = access["token"]
        assert token != llm["api_key"] and "no-store" in headers["Cache-Control"]
        http("/gateway/v1/models", expected=401)
        http("/gateway/v1/models", token="invalid", expected=401)
        models = json.loads(http("/gateway/v1/models", token=token)[0])["data"]
        assert len(models) == 1
        model = models[0]["id"]
        received, headers = http("/gateway/v1/chat/completions", {"model": model,
            "messages": [{"role": "user", "content": "Reply only GATEWAY_ACCEPTED. Do not use tools."}],
            "stream": True, "stream_options": {"include_usage": True}, "max_tokens": 64}, token)
        assert "text/event-stream" in headers["Content-Type"]
        (root / "upstream.sse").write_bytes(received)
        assert len(captures) == 1 and received == captures[0], "Gateway changed real upstream SSE bytes"
        events = [json.loads(line[6:]) for line in received.decode().splitlines()
            if line.startswith("data: ") and line[6:] != "[DONE]"]
        answer = "".join(choice.get("delta", {}).get("content") or ""
            for event in events for choice in event.get("choices", []))
        assert b"[DONE]" in received and "GATEWAY_ACCEPTED" in answer, "Missing completed real-model answer"
        rows = api("/api/model-gateway/attempts?limit=10&offset=0")
        assert len(rows) == 1
        record = rows[0]["record"]
        assert record["outcome"] == "completed" and record["http_status"] == 200
        provider_usage = next((event["usage"] for event in reversed(events) if event.get("usage")), None)
        if provider_usage is None:
            assert record["usage"] is None, "Unreported upstream usage must remain unknown"
        else:
            assert record["usage"]["input_tokens"] == provider_usage.get("prompt_tokens")
            assert record["usage"]["output_tokens"] == provider_usage.get("completion_tokens")
            assert record["usage"]["cached_input_tokens"] == (provider_usage.get("prompt_tokens_details") or {}).get("cached_tokens")
        assert record["client_access_revision"] == access["revision"]
        assert record["connection_id"] == committed[0]["connection_id"] and record["account_id"] is not None
        assert model == f"nola/{record['profile_id']}/{record['profile_revision']}"
        with sqlite3.connect((root / "nola.db").as_uri() + "?mode=ro", uri=True) as db:
            assert db.execute("SELECT COUNT(*) FROM runs").fetchone()[0] == 0, "Gateway incorrectly entered Agent execution"
        process.terminate()
        process.wait(timeout=20)
        process = None
        process = start()
        reopened = api("/api/model-gateway/attempts?limit=10&offset=0")
        assert reopened == rows, "Gateway usage did not survive Host restart"
        assert json.loads(http("/gateway/v1/models", token=token)[0])["data"] == models
        api("/api/model-gateway/access", {"expected_revision": access["revision"], "revoke": True})
        http("/gateway/v1/models", token=token, expected=401)
        (root / "upstream.sse").write_bytes(received)
        evidence = {"real_target": "192.168.31.18:8081", "relay": "transparent byte observer",
            "physical_requests": len(captures), "sse_bytes": len(received),
            "sse_sha256": hashlib.sha256(received).hexdigest(), "gateway_model_id": model,
            "attempts_after_restart": reopened, "agent_runs": 0, "revoked_token_rejected": True}
        (root / "evidence.json").write_text(json.dumps(evidence, indent=2))
        print("PASS: isolated account/connection, independent token, real LAN SSE byte equality, completed usage survives restart, revocation", flush=True)
    finally:
        if process is not None:
            process.terminate()
            process.wait(timeout=20)
        relay.shutdown()
        relay.server_close()
        fixture.unlink(missing_ok=True)
        scrub_case_credentials(root)
        # Only this newly created isolated Host's bootstrap token is removed.
        for token_path in root.rglob("mobile-access-token"):
            assert token_path.is_file() and not token_path.is_symlink()
            token_path.unlink()


if __name__ == "__main__":
    main()
