# SPDX-License-Identifier: Apache-2.0
"""Owned loopback fault injection, then transparent real-provider forwarding."""
from __future__ import annotations

import json
from http.server import ThreadingHTTPServer
from pathlib import Path
import threading
from urllib.parse import urlsplit

from .provider_probe import Handler, Probe


class GenerationFaultProbe(Probe):
    def __init__(self, upstream: str, log: Path, protocol: str, model: str,
                 status: int, body: bytes, *, armed: bool = True,
                 local_secret: str | None = None):
        super().__init__(upstream, log)
        suffix, self.input_field = {
            "chat_completions": ("/chat/completions", "messages"),
            "responses": ("/responses", "input"),
            "anthropic": ("/messages", "messages"),
        }[protocol]
        base = self.upstream.path.rstrip("/")
        self.generation_path = base if base.endswith(suffix) else base + suffix
        self.model = model
        self.injection_status = status
        self.injection_body = body
        self.armed = armed
        self.local_secret = local_secret
        self.injected = 0
        self.blocked_secret_requests = 0

    def is_generation(self, path: str, request: dict) -> bool:
        if not isinstance(request, dict):
            return False
        value = request.get(self.input_field)
        has_input = ((isinstance(value, list) and bool(value))
            or (self.input_field == "input" and isinstance(value, str) and bool(value.strip())))
        return (urlsplit(path).path == self.generation_path
            and request.get("model") == self.model and has_input)

    def response_override(self, path: str, request: dict, headers) -> tuple[int, bytes] | None:
        # A deliberate debug credential must never leave this loopback server,
        # including discovery requests and any unexpected client retry.
        secret_present = self.local_secret is not None and (
            any(self.local_secret in value for value in headers.values())
            or self.local_secret in json.dumps(request, ensure_ascii=False))
        with self.lock:
            first = self.armed and self.is_generation(path, request) and self.injected == 0
            if first:
                self.injected += 1
            if secret_present:
                self.blocked_secret_requests += 1
            if secret_present:
                return 401, b'{"error":{"type":"authentication_error","code":"invalid_api_key","message":"Injected authentication failure"}}'
            if first:
                return self.injection_status, self.injection_body
        return None


class GenerationFaultRelay:
    def __init__(self, probe: GenerationFaultProbe):
        self.probe = probe
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.probe = self.probe
        self.thread = threading.Thread(target=self.server.serve_forever,
            kwargs={"poll_interval": 0.1}, name="owned-generation-fault-relay")
        self.endpoint = f"http://127.0.0.1:{self.server.server_port}{self.probe.upstream.path}"

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, _kind, primary, _trace):
        try:
            self.server.shutdown()
            self.server.server_close()
            self.thread.join(timeout=3)
            if self.thread.is_alive():
                raise RuntimeError("owned fault relay did not terminate")
        except Exception as cleanup:
            failure = RuntimeError("owned fault relay cleanup failed")
            failure.primary_failure = primary
            failure.cleanup_failures = [cleanup]
            raise failure from primary

    def assert_injected(self):
        events = [json.loads(line) for line in self.probe.log_path.read_text().splitlines()]
        injected = [event for event in events if event.get("injectedHttpError")
            and event.get("requestInputPresent") and event.get("model") == self.probe.model
            and urlsplit(event.get("upstreamPath", "")).path == self.probe.generation_path]
        assert self.probe.injected == 1 and len(injected) == 1, "expected exactly one injected generation failure"
        assert injected[0]["status"] == self.probe.injection_status and injected[0]["forwarded"] is False

    def assert_recovered(self):
        self.assert_injected()
        events = [json.loads(line) for line in self.probe.log_path.read_text().splitlines()]
        failed_index = next(index for index, event in enumerate(events)
            if event.get("injectedHttpError") and event.get("requestInputPresent")
            and event.get("model") == self.probe.model
            and urlsplit(event.get("upstreamPath", "")).path == self.probe.generation_path)
        assert any(event.get("forwarded") is True and event.get("status") == 200
            and event.get("requestInputPresent")
            and event.get("model") == self.probe.model
            and urlsplit(event.get("upstreamPath", "")).path == self.probe.generation_path
            and event.get("requestId") for event in events[failed_index + 1:]), "real generation was not forwarded successfully"


class AuthenticationFaultRelay(GenerationFaultRelay):
    def __init__(self, upstream: str, log: Path, protocol: str, model: str,
                 local_secret: str | None = None):
        super().__init__(GenerationFaultProbe(upstream, log, protocol, model, 401,
            b'{"error":{"type":"authentication_error","code":"invalid_api_key","message":"Injected authentication failure"}}',
            local_secret=local_secret))


class ContextLimitFaultRelay(GenerationFaultRelay):
    def __init__(self, upstream: str, log: Path, protocol: str, model: str):
        # Establish real completed history on this same immutable endpoint
        # before arming the one failure for the next generation request.
        super().__init__(GenerationFaultProbe(upstream, log, protocol, model, 400,
            b'{"error":{"type":"invalid_request_error","code":"context_length_exceeded","message":"Injected context limit failure; not a natural provider overflow"}}',
            armed=False))

    def arm(self):
        with self.probe.lock:
            assert not self.probe.armed and self.probe.injected == 0, "context fault already armed"
            self.probe.armed = True
