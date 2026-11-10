#!/usr/bin/env python3
"""E2E: browser settings -> authenticated Nola Host proxy -> channel service."""

from __future__ import annotations

import http.cookiejar
import json
import os
import socket
import subprocess
import tempfile
import threading
import time
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from e2e.cargo_paths import cargo_binary


ROOT = Path(__file__).resolve().parents[1]


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


class State:
    requests: list[dict] = []


class ChannelHandler(BaseHTTPRequestHandler):
    def _reply(self, value: dict) -> None:
        encoded = json.dumps(value).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def do_GET(self) -> None:  # noqa: N802
        if self.path != "/v1/channels" or self.headers.get("authorization") != "Bearer private-channel-token":
            self.send_response(401)
            self.end_headers()
            return
        State.requests.append({"method": "GET", "authorization": self.headers.get("authorization")})
        self._reply({"channels": [{"connection_id": "telegram-primary", "kind": "telegram", "display_name": "Primary bot", "enabled": True, "state": "connected"}]})

    def do_PUT(self) -> None:  # noqa: N802
        length = int(self.headers.get("content-length", "0"))
        body = json.loads(self.rfile.read(length) or b"{}")
        if self.path != "/v1/channels" or self.headers.get("authorization") != "Bearer private-channel-token":
            self.send_response(401)
            self.end_headers()
            return
        State.requests.append({"method": "PUT", "authorization": self.headers.get("authorization"), "body": body})
        self._reply({"connection_id": body["connection_id"], "kind": body["kind"], "state": "awaiting_verification", "enabled": body.get("enabled", False)})

    def log_message(self, _format: str, *_args: object) -> None:
        return


def main() -> int:
    binary = cargo_binary(ROOT)
    if not binary.is_file():
        raise SystemExit("build cli debug binary first")
    channel_port, web_port = free_port(), free_port()
    channel_server = ThreadingHTTPServer(("127.0.0.1", channel_port), ChannelHandler)
    threading.Thread(target=channel_server.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix="nola-channel-proxy-e2e-") as temp:
        home = Path(temp)
        config_dir = home / "cli"
        config_dir.mkdir(parents=True)
        config = {
            "schema_version": 1,
            "enabled": False,
            "backend_url": f"http://127.0.0.1:{channel_port}",
            "access_token": "private-channel-token",
            "poll_seconds": 2,
            "turn_timeout_seconds": 900,
            "execution_backend": "nola",
            "project_root": None,
        }
        path = config_dir / "messaging-channel-worker.json"
        path.write_text(json.dumps(config))
        path.chmod(0o600)
        env = os.environ.copy()
        env["NOLA_HOME"] = str(home)
        process = subprocess.Popen([
            str(binary), "web", "--host", "127.0.0.1", "--port", str(web_port),
            "--project", str(ROOT), "--web-root", str(ROOT / "apps/web"),
        ], cwd=ROOT, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        base = f"http://127.0.0.1:{web_port}"
        opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
        try:
            deadline = time.monotonic() + 15
            while True:
                try:
                    opener.open(base + "/api/health", timeout=0.5).read()
                    break
                except Exception:
                    if process.poll() is not None or time.monotonic() >= deadline:
                        raise RuntimeError("Nola Web Host did not become ready")
                    time.sleep(0.1)

            def request(pathname: str, payload: dict | None = None, method: str = "GET") -> dict:
                raw = json.dumps(payload).encode() if payload is not None else None
                req = urllib.request.Request(base + pathname, data=raw, method=method, headers={"content-type": "application/json"} if raw else {})
                with opener.open(req, timeout=20) as response:
                    return json.load(response)

            listed = request("/api/messaging-channels")
            if listed.get("channels", [{}])[0].get("state") != "connected":
                raise RuntimeError(f"Host did not proxy the channel list: {listed}")
            payload = {
                "connection_id": "whatsapp-primary", "kind": "whatsapp", "display_name": "Support",
                "account_id": "phone-1", "credential_ref": "WA_TOKEN", "webhook_secret_ref": "WA_APP_SECRET",
                "verify_token_ref": "WA_VERIFY", "provider_config": {"phone_number_id": "phone-1"}, "enabled": True,
            }
            saved = request("/api/messaging-channels", payload, "PUT")
            if saved.get("connection_id") != "whatsapp-primary" or saved.get("state") != "awaiting_verification":
                raise RuntimeError(f"Host did not proxy channel save: {saved}")
            if "private-channel-token" in json.dumps(listed) + json.dumps(saved):
                raise RuntimeError("Host leaked its private channel access token")
            if [item["method"] for item in State.requests] != ["GET", "PUT"]:
                raise RuntimeError(f"unexpected backend requests: {State.requests}")
            print(json.dumps({"ok": True, "proxied": ["GET", "PUT"], "token_leaked": False, "connection": saved["connection_id"]}))
            return 0
        finally:
            process.terminate()
            try:
                process.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.communicate()
            channel_server.shutdown()
            channel_server.server_close()


if __name__ == "__main__":
    raise SystemExit(main())
