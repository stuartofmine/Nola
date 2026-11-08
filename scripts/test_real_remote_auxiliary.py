#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Opt-in real 192.168 model acceptance; isolated Host/database, no fake model.

A transparent counting relay forwards actual requests and actual provider
responses. Its drop mode discards a real response to test unknown transport
outcomes; it never synthesizes model output or usage. No build/cargo invocation.
--binary must name the CLI `nola` (which supports `web`), not `nola-service`.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import sqlite3
import threading
import urllib.error
import urllib.parse
import urllib.request
import uuid

from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from test_real_submission_journal import Host, require


def b64(value):
    return base64.urlsafe_b64encode(value).decode().rstrip("=")


def unb64(value):
    return base64.urlsafe_b64decode(value + "=" * (-len(value) % 4))


class Relay(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, upstream):
        super().__init__(("127.0.0.1", 0), RelayHandler)
        self.upstream = upstream
        self.drop_response = False
        self.calls = []
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()


class RelayHandler(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass  # Never log credential-bearing paths or request/response bodies.

    def do_POST(self):
        relay = self.server
        data = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        request_body = json.loads(data)
        require(request_body.get("stream") is False, "Auxiliary physical request must be nonstreaming")
        require(not request_body.get("tools"), "Auxiliary physical request must not expose tools")
        require("tool_calls" not in request_body, "Auxiliary request contains tool calls")
        call = {"usage": None}
        relay.calls.append(call)
        target = urllib.parse.urlunsplit((relay.upstream.scheme, relay.upstream.netloc, self.path, "", ""))
        headers = {key: self.headers[key] for key in
                   ("Authorization", "Content-Type", "x-api-key", "anthropic-version") if key in self.headers}
        request = urllib.request.Request(target, data=data, headers=headers, method="POST")
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        try:
            response = opener.open(request, timeout=360)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            body = response.read()
            try:
                call["usage"] = json.loads(body).get("usage")
            except (ValueError, AttributeError):
                pass
            if relay.drop_response:
                self.close_connection = True
                self.connection.shutdown(socket.SHUT_RDWR)
                self.connection.close()
                return
            self.send_response(response.status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)


def mobile(host, token, action, body):
    nonce = os.urandom(12)
    key = hashlib.sha256(token.encode()).digest()
    sealed = AESGCM(key).encrypt(nonce, json.dumps(body, separators=(",", ":")).encode(), b"nola-mobile-bridge-v1")
    envelope = dict(type="encrypted", v=1, alg="AES-256-GCM", nonce=b64(nonce), ciphertext=b64(sealed[:-16]), tag=b64(sealed[-16:]))
    request = urllib.request.Request(host.base + "/api/mobile/auxiliary/" + action,
        data=json.dumps(envelope).encode(), headers={"Authorization": "Bearer " + token,
            "Content-Type": "application/json", "X-Nola-Mobile-Crypto": "aes-gcm-v1"}, method="POST")
    with host.http.open(request, timeout=390) as response:
        encrypted = json.load(response)
    require(encrypted.get("type") == "encrypted", "Source response lacks authenticated encryption")
    result = json.loads(AESGCM(key).decrypt(unb64(encrypted["nonce"]),
        unb64(encrypted["ciphertext"]) + unb64(encrypted["tag"]), b"nola-mobile-bridge-v1"))
    require(result["source_host_id"] == host.host_id, "Source Host identity changed")
    return result


def frontier(host, operation):
    path = host.directory / "acceptance.sqlite"
    with sqlite3.connect(path.as_uri() + "?mode=ro", uri=True) as database:
        require(database.execute("SELECT COUNT(*) FROM runs").fetchone()[0] == 0, "Auxiliary request fabricated a Run")
        rows = database.execute("SELECT record_json FROM run_attempts WHERE operation_id=? AND run_id IS NULL", (operation,)).fetchall()
    require(len(rows) == 1, "Expected one canonical physical Attempt")
    return json.loads(rows[0][0])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True, help="CLI nola executable with the web subcommand (not nola-service)")
    parser.add_argument("--config", type=Path, default=Path.home() / ".nola/config.json")
    parser.add_argument("--evidence", type=Path, default=Path("/tmp") / ("nola-real-auxiliary-" + uuid.uuid4().hex + ".json"))
    args = parser.parse_args()
    require(os.environ.get("NOLA_RUN_LIVE_AUXILIARY_E2E") == "1", "Set NOLA_RUN_LIVE_AUXILIARY_E2E=1 to issue real model requests")
    configuration = json.loads(args.config.read_text())
    llm = configuration["llm"]
    upstream = urllib.parse.urlsplit(llm["endpoint"])
    require((upstream.hostname or "").startswith("192.168.") and not upstream.query
            and not upstream.username and not upstream.fragment, "Acceptance requires an explicit 192.168.* model route without query/user-info")
    relay = Relay(upstream)
    host = None
    report = {"real_upstream": True, "physical_http_calls_observed": True, "passed": False}
    try:
        host = Host(args.binary.resolve(strict=True), args.config.resolve(strict=True))
        token = (host.directory / ".nola/web/mobile-access-token").read_text().strip()
        existing = host.json("/api/model-profiles")["profiles"]
        protocol = {"openai-responses": "responses", "responses": "responses", "anthropic": "anthropic"}.get(llm.get("provider"), "chat_completions")
        relay_endpoint = f"http://127.0.0.1:{relay.server_port}" + upstream.path.rstrip("/")
        catalog = host.json("/api/model-profiles", dict(expectedRevisions={row["id"]: row["revision"] for row in existing},
            modelName=llm["model"], endpointName="Isolated real-model auxiliary acceptance", baseURL=relay_endpoint,
            wireProtocol=protocol, priority=50, capabilities=["chat"], executionCapabilities=dict(
                toolCalls=False, imageInput=False, reasoning=False, samplingParameters=True)))
        profile = next(row for row in catalog["profiles"] if row["endpoint"]["baseURL"] == relay_endpoint)
        endpoint = profile["endpoint"]["id"]
        credential_path = "/api/model-endpoints/" + urllib.parse.quote(endpoint, safe="") + "/credential"
        credential = host.json(credential_path)
        if llm.get("api_key"):
            host.json(credential_path, dict(expectedRevisions=credential["expectedRevisions"], baseURL=relay_endpoint,
                wireProtocol=protocol, credentialRevision=credential["credentialRevision"], action="replace", secret=llm["api_key"]))
        prepared = mobile(host, token, "prepare", dict(expected_host_id=host.host_id, profile_id=profile["id"]))

        def request_body(marker):
            operation = str(uuid.uuid4())
            model_input = dict(dateRange=dict(start="2026-10-15", end="2026-10-16"), candidates=[], attachments=[],
                userPrefs=dict(message="Return a valid summary JSON with empty items and explain exactly " + marker))
            return dict(expected_host_id=host.host_id, operation_id=operation, attempt_id=operation + ":physical",
                profile_id=prepared["profile_id"], expected_profile_revision=prepared["revision"], scenario="event_summary",
                input_json=json.dumps(model_input, separators=(",", ":")), maximum_duration_ms=300000)

        success = request_body("REAL_AUX_" + secrets.token_hex(8))
        ready = mobile(host, token, "admit", success)
        require(ready["receipt"]["attempt"] is None and ready["receipt"]["operation"]["state"] == "ready", "Admission fabricated a physical result")
        require(not relay.calls, "Admission must not dispatch")
        complete = mobile(host, token, "invoke", success)
        require(complete["receipt"]["attempt"]["state"] == "succeeded", "Real model did not return a complete structured summary")
        require(len(relay.calls) == 1, "Expected exactly one real provider HTTP request")
        ledger = frontier(host, success["operation_id"])
        require(ledger == complete["receipt"]["attempt"], "HTTP receipt differs from durable physical ledger")
        if relay.calls[0]["usage"] is not None:
            require(ledger["usage"]["provider_raw"] == relay.calls[0]["usage"], "Real upstream raw usage was lost")
        repeated = mobile(host, token, "invoke", success)
        require(repeated == complete and len(relay.calls) == 1, "Duplicate invoke redispatched the provider")

        relay.drop_response = True
        uncertain = request_body("REAL_UNKNOWN_" + secrets.token_hex(8))
        mobile(host, token, "admit", uncertain)
        unknown = mobile(host, token, "invoke", uncertain)
        require(unknown["receipt"]["attempt"]["state"] == "running" and unknown["receipt"]["attempt"]["usage"]["coverage"] == "unknown", "Lost real response must remain unconfirmed")
        require(len(relay.calls) == 2, "Unknown path retried physical HTTP")
        control = {key: uncertain[key] for key in ("expected_host_id", "operation_id", "attempt_id")}
        mobile(host, token, "cancel", control)
        host.stop()
        host.start()  # The fixture's durable Host identity and token are retained.
        recovered = mobile(host, token, "inspect", control)
        require(recovered["receipt"]["operation"]["cancellation_requested_at_ms"] is not None, "Cancellation intent was not durable")
        require(recovered["receipt"]["attempt"] == frontier(host, uncertain["operation_id"]), "Unknown physical identity changed across restart")
        mobile(host, token, "invoke", uncertain)
        require(len(relay.calls) == 2, "Restart or cancellation silently redispatched unknown work")
        report.update(passed=True, provider_http_calls=len(relay.calls), success=complete, recovered_unknown=recovered,
                      automatic_late_usage_sync=False, late_unknown_policy="Retained original Running Attempt; inspect never redispatches")
    finally:
        if host:
            host.close()
            shutil.rmtree(host.directory)  # Only this script's mktemp fixture; never a user database.
        relay.shutdown()
        relay.server_close()
        with args.evidence.open("x", encoding="utf-8") as output:
            os.chmod(args.evidence, 0o600)
            json.dump(report, output, ensure_ascii=False, indent=2)
        print("Real auxiliary evidence:", args.evidence)
    require(report["passed"], "Real auxiliary acceptance failed; see redacted evidence")


if __name__ == "__main__":
    main()
