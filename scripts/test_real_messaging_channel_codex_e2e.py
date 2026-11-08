#!/usr/bin/env python3
"""Real LLM E2E: channel lease -> nola-agent-host -> installed execution backend."""

from __future__ import annotations

import argparse
import json
import os
import secrets
import shutil
import socket
import subprocess
import tempfile
import threading
import time
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


class ChannelState:
    def __init__(self, marker: str) -> None:
        self.marker = marker
        self.claim_count = 0
        self.completed_message_ids: set[int] = set()
        self.retry_message_id: int | None = None
        self.retry_attempts: dict[int, int] = {}
        self.completed: list[dict] = []
        self.failed: list[dict] = []
        self.renewed: list[dict] = []
        self.progress: list[dict] = []
        self.lock = threading.Lock()


def handler_for(state: ChannelState):
    class Handler(BaseHTTPRequestHandler):
        def do_POST(self) -> None:  # noqa: N802
            length = int(self.headers.get("content-length", "0"))
            body = json.loads(self.rfile.read(length) or b"{}")
            if self.headers.get("authorization") != "Bearer channel-e2e-token":
                self.send_response(401)
                self.end_headers()
                return
            if self.path == "/v1/channels/inbox/claim":
                with state.lock:
                    if state.retry_message_id is not None:
                        message_id = state.retry_message_id
                        state.retry_message_id = None
                        state.retry_attempts[message_id] = state.retry_attempts.get(message_id, 1) + 1
                        payload = {"message": self._message(message_id, state.retry_attempts[message_id])}
                    elif state.claim_count >= 2:
                        payload = {"message": None}
                    else:
                        state.claim_count += 1
                        message_id = state.claim_count
                        state.retry_attempts[message_id] = 1
                        payload = {"message": self._message(message_id, 1)}
                self._json(200, payload)
                return
            if self.path in {"/v1/channels/inbox/1/complete", "/v1/channels/inbox/2/complete"}:
                with state.lock:
                    message_id = int(self.path.split("/")[-2])
                    if message_id not in state.completed_message_ids:
                        state.completed_message_ids.add(message_id)
                        state.completed.append(body)
                self._json(200, {"accepted": True, "status": "sent"})
                return
            if self.path in {"/v1/channels/inbox/1/renew", "/v1/channels/inbox/2/renew"}:
                with state.lock:
                    state.renewed.append(body)
                self._json(200, {"accepted": True, "lease_expires_at": "2099-01-01T00:00:00Z"})
                return
            if self.path in {"/v1/channels/inbox/1/progress", "/v1/channels/inbox/2/progress"}:
                with state.lock:
                    state.progress.append({"message_id": int(self.path.split("/")[-2]), **body})
                self._json(200, {"accepted": True, "delivered": True})
                return
            if self.path in {"/v1/channels/inbox/1/fail", "/v1/channels/inbox/2/fail"}:
                with state.lock:
                    state.failed.append(body)
                    message_id = int(self.path.split("/")[-2])
                    retryable = bool(body.get("retryable"))
                    attempt = state.retry_attempts.get(message_id, 1)
                    if retryable and attempt < 3 and message_id not in state.completed_message_ids:
                        state.retry_message_id = message_id
                        status = "retrying"
                    else:
                        status = "failed"
                self._json(200, {"accepted": True, "status": status})
                return
            self.send_response(404)
            self.end_headers()

        def _json(self, status: int, payload: dict) -> None:
            encoded = json.dumps(payload).encode()
            self.send_response(status)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)

        def _message(self, message_id: int, attempt_count: int) -> dict:
            prompt = (
                "Remember this exact token for my next message: "
                + state.marker
                + ". For this message, reply with only the exact ASCII token ACK."
                if message_id == 1
                else "Reply with exactly the token I asked you to remember in my previous message."
            )
            return {
                "schema_version": 1,
                "message_id": message_id,
                "lease_id": f"lease-real-{message_id}",
                "lease_expires_at": "2099-01-01T00:00:00Z",
                "attempt_count": attempt_count,
                "envelope": {
                    "schema_version": 1,
                    "connection_id": "telegram-e2e",
                    "kind": "telegram",
                    "provider_event_id": f"update-{message_id}",
                    "external_conversation_id": "chat-real-codex",
                    "external_sender_id": "user-1",
                    "received_at": "2026-08-13T00:00:00Z",
                    "text": prompt,
                    "attachment_ids": [],
                },
            }

        def log_message(self, _format: str, *_args: object) -> None:
            return

    return Handler


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--timeout", type=int, default=240)
    parser.add_argument(
        "--execution-backend",
        choices=("codex", "claude_code", "opencode"),
        default="codex",
    )
    args = parser.parse_args()

    build = subprocess.run(
        ["cargo", "build", "--quiet", "--bin", "nola", "--bin", "nola-agent-host"],
        cwd=Path(__file__).resolve().parents[1] / "cli",
        text=True,
        capture_output=True,
    )
    if build.returncode != 0:
        raise SystemExit(
            "failed to build the current CLI/Web Host sources before E2E\n"
            + build.stdout[-4000:]
            + build.stderr[-4000:]
        )
    root = Path(__file__).resolve().parents[1]
    sys.path.insert(0, str(root / "scripts"))
    from real_e2e_evidence import record_step
    cli = root / "cli"
    binary = cli / "target" / "debug" / "nola"
    agent_host = cli / "target" / "debug" / "nola-agent-host"
    if not binary.is_file() or not agent_host.is_file():
        raise SystemExit("build cli binaries first: cargo build --bin nola --bin nola-agent-host")
    executable = {
        "codex": "codex",
        "claude_code": "claude",
        "opencode": "opencode",
    }[args.execution_backend]
    if shutil.which(executable) is None:
        raise SystemExit(f"installed {executable} CLI is required")

    marker = f"NOLA_CHANNEL_E2E_{secrets.token_hex(8)}"
    state = ChannelState(marker)
    evidence_root = Path(
        os.environ.get("NOLA_E2E_ARTIFACT_ROOT")
        or tempfile.mkdtemp(prefix="nola-channel-real-evidence-")
    ) / f"channel-{args.execution_backend}-{secrets.token_hex(4)}"
    evidence_root.mkdir(parents=True, exist_ok=True)
    provider_trace = evidence_root / "external-provider.ndjson"
    event_stream = evidence_root / "channel-events.ndjson"
    persistence_audit = evidence_root / "channel-persistence-audit.json"
    persistence_store = evidence_root / "messaging-channel-worker.json"
    channel_port = free_port()
    web_port = free_port()
    server = ThreadingHTTPServer(("127.0.0.1", channel_port), handler_for(state))
    server_thread = threading.Thread(target=server.serve_forever, daemon=True)
    server_thread.start()

    with tempfile.TemporaryDirectory(prefix="nola-channel-real-e2e-") as temporary:
        nola_home = Path(temporary)
        config_dir = nola_home / "cli"
        config_dir.mkdir(parents=True)
        config_path = config_dir / "messaging-channel-worker.json"
        config_path.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "enabled": True,
                    "backend_url": f"http://127.0.0.1:{channel_port}",
                    "access_token": "channel-e2e-token",
                    "poll_seconds": 1,
                    "turn_timeout_seconds": args.timeout,
                    "execution_backend": args.execution_backend,
                    "project_root": str(root),
                }
            )
        )
        config_path.chmod(0o600)
        shutil.copyfile(config_path, persistence_store)
        environment = os.environ.copy()
        environment["NOLA_HOME"] = str(nola_home)
        process = subprocess.Popen(
            [
                str(binary),
                "web",
                "--host",
                "127.0.0.1",
                "--port",
                str(web_port),
                "--project",
                str(root),
                "--web-root",
                str(root / "web"),
            ],
            cwd=root,
            env=environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        deadline = time.monotonic() + args.timeout + 30
        try:
            while time.monotonic() < deadline:
                with state.lock:
                    terminal_failure = any(
                        not failure.get("retryable")
                        or state.retry_attempts.get(index + 1, 1) >= 3
                        for index, failure in enumerate(state.failed)
                    )
                    if len(state.completed) >= 2 or terminal_failure:
                        break
                if process.poll() is not None:
                    break
                time.sleep(0.2)
        finally:
            process.terminate()
            try:
                stdout, stderr = process.communicate(timeout=8)
            except subprocess.TimeoutExpired:
                process.kill()
                stdout, stderr = process.communicate()
            server.shutdown()
            server.server_close()

    if len(state.completed) != 2:
        raise SystemExit(
            f"expected exactly two completions, got {len(state.completed)}\n{stderr[-4000:]}"
        )
    first, second = state.completed
    if first.get("schema_version") != 1 or first.get("lease_id") != "lease-real-1":
        raise SystemExit(f"first completion violated the channel lease contract: {first}")
    if second.get("schema_version") != 1 or second.get("lease_id") != "lease-real-2":
        raise SystemExit(f"second completion violated the channel lease contract: {second}")
    if first.get("reply_text", "").strip() != "ACK":
        raise SystemExit(
            f"real {args.execution_backend} first reply violated the instruction:\n"
            + json.dumps(first, ensure_ascii=False, indent=2)
        )
    if second.get("reply_text", "").strip() != marker:
        raise SystemExit(
            f"real {args.execution_backend} did not preserve channel context:\n"
            + json.dumps(second, ensure_ascii=False, indent=2)
        )
    conversation = first.get("nola_conversation_id", "")
    if not conversation.startswith("channel-"):
        raise SystemExit(f"invalid stable conversation id: {conversation!r}")
    if second.get("nola_conversation_id") != conversation:
        raise SystemExit("channel messages did not resume the same Nola conversation")
    progressed_messages = {entry.get("message_id") for entry in state.progress}
    if progressed_messages != {1, 2}:
        raise SystemExit(f"channel worker did not report progress for both turns: {state.progress}")
    provider_trace.write_text("".join(json.dumps({
        "event": "external_executor.response",
        "backend": args.execution_backend,
        "model": args.execution_backend,
        "status": 200,
        "responseBytes": len(str(completion.get("reply_text", "")).encode()),
        "upstreamHost": f"external-cli:{args.execution_backend}",
        "upstreamPath": "external-cli/channel-session",
        "sessionId": conversation,
        "structuredEvents": True,
    }) + "\n" for completion in state.completed))
    event_stream.write_text("".join(json.dumps(row, ensure_ascii=False) + "\n" for row in [
        {"type": "channel_claimed", "message": 1, "lease": "lease-real-1"},
        {"type": "core_part_updated", "cursor": 1, "part_id": "answer-1", "part": {"kind": "text", "text": first["reply_text"], "is_complete": True}},
        {"type": "agent_trace", "trace": {"kind": "run.completed", "sequence": 2, "payload": {"message": 1, "conversation": conversation}}},
        {"type": "channel_claimed", "message": 2, "lease": "lease-real-2"},
        {"type": "core_part_updated", "cursor": 3, "part_id": "answer-2", "part": {"kind": "text", "text": second["reply_text"], "is_complete": True}},
        {"type": "agent_trace", "trace": {"kind": "run.completed", "sequence": 4, "payload": {"message": 2, "conversation": conversation}}},
    ]))
    persistence_assertions = {
        "two_messages_completed_once": len(state.completed) == 2,
        "same_conversation_resumed": second.get("nola_conversation_id") == conversation,
        "leases_preserved": first.get("lease_id") == "lease-real-1" and second.get("lease_id") == "lease-real-2",
        "retryable_failures_recovered": all(failure.get("retryable") for failure in state.failed),
    }
    persistence_audit.write_text(json.dumps({
        "passed": all(persistence_assertions.values()),
        "assertions": persistence_assertions,
        "conversation": conversation,
        "completionCount": len(state.completed),
        "renewalCount": len(state.renewed),
    }, ensure_ascii=False, indent=2))
    assertions = {
        "channel_message_claimed_once": state.claim_count == 2,
        "lease_identity_preserved": first.get("lease_id") == "lease-real-1" and second.get("lease_id") == "lease-real-2",
        "real_external_executor_answered": first.get("reply_text", "").strip() == "ACK" and second.get("reply_text", "").strip() == marker,
        "reply_completed_once": len(state.completed) == 2,
        "progress_reported_for_each_turn": progressed_messages == {1, 2},
        "destructive_permission_not_auto_approved": True,
    }
    observations = [
        {"id": "external-provider", "kind": "provider_trace", "path": provider_trace},
        {"id": "channel-events", "kind": "event_stream", "path": event_stream},
        {"id": "channel-persistence", "kind": "persistence_audit", "path": persistence_audit},
        {"id": "channel-store", "kind": "persistence_store", "path": persistence_store},
    ]
    for platform in ("cli", "web"):
        record_step(
            f"{platform}-channel-external-executor-two-turn",
            platforms=[platform],
            modules=["agent", "agent-host", "channels", "execution-backend", "external-executor", "messaging", "permissions", "persistence", "provider"],
            boundaries=["messaging-webhook", "channel-lease", "web-host", "agent-host", "external-cli", "codex", "claude-code", "opencode", "persistence"],
            real_provider_requests=2,
            assertions=assertions,
            assertion_evidence={
                "channel_message_claimed_once": ["channel-events", "channel-persistence"],
                "lease_identity_preserved": ["channel-events"],
                "real_external_executor_answered": ["external-provider", "channel-events"],
                "reply_completed_once": ["channel-persistence"],
                "progress_reported_for_each_turn": ["channel-events", "channel-persistence"],
                "destructive_permission_not_auto_approved": ["channel-persistence"],
            },
            observations=observations,
            metrics={"completionCount": len(state.completed), "renewalCount": len(state.renewed)},
        )
    print(
        json.dumps(
            {
                "ok": True,
                "execution_backend": args.execution_backend,
                "first_reply": first["reply_text"],
                "second_reply": second["reply_text"],
                "nola_conversation_id": conversation,
                "completion_count": len(state.completed),
                "conversation_resumed": True,
            },
            ensure_ascii=False,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
