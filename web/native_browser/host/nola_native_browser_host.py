#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# File Overview:
# - Purpose: Implements nola native browser host logic in the web client.
# - Interfaces:
#   - NativeBrowserHost
#   - main
# - APIs:
#   - none
#
"""Chrome Native Messaging host for the Nola Web Agent browser bridge."""

from __future__ import annotations

import json
import os
import struct
import sys
import threading
import time
from collections import deque
import urllib.error
import urllib.request
import uuid
from pathlib import Path
from typing import Any

DEFAULT_CONFIG_PATH = Path.home() / ".nola" / "native_browser_host.json"
DEFAULT_LOG_PATH = Path.home() / ".nola" / "native_browser_host.log"
_RESULT_QUEUE_MAX = 256
_RESULT_SEND_RETRIES = 3
_RESULT_RETRY_BASE_SEC = 0.6
_POLL_BACKOFF_MAX_SEC = 10.0


def _log(message: str) -> None:
    line = f"[nola-native-browser] {message}"
    print(line, file=sys.stderr, flush=True)
    try:
        DEFAULT_LOG_PATH.parent.mkdir(parents=True, exist_ok=True)
        with DEFAULT_LOG_PATH.open("a", encoding="utf-8") as handle:
            handle.write(line + "\n")
    except OSError:
        pass


def _load_config() -> dict[str, Any]:
    path = Path(os.getenv("NOLA_NATIVE_BROWSER_CONFIG") or DEFAULT_CONFIG_PATH)
    data: dict[str, Any] = {}
    if path.exists():
        data = json.loads(path.read_text(encoding="utf-8"))
    server_url = os.getenv("NOLA_NATIVE_BROWSER_SERVER_URL") or data.get("server_url") or "http://127.0.0.1:5001"
    token = os.getenv("NOLA_NATIVE_BROWSER_TOKEN") or data.get("token") or ""
    host_id = os.getenv("NOLA_NATIVE_BROWSER_HOST_ID") or data.get("host_id") or "chrome-default"
    return {
        "server_url": str(server_url).rstrip("/"),
        "token": str(token),
        "host_id": str(host_id),
    }


def _read_message() -> dict[str, Any] | None:
    raw_length = sys.stdin.buffer.read(4)
    if not raw_length:
        return None
    length = struct.unpack("<I", raw_length)[0]
    if length <= 0 or length > 16 * 1024 * 1024:
        raise RuntimeError(f"invalid native message length: {length}")
    payload = sys.stdin.buffer.read(length)
    if len(payload) != length:
        return None
    return json.loads(payload.decode("utf-8"))


_WRITE_LOCK = threading.Lock()


def _write_message(message: dict[str, Any]) -> None:
    encoded = json.dumps(message, ensure_ascii=False).encode("utf-8")
    with _WRITE_LOCK:
        sys.stdout.buffer.write(struct.pack("<I", len(encoded)))
        sys.stdout.buffer.write(encoded)
        sys.stdout.buffer.flush()


def _post_json(config: dict[str, Any], path: str, payload: dict[str, Any], *, timeout: float) -> dict[str, Any]:
    body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
    request = urllib.request.Request(
        f"{config['server_url']}{path}",
        data=body,
        headers={
            "Content-Type": "application/json",
            "X-Nola-Native-Browser-Token": config["token"],
        },
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:  # noqa: S310 - local configured URL
        return json.loads(response.read().decode("utf-8"))


class NativeBrowserHost:
    def __init__(self, config: dict[str, Any]) -> None:
        self.config = config
        self.instance_id = uuid.uuid4().hex
        self.manifest_id = f"native-browser:{config['host_id']}:{self.instance_id}"
        self.capabilities: list[str] = []
        self.running = True
        self.pending_results: deque[dict[str, Any]] = deque()
        self.results_lock = threading.Lock()

    def _queue_result(self, command_id: str, result: dict[str, Any]) -> None:
        payload = {
            "host_id": self.config["host_id"],
            "instance_id": self.instance_id,
            "manifest_id": self.manifest_id,
            "command_id": command_id,
            "result": result or {},
        }
        with self.results_lock:
            self.pending_results.append(payload)
            while len(self.pending_results) > _RESULT_QUEUE_MAX:
                dropped = self.pending_results.popleft()
                _log(f"dropped stale result due to queue overflow command_id={dropped.get('command_id')}")

    def _post_result_payload(self, payload: dict[str, Any]) -> bool:
        for attempt in range(1, _RESULT_SEND_RETRIES + 1):
            try:
                _post_json(
                    self.config,
                    "/api/native-browser/host/result",
                    payload,
                    timeout=10,
                )
                return True
            except (urllib.error.URLError, TimeoutError, OSError) as exc:
                if attempt >= _RESULT_SEND_RETRIES:
                    _log(f"result submit failed command_id={payload.get('command_id')} error={exc}")
                    return False
                delay = min(_RESULT_RETRY_BASE_SEC * (2 ** (attempt - 1)), 4.0)
                time.sleep(delay)
            except Exception as exc:  # noqa: BLE001
                _log(f"result submit unexpected error command_id={payload.get('command_id')} error={exc}")
                return False
        return False

    def _flush_pending_results(self, *, max_items: int = 16) -> None:
        sent = 0
        while sent < max_items:
            with self.results_lock:
                payload = self.pending_results[0] if self.pending_results else None
            if payload is None:
                return
            if not self._post_result_payload(payload):
                return
            with self.results_lock:
                if self.pending_results and self.pending_results[0] is payload:
                    self.pending_results.popleft()
            sent += 1

    def poll_loop(self) -> None:
        backoff_sec = 1.0
        while self.running:
            try:
                self._flush_pending_results(max_items=32)
                if not self.capabilities:
                    time.sleep(0.1)
                    continue
                now_ms = int(time.time() * 1000)
                response = _post_json(
                    self.config,
                    "/api/native-browser/host/poll",
                    {
                        "host_id": self.config["host_id"],
                        "instance_id": self.instance_id,
                        "manifest_id": self.manifest_id,
                        "manifest_version": 1,
                        "issued_at_ms": now_ms,
                        "expires_at_ms": now_ms + 90_000,
                        "capabilities": self.capabilities,
                        "wait_sec": 25,
                    },
                    timeout=35,
                )
                if response.get("type") == "command" and response.get("command"):
                    _write_message({"type": "command", "command": response["command"]})
                else:
                    time.sleep(float(response.get("retry_ms") or 500) / 1000.0)
                backoff_sec = 1.0
            except (urllib.error.URLError, TimeoutError, OSError) as exc:
                _log(f"poll failed: {exc}")
                time.sleep(backoff_sec)
                backoff_sec = min(backoff_sec * 1.8, _POLL_BACKOFF_MAX_SEC)
            except Exception as exc:  # noqa: BLE001
                _log(f"poll loop error: {exc}")
                time.sleep(backoff_sec)
                backoff_sec = min(backoff_sec * 1.8, _POLL_BACKOFF_MAX_SEC)

    def run(self) -> None:
        thread = threading.Thread(target=self.poll_loop, daemon=True)
        thread.start()
        try:
            while self.running:
                message = _read_message()
                if message is None:
                    break
                msg_type = message.get("type")
                if msg_type == "hello":
                    self.capabilities = [
                        str(item) for item in (message.get("capabilities") or []) if str(item or "").strip()
                    ]
                    _log(f"extension connected capabilities={self.capabilities}")
                elif msg_type == "result":
                    command_id = str(message.get("command_id") or "").strip()
                    if command_id:
                        self._queue_result(command_id, message.get("result") or {})
        finally:
            self.running = False
            self._flush_pending_results(max_items=128)


def main() -> int:
    try:
        config = _load_config()
        _log(f"starting host_id={config['host_id']} server_url={config['server_url']}")
        if not config["token"]:
            _log("missing token; set NOLA_NATIVE_BROWSER_TOKEN or ~/.nola/native_browser_host.json")
            return 2
        NativeBrowserHost(config).run()
        _log("host stopped")
        return 0
    except Exception as exc:  # noqa: BLE001
        _log(f"fatal error: {type(exc).__name__}: {exc}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
