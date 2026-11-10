#!/usr/bin/env python3
"""Official Telegram webhook -> Worker lease -> real Codex -> Telegram delivery E2E."""

from __future__ import annotations

import json
import os
import secrets
import shutil
import socket
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from e2e.cargo_paths import cargo_binary


ROOT = Path(__file__).resolve().parents[1]
WORKER = ROOT / "legacy/backend/worker"
WRANGLER = WORKER / "node_modules/.bin/wrangler"
NOLA = cargo_binary(ROOT)


def free_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def run(*arguments: str) -> str:
    completed = subprocess.run(arguments, cwd=WORKER, text=True, capture_output=True)
    if completed.returncode != 0:
        raise RuntimeError(
            f"command failed ({completed.returncode}): {' '.join(arguments)}\n"
            f"stdout:\n{completed.stdout[-4000:]}\nstderr:\n{completed.stderr[-4000:]}"
        )
    return completed.stdout


def request_json(
    url: str,
    payload: dict | None = None,
    *,
    token: str = "",
    headers: dict[str, str] | None = None,
    timeout: float = 20,
) -> tuple[int, dict]:
    encoded = json.dumps(payload).encode() if payload is not None else None
    request_headers = {"content-type": "application/json"} if encoded else {}
    if token:
        request_headers["authorization"] = f"Bearer {token}"
    request_headers.update(headers or {})
    request = urllib.request.Request(
        url,
        data=encoded,
        headers=request_headers,
        method="POST" if encoded is not None else "GET",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return response.status, json.loads(response.read() or b"{}")
    except urllib.error.HTTPError as error:
        return error.code, json.loads(error.read() or b"{}")


class TelegramProvider(BaseHTTPRequestHandler):
    records: list[dict] = []
    lock = threading.Lock()

    def log_message(self, *_: object) -> None:
        return

    def do_POST(self) -> None:  # noqa: N802
        raw = self.rfile.read(int(self.headers.get("content-length", "0")))
        body = json.loads(raw or b"{}")
        with self.lock:
            self.records.append({"path": self.path, "body": body})
            reply_index = sum(record["path"].endswith("/sendMessage") for record in self.records)
        if self.path.endswith("/setWebhook"):
            self.respond({"ok": True, "result": True})
        elif self.path.endswith("/sendMessageDraft"):
            self.respond({"ok": True, "result": True})
        elif self.path.endswith("/sendMessage"):
            self.respond({"ok": True, "result": {"message_id": 8000 + reply_index}})
        else:
            self.respond({"ok": False, "description": "unexpected provider path"}, 404)

    def respond(self, payload: dict, status: int = 200) -> None:
        encoded = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)


def wait_ready(url: str, process: subprocess.Popen[str], timeout: float = 30) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"process exited before readiness: {url}")
        try:
            if urllib.request.urlopen(url, timeout=0.5).status == 200:
                return
        except Exception:
            time.sleep(0.1)
    raise RuntimeError(f"readiness timed out: {url}")


def query_rows(persist: Path) -> list[dict]:
    output = run(
        str(WRANGLER), "d1", "execute", "nola_backend_fresh", "--local",
        "--persist-to", str(persist), "--json", "--command",
        "SELECT provider_event_id,status,reply_text,provider_reply_id,nola_conversation_id "
        "FROM messagingchannelmessagerecord ORDER BY id;",
    )
    decoded = json.loads(output)
    return decoded[0]["results"] if isinstance(decoded, list) else decoded["results"]


def main() -> int:
    if not WRANGLER.is_file():
        raise SystemExit("backend/worker dependencies are not installed")
    if shutil.which("codex") is None:
        raise SystemExit("installed Codex CLI is required")

    build = subprocess.run(
        ["cargo", "build", "--quiet", "--bin", "nola", "--bin", "nola-agent-host"],
        cwd=ROOT / "apps/cli",
        text=True,
        capture_output=True,
    )
    if build.returncode != 0 or not NOLA.is_file():
        raise SystemExit(
            "failed to build the current CLI/Web Host sources before E2E\n"
            + build.stdout[-4000:]
            + build.stderr[-4000:]
        )

    TelegramProvider.records = []
    worker_port, provider_port, web_port = free_port(), free_port(), free_port()
    provider = ThreadingHTTPServer(("127.0.0.1", provider_port), TelegramProvider)
    provider_thread = threading.Thread(target=provider.serve_forever, daemon=True)
    provider_thread.start()
    marker = f"NOLA_TELEGRAM_CODEX_{secrets.token_hex(8)}"
    telegram_secret = f"telegram-secret-{secrets.token_hex(8)}"
    provider_base = f"http://127.0.0.1:{provider_port}"
    worker_base = f"http://127.0.0.1:{worker_port}"
    worker_process: subprocess.Popen[str] | None = None
    web_process: subprocess.Popen[str] | None = None

    with tempfile.TemporaryDirectory(prefix="nola-telegram-codex-full-e2e-") as temporary:
        root = Path(temporary)
        persist = root / "worker-state"
        persist.mkdir()
        nola_home = root / "nola-home"
        (nola_home / "cli").mkdir(parents=True)
        real_home = Path.home()
        for name in (".codex", ".claude"):
            source = real_home / name
            target = root / "home" / name
            if source.exists():
                target.parent.mkdir(parents=True, exist_ok=True)
                target.symlink_to(source, target_is_directory=True)

        run(
            str(WRANGLER), "d1", "migrations", "apply", "nola_backend_fresh",
            "--local", "--persist-to", str(persist),
        )
        environment = os.environ.copy()
        environment.pop("CLOUDFLARE_API_TOKEN", None)
        worker_log = (root / "worker.log").open("w", encoding="utf-8")
        worker_process = subprocess.Popen(
            [
                str(WRANGLER), "dev", "--local", "--port", str(worker_port),
                "--persist-to", str(persist),
                "--var", "SERVER_JWT_SECRET:telegram-full-e2e-jwt-secret",
                "--var", "TELEGRAM_TOKEN:telegram-full-e2e-token",
                "--var", f"TELEGRAM_SECRET:{telegram_secret}",
                "--var", f"TELEGRAM_API_BASE:{provider_base}",
            ],
            cwd=WORKER,
            env=environment,
            stdout=worker_log,
            stderr=subprocess.STDOUT,
            text=True,
        )
        try:
            wait_ready(worker_base + "/healthz", worker_process)
            status, registration = request_json(
                worker_base + "/v1/auth/register",
                {
                    "email": f"telegram-e2e-{secrets.token_hex(5)}@e2e.invalid",
                    "password": "Nola-telegram-full-e2e-password",
                    "displayName": "Telegram Codex Full E2E",
                },
            )
            access_token = str(registration.get("accessToken") or "")
            if status != 200 or not access_token:
                raise RuntimeError(f"registration failed: HTTP {status} {registration}")

            connection_body = {
                "connection_id": "telegram-real-codex",
                "kind": "telegram",
                "display_name": "Telegram real Codex",
                "credential_ref": "TELEGRAM_TOKEN",
                "webhook_secret_ref": "TELEGRAM_SECRET",
                "enabled": True,
            }
            request = urllib.request.Request(
                worker_base + "/v1/channels",
                data=json.dumps(connection_body).encode(),
                headers={
                    "content-type": "application/json",
                    "authorization": f"Bearer {access_token}",
                },
                method="PUT",
            )
            with urllib.request.urlopen(request, timeout=20) as response:
                connection = json.load(response)
            if connection.get("state") != "connected":
                raise RuntimeError(f"Telegram connection was not connected: {connection}")

            config_path = nola_home / "cli/messaging-channel-worker.json"
            config_path.write_text(json.dumps({
                "schema_version": 1,
                "enabled": True,
                "backend_url": worker_base,
                "access_token": access_token,
                "poll_seconds": 1,
                "turn_timeout_seconds": 240,
                "execution_backend": "codex",
                "project_root": str(ROOT),
            }))
            config_path.chmod(0o600)
            web_log = (root / "web-host.log").open("w", encoding="utf-8")
            host_environment = os.environ.copy()
            host_environment.update({
                "HOME": str(root / "home"),
                "NOLA_HOME": str(nola_home),
                "NOLA_DB_PATH": str(root / "nola.sqlite"),
            })
            web_process = subprocess.Popen(
                [
                    str(NOLA), "web", "--host", "127.0.0.1", "--port", str(web_port),
                    "--project", str(ROOT), "--web-root", str(ROOT / "apps/web"),
                ],
                cwd=ROOT,
                env=host_environment,
                stdout=web_log,
                stderr=subprocess.STDOUT,
                text=True,
            )
            wait_ready(f"http://127.0.0.1:{web_port}/login", web_process)

            def send_update(update_id: int, text: str) -> None:
                status, body = request_json(
                    worker_base + "/v1/channels/webhook/telegram/telegram-real-codex",
                    {
                        "update_id": update_id,
                        "message": {
                            "message_id": update_id,
                            "date": 1786579200 + update_id,
                            "text": text,
                            "chat": {"id": 7001},
                            "from": {"id": 9001},
                        },
                    },
                    headers={"X-Telegram-Bot-Api-Secret-Token": telegram_secret},
                )
                if status != 200 or body.get("inserted") != 1:
                    raise RuntimeError(f"Telegram webhook failed: HTTP {status} {body}")

            send_update(501, f"Remember this exact token: {marker}. Reply with only ACK.")
            deadline = time.monotonic() + 260
            while time.monotonic() < deadline:
                rows = query_rows(persist)
                if len(rows) == 1 and rows[0]["status"] == "sent":
                    break
                if web_process.poll() is not None:
                    raise RuntimeError("Web Host exited during first Telegram turn")
                time.sleep(0.5)
            else:
                raise RuntimeError(f"first Telegram turn timed out: {query_rows(persist)}")

            send_update(502, "Reply with only the exact token I asked you to remember previously.")
            deadline = time.monotonic() + 260
            while time.monotonic() < deadline:
                rows = query_rows(persist)
                if len(rows) == 2 and all(row["status"] == "sent" for row in rows):
                    break
                if web_process.poll() is not None:
                    raise RuntimeError("Web Host exited during second Telegram turn")
                time.sleep(0.5)
            else:
                raise RuntimeError(f"second Telegram turn timed out: {query_rows(persist)}")

            rows = query_rows(persist)
            replies = [
                record for record in TelegramProvider.records
                if record["path"].endswith("/sendMessage")
            ]
            drafts = [
                record for record in TelegramProvider.records
                if record["path"].endswith("/sendMessageDraft")
            ]
            if rows[0]["reply_text"].strip() != "ACK" or rows[1]["reply_text"].strip() != marker:
                raise RuntimeError(f"real Codex responses violated instructions: {rows}")
            if len(replies) != 2 or [item["body"]["text"] for item in replies] != ["ACK", marker]:
                raise RuntimeError(f"Telegram delivery mismatch: {replies}")
            if len(drafts) < 2 or any(item["body"].get("chat_id") != 7001 for item in drafts):
                raise RuntimeError(f"Telegram streaming drafts were not delivered: {drafts}")
            draft_ids = {item["body"].get("draft_id") for item in drafts}
            if len(draft_ids) != 2 or any(not isinstance(value, int) or value <= 0 for value in draft_ids):
                raise RuntimeError(f"Telegram draft identity was not stable per inbox message: {drafts}")
            if not rows[0]["nola_conversation_id"] or rows[0]["nola_conversation_id"] != rows[1]["nola_conversation_id"]:
                raise RuntimeError(f"Telegram conversation did not resume: {rows}")
            print(json.dumps({
                "ok": True,
                "protocol": "telegram",
                "execution_backend": "codex",
                "webhook_configured": True,
                "inbound_messages": 2,
                "provider_replies": 2,
                "provider_drafts": len(drafts),
                "conversation_resumed": True,
                "marker": marker,
            }))
        finally:
            if web_process is not None:
                web_process.terminate()
                try:
                    web_process.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    web_process.kill()
            if worker_process is not None:
                worker_process.terminate()
                try:
                    worker_process.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    worker_process.kill()
            worker_log.close()
            if 'web_log' in locals():
                web_log.close()
            provider.shutdown()
            provider.server_close()
            provider_thread.join(timeout=3)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
