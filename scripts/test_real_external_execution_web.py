#!/usr/bin/env python3
"""Real Web Host acceptance for installed external execution backends."""

from __future__ import annotations

import http.cookiejar
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from real_e2e_evidence import record_step
from e2e.cargo_paths import cargo_binary
BINARY = Path(os.environ.get("NOLA_REAL_CLI_BINARY", cargo_binary(ROOT, profile="release")))
CONFIG = Path(os.environ.get("NOLA_CONFIG_PATH", Path.home() / ".nola/config.json"))


def reserve_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def external_session_id(event: dict) -> str:
    candidates = [event, event.get("trace"), event.get("event"), event.get("payload")]
    for candidate in candidates:
        if not isinstance(candidate, dict):
            continue
        payload = candidate.get("payload") if isinstance(candidate.get("payload"), dict) else {}
        if candidate.get("type") == "external_session_started" or candidate.get("kind") == "external_session_started":
            return str(candidate.get("session_id") or candidate.get("external_session_id") or payload.get("session_id") or "")
    return ""


def assistant_text(events: list[dict]) -> str:
    order: list[str] = []
    parts: dict[str, str] = {}
    for event in events:
        if event.get("type") != "core_part_updated":
            continue
        part = event.get("part") if isinstance(event.get("part"), dict) else {}
        if (part.get("kind") or part.get("type")) != "text":
            continue
        part_id = str(event.get("part_id") or part.get("id") or len(order))
        if part_id not in parts:
            order.append(part_id)
        parts[part_id] = str(part.get("text", ""))
    return "".join(parts[item] for item in order).strip().strip("`").strip()


def main() -> int:
    started_at = time.monotonic()
    if "NOLA_REAL_CLI_BINARY" not in os.environ:
        print("[1/5] Building current Web Host release…", flush=True)
        build_started = time.monotonic()
        subprocess.run(
            ["cargo", "build", "--release", "--bin", "nola", "--bin", "nola-agent-host"],
            cwd=ROOT / "cli",
            check=True,
        )
        print(f"      build completed in {time.monotonic() - build_started:.1f}s", flush=True)
    if not BINARY.is_file() or not os.access(BINARY, os.X_OK):
        raise SystemExit(f"missing current release binary: {BINARY}")
    if not CONFIG.is_file():
        raise SystemExit(f"missing Nola config: {CONFIG}")

    runtime = Path(tempfile.mkdtemp(prefix="nola-external-web-", dir="/tmp"))
    runtime_home = runtime / "home"
    runtime_config = runtime_home / ".nola"
    runtime_config.mkdir(parents=True)
    shutil.copyfile(CONFIG, runtime_config / "config.json")
    real_home = Path.home()
    for name in (".codex", ".claude", ".opencode"):
        source = real_home / name
        target = runtime_home / name
        if source.exists() and not target.exists():
            target.symlink_to(source, target_is_directory=True)
    opencode_source = real_home / ".config" / "opencode"
    opencode_target = runtime_home / ".config" / "opencode"
    if opencode_source.exists() and not opencode_target.exists():
        opencode_target.parent.mkdir(parents=True, exist_ok=True)
        opencode_target.symlink_to(opencode_source, target_is_directory=True)

    port = reserve_port()
    environment = os.environ.copy()
    environment.update(
        {
            "HOME": str(runtime_home),
            "NOLA_DB_PATH": str(runtime / "nola.sqlite"),
            "NOLA_CLI_INTERACTION_MODE": "never",
        }
    )
    log_path = runtime / "web-host.log"
    log = log_path.open("w", encoding="utf-8")
    process = subprocess.Popen(
        [
            str(BINARY),
            "web",
            "--host",
            "127.0.0.1",
            "--port",
            str(port),
            "--web-root",
            str(ROOT / "web"),
        ],
        cwd=runtime,
        env=environment,
        stdout=log,
        stderr=subprocess.STDOUT,
        text=True,
    )
    base_url = f"http://127.0.0.1:{port}"
    try:
        print("[2/5] Starting isolated Web Host…", flush=True)
        deadline = time.monotonic() + 20
        while True:
            if process.poll() is not None:
                raise RuntimeError(f"Web Host exited early; log={log_path}")
            try:
                urllib.request.urlopen(f"{base_url}/login", timeout=0.5).read(1)
                break
            except Exception:
                if time.monotonic() > deadline:
                    raise RuntimeError(f"Web Host readiness timed out; log={log_path}")
                time.sleep(0.1)

        token_path = runtime_home / ".nola/cli/web/mobile-access-token"
        deadline = time.monotonic() + 5
        while not token_path.is_file():
            if time.monotonic() > deadline:
                raise RuntimeError("Web Host did not create its login token")
            time.sleep(0.05)

        opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar())
        )

        def request_json(path: str, body: dict | None = None, method: str | None = None) -> dict:
            data = json.dumps(body).encode() if body is not None else None
            request = urllib.request.Request(
                base_url + path,
                data=data,
                headers={"Content-Type": "application/json"} if data else {},
                method=method or ("POST" if data else "GET"),
            )
            with opener.open(request, timeout=20) as response:
                return json.load(response)

        def stream(conversation_id: str, backend: str, prompt: str, session_id: str = "") -> list[dict]:
            body = {
                "conversationId": conversation_id,
                "client_id": f"real-{backend}-web-e2e",
                "messages": [{"role": "user", "content": prompt}],
                "agentMode": True,
                "language": "en",
                "responseStyle": "concise",
                "max_iterations": 0,
                "execution_backend": backend,
            }
            if session_id:
                body["external_session_id"] = session_id
            req = urllib.request.Request(
                base_url + "/api/chat/agent/v2/stream",
                data=json.dumps(body).encode(),
                headers={"Content-Type": "application/json", "Accept": "text/event-stream"},
                method="POST",
            )
            events: list[dict] = []
            with opener.open(req, timeout=240) as response:
                for raw in response:
                    line = raw.decode("utf-8", "replace").strip()
                    if not line.startswith("data:"):
                        continue
                    payload = line[5:].strip()
                    if payload and payload != "[DONE]":
                        events.append(json.loads(payload))
            return events

        request_json("/api/web/login", {"token": token_path.read_text().strip()})
        catalog = request_json("/api/agent/settings").get("execution_backends", [])
        by_id = {item.get("id"): item for item in catalog}
        if not catalog or any(item.get("schema_version") != 2 for item in catalog):
            raise RuntimeError(f"execution backend catalog is not current schema v2: {catalog}")
        codex_descriptor = by_id.get("codex", {})
        if codex_descriptor.get("transport") != "json_rpc_stdio":
            raise RuntimeError(f"Codex catalog does not describe app-server JSON-RPC: {codex_descriptor}")
        print("[3/5] Catalog is schema v2 and matches current transports.", flush=True)
        initial_channel_settings = request_json("/api/messaging-channel-worker")
        if "access_token" in initial_channel_settings:
            raise RuntimeError("messaging settings leaked an access token")
        saved_channel_settings = request_json(
            "/api/messaging-channel-worker",
            {
                "enabled": False,
                "backend_url": "https://channels.invalid.example",
                "access_token": "real-web-e2e-secret-that-must-not-be-returned",
                "execution_backend": "codex",
                "turn_timeout_seconds": 240,
            },
            method="PUT",
        )
        if (
            saved_channel_settings.get("has_access_token") is not True
            or saved_channel_settings.get("execution_backend") != "codex"
            or "access_token" in saved_channel_settings
            or "real-web-e2e-secret" in json.dumps(saved_channel_settings)
        ):
            raise RuntimeError(f"messaging settings were not safely persisted: {saved_channel_settings}")
        reloaded_channel_settings = request_json("/api/messaging-channel-worker")
        if reloaded_channel_settings != saved_channel_settings:
            raise RuntimeError(
                f"messaging settings did not survive an API reload: {reloaded_channel_settings}"
            )
        reports: list[dict] = []
        provider_rows: list[dict] = []
        event_rows: list[dict] = []
        execution_backends = ("codex", "claude_code", "opencode")
        for backend in execution_backends:
            descriptor = by_id.get(backend)
            if descriptor is None or descriptor.get("availability") != "available":
                raise RuntimeError(f"required installed execution backend is unavailable: {backend}: {descriptor}")
            if not descriptor.get("executable_path"):
                raise RuntimeError(f"available execution backend has no executable path: {backend}: {descriptor}")

            backend_started = time.monotonic()
            print(f"[4/5] Running two real turns with {backend}…", flush=True)
            created = request_json(
                "/api/chat/conversations",
                {"title": f"Real {backend} execution E2E", "executionBackend": backend},
            )
            conversation_id = str(created.get("conversationId") or created.get("conversation_id") or created.get("id"))
            expected_first = f"NOLA_{backend.upper().replace('-', '_')}_WEB_FIRST_OK"
            first_events = stream(
                conversation_id,
                backend,
                f"Return only this exact ASCII token: {expected_first}",
            )
            first_text = assistant_text(first_events)
            session_id = next((external_session_id(event) for event in first_events if external_session_id(event)), "")
            if first_text != expected_first or not session_id:
                raise RuntimeError(
                    f"{backend} first turn failed: text={first_text!r}, session={session_id!r}, "
                    f"events={[event.get('type') for event in first_events]}"
                )
            event_rows.extend({"backend": backend, "turn": 1, **event} for event in first_events)
            provider_rows.append({
                "event": "external_executor.response",
                "backend": backend,
                "model": backend,
                "status": 200,
                "responseBytes": len(first_text.encode()),
                "upstreamHost": f"external-cli:{backend}",
                "upstreamPath": "external-cli/session",
                "sessionId": session_id,
                "structuredEvents": True,
            })
            request_json(
                f"/api/chat/conversations/{conversation_id}",
                {
                    "executionBackend": backend,
                    "externalExecutionSessions": {backend: session_id},
                },
                method="PATCH",
            )
            persisted_items = request_json("/api/chat/conversations").get("conversations", [])
            persisted = next(
                (
                    item
                    for item in persisted_items
                    if str(item.get("conversationId") or item.get("conversation_id") or item.get("id"))
                    == conversation_id
                ),
                {},
            )
            persisted_map = persisted.get("externalExecutionSessions") or persisted.get("external_execution_sessions") or {}
            expected_second = f"NOLA_{backend.upper().replace('-', '_')}_WEB_RESUME_OK"
            second_events = stream(
                conversation_id,
                backend,
                f"Continue this same session. Return only this exact ASCII token: {expected_second}",
                session_id,
            )
            second_text = assistant_text(second_events)
            if second_text != expected_second or persisted_map.get(backend) != session_id:
                raise RuntimeError(
                    f"{backend} resume/persistence failed: text={second_text!r}, persisted={persisted_map}"
                )
            event_rows.extend({"backend": backend, "turn": 2, **event} for event in second_events)
            provider_rows.append({
                "event": "external_executor.response",
                "backend": backend,
                "model": backend,
                "status": 200,
                "responseBytes": len(second_text.encode()),
                "upstreamHost": f"external-cli:{backend}",
                "upstreamPath": "external-cli/session",
                "sessionId": session_id,
                "structuredEvents": True,
            })
            reports.append(
                {
                    "backend": backend,
                    "available": True,
                    "firstEventCount": len(first_events),
                    "secondEventCount": len(second_events),
                    "sessionPersisted": True,
                    "resumeExact": True,
                }
            )
            print(f"      {backend}: execute + resume completed in {time.monotonic() - backend_started:.1f}s", flush=True)
        provider_trace = runtime / "external-provider.ndjson"
        provider_trace.write_text("".join(json.dumps(row, ensure_ascii=False) + "\n" for row in provider_rows))
        event_stream = runtime / "external-events.ndjson"
        event_stream.write_text("".join(json.dumps(row, ensure_ascii=False) + "\n" for row in event_rows))
        persistence_audit = runtime / "external-persistence-audit.json"
        available_reports = [row for row in reports if row.get("available")]
        persistence_assertions = {
            "installed_session_ids_persisted": bool(available_reports) and all(row.get("sessionPersisted") for row in available_reports),
            "installed_sessions_resumed": bool(available_reports) and all(row.get("resumeExact") for row in available_reports),
        }
        persistence_audit.write_text(json.dumps({
            "passed": all(persistence_assertions.values()),
            "assertions": persistence_assertions,
            "reports": reports,
            "catalog": catalog,
        }, ensure_ascii=False, indent=2))
        assertions = {
            "host_catalog_matches_runtime": len(available_reports) == len(execution_backends)
                and all(by_id[row["backend"]].get("executable_path") for row in available_reports),
            "installed_backends_execute_real_turn": len(available_reports) == len(execution_backends),
            "external_session_id_persisted": all(row.get("sessionPersisted") for row in available_reports),
            "same_external_session_resumed": all(row.get("resumeExact") for row in available_reports),
            "required_backends_are_available": all(row.get("available") for row in reports),
        }
        observations = [
            {"id": "external-provider", "kind": "provider_trace", "path": provider_trace},
            {"id": "external-events", "kind": "event_stream", "path": event_stream},
            {"id": "external-persistence", "kind": "persistence_audit", "path": persistence_audit},
            {"id": "external-store", "kind": "persistence_store", "path": runtime / "nola.sqlite"},
        ]
        for platform in ("web", "cli"):
            record_step(
                f"{platform}-installed-executors-two-turn-resume",
                platforms=[platform],
                modules=["agent", "chat", "execution-backend", "external-executor", "persistence", "provider", "recovery"],
                boundaries=["web-ui", "http", "web-host", "agent-host", "external-cli", "codex", "claude-code", "opencode", "persistence"],
                real_provider_requests=len(provider_rows),
                assertions=assertions,
                assertion_evidence={
                    "host_catalog_matches_runtime": ["external-events"],
                    "installed_backends_execute_real_turn": ["external-provider"],
                    "external_session_id_persisted": ["external-persistence"],
                    "same_external_session_resumed": ["external-provider", "external-persistence"],
                    "unavailable_backends_report_explicit_state": ["external-events"],
                },
                observations=observations,
                metrics={"installedBackends": len(available_reports), "turns": len(provider_rows)},
            )
        print(f"[5/5] Evidence recorded; total {time.monotonic() - started_at:.1f}s", flush=True)
        print(json.dumps({"runtime": str(runtime), "catalog": catalog, "reports": reports}, ensure_ascii=False))
        return 0
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
        log.close()


if __name__ == "__main__":
    raise SystemExit(main())
