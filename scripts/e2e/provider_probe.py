#!/usr/bin/env python3
"""Transparent OpenAI-compatible provider probe with sanitized JSONL evidence."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import threading
import time
from urllib.parse import urlsplit
import uuid


def last_user_text(request: dict) -> str | None:
    """Opt-in observation of text input for current external provider protocols."""
    messages = request.get("input", request.get("messages", []))
    if isinstance(messages, str):
        return messages[:8192]
    if not isinstance(messages, list):
        return None
    for message in reversed(messages):
        if not isinstance(message, dict) or message.get("role") != "user":
            continue
        content = message.get("content")
        if isinstance(content, str):
            return content[:8192]
        if isinstance(content, list):
            text = "\n".join(block["text"] for block in content
                if isinstance(block, dict) and block.get("type") in {"text", "input_text"}
                and isinstance(block.get("text"), str))
            return text[:8192] if text else None
        return None
    return None


def usage_counters(usage: object) -> dict | None:
    """Preserve reported counters only; absent usage is not a cache miss."""
    if not isinstance(usage, dict):
        return None
    counters = {
        key: usage[key]
        for key in ("input_tokens", "output_tokens", "total_tokens", "prompt_tokens",
                    "completion_tokens", "prompt_cache_hit_tokens", "prompt_cache_miss_tokens")
        if type(usage.get(key)) is int and usage[key] >= 0
    }
    for key in ("input_tokens_details", "prompt_tokens_details"):
        details = usage.get(key)
        if isinstance(details, dict) and type(details.get("cached_tokens")) is int and details["cached_tokens"] >= 0:
            counters[key] = {"cached_tokens": details["cached_tokens"]}
    return counters or None


def tool_schema_metrics(request: dict) -> dict:
    """Numeric schema costs only; never retain descriptions or parameter data.

    Token estimate uses Core's ASCII/4 + non-ASCII-character heuristic, not
    a provider tokenizer. Provider wire framing is not included.
    """
    tools = request.get("tools", [])
    if not isinstance(tools, list):
        return {"toolSchemaUtf8Bytes": None, "toolSchemaEstimatedTokens": None,
                "toolSchemaTokenMethod": "ascii4_nonascii1_json"}
    texts = [json.dumps(tool, ensure_ascii=False, separators=(",", ":")) for tool in tools]
    tokens = 0
    for text in texts:
        ascii_count = sum(ord(char) < 128 for char in text)
        tokens += max(1, (ascii_count + 3) // 4 + len(text) - ascii_count)
    return {"toolSchemaUtf8Bytes": sum(len(text.encode("utf-8")) for text in texts),
            "toolSchemaEstimatedTokens": tokens, "toolSchemaTokenMethod": "ascii4_nonascii1_json",
            "toolSchemaSha256": hashlib.sha256(json.dumps(tools, sort_keys=True, separators=(",", ":")).encode()).hexdigest()}


def output_limit(request: dict) -> int | None:
    for key in ("max_output_tokens", "max_completion_tokens", "max_tokens"):
        if type(request.get(key)) is int:
            return request[key]
    return None


def inspect_ui_action_revisions(request: dict) -> dict:
    """Observe admitted UI revision metadata, never form values or prompts."""
    revisions = []
    versions = []
    for field in ("messages", "input"):
        items = request.get(field, [])
        if isinstance(items, str):
            items = [{"content": items}]
        if not isinstance(items, list):
            continue
        for item in items:
            if not isinstance(item, dict):
                continue
            content = item.get("content", [])
            if isinstance(content, str):
                content = [{"text": content}]
            if not isinstance(content, list):
                continue
            for part in content:
                if not isinstance(part, dict) or not isinstance(part.get("text"), str):
                    continue
                try:
                    value = json.loads(part["text"])
                except (ValueError, TypeError):
                    continue
                if not isinstance(value, dict) or value.get("type") != "ui_action":
                    continue
                revision = value.get("surface_revision")
                revisions.append(revision if type(revision) is int and revision > 0 else None)
                message = value.get("message", {})
                versions.append(isinstance(message, dict) and message.get("version") == "v0.9.1")
    return {"modelUiActionSurfaceRevisions": revisions, "modelUiActionProtocolValid": versions}


def inspect_expected_instructions(request: dict, expectation_path: Path | None) -> dict:
    """Observe exact prompt bytes; retain only expected digests and booleans."""
    if expectation_path is None:
        return {}
    try:
        if expectation_path.stat().st_size > 1_048_576:
            raise ValueError("expectation bound")
        expected = json.loads(expectation_path.read_text(encoding="utf-8"))
        keys = ("templateContent", "styleInstruction", "customInstruction")
        if not isinstance(expected, dict) or any(not isinstance(expected.get(key), str) or not expected[key] for key in keys):
            raise ValueError("expectation shape")
    except (OSError, ValueError):
        return {"requestInstructionObservation": "expected_instructions_unavailable", "requestInstructionMatches": {}}

    texts: list[bytes] = []
    def content(value: object) -> None:
        if isinstance(value, str):
            texts.append(value.encode("utf-8"))
        elif isinstance(value, list):
            for item in value:
                if isinstance(item, dict) and item.get("type") in {"text", "input_text", "output_text"} and isinstance(item.get("text"), str):
                    texts.append(item["text"].encode("utf-8"))
    for field in ("messages", "input"):
        value = request.get(field)
        if isinstance(value, str):
            content(value)
        elif isinstance(value, list):
            for item in value:
                if isinstance(item, dict):
                    content(item.get("content"))
    content(request.get("system"))
    content(request.get("instructions"))
    matches = {}
    for key in keys:
        exact = expected[key].encode("utf-8")
        matches[key] = {"sha256": hashlib.sha256(exact).hexdigest(), "present": any(exact in text for text in texts)}
    return {"requestInstructionObservation": "observed", "requestInstructionMatches": matches}


def inspect_response(body: bytes) -> dict:
    """Extract protocol evidence without persisting model or user text."""
    events = 0
    text_bytes = 0
    reasoning_bytes = 0
    text_deltas = 0
    reasoning_deltas = 0
    tool_names: list[str] = []
    tool_argument_bytes = 0
    finish_reasons: list[str] = []
    response_models: list[str] = []
    response_usage = None
    for raw_line in body.splitlines():
        line = raw_line.strip()
        if line.startswith(b"data:"):
            line = line[5:].strip()
        if not line or line == b"[DONE]":
            continue
        try:
            value = json.loads(line)
        except (json.JSONDecodeError, UnicodeDecodeError):
            continue
        events += 1
        if isinstance(value, dict):
            if isinstance(value.get("model"), str) and value["model"] not in response_models:
                response_models.append(value["model"])
            nested_response = value.get("response")
            if isinstance(nested_response, dict) and isinstance(nested_response.get("model"), str) and nested_response["model"] not in response_models:
                response_models.append(nested_response["model"])
            reported_usage = usage_counters(value.get("usage"))
            if reported_usage is not None:
                response_usage = reported_usage
            kind = value.get("type")
            delta = value.get("delta")
            if kind == "response.output_text.delta" and isinstance(delta, str):
                text_bytes += len(delta.encode("utf-8"))
                text_deltas += bool(delta)
            elif kind in ("response.reasoning_text.delta", "response.reasoning_summary_text.delta") and isinstance(delta, str):
                reasoning_bytes += len(delta.encode("utf-8"))
                reasoning_deltas += bool(delta)
            elif kind == "response.function_call_arguments.delta" and isinstance(delta, str):
                tool_argument_bytes += len(delta.encode("utf-8"))
            if kind in ("response.completed", "response.incomplete"):
                response = value.get("response", {})
                if isinstance(response, dict):
                    reason = "completed" if kind == "response.completed" else (response.get("incomplete_details") or {}).get("reason", "incomplete")
                    if reason not in finish_reasons:
                        finish_reasons.append(reason)
                    reported_usage = usage_counters(response.get("usage"))
                    if reported_usage is not None:
                        response_usage = reported_usage
                    final_text = final_reasoning = final_arguments = 0
                    for item in response.get("output") or []:
                        if not isinstance(item, dict):
                            continue
                        if item.get("type") == "function_call":
                            name = item.get("name")
                            if isinstance(name, str) and name not in tool_names:
                                tool_names.append(name)
                            args = item.get("arguments", "")
                            if isinstance(args, str):
                                final_arguments += len(args.encode("utf-8"))
                        content = item.get("content") or []
                        summary = item.get("summary") or []
                        blocks = (content if isinstance(content, list) else []) + (summary if isinstance(summary, list) else [])
                        for block in blocks:
                            if not isinstance(block, dict) or not isinstance(block.get("text"), str):
                                continue
                            size = len(block["text"].encode("utf-8"))
                            if item.get("type") == "reasoning":
                                final_reasoning += size
                            elif item.get("type") == "message":
                                final_text += size
                    # Terminal output repeats the deltas; count it only as a fallback.
                    if not text_deltas:
                        text_bytes = final_text
                    if not reasoning_deltas:
                        reasoning_bytes = final_reasoning
                    if not tool_argument_bytes:
                        tool_argument_bytes = final_arguments
        choices = value.get("choices", []) if isinstance(value, dict) else []
        for choice in choices if isinstance(choices, list) else []:
            if not isinstance(choice, dict):
                continue
            finish = choice.get("finish_reason")
            if finish and str(finish) not in finish_reasons:
                finish_reasons.append(str(finish))
            is_delta = isinstance(choice.get("delta"), dict)
            message = choice.get("delta") if is_delta else choice.get("message") or {}
            if not isinstance(message, dict):
                continue
            content = message.get("content")
            reasoning = message.get("reasoning_content") or message.get("reasoning")
            if isinstance(content, str):
                text_bytes += len(content.encode("utf-8"))
                if content and is_delta:
                    text_deltas += 1
            if isinstance(reasoning, str):
                reasoning_bytes += len(reasoning.encode("utf-8"))
                if reasoning and is_delta:
                    reasoning_deltas += 1
            calls = message.get("tool_calls", [])
            for call in calls if isinstance(calls, list) else []:
                function = call.get("function", {}) if isinstance(call, dict) else {}
                if not isinstance(function, dict):
                    continue
                name = function.get("name")
                if isinstance(name, str) and name and name not in tool_names:
                    tool_names.append(name)
                arguments = function.get("arguments")
                if isinstance(arguments, str):
                    tool_argument_bytes += len(arguments.encode("utf-8"))
    return {
        "responseEventCount": events,
        "responseTextBytes": text_bytes,
        "responseTextDeltaCount": text_deltas,
        "responseReasoningBytes": reasoning_bytes,
        "responseReasoningDeltaCount": reasoning_deltas,
        "responseToolNames": tool_names,
        "responseToolArgumentBytes": tool_argument_bytes,
        "finishReasons": finish_reasons,
        "responseUsage": response_usage,
        "responseModels": response_models,
    }


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--upstream", required=True)
    parser.add_argument("--log", required=True, type=Path)
    parser.add_argument("--ready", required=True, type=Path)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--interrupt-once-after-text-bytes", type=int, default=0,
                        help="Opt-in isolated recovery test: end one SSE response after this much public text.")
    parser.add_argument("--truncate-compaction-once-tokens", type=int, default=0,
                        help="Isolated acceptance only: reduce one Responses checkpoint's output allowance at the real upstream.")
    parser.add_argument(
        "--upstream-timeout-seconds",
        type=float,
        default=45.0,
        help="Bound one upstream request below the caller's scenario timeout.",
    )
    parser.add_argument(
        "--capture-last-user-message",
        action="store_true",
        help="Record the bounded final user message for isolated E2E evidence.",
    )
    parser.add_argument("--expected-request-instructions", type=Path,
                        help="Observe synthetic isolated-run instruction bytes without retaining prompt text.")
    return parser.parse_args()


class ResponseEvidence:
    """Inspect the entire stream with bounded memory, without retaining text.

    A prefix capture silently drops late finish reasons on long generations.
    Retain only the unfinished line, discard oversized lines explicitly, and
    continue parsing subsequent events (including the terminal event).
    """

    MAX_LINE_BYTES = 524_288

    def __init__(self) -> None:
        self.pending = bytearray()
        self.discarding = False
        self.value = inspect_response(b"")
        self.value.update(responseOversizedLineCount=0, responseDoneMarkerCount=0)

    def _line(self, line: bytes) -> None:
        if line.strip() == b"data: [DONE]":
            self.value["responseDoneMarkerCount"] += 1
        evidence = inspect_response(line)
        payload = line.strip()
        if payload.startswith(b"data:"):
            payload = payload[5:].strip()
        try:
            event = json.loads(payload)
        except (json.JSONDecodeError, UnicodeDecodeError):
            event = None
        terminal_response = isinstance(event, dict) and event.get("type") in (
            "response.completed", "response.incomplete")
        for key, value in evidence.items():
            if key == "responseUsage":
                if value is not None:
                    self.value[key] = value
                continue
            if terminal_response and (
                (key == "responseTextBytes" and self.value["responseTextDeltaCount"])
                or (key == "responseReasoningBytes" and self.value["responseReasoningDeltaCount"])
                or (key == "responseToolArgumentBytes" and self.value[key])
            ):
                continue
            if isinstance(value, list):
                for item in value:
                    if item not in self.value[key] and len(self.value[key]) < 128:
                        self.value[key].append(item)
            else:
                self.value[key] += value

    def feed(self, chunk: bytes) -> None:
        lines = chunk.split(b"\n")
        for index, line in enumerate(lines):
            terminated = index < len(lines) - 1
            if not self.discarding:
                if len(self.pending) + len(line) > self.MAX_LINE_BYTES:
                    self.pending.clear()
                    self.discarding = True
                    self.value["responseOversizedLineCount"] += 1
                else:
                    self.pending.extend(line)
            if terminated:
                if not self.discarding:
                    self._line(bytes(self.pending))
                self.pending.clear()
                self.discarding = False

    def finish(self) -> dict:
        if self.pending and not self.discarding:
            self._line(bytes(self.pending))
        self.pending.clear()
        return dict(self.value)


class Probe:
    def __init__(
        self,
        upstream: str,
        log_path: Path,
        upstream_timeout_seconds: float = 45.0,
        capture_last_user_message: bool = False,
        interrupt_once_after_text_bytes: int = 0,
        truncate_compaction_once_tokens: int = 0,
        expected_request_instructions: Path | None = None,
    ) -> None:
        self.upstream = urlsplit(upstream.rstrip("/"))
        if self.upstream.scheme not in {"http", "https"} or not self.upstream.hostname:
            raise ValueError("upstream must be an HTTP(S) URL")
        self.log_path = log_path
        self.log_path.parent.mkdir(parents=True, exist_ok=True)
        self.log_path.touch(exist_ok=True)
        self.lock = threading.Lock()
        self.interrupt_after_text_bytes = max(0, interrupt_once_after_text_bytes)
        self.interruption_claimed = False
        self.truncation_tokens = max(0, truncate_compaction_once_tokens)
        self.truncation_claimed = False
        self.upstream_timeout_seconds = max(1.0, upstream_timeout_seconds)
        self.capture_last_user_message = capture_last_user_message
        self.expected_request_instructions = expected_request_instructions

    def claim_compaction_truncation(self, request: dict) -> bool:
        # Match an exact maintenance envelope, never generic user keywords.
        # No synthetic SSE: the upstream must itself report max_output_tokens.
        if not self.truncation_tokens or not isinstance(request, dict):
            return False
        items = request.get("input")
        if (not isinstance(items, list)
                or request.get("reasoning") != {"effort": "none"}):
            return False
        users = [item for item in items if isinstance(item, dict) and item.get("role") == "user"]
        if not users:
            return False
        content = users[-1].get("content")
        if isinstance(content, list):
            content = "".join(block.get("text", "") for block in content
                              if isinstance(block, dict) and block.get("type") == "input_text"
                              and isinstance(block.get("text"), str))
        hot = isinstance(content, str) and content.startswith(
            "Context maintenance only. Do not execute any task or call any tool. Return only the checkpoint text. ")
        if not hot:
            if len(users) != 1 or request.get("tools"):
                return False
            try:
                envelope = json.loads(content)
            except (TypeError, ValueError):
                return False
            if not isinstance(envelope, dict) or set(envelope) != {"previous_checkpoint", "next_archive_segment"}:
                return False
        with self.lock:
            if (not self.truncation_tokens or self.truncation_claimed
                    or request.get("max_output_tokens", 0) <= self.truncation_tokens):
                return False
            self.truncation_claimed = True
            return True

    def claim_interruption(self, evidence: dict) -> bool:
        with self.lock:
            if (not self.interrupt_after_text_bytes or self.interruption_claimed
                    or evidence["responseTextBytes"] < self.interrupt_after_text_bytes
                    or evidence["finishReasons"] or evidence["responseDoneMarkerCount"]):
                return False
            self.interruption_claimed = True
            return True

    def append(self, value: dict) -> None:
        line = json.dumps(value, ensure_ascii=False, separators=(",", ":")) + "\n"
        with self.lock:
            self.log_path.parent.mkdir(parents=True, exist_ok=True)
            with self.log_path.open("a", encoding="utf-8") as handle:
                handle.write(line)
                handle.flush()

    def response_override(self, path: str, request: dict, headers) -> tuple[int, bytes] | None:
        """Test-owned subclasses may inject transport faults before any I/O."""
        return None


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "NolaE2EProviderProbe/1"

    def log_message(self, format: str, *args: object) -> None:
        return

    def do_GET(self) -> None:
        # Metadata discovery is part of the real client path, not a mocked capability.
        self.do_POST()

    def do_POST(self) -> None:
        probe: Probe = self.server.probe  # type: ignore[attr-defined]
        started = time.monotonic()
        started_at = dt.datetime.now(dt.timezone.utc).isoformat()
        request_id = uuid.uuid4().hex
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        request_sha = hashlib.sha256(body).hexdigest()
        try:
            request_value = json.loads(body)
        except json.JSONDecodeError:
            request_value = {}
        generation_input = request_value.get("messages", request_value.get("input"))
        input_present = ((isinstance(generation_input, list) and bool(generation_input))
            or (isinstance(generation_input, str) and bool(generation_input.strip())))
        instruction_evidence = inspect_expected_instructions(request_value, probe.expected_request_instructions)
        instruction_evidence.update(inspect_ui_action_revisions(request_value))
        override = probe.response_override(self.path, request_value, self.headers)
        if override is not None:
            status, response = override
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(response)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(response)
            self.wfile.flush()
            self.close_connection = True
            probe.append({"schemaVersion": 1, "event": "provider.response", "requestId": request_id,
                "startedAt": started_at, "status": status, "upstreamPath": self.path,
                "requestSha256": request_sha, "model": request_value.get("model"),
                "requestInputPresent": input_present,
                **tool_schema_metrics(request_value),
                **instruction_evidence,
                "injectedHttpError": True, "forwarded": False, "error": "InjectedHttpError"})
            return
        injected_truncation = probe.claim_compaction_truncation(request_value)
        upstream_value = request_value
        if injected_truncation:
            upstream_value = {**request_value, "max_output_tokens": probe.truncation_tokens}
            body = json.dumps(upstream_value).encode("utf-8")
        # Expose the same path shape on the local origin. Clients frequently
        # derive capability and chat routes from the configured endpoint, so
        # prepending or stripping a proxy prefix changes production behavior.
        upstream_path = self.path or "/"
        connection_type = http.client.HTTPSConnection if probe.upstream.scheme == "https" else http.client.HTTPConnection
        connection = connection_type(
            probe.upstream.hostname,
            probe.upstream.port,
            timeout=probe.upstream_timeout_seconds,
        )
        headers = {
            key: value for key, value in self.headers.items()
            if key.lower() not in {"host", "content-length", "connection", "accept-encoding"}
        }
        headers["Content-Length"] = str(len(body))
        headers["Connection"] = "close"
        status = 502
        response_bytes = 0
        response_evidence = ResponseEvidence()
        error = None
        transfer_stage = "upstream_request"
        error_stage = None
        downstream_started = False
        upstream_complete = False
        injected_interruption = False
        try:
            connection.request(self.command, upstream_path, body=body, headers=headers)
            transfer_stage = "upstream_headers"
            upstream = connection.getresponse()
            status = upstream.status
            transfer_stage = "downstream_headers"
            self.send_response(status, upstream.reason)
            content_type = upstream.getheader("Content-Type") or "application/octet-stream"
            self.send_header("Content-Type", content_type)
            self.send_header("Transfer-Encoding", "chunked")
            self.send_header("Connection", "close")
            self.end_headers()
            downstream_started = True
            while True:
                try:
                    transfer_stage = "upstream_body"
                    # read(n) may wait to fill n bytes even while SSE events
                    # are already available. The observer must not turn a
                    # streaming request into buffered bursts or manufacture
                    # client inactivity timeouts. read1 forwards available
                    # bytes after at most one underlying buffered read.
                    chunk = upstream.read1(8192)
                except http.client.IncompleteRead as incomplete:
                    chunk = incomplete.partial
                    error = f"IncompleteRead: {incomplete}"
                    error_stage = transfer_stage
                if not chunk:
                    break
                response_bytes += len(chunk)
                response_evidence.feed(chunk)
                transfer_stage = "downstream_body"
                self.wfile.write(f"{len(chunk):X}\r\n".encode("ascii"))
                self.wfile.write(chunk)
                self.wfile.write(b"\r\n")
                self.wfile.flush()
                if (request_value.get("stream") and status == 200
                        and "text/event-stream" in content_type.lower()
                        and probe.claim_interruption(response_evidence.value)):
                    injected_interruption = True
                    error = "InjectedIncompleteStream"
                    break
                if error is not None:
                    break
            upstream_complete = error is None
            # Always close the downstream HTTP chunk framing. If the upstream
            # stream was truncated, the missing provider terminal SSE event is
            # the product-visible incomplete-stream signal. Leaving the chunked
            # response itself unterminated makes clients wait for their total
            # HTTP timeout and tests the probe's malformed transport instead of
            # the product's stream recovery policy.
        except Exception as caught:
            error = f"{type(caught).__name__}: {caught}"
            error_stage = transfer_stage
            if not downstream_started and not self.wfile.closed:
                try:
                    self.send_error(502, "Provider probe forwarding failed")
                except Exception:
                    pass
        finally:
            if downstream_started:
                try:
                    self.wfile.write(b"0\r\n\r\n")
                    self.wfile.flush()
                except (OSError, ValueError):
                    # A disconnected client cannot receive the framing, but
                    # must not prevent the original upstream evidence record.
                    pass
            connection.close()
            self.close_connection = True
            evidence = response_evidence.finish()
            last_user_message = last_user_text(request_value) if probe.capture_last_user_message else None
            probe.append({
                "schemaVersion": 1,
                "event": "provider.response",
                "requestId": request_id,
                "startedAt": started_at,
                "durationMilliseconds": round((time.monotonic() - started) * 1000, 3),
                "upstreamHost": probe.upstream.hostname,
                "upstreamPath": upstream_path,
                "status": status,
                "responseBytes": response_bytes,
                "requestSha256": request_sha,
                "upstreamRequestSha256": hashlib.sha256(body).hexdigest(),
                "requestedOutputTokens": output_limit(request_value),
                "upstreamOutputTokens": output_limit(upstream_value),
                "injectedCompactionTruncation": injected_truncation,
                "model": request_value.get("model"),
                "requestInputPresent": input_present,
                "stream": bool(request_value.get("stream")),
                "messageCount": len(request_value.get("messages", [])),
                "messageRoles": [
                    message.get("role")
                    for message in request_value.get("messages", [])
                    if isinstance(message, dict)
                ],
                "lastUserMessage": last_user_message,
                "toolChoice": request_value.get("tool_choice"),
                "reasoningControl": request_value.get("reasoning"),
                "thinkingControl": request_value.get("thinking"),
                "tools": [
                    tool.get("function", {}).get("name") or tool.get("name")
                    for tool in request_value.get("tools", [])
                    if isinstance(tool, dict)
                ],
                **tool_schema_metrics(request_value),
                **instruction_evidence,
                "error": error,
                "errorStage": error_stage,
                "injectedInterruption": injected_interruption,
                "injectedHttpError": False,
                "forwarded": True,
                **evidence,
            })


def main() -> int:
    args = arguments()
    probe = Probe(
        args.upstream,
        args.log,
        args.upstream_timeout_seconds,
        capture_last_user_message=args.capture_last_user_message,
        interrupt_once_after_text_bytes=args.interrupt_once_after_text_bytes,
        truncate_compaction_once_tokens=args.truncate_compaction_once_tokens,
        expected_request_instructions=args.expected_request_instructions,
    )
    server = ThreadingHTTPServer((args.host, args.port), Handler)
    server.probe = probe  # type: ignore[attr-defined]
    args.ready.parent.mkdir(parents=True, exist_ok=True)
    temporary = args.ready.with_suffix(args.ready.suffix + ".tmp")
    upstream_path = probe.upstream.path or ""
    if probe.upstream.query:
        upstream_path += "?" + probe.upstream.query
    temporary.write_text(
        json.dumps({"endpoint": f"http://{args.host}:{server.server_port}{upstream_path}"}) + "\n",
        encoding="utf-8",
    )
    temporary.replace(args.ready)
    try:
        server.serve_forever(poll_interval=0.1)
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
