#!/usr/bin/env python3
"""Run real HTTP webhook acceptance against the Cloudflare Worker and local D1."""

from __future__ import annotations

import base64
import hashlib
import hmac
import json
import os
import secrets
import socket
import struct
import subprocess
import tempfile
import time
from contextlib import contextmanager
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKER = ROOT / "legacy/backend/worker"
WRANGLER = WORKER / "node_modules/.bin/wrangler"
FIXTURE_DATABASE = "nola_webhook_fixture"


@contextmanager
def retained_fixture_directory():
    root = Path(tempfile.mkdtemp(prefix="nola-worker-channel-e2e-"))
    try:
        yield root
    finally:
        print(json.dumps({"preserved_webhook_fixture": str(root)}), flush=True)


def isolated_environment(root: Path) -> dict[str, str]:
    home = os.environ.get("HOME", "")
    if not home or os.path.lexists(Path(home) / ".wrangler"):
        raise RuntimeError("Cannot isolate Wrangler: HOME is absent or legacy ~/.wrangler exists; no credentials were read")
    environment = {name: os.environ[name] for name in ("PATH", "HOME", "LANG") if name in os.environ}
    environment.setdefault("PATH", os.defpath)
    for variable, directory in (
        ("XDG_CONFIG_HOME", "config"), ("XDG_CACHE_HOME", "cache"),
        ("XDG_DATA_HOME", "data"), ("TMPDIR", "tmp"),
    ):
        location = root / directory
        location.mkdir(mode=0o700)
        environment[variable] = str(location)
    environment.update({
        "CI": "true", "NO_PROXY": "*", "no_proxy": "*",
        "CLOUDFLARE_LOAD_DEV_VARS_FROM_DOT_ENV": "false",
        "CLOUDFLARE_INCLUDE_PROCESS_ENV": "false",
        "WRANGLER_SEND_METRICS": "false", "WRANGLER_SEND_ERROR_REPORTS": "false",
        "WRANGLER_LOG_PATH": str(root / "wrangler.log"),
    })
    return environment


def prepare_fixture(root: Path) -> dict[str, str]:
    environment = isolated_environment(root)
    (root / "state").mkdir(mode=0o700)
    # Do not copy production routes, bindings, vars or credential references.
    configuration = (
        'name = "nola-webhook-fixture"\n'
        f'main = {json.dumps(str(WORKER / "src/index.ts"))}\n'
        'compatibility_date = "2026-05-10"\n'
        'send_metrics = false\n'
        '[[d1_databases]]\n'
        'binding = "DB"\n'
        f'database_name = "{FIXTURE_DATABASE}"\n'
        'database_id = "00000000-0000-4000-8000-000000000001"\n'
        f'migrations_dir = {json.dumps(str(WORKER / "migrations"))}\n'
    )
    for name, content in (("wrangler.toml", configuration), ("empty.env", "")):
        with (root / name).open("x", encoding="utf-8") as stream:
            stream.write(content)
        (root / name).chmod(0o600)
    return environment


def wrangler_command(root: Path, *arguments: str) -> list[str]:
    permitted = arguments[:1] == ("dev",) or arguments[:4] in (
        ("d1", "migrations", "apply", FIXTURE_DATABASE),
    ) or arguments[:3] == ("d1", "execute", FIXTURE_DATABASE)
    overridden = any(argument.split("=", 1)[0] in {
        "--remote", "--config", "-c", "--cwd", "--env-file", "--persist-to", "--env", "-e",
    } for argument in arguments)
    if not permitted or overridden:
        raise ValueError("Webhook fixture permits only its fixed local Wrangler commands")
    # Wrangler's array-valued env-file flag consumes following positionals.
    # Bind its one value explicitly and keep it last, after the full subcommand.
    return [str(WRANGLER), "--config", str(root / "wrangler.toml"), *arguments,
            "--local", "--persist-to", str(root / "state"),
            f"--env-file={root / 'empty.env'}"]


def free_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def run(root: Path, environment: dict[str, str], *arguments: str) -> str:
    command = wrangler_command(root, *arguments)
    completed = subprocess.run(command, cwd=root, env=environment, text=True, capture_output=True)
    if completed.returncode != 0:
        raise RuntimeError(
            f"command failed ({completed.returncode}): {' '.join(command)}\n"
            f"stdout:\n{completed.stdout[-6000:]}\n"
            f"stderr:\n{completed.stderr[-6000:]}"
        )
    return completed.stdout


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def request(url: str, body: bytes | None = None, headers: dict[str, str] | None = None,
            *, timeout: float = 10) -> tuple[int, str]:
    target = urllib.parse.urlsplit(url)
    if target.scheme != "http" or target.hostname != "127.0.0.1" or target.username is not None \
            or target.password is not None or target.fragment or not target.port:
        raise ValueError("Webhook fixture requests must use literal loopback HTTP")
    req = urllib.request.Request(url, data=body, headers=headers or {}, method="POST" if body is not None else "GET")
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    try:
        with opener.open(req, timeout=timeout) as response:
            return response.status, response.read().decode("utf-8")
    except urllib.error.HTTPError as error:
        return error.code, error.read().decode("utf-8")


def pkcs7(value: bytes, block_size: int = 32) -> bytes:
    padding = block_size - len(value) % block_size
    return value + bytes([padding]) * padding


def wecom_ciphertext(message: str, receive_id: str, key: bytes, environment: dict[str, str]) -> str:
    random_prefix = secrets.token_bytes(16)
    message_bytes = message.encode()
    plaintext = pkcs7(random_prefix + struct.pack(">I", len(message_bytes)) + message_bytes + receive_id.encode())
    process = subprocess.run(
        ["openssl", "enc", "-aes-256-cbc", "-K", key.hex(), "-iv", key[:16].hex(), "-nopad"],
        input=plaintext,
        check=True,
        capture_output=True,
        env=environment,
    )
    return base64.b64encode(process.stdout).decode()


def sql_quote(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def main() -> int:
    if not WRANGLER.is_file():
        raise SystemExit("backend/worker dependencies are not installed")
    port = free_port()
    telegram_secret = "telegram-e2e-secret"
    whatsapp_secret = "whatsapp-e2e-secret"
    whatsapp_verify = "whatsapp-e2e-verify"
    wecom_token = "wecom-e2e-token"
    wechat_token = "wechat-official-e2e-token"
    wecom_key = secrets.token_bytes(32)
    wecom_encoding_key = base64.b64encode(wecom_key).decode().rstrip("=")
    corp_id = "ww-nola-e2e"

    with retained_fixture_directory() as temporary:
        environment = prepare_fixture(temporary)
        run(temporary, environment, "d1", "migrations", "apply", FIXTURE_DATABASE)
        now = "2026-08-13T00:00:00Z"
        rows = [
            ("telegram-e2e", "telegram", "TELEGRAM_TOKEN", "TELEGRAM_SECRET", "", "{}"),
            ("whatsapp-e2e", "whatsapp", "WHATSAPP_TOKEN", "WHATSAPP_SECRET", "WHATSAPP_VERIFY", '{"phone_number_id":"phone-e2e"}'),
            ("wechat-e2e", "wechat_official", "WECHAT_APP_SECRET", "WECHAT_TOKEN", "", '{"app_id":"wx-e2e"}'),
            ("wecom-e2e", "we_com", "WECOM_SECRET", "WECOM_KEY", "WECOM_TOKEN", json.dumps({"corp_id": corp_id})),
        ]
        # Model a previously valid connection whose referenced secret was revoked.
        # Creation-time validation must not be the callback authentication boundary.
        unavailable = [("missing", "E2E_MISSING", ""), ("empty", "E2E_EMPTY", ""), ("blank", "E2E_BLANK", "   ")]
        for label, reference, _ in unavailable:
            rows.extend([
                (f"telegram-{label}", "telegram", "TELEGRAM_TOKEN", reference, "", "{}"),
                (f"whatsapp-{label}", "whatsapp", "WHATSAPP_TOKEN", reference, reference, "{}"),
                (f"wechat-{label}", "wechat_official", "WECHAT_APP_SECRET", reference, "", "{}"),
                (f"wecom-token-{label}", "we_com", "WECOM_SECRET", "WECOM_KEY", reference, json.dumps({"corp_id": corp_id})),
                (f"wecom-key-{label}", "we_com", "WECOM_SECRET", reference, "WECOM_TOKEN", json.dumps({"corp_id": corp_id})),
            ])
        values = ",".join(
            "(" + ",".join(map(sql_quote, (
                connection_id, "channel-e2e-client", kind, kind, "", "", credential, webhook, verify,
                provider_config, "1", "awaiting_verification", now, now,
            ))) + ")"
            for connection_id, kind, credential, webhook, verify, provider_config in rows
        )
        seed = (
            "INSERT INTO messagingchannelconnectionrecord "
            "(connection_id,client_uuid,kind,display_name,account_id,secret_ref,credential_ref,webhook_secret_ref,verify_token_ref,provider_config_json,enabled,state,created_at,updated_at) VALUES "
            + values + ";"
        )
        run(temporary, environment, "d1", "execute", FIXTURE_DATABASE, "--command", seed)

        command = wrangler_command(temporary,
            "dev", "--ip", "127.0.0.1", "--port", str(port),
            "--var", "TELEGRAM_TOKEN:test-token", "--var", f"TELEGRAM_SECRET:{telegram_secret}",
            "--var", "WHATSAPP_TOKEN:test-token", "--var", f"WHATSAPP_SECRET:{whatsapp_secret}",
            "--var", f"WHATSAPP_VERIFY:{whatsapp_verify}", "--var", "WECOM_SECRET:test-secret",
            "--var", "WECHAT_APP_SECRET:test-secret", "--var", f"WECHAT_TOKEN:{wechat_token}",
            "--var", f"WECOM_KEY:{wecom_encoding_key}", "--var", f"WECOM_TOKEN:{wecom_token}",
            # JSON binding preserves exact blank bytes instead of depending on
            # Wrangler's scalar --var parsing, and overrides ambient bindings.
            "--var", "nola_backend_env:" + json.dumps({"E2E_MISSING": None, "E2E_EMPTY": "", "E2E_BLANK": "   "}),
        )
        process = subprocess.Popen(command, cwd=temporary, env=environment, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        base = f"http://127.0.0.1:{port}"
        try:
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise RuntimeError("wrangler exited before becoming ready")
                try:
                    if request(base + "/healthz", timeout=0.5)[0] == 200:
                        break
                except Exception:
                    time.sleep(0.1)
            else:
                raise RuntimeError("worker readiness timed out")

            telegram = json.dumps({"update_id": 101, "message": {"message_id": 7, "date": 1786579200, "text": "telegram hello", "chat": {"id": 2001}, "from": {"id": 3001}}}).encode()
            telegram_url = base + "/v1/channels/webhook/telegram/telegram-e2e"
            assert request(telegram_url, telegram, {"content-type": "application/json"})[0] == 403
            first_status, first_body = request(telegram_url, telegram, {"content-type": "application/json", "X-Telegram-Bot-Api-Secret-Token": telegram_secret})
            duplicate_status, duplicate_body = request(telegram_url, telegram, {"content-type": "application/json", "X-Telegram-Bot-Api-Secret-Token": telegram_secret})
            assert first_status == duplicate_status == 200
            assert json.loads(first_body)["inserted"] == 1
            assert json.loads(duplicate_body)["duplicates"] == 1

            verify_url = base + "/v1/channels/webhook/whatsapp/whatsapp-e2e?hub.mode=subscribe&hub.verify_token=" + whatsapp_verify + "&hub.challenge=nola-challenge"
            assert request(verify_url) == (200, "nola-challenge")
            whatsapp = json.dumps({"entry": [{"changes": [{"value": {"messages": [{"id": "wamid-e2e", "from": "15550001", "timestamp": "1786579200", "type": "text", "text": {"body": "whatsapp hello"}}]}}]}]}, separators=(",", ":")).encode()
            signature = hmac.new(whatsapp_secret.encode(), whatsapp, hashlib.sha256).hexdigest()
            whatsapp_url = base + "/v1/channels/webhook/whatsapp/whatsapp-e2e"
            assert request(whatsapp_url, whatsapp, {"content-type": "application/json", "X-Hub-Signature-256": "sha256=bad"})[0] == 403
            assert request(whatsapp_url, whatsapp, {"content-type": "application/json", "X-Hub-Signature-256": "sha256=" + signature})[0] == 200

            wechat_timestamp, wechat_nonce = "1786579200", "wechat-nonce"
            wechat_signature = hashlib.sha1("".join(sorted([wechat_token, wechat_timestamp, wechat_nonce])).encode()).hexdigest()
            wechat_base = f"{base}/v1/channels/webhook/wechat_official/wechat-e2e?timestamp={wechat_timestamp}&nonce={wechat_nonce}"
            assert request(wechat_base + "&signature=bad&echostr=nola-wechat") [0] == 403
            assert request(wechat_base + "&signature=" + wechat_signature + "&echostr=nola-wechat") == (200, "nola-wechat")
            wechat_xml = b"<xml><ToUserName><![CDATA[gh_nola]]></ToUserName><FromUserName><![CDATA[wechat-user]]></FromUserName><CreateTime>1786579200</CreateTime><MsgType><![CDATA[text]]></MsgType><Content><![CDATA[wechat hello]]></Content><MsgId>wechat-message-e2e</MsgId></xml>"
            wechat_callback = wechat_base + "&signature=" + wechat_signature
            assert request(wechat_callback, wechat_xml, {"content-type": "application/xml"})[0] == 200
            duplicate_status, duplicate_body = request(wechat_callback, wechat_xml, {"content-type": "application/xml"})
            assert duplicate_status == 200 and json.loads(duplicate_body)["duplicates"] == 1

            xml = "<xml><ToUserName><![CDATA[" + corp_id + "]]></ToUserName><FromUserName><![CDATA[wecom-user]]></FromUserName><CreateTime>1786579200</CreateTime><MsgType><![CDATA[text]]></MsgType><Content><![CDATA[wecom hello]]></Content><MsgId>wecom-message-e2e</MsgId></xml>"
            encrypted = wecom_ciphertext(xml, corp_id, wecom_key, environment)
            timestamp, nonce = "1786579200", "nola-nonce"
            signature = hashlib.sha1("".join(sorted([wecom_token, timestamp, nonce, encrypted])).encode()).hexdigest()
            callback = f"{base}/v1/channels/webhook/we_com/wecom-e2e?timestamp={timestamp}&nonce={nonce}&msg_signature={signature}"
            callback_body = f"<xml><Encrypt><![CDATA[{encrypted}]]></Encrypt></xml>".encode()
            assert request(callback.replace(signature, "bad"), callback_body, {"content-type": "application/xml"})[0] == 403
            assert request(callback, callback_body, {"content-type": "application/xml"}) == (200, "success")

            for label, _, unavailable_secret in unavailable:
                telegram_invalid = f"{base}/v1/channels/webhook/telegram/telegram-{label}"
                assert request(telegram_invalid, telegram, {"content-type": "application/json"})[0] == 403
                assert request(telegram_invalid, telegram, {"content-type": "application/json", "X-Telegram-Bot-Api-Secret-Token": unavailable_secret})[0] == 403

                whatsapp_invalid = f"{base}/v1/channels/webhook/whatsapp/whatsapp-{label}"
                empty_key_signature = hmac.new(unavailable_secret.encode(), whatsapp, hashlib.sha256).hexdigest()
                assert request(whatsapp_invalid, whatsapp, {"content-type": "application/json", "X-Hub-Signature-256": "sha256=" + empty_key_signature})[0] == 403
                assert request(whatsapp_invalid + "?" + urllib.parse.urlencode({"hub.mode": "subscribe", "hub.verify_token": unavailable_secret, "hub.challenge": "must-not-echo"}))[0] == 403

                wechat_invalid_signature = hashlib.sha1("".join(sorted([unavailable_secret, wechat_timestamp, wechat_nonce])).encode()).hexdigest()
                wechat_invalid = f"{base}/v1/channels/webhook/wechat_official/wechat-{label}?timestamp={wechat_timestamp}&nonce={wechat_nonce}&signature={wechat_invalid_signature}"
                assert request(wechat_invalid + "&echostr=must-not-echo")[0] == 403
                assert request(wechat_invalid, wechat_xml, {"content-type": "application/xml"})[0] == 403

                wecom_invalid_signature = hashlib.sha1("".join(sorted([unavailable_secret, timestamp, nonce, encrypted])).encode()).hexdigest()
                wecom_invalid = f"{base}/v1/channels/webhook/we_com/wecom-token-{label}?timestamp={timestamp}&nonce={nonce}&msg_signature={wecom_invalid_signature}"
                assert request(wecom_invalid + "&" + urllib.parse.urlencode({"echostr": encrypted}))[0] == 403
                assert request(wecom_invalid, callback_body, {"content-type": "application/xml"})[0] == 403
                # A valid signature token cannot compensate for a revoked AES key.
                wecom_invalid_key = callback.replace("wecom-e2e?", f"wecom-key-{label}?")
                assert request(wecom_invalid_key + "&" + urllib.parse.urlencode({"echostr": encrypted}))[0] == 403
                assert request(wecom_invalid_key, callback_body, {"content-type": "application/xml"})[0] == 403
        finally:
            process.terminate()
            try:
                output, _ = process.communicate(timeout=8)
            except subprocess.TimeoutExpired:
                process.kill()
                output, _ = process.communicate()
            (temporary / "worker-output.log").write_text(output, encoding="utf-8")

        query = run(
            temporary, environment, "d1", "execute", FIXTURE_DATABASE,
            "--json", "--command",
            "SELECT connection_id,provider_event_id,text,status FROM messagingchannelmessagerecord ORDER BY connection_id;",
        )
        decoded = json.loads(query)
        results = decoded[0]["results"] if isinstance(decoded, list) else decoded["results"]
        expected = {
            ("telegram-e2e", "101", "telegram hello"),
            ("whatsapp-e2e", "wamid-e2e", "whatsapp hello"),
            ("wechat-e2e", "wechat-message-e2e", "wechat hello"),
            ("wecom-e2e", "wecom-message-e2e", "wecom hello"),
        }
        actual = {(row["connection_id"], row["provider_event_id"], row["text"]) for row in results}
        if actual != expected or any(row["status"] != "received" for row in results):
            raise RuntimeError(f"unexpected inbox projection: {results}\nworker output:\n{output[-4000:]}")
        print(json.dumps({"ok": True, "protocols": ["telegram", "whatsapp", "wechat_official", "we_com"], "inbox_count": len(results), "deduplication": True, "invalid_signatures_rejected": True, "unavailable_secrets_rejected_without_inbox_writes": True}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
