#!/usr/bin/env python3
"""Exercise Worker claim -> provider delivery -> durable sent state for every channel."""

from __future__ import annotations

import json
import os
import socket
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKER = ROOT / "backend/worker"
WRANGLER = WORKER / "node_modules/.bin/wrangler"


def free_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def run(*arguments: str) -> str:
    completed = subprocess.run(arguments, cwd=WORKER, text=True, capture_output=True)
    if completed.returncode != 0:
        raise RuntimeError(
            f"command failed ({completed.returncode}): {' '.join(arguments)}\n"
            f"stdout:\n{completed.stdout[-6000:]}\n"
            f"stderr:\n{completed.stderr[-6000:]}"
        )
    return completed.stdout


def request_json(base: str, path: str, token: str, payload: dict) -> tuple[int, dict]:
    request = urllib.request.Request(
        base + path,
        data=json.dumps(payload).encode(),
        headers={"content-type": "application/json", "authorization": f"Bearer {token}"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=15) as response:
            return response.status, json.loads(response.read() or b"{}")
    except urllib.error.HTTPError as error:
        return error.code, json.loads(error.read() or b"{}")


class ProviderHandler(BaseHTTPRequestHandler):
    records: list[dict] = []
    lock = threading.Lock()

    def log_message(self, *_: object) -> None:
        return

    def _respond(self, payload: dict, status: int = 200) -> None:
        encoded = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def do_GET(self) -> None:  # noqa: N802
        with self.lock:
            self.records.append({"method": "GET", "path": self.path, "authorization": self.headers.get("authorization", "")})
        if self.path.startswith("/wecom/cgi-bin/gettoken"):
            self._respond({"errcode": 0, "access_token": "wecom-access"})
        elif self.path.startswith("/wechat/cgi-bin/token"):
            self._respond({"access_token": "wechat-access", "expires_in": 7200})
        else:
            self._respond({"error": "unexpected GET"}, 404)

    def do_POST(self) -> None:  # noqa: N802
        raw = self.rfile.read(int(self.headers.get("content-length", "0")))
        body = json.loads(raw or b"{}")
        with self.lock:
            self.records.append({"method": "POST", "path": self.path, "authorization": self.headers.get("authorization", ""), "body": body})
        if self.path == "/telegram/bottelegram-token/sendMessageDraft":
            self._respond({"ok": True, "result": True})
        elif self.path == "/telegram/bottelegram-token/sendMessage":
            self._respond({"ok": True, "result": {"message_id": 7101}})
        elif self.path == "/whatsapp/v23.0/phone-e2e/messages":
            self._respond({"messages": [{"id": "wamid-reply-e2e"}]})
        elif self.path.startswith("/wecom/cgi-bin/message/send"):
            self._respond({"errcode": 0, "msgid": "wecom-reply-e2e"})
        elif self.path.startswith("/wechat/cgi-bin/message/custom/send"):
            self._respond({"errcode": 0, "msgid": "wechat-reply-e2e"})
        else:
            self._respond({"error": "unexpected POST"}, 404)


def quote(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def main() -> int:
    if not WRANGLER.is_file():
        raise SystemExit("backend/worker dependencies are not installed")
    worker_port, provider_port = free_port(), free_port()
    provider = ThreadingHTTPServer(("127.0.0.1", provider_port), ProviderHandler)
    provider_thread = threading.Thread(target=provider.serve_forever, daemon=True)
    provider_thread.start()
    client_uuid = str(uuid.uuid4())
    jwt_secret = "messaging-delivery-e2e-jwt-secret"
    now = "2026-08-13T00:00:00Z"
    provider_base = f"http://127.0.0.1:{provider_port}"
    with tempfile.TemporaryDirectory(prefix="nola-channel-delivery-e2e-") as temporary:
        persist = Path(temporary) / "state"
        persist.mkdir()
        run(str(WRANGLER), "d1", "migrations", "apply", "nola_backend_fresh", "--local", "--persist-to", str(persist))
        connections = [
            ("telegram-delivery", "telegram", "TELEGRAM_TOKEN", "", "{}"),
            ("whatsapp-delivery", "whatsapp", "WHATSAPP_TOKEN", "", '{"phone_number_id":"phone-e2e","graph_version":"v23.0"}'),
            ("wechat-delivery", "wechat_official", "WECHAT_SECRET", "", '{"app_id":"wx-e2e"}'),
            ("wecom-delivery", "we_com", "WECOM_SECRET", "1000002", '{"corp_id":"ww-e2e","agent_id":"1000002"}'),
        ]
        values = ",".join(
            "(" + ",".join(map(quote, (connection_id, client_uuid, kind, kind, account_id, "", credential, "unused-secret", "", config, "1", "connected", now, now))) + ")"
            for connection_id, kind, credential, account_id, config in connections
        )
        seed_connections = "INSERT INTO messagingchannelconnectionrecord (connection_id,client_uuid,kind,display_name,account_id,secret_ref,credential_ref,webhook_secret_ref,verify_token_ref,provider_config_json,enabled,state,created_at,updated_at) VALUES " + values + ";"
        run(str(WRANGLER), "d1", "execute", "nola_backend_fresh", "--local", "--persist-to", str(persist), "--command", seed_connections)
        message_values = ",".join(
            "(" + ",".join(map(quote, (connection_id, f"event-{index}", "7001" if kind == "telegram" else f"conversation-{index}", f"sender-{index}", "inbound", "received", f"question-{kind}", "{}", now, now))) + ")"
            for index, (connection_id, kind, *_rest) in enumerate(connections, 1)
        )
        seed_messages = "INSERT INTO messagingchannelmessagerecord (connection_id,provider_event_id,external_conversation_id,external_sender_id,direction,status,text,payload_json,received_at,created_at) VALUES " + message_values + ";"
        run(str(WRANGLER), "d1", "execute", "nola_backend_fresh", "--local", "--persist-to", str(persist), "--command", seed_messages)

        env = os.environ.copy()
        env.pop("CLOUDFLARE_API_TOKEN", None)
        command = [
            str(WRANGLER), "dev", "--local", "--port", str(worker_port), "--persist-to", str(persist),
            "--var", f"SERVER_JWT_SECRET:{jwt_secret}",
            "--var", "TELEGRAM_TOKEN:telegram-token", "--var", "WHATSAPP_TOKEN:whatsapp-token",
            "--var", "WECHAT_SECRET:wechat-secret", "--var", "WECOM_SECRET:wecom-secret",
            "--var", f"TELEGRAM_API_BASE:{provider_base}/telegram",
            "--var", f"WHATSAPP_GRAPH_API_BASE:{provider_base}/whatsapp",
            "--var", f"WECHAT_API_BASE:{provider_base}/wechat", "--var", f"WECOM_API_BASE:{provider_base}/wecom",
        ]
        process = subprocess.Popen(command, cwd=WORKER, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        base = f"http://127.0.0.1:{worker_port}"
        worker_output = ""
        try:
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise RuntimeError("wrangler exited before readiness")
                try:
                    if urllib.request.urlopen(base + "/healthz", timeout=0.5).status == 200:
                        break
                except Exception:
                    time.sleep(0.1)
            else:
                raise RuntimeError("worker readiness timed out")
            status, registration = request_json(
                base,
                "/v1/auth/register",
                "",
                {"email": f"{client_uuid}@e2e.invalid", "password": "Nola-channel-e2e-password", "displayName": "Messaging Delivery E2E"},
            )
            token = str(registration.get("accessToken") or "")
            registered_client = str(registration.get("clientUuid") or "").replace("-", "").lower()
            if status != 200 or not token or not registered_client:
                raise RuntimeError(f"registration failed: HTTP {status} {registration}")
            run(
                str(WRANGLER), "d1", "execute", "nola_backend_fresh", "--local", "--persist-to", str(persist),
                "--command", f"UPDATE messagingchannelconnectionrecord SET client_uuid={quote(registered_client)} WHERE client_uuid={quote(client_uuid)};",
            )
            completed: list[dict] = []
            for index in range(1, 5):
                status, claim = request_json(base, "/v1/channels/inbox/claim", token, {"schema_version": 1, "lease_seconds": 180})
                if status != 200 or not claim.get("message"):
                    raise RuntimeError(f"claim {index} failed: HTTP {status} {claim}")
                message = claim["message"]
                status, progress = request_json(
                    base,
                    f"/v1/channels/inbox/{message['message_id']}/progress",
                    token,
                    {
                        "schema_version": 1,
                        "lease_id": message["lease_id"],
                        "text": "" if message["envelope"]["kind"] == "telegram" else "working",
                    },
                )
                if status != 200 or progress.get("accepted") is not True:
                    raise RuntimeError(f"progress {index} failed: HTTP {status} {progress}")
                if progress.get("delivered") is not (message["envelope"]["kind"] == "telegram"):
                    raise RuntimeError(f"progress delivery capability mismatch: {message['envelope']['kind']} {progress}")
                status, result = request_json(
                    base,
                    f"/v1/channels/inbox/{message['message_id']}/complete",
                    token,
                    {"schema_version": 1, "lease_id": message["lease_id"], "reply_text": f"reply-{message['envelope']['kind']}", "nola_conversation_id": f"nola-{index}"},
                )
                if status != 200 or result.get("status") != "sent":
                    raise RuntimeError(f"complete {index} failed: HTTP {status} {result}")
                completed.append({"kind": message["envelope"]["kind"], **result})
        finally:
            process.terminate()
            try:
                worker_output, _ = process.communicate(timeout=8)
            except subprocess.TimeoutExpired:
                process.kill()
                worker_output, _ = process.communicate()
            provider.shutdown()
            provider.server_close()
            provider_thread.join(timeout=3)

        query = run(
            str(WRANGLER), "d1", "execute", "nola_backend_fresh", "--local", "--persist-to", str(persist), "--json", "--command",
            "SELECT c.kind,m.status,m.reply_text,m.provider_reply_id,m.nola_conversation_id,m.lease_id FROM messagingchannelmessagerecord m JOIN messagingchannelconnectionrecord c ON c.connection_id=m.connection_id ORDER BY m.id;",
        )
        decoded = json.loads(query)
        rows = decoded[0]["results"] if isinstance(decoded, list) else decoded["results"]
        expected_ids = {"telegram": "7101", "whatsapp": "wamid-reply-e2e", "wechat_official": "wechat-reply-e2e", "we_com": "wecom-reply-e2e"}
        if len(rows) != 4 or any(row["status"] != "sent" or row["provider_reply_id"] != expected_ids[row["kind"]] or row["lease_id"] is not None for row in rows):
            raise RuntimeError(f"durable delivery audit failed: {rows}\nworker output:\n{worker_output[-4000:]}")
        paths = {record["path"].split("?")[0] for record in ProviderHandler.records}
        expected_paths = {
            "/telegram/bottelegram-token/sendMessageDraft", "/telegram/bottelegram-token/sendMessage", "/whatsapp/v23.0/phone-e2e/messages",
            "/wecom/cgi-bin/gettoken", "/wecom/cgi-bin/message/send",
            "/wechat/cgi-bin/token", "/wechat/cgi-bin/message/custom/send",
        }
        if not all(any(path.startswith(expected) for path in paths) for expected in expected_paths):
            raise RuntimeError(f"provider contract coverage incomplete: {sorted(paths)}")
        whatsapp = next(record for record in ProviderHandler.records if record["path"] == "/whatsapp/v23.0/phone-e2e/messages")
        if whatsapp["authorization"] != "Bearer whatsapp-token":
            raise RuntimeError("WhatsApp bearer credential was not forwarded correctly")
        draft = next(record for record in ProviderHandler.records if record["path"] == "/telegram/bottelegram-token/sendMessageDraft")
        if draft["body"].get("chat_id") != 7001 or draft["body"].get("draft_id", 0) <= 0 or draft["body"].get("text") != "":
            raise RuntimeError(f"Telegram draft contract mismatch: {draft}")
        print(json.dumps({"ok": True, "protocols": sorted(expected_ids), "claimed": 4, "progressed": 4, "telegram_drafts": 1, "delivered": 4, "durable_sent": 4, "provider_requests": len(ProviderHandler.records)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
