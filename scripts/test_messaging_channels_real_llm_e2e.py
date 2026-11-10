#!/usr/bin/env python3
"""Real messaging E2E: official wire formats -> Worker -> Host -> real Codex -> provider APIs."""

from __future__ import annotations

import base64
import hashlib
import hmac
import json
import os
import secrets
import select
import socket
import struct
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from e2e.cargo_paths import cargo_binary


ROOT = Path(__file__).resolve().parents[1]
WORKER = ROOT / "legacy/backend/worker"
WRANGLER = WORKER / "node_modules/.bin/wrangler"
NOLA = cargo_binary(ROOT)
CLIENT_UUID = "805beab8-3185-4b21-984f-c7611f5895d3"


def free_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def run(*args: str, cwd: Path = ROOT) -> str:
    result = subprocess.run(args, cwd=cwd, text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError(f"command failed: {' '.join(args)}\n{result.stdout[-4000:]}\n{result.stderr[-4000:]}")
    return result.stdout


def b64url(value: bytes) -> str:
    return base64.urlsafe_b64encode(value).decode().rstrip("=")


def jwt(secret: str) -> str:
    header = b64url(json.dumps({"typ": "JWT", "alg": "HS256"}, separators=(",", ":")).encode())
    payload = b64url(json.dumps({"sub": f"client:{CLIENT_UUID}", "client_uuid": CLIENT_UUID}, separators=(",", ":")).encode())
    body = f"{header}.{payload}"
    return f"{body}.{b64url(hmac.new(secret.encode(), body.encode(), hashlib.sha256).digest())}"


def request(url: str, body: bytes | None = None, headers: dict[str, str] | None = None) -> tuple[int, str]:
    method = "POST" if body is not None else "GET"
    req = urllib.request.Request(url, data=body, headers=headers or {}, method=method)
    try:
        with urllib.request.urlopen(req, timeout=10) as response:
            return response.status, response.read().decode()
    except urllib.error.HTTPError as error:
        return error.code, error.read().decode()


def wait_ready(url: str, process: subprocess.Popen[str], timeout: float = 40) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"process exited before readiness: {process.returncode}")
        try:
            if urllib.request.urlopen(url, timeout=0.5).status == 200:
                return
        except Exception:
            time.sleep(0.1)
    raise RuntimeError(f"readiness timed out: {url}")


def wait_delivery(deliveries: list[dict], lock: threading.Lock, kind: str, count: int, timeout: float | None = None) -> list[dict]:
    timeout = timeout or float(os.environ.get("NOLA_MESSAGING_E2E_DELIVERY_TIMEOUT", "240"))
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        with lock:
            matches = [item.copy() for item in deliveries if item["kind"] == kind and not item["draft"]]
        if len(matches) >= count:
            return matches
        time.sleep(0.2)
    raise RuntimeError(f"timed out waiting for {kind} delivery {count}")


def host_output_snapshot(process: subprocess.Popen[str]) -> str:
    if process.stdout is None:
        return ""
    lines: list[str] = []
    while select.select([process.stdout], [], [], 0)[0]:
        line = process.stdout.readline()
        if not line:
            break
        lines.append(line)
    return "".join(lines)[-12000:]


def pkcs7(value: bytes, block_size: int = 32) -> bytes:
    padding = block_size - len(value) % block_size
    return value + bytes([padding]) * padding


def wecom_ciphertext(message: str, receive_id: str, key: bytes) -> str:
    raw = secrets.token_bytes(16) + struct.pack(">I", len(message.encode())) + message.encode() + receive_id.encode()
    encrypted = subprocess.run(
        ["openssl", "enc", "-aes-256-cbc", "-K", key.hex(), "-iv", key[:16].hex(), "-nopad"],
        input=pkcs7(raw), check=True, capture_output=True,
    ).stdout
    return base64.b64encode(encrypted).decode()


class ProviderHandler(BaseHTTPRequestHandler):
    deliveries: list[dict] = []
    lock = threading.Lock()

    def log_message(self, _format: str, *_args: object) -> None:
        pass

    def do_GET(self) -> None:  # noqa: N802
        if self.path.startswith("/cgi-bin/token") or self.path.startswith("/cgi-bin/gettoken"):
            self.reply({"errcode": 0, "access_token": "provider-e2e-token", "expires_in": 7200})
        else:
            self.send_error(404)

    def do_POST(self) -> None:  # noqa: N802
        size = int(self.headers.get("content-length", "0"))
        payload = json.loads(self.rfile.read(size) or b"{}")
        path = urllib.parse.urlsplit(self.path).path
        if path.endswith("/sendMessageDraft"):
            kind, draft, response = "telegram", True, {"ok": True, "result": True}
        elif path.endswith("/sendMessage"):
            kind, draft, response = "telegram", False, {"ok": True, "result": {"message_id": 9101}}
        elif path.endswith("/messages"):
            kind, draft, response = "whatsapp", False, {"messages": [{"id": "wamid.reply.e2e"}]}
        elif path.endswith("/message/custom/send"):
            kind, draft, response = "wechat_official", False, {"errcode": 0, "msgid": 9201}
        elif path.endswith("/message/send"):
            kind, draft, response = "we_com", False, {"errcode": 0, "msgid": "wecom-reply-e2e"}
        else:
            self.send_error(404)
            return
        with self.lock:
            self.deliveries.append({"kind": kind, "draft": draft, "path": self.path, "payload": payload})
        self.reply(response)

    def reply(self, payload: dict) -> None:
        data = json.dumps(payload).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


def sql_quote(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def send_telegram(base: str, secret: str, update_id: int, text: str) -> None:
    payload = json.dumps({"update_id": update_id, "message": {"message_id": update_id, "date": int(time.time()), "text": text, "chat": {"id": 2001}, "from": {"id": 3001}}}).encode()
    status, body = request(f"{base}/v1/channels/webhook/telegram/telegram-real", payload, {"content-type": "application/json", "X-Telegram-Bot-Api-Secret-Token": secret})
    assert status == 200, body


def main() -> int:
    if os.environ.get("NOLA_RUN_REAL_MESSAGING_LLM_E2E") != "1":
        print("skip: set NOLA_RUN_REAL_MESSAGING_LLM_E2E=1")
        return 0
    if not WRANGLER.is_file() or not NOLA.is_file():
        raise SystemExit("build cli binaries and install backend/worker dependencies first")

    worker_port, host_port, provider_port = free_port(), free_port(), free_port()
    provider = ThreadingHTTPServer(("127.0.0.1", provider_port), ProviderHandler)
    provider_thread = threading.Thread(target=provider.serve_forever, daemon=True)
    provider_thread.start()
    provider_base = f"http://127.0.0.1:{provider_port}"
    jwt_secret = "nola-messaging-real-e2e-jwt"
    telegram_secret = "telegram-real-e2e-secret"
    whatsapp_secret = "whatsapp-real-e2e-secret"
    wechat_token = "wechat-real-e2e-token"
    wecom_token = "wecom-real-e2e-token"
    corp_id = "ww-nola-real-e2e"
    wecom_key = secrets.token_bytes(32)
    encoding_key = base64.b64encode(wecom_key).decode().rstrip("=")

    with tempfile.TemporaryDirectory(prefix="nola-messaging-real-llm-e2e-") as temporary:
        root = Path(temporary)
        persist, nola_home, workspace = root / "worker", root / "home", root / "workspace"
        persist.mkdir(); workspace.mkdir(); (nola_home / "cli").mkdir(parents=True)
        run(str(WRANGLER), "d1", "migrations", "apply", "nola_backend_fresh", "--local", "--persist-to", str(persist), cwd=WORKER)
        now = "2026-08-14T00:00:00Z"
        rows = [
            ("telegram-real", "telegram", "TELEGRAM_TOKEN", "TELEGRAM_SECRET", "", "{}"),
            ("whatsapp-real", "whatsapp", "WHATSAPP_TOKEN", "WHATSAPP_SECRET", "WHATSAPP_VERIFY", '{"phone_number_id":"phone-real","graph_version":"v23.0"}'),
            ("wechat-real", "wechat_official", "WECHAT_SECRET", "WECHAT_TOKEN", "", '{"app_id":"wx-real"}'),
            ("wecom-real", "we_com", "WECOM_SECRET", "WECOM_KEY", "WECOM_TOKEN", json.dumps({"corp_id": corp_id, "agent_id": "1000002"})),
        ]
        values = ",".join("(" + ",".join(map(sql_quote, (cid, CLIENT_UUID.replace("-", ""), kind, kind, "1000002" if kind == "we_com" else "", "", cred, hook, verify, cfg, "1", "awaiting_verification", now, now))) + ")" for cid, kind, cred, hook, verify, cfg in rows)
        seed = "INSERT INTO messagingchannelconnectionrecord (connection_id,client_uuid,kind,display_name,account_id,secret_ref,credential_ref,webhook_secret_ref,verify_token_ref,provider_config_json,enabled,state,created_at,updated_at) VALUES " + values
        run(str(WRANGLER), "d1", "execute", "nola_backend_fresh", "--local", "--persist-to", str(persist), "--command", seed, cwd=WORKER)

        backend = f"http://127.0.0.1:{worker_port}"
        config = {"schema_version": 1, "enabled": True, "backend_url": backend, "access_token": jwt(jwt_secret), "poll_seconds": 1, "turn_timeout_seconds": 240, "execution_backend": "codex", "project_root": str(workspace)}
        (nola_home / "cli/messaging-channel-worker.json").write_text(json.dumps(config))
        os.chmod(nola_home / "cli/messaging-channel-worker.json", 0o600)

        worker_config = {
            "SERVER_JWT_SECRET": jwt_secret,
            "TELEGRAM_TOKEN": "test", "TELEGRAM_SECRET": telegram_secret,
            "WHATSAPP_TOKEN": "test", "WHATSAPP_SECRET": whatsapp_secret, "WHATSAPP_VERIFY": "verify",
            "WECHAT_SECRET": "test", "WECHAT_TOKEN": wechat_token,
            "WECOM_SECRET": "test", "WECOM_KEY": encoding_key, "WECOM_TOKEN": wecom_token,
            "TELEGRAM_API_BASE": provider_base, "WHATSAPP_GRAPH_API_BASE": provider_base,
            "WECHAT_API_BASE": provider_base, "WECOM_API_BASE": provider_base,
        }
        worker_args = [str(WRANGLER), "dev", "--local", "--port", str(worker_port), "--persist-to", str(persist),
            "--var", "nola_backend_env:" + json.dumps(worker_config, separators=(",", ":"))]
        worker = subprocess.Popen(worker_args, cwd=WORKER, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        host_env = {**os.environ, "NOLA_HOME": str(nola_home), "NOLA_DB_PATH": str(root / "nola.db")}
        host = subprocess.Popen([str(NOLA), "web", "--host", "127.0.0.1", "--port", str(host_port), "--project", str(workspace), "--web-root", str(ROOT / "apps/web")], cwd=ROOT, env=host_env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        try:
            wait_ready(backend + "/healthz", worker)
            wait_ready(f"http://127.0.0.1:{host_port}/api/health", host)
            auth_headers = {"authorization": "Bearer " + config["access_token"]}
            channel_status, channel_body = request(backend + "/v1/channels", headers=auth_headers)
            if channel_status != 200 or len(json.loads(channel_body).get("channels", [])) != 4:
                raise RuntimeError(f"authenticated channel ownership mismatch: {channel_status} {channel_body}")

            send_telegram(backend, telegram_secret, 101, "The fictional plant for this conversation is CobaltLotus7319. Reply with exactly STORED.")
            try:
                first_telegram = wait_delivery(ProviderHandler.deliveries, ProviderHandler.lock, "telegram", 1)
            except Exception as error:
                status_query = run(str(WRANGLER), "d1", "execute", "nola_backend_fresh", "--local", "--persist-to", str(persist), "--json", "--command", "SELECT connection_id,status,attempt_count,last_error,reply_text FROM messagingchannelmessagerecord ORDER BY id", cwd=WORKER)
                raise RuntimeError(
                    f"{error}\nDB:\n{status_query}\nHOST:\n{host_output_snapshot(host)}\nWORKER:\n{host_output_snapshot(worker)}"
                ) from error
            assert first_telegram[0]["payload"]["text"] == "STORED"
            send_telegram(backend, telegram_secret, 102, "What was the fictional plant named? Reply with only its name.")
            assert wait_delivery(ProviderHandler.deliveries, ProviderHandler.lock, "telegram", 2)[1]["payload"]["text"] == "CobaltLotus7319"

            whatsapp = json.dumps({"entry": [{"changes": [{"value": {"messages": [{"id": "wamid-real", "from": "15550001", "timestamp": str(int(time.time())), "type": "text", "text": {"body": "Reply with exactly WHATSAPP_REAL_LLM_OK"}}]}}]}]}, separators=(",", ":")).encode()
            signature = hmac.new(whatsapp_secret.encode(), whatsapp, hashlib.sha256).hexdigest()
            assert request(backend + "/v1/channels/webhook/whatsapp/whatsapp-real", whatsapp, {"content-type": "application/json", "X-Hub-Signature-256": "sha256=" + signature})[0] == 200
            assert wait_delivery(ProviderHandler.deliveries, ProviderHandler.lock, "whatsapp", 1)[0]["payload"]["text"]["body"] == "WHATSAPP_REAL_LLM_OK"

            timestamp, nonce = str(int(time.time())), "wechat-real-nonce"
            signature = hashlib.sha1("".join(sorted([wechat_token, timestamp, nonce])).encode()).hexdigest()
            xml = f"<xml><ToUserName><![CDATA[gh_nola]]></ToUserName><FromUserName><![CDATA[wechat-user]]></FromUserName><CreateTime>{timestamp}</CreateTime><MsgType><![CDATA[text]]></MsgType><Content><![CDATA[Reply with exactly WECHAT_REAL_LLM_OK]]></Content><MsgId>wechat-real-message</MsgId></xml>".encode()
            assert request(f"{backend}/v1/channels/webhook/wechat_official/wechat-real?timestamp={timestamp}&nonce={nonce}&signature={signature}", xml, {"content-type": "application/xml"})[0] == 200
            assert wait_delivery(ProviderHandler.deliveries, ProviderHandler.lock, "wechat_official", 1)[0]["payload"]["text"]["content"] == "WECHAT_REAL_LLM_OK"

            inner = f"<xml><ToUserName><![CDATA[{corp_id}]]></ToUserName><FromUserName><![CDATA[wecom-user]]></FromUserName><CreateTime>{timestamp}</CreateTime><MsgType><![CDATA[text]]></MsgType><Content><![CDATA[Reply with exactly WECOM_REAL_LLM_OK]]></Content><MsgId>wecom-real-message</MsgId></xml>"
            encrypted = wecom_ciphertext(inner, corp_id, wecom_key)
            signature = hashlib.sha1("".join(sorted([wecom_token, timestamp, nonce, encrypted])).encode()).hexdigest()
            body = f"<xml><Encrypt><![CDATA[{encrypted}]]></Encrypt></xml>".encode()
            assert request(f"{backend}/v1/channels/webhook/we_com/wecom-real?timestamp={timestamp}&nonce={nonce}&msg_signature={signature}", body, {"content-type": "application/xml"}) == (200, "success")
            assert wait_delivery(ProviderHandler.deliveries, ProviderHandler.lock, "we_com", 1)[0]["payload"]["text"]["content"] == "WECOM_REAL_LLM_OK"
        finally:
            host.terminate(); worker.terminate()
            try: host_output, _ = host.communicate(timeout=10)
            except subprocess.TimeoutExpired: host.kill(); host_output, _ = host.communicate()
            try: worker_output, _ = worker.communicate(timeout=10)
            except subprocess.TimeoutExpired: worker.kill(); worker_output, _ = worker.communicate()

        query = run(str(WRANGLER), "d1", "execute", "nola_backend_fresh", "--local", "--persist-to", str(persist), "--json", "--command", "SELECT connection_id,status,nola_conversation_id,reply_text FROM messagingchannelmessagerecord ORDER BY id", cwd=WORKER)
        results = json.loads(query)[0]["results"]
        if len(results) != 5 or any(row["status"] != "sent" or not row["nola_conversation_id"] for row in results):
            raise RuntimeError(f"unexpected delivery state: {results}\nHOST:\n{host_output[-5000:]}\nWORKER:\n{worker_output[-5000:]}")
        telegram_sessions = {row["nola_conversation_id"] for row in results if row["connection_id"] == "telegram-real"}
        assert len(telegram_sessions) == 1
        with ProviderHandler.lock:
            draft_count = sum(1 for item in ProviderHandler.deliveries if item["draft"])
        print(json.dumps({"ok": True, "real_execution_backend": "codex", "real_llm_turns": 5, "protocols": ["telegram", "whatsapp", "wechat_official", "we_com"], "telegram_resume": True, "sent": len(results), "streaming_drafts": draft_count}))
    provider.shutdown(); provider.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
