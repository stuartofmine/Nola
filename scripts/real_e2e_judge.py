#!/usr/bin/env python3
"""Derive cross-platform E2E assertions from immutable observations.

Platform adapters may collect evidence, but they do not decide whether a
Journey passed. This module is the shared trust boundary for CLI, Web, macOS,
iOS, and Android receipts.
"""

from __future__ import annotations

from collections import Counter
import json
import os
from pathlib import Path
import re
import sqlite3
import struct
from typing import Any, Iterable

from real_e2e_events import summarize
from provider_trace_metrics import provider_trace_metrics
from verify_mac_ui_terminal import verify as verify_run_terminal


def _paths(receipt: dict[str, Any], kind: str) -> list[Path]:
    return [
        Path(str(observation["path"]))
        for step in receipt.get("steps", [])
        for observation in step.get("observations", [])
        if observation.get("kind") == kind and observation.get("path")
    ]


def _load_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8", errors="replace"))


def _event_summaries(receipt: dict[str, Any]) -> list[dict[str, Any]]:
    result = []
    for path in _paths(receipt, "event_stream"):
        try:
            result.append(summarize(path))
        except (OSError, ValueError, json.JSONDecodeError):
            continue
    return result


def _provider_metrics(receipt: dict[str, Any]) -> dict[str, Any]:
    return provider_trace_metrics(_paths(receipt, "provider_trace"))


def _transcripts(receipt: dict[str, Any]) -> list[list[dict[str, Any]]]:
    result = []
    for path in _paths(receipt, "transcript"):
        try:
            value = _load_json(path)
        except (OSError, json.JSONDecodeError):
            continue
        if isinstance(value, list) and all(isinstance(item, dict) for item in value):
            result.append(value)
    return result


def _timelines(receipt: dict[str, Any]) -> list[list[dict[str, Any]]]:
    result = []
    for path in _paths(receipt, "ui_timeline"):
        try:
            value = _load_json(path)
        except (OSError, json.JSONDecodeError):
            continue
        if isinstance(value, list) and all(isinstance(item, dict) for item in value):
            result.append(value)
    return result


def _result(
    passed: bool,
    detail: str,
    evidence: Iterable[str] = (),
    metrics: dict[str, Any] | None = None,
) -> dict[str, Any]:
    return {
        "passed": bool(passed),
        "detail": detail,
        "evidence": sorted(set(evidence)),
        "metrics": metrics or {},
    }


def _judge_provider_real(receipt: dict[str, Any]) -> dict[str, Any]:
    metrics = _provider_metrics(receipt)
    hosts = set(metrics["successfulModelUpstreamHosts"])
    passed = (
        metrics["successfulModelRequests"] > 0
        and metrics["malformedRows"] == 0
        and metrics["unreadableFiles"] == 0
        and bool(hosts - {"127.0.0.1", "localhost", "::1"})
    )
    return _result(
        passed,
        "real upstream model traffic was observed" if passed else "no valid non-loopback model response was observed",
        (str(path) for path in _paths(receipt, "provider_trace")),
        metrics,
    )


def _judge_event_terminal(receipt: dict[str, Any]) -> dict[str, Any]:
    # Keep evidence paired within a step; do not borrow another step's database.
    audits, errors, evidence = [], [], []
    for index, step in enumerate(receipt.get("steps", [])):
        scoped = {"steps": [step]}
        transcripts = sorted(set(_paths(scoped, "transcript")))
        stores = sorted(set(_paths(scoped, "persistence_store")))
        if not transcripts and not stores:
            continue
        if len(transcripts) != 1 or len(stores) != 1:
            errors.append(f"step {index}: requires one transcript paired with one persistence store")
            continue
        evidence.extend(map(str, [*transcripts, *stores]))
        try:
            with sqlite3.connect(stores[0].resolve().as_uri() + "?mode=ro", uri=True) as connection:
                audit = verify_run_terminal(connection, _load_json(transcripts[0]), 1,
                                            require_success=False)
            audits.append(audit)
        except (OSError, ValueError, TypeError, sqlite3.Error) as error:
            errors.append(f"step {index}: {error}")
    passed = bool(audits) and not errors
    return _result(
        passed,
        "production terminal results match persisted seals; outcomes remain distinct"
        if passed else "missing or inconsistent production terminal and persisted seal evidence",
        evidence,
        {"audits": audits, "errors": errors},
    )


def _judge_tool_pairing(receipt: dict[str, Any]) -> dict[str, Any]:
    summaries = _event_summaries(receipt)
    started = Counter(name for item in summaries for name in item.get("toolsStarted", []))
    completed = Counter(name for item in summaries for name in item.get("toolsCompleted", []))
    started_calls = Counter(
        call_id
        for item in summaries
        if "run.cancelled" not in item.get("terminalKinds", [])
        for call_id in item.get("toolCallsStarted", [])
    )
    completed_calls = Counter(
        call_id
        for item in summaries
        if "run.cancelled" not in item.get("terminalKinds", [])
        for call_id in item.get("toolCallsCompleted", [])
    )
    required_started = started_calls or Counter(
        name
        for item in summaries
        if "run.cancelled" not in item.get("terminalKinds", [])
        for name in item.get("toolsStarted", [])
    )
    required_completed = completed_calls or Counter(
        name
        for item in summaries
        if "run.cancelled" not in item.get("terminalKinds", [])
        for name in item.get("toolsCompleted", [])
    )
    missing = {
        name: count - required_completed[name]
        for name, count in required_started.items()
        if required_completed[name] < count
    }
    passed = bool(started) and not missing
    return _result(
        passed,
        "every observed tool start has a completion" if passed else "tool lifecycle is empty or incomplete",
        (str(path) for path in _paths(receipt, "event_stream")),
        {
            "started": dict(started),
            "completed": dict(completed),
            "startedCalls": dict(started_calls),
            "completedCalls": dict(completed_calls),
            "pairingBasis": "call_id" if started_calls else "tool_name",
            "missing": missing,
            "cancelledStreams": sum(
                "run.cancelled" in item.get("terminalKinds", []) for item in summaries
            ),
        },
    )


def _judge_assistant_nonempty(receipt: dict[str, Any]) -> dict[str, Any]:
    summaries = _event_summaries(receipt)
    answers = [str(item.get("assistant") or "").strip() for item in summaries]
    passed = any(answers)
    return _result(
        passed,
        "a canonical final assistant answer was observed" if passed else "event streams contain no final assistant answer",
        (str(path) for path in _paths(receipt, "event_stream")),
        {"answerCharacters": [len(value) for value in answers]},
    )


def _judge_persistence_present(receipt: dict[str, Any]) -> dict[str, Any]:
    audits = []
    failures = []
    for path in _paths(receipt, "persistence_audit"):
        try:
            value = _load_json(path)
        except (OSError, json.JSONDecodeError):
            failures.append(f"unreadable audit: {path}")
            continue
        assertions = value.get("assertions", []) if isinstance(value, dict) else []
        if isinstance(assertions, dict):
            assertions_pass = bool(assertions) and all(item is True for item in assertions.values())
        else:
            assertions_pass = bool(assertions) and all(
                isinstance(item, dict) and item.get("passed") is True for item in assertions
            )
        if isinstance(value, dict) and value.get("passed") is True and assertions_pass:
            audits.append(path)
        else:
            failures.append(f"failed or assertion-free audit: {path}")

    sqlite_stores = []
    for path in _paths(receipt, "persistence_store"):
        if not path.is_file() or path.stat().st_size < 100:
            continue
        try:
            if path.read_bytes()[:16] != b"SQLite format 3\x00":
                continue
            connection = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
            try:
                tables = connection.execute(
                    "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"
                ).fetchall()
            finally:
                connection.close()
            if tables:
                sqlite_stores.append(path)
        except (OSError, sqlite3.Error):
            continue
    existing = [*audits, *sqlite_stores]
    passed = bool(existing) and not failures
    return _result(
        passed,
        "durable product state was independently verified" if passed else "persistence evidence is absent or not independently verified",
        (str(path) for path in existing),
        {
            "verifiedAudits": len(audits),
            "verifiedSQLiteStores": len(sqlite_stores),
            "failures": failures[:8],
        },
    )


def _judge_http_api(receipt: dict[str, Any]) -> dict[str, Any]:
    valid = []
    failures = []
    for path in _paths(receipt, "http_transcript"):
        try:
            value = _load_json(path)
        except (OSError, json.JSONDecodeError):
            failures.append(f"unreadable HTTP transcript: {path}")
            continue
        exchanges = value.get("exchanges", []) if isinstance(value, dict) else []
        names = {str(item.get("name")) for item in exchanges if isinstance(item, dict)}
        statuses = {
            str(item.get("name")): int(item.get("status", 0) or 0)
            for item in exchanges if isinstance(item, dict)
        }
        required = {"health", "register", "profile", "create_conversation", "append_message", "list_messages"}
        responses = value.get("responses", {}) if isinstance(value, dict) else {}
        identity = responses.get("register", {}).get("userId")
        profile_identity = responses.get("profile", {}).get("userId")
        marker = str(value.get("marker") or "")
        messages = responses.get("list_messages", {}).get("messages", [])
        round_trip = any(
            isinstance(item, dict) and item.get("content") == marker
            for item in messages
        )
        passed = (
            required.issubset(names)
            and all(200 <= statuses.get(name, 0) < 300 for name in required)
            and bool(identity)
            and identity == profile_identity
            and bool(marker)
            and round_trip
        )
        if passed:
            valid.append(path)
        else:
            failures.append(f"incomplete authenticated HTTP lifecycle: {path}")
    passed = bool(valid) and not failures
    return _result(
        passed,
        "real HTTP authentication and persistence round-trip completed" if passed else "HTTP evidence does not prove the production lifecycle",
        (str(path) for path in valid),
        {"verifiedTranscripts": len(valid), "failures": failures[:8]},
    )


def _judge_ui_screenshot(receipt: dict[str, Any]) -> dict[str, Any]:
    images = []
    failures = []
    for path in _paths(receipt, "ui_screenshot"):
        try:
            payload = path.read_bytes()
        except OSError:
            failures.append(f"unreadable screenshot: {path}")
            continue
        if len(payload) < 1024 or payload[:8] != b"\x89PNG\r\n\x1a\n" or len(payload) < 24:
            failures.append(f"invalid PNG screenshot: {path}")
            continue
        width, height = struct.unpack(">II", payload[16:24])
        if width < 200 or height < 200 or len(set(payload[24:])) < 32:
            failures.append(f"implausible screenshot dimensions/content: {path}")
            continue
        images.append({"path": str(path), "width": width, "height": height, "bytes": len(payload)})
    passed = bool(images) and not failures
    return _result(
        passed,
        "a parseable non-trivial product screenshot was captured" if passed else "product screenshot evidence is invalid or missing",
        (item["path"] for item in images),
        {"images": images, "failures": failures[:8]},
    )


def _judge_transcript_order(receipt: dict[str, Any]) -> dict[str, Any]:
    transcripts = _transcripts(receipt)
    failures = []
    message_count = 0
    for transcript in transcripts:
        roles = [str(item.get("role") or "").split(".")[-1].lower() for item in transcript]
        message_count += len(roles)
        if len(roles) < 2 or roles[0] != "user":
            failures.append("transcript does not begin with a user turn")
            continue
        for index, role in enumerate(roles):
            expected = "user" if index % 2 == 0 else "assistant"
            if role != expected:
                failures.append(f"message {index} is {role}, expected {expected}")
        for item in transcript:
            role = str(item.get("role") or "").split(".")[-1].lower()
            if role == "assistant" and not str(item.get("text") or "").strip():
                failures.append("assistant message is empty")
    passed = bool(transcripts) and not failures
    return _result(
        passed,
        "user and assistant messages preserve turn ownership" if passed else "transcript order is missing or invalid",
        (str(path) for path in _paths(receipt, "transcript")),
        {"transcripts": len(transcripts), "messages": message_count, "failures": failures[:8]},
    )


def _judge_ui_terminal(receipt: dict[str, Any]) -> dict[str, Any]:
    timelines = _timelines(receipt)
    terminal = [bool(items) and items[-1].get("running") is False for items in timelines]
    passed = bool(terminal) and all(terminal)
    return _result(
        passed,
        "every UI timeline reached an idle terminal state" if passed else "UI timeline is missing or remained running",
        (str(path) for path in _paths(receipt, "ui_timeline")),
        {"timelines": len(timelines), "terminal": terminal},
    )


def _judge_ui_progressive(receipt: dict[str, Any]) -> dict[str, Any]:
    timelines = _timelines(receipt)
    progressive_stages = 0
    regressions = 0
    for timeline in timelines:
        stages: dict[str, list[tuple[int, str | None]]] = {}
        for sample in timeline:
            stage = str(sample.get("stage") or "default")
            text = sample.get("assistantText") if isinstance(sample.get("assistantText"), str) else None
            stages.setdefault(stage, []).append((int(sample.get("assistantCharacters", 0) or 0), text))
        for values in stages.values():
            positive = [value for value in values if value[0] > 0]
            if len(set(positive)) >= 2:
                progressive_stages += 1
            if any(right[0] < left[0] for left, right in zip(positive, positive[1:])):
                regressions += 1
            if any(
                left[1] is not None and right[1] is not None and not right[1].startswith(left[1])
                for left, right in zip(positive, positive[1:])
            ):
                regressions += 1
    passed = bool(timelines) and progressive_stages > 0 and regressions == 0
    return _result(
        passed,
        "assistant text advanced through stable UI frames" if passed else "no stable progressive text timeline was observed",
        (str(path) for path in _paths(receipt, "ui_timeline")),
        {"progressiveStages": progressive_stages, "regressions": regressions},
    )


def _judge_accessibility_matches_transcript(receipt: dict[str, Any]) -> dict[str, Any]:
    def structured_text(path: Path) -> str:
        raw = path.read_text(encoding="utf-8", errors="replace")
        try:
            value = json.loads(raw)
        except json.JSONDecodeError:
            return raw
        strings: list[str] = []

        def collect(item: Any) -> None:
            if isinstance(item, str):
                strings.append(item)
            elif isinstance(item, dict):
                for child in item.values():
                    collect(child)
            elif isinstance(item, list):
                for child in item:
                    collect(child)

        collect(value)
        return "\n".join(strings)

    accessibility_documents = [
        structured_text(path)
        for path in _paths(receipt, "accessibility_tree")
        if path.is_file()
    ]
    accessibility = "\n".join(accessibility_documents)
    visible_labels = "\n".join(
        match.group(1)
        for document in accessibility_documents
        for line in document.splitlines()
        if (match := re.search(r"\blabel: '(.*)'\s*$", line))
    )
    assistants = [
        str(item.get("text") or "")
        for transcript in _transcripts(receipt)
        for item in transcript
        if str(item.get("role") or "").split(".")[-1].lower() == "assistant"
    ]
    markers = {
        marker
        for text in assistants
        for marker in re.findall(r"NOLA-[A-Z0-9-]{8,}|[A-Za-z0-9_\-]+\.docx", text)
    }
    if markers:
        missing = sorted(marker for marker in markers if marker not in accessibility)
        passed = not missing and bool(accessibility)
    else:
        normalized_tree = re.sub(r"\s+", " ", f"{accessibility}\n{visible_labels}")
        candidates = [
            re.sub(r"\s+", " ", line).strip()[:80]
            for value in assistants
            for line in value.splitlines()
            if len(line.strip()) >= 24
        ]
        missing = [value for value in candidates if value and value not in normalized_tree]
        passed = bool(candidates) and not missing
    return _result(
        passed,
        "rendered assistant content is represented in accessibility" if passed else "accessibility does not match the rendered transcript",
        [
            *(str(path) for path in _paths(receipt, "accessibility_tree")),
            *(str(path) for path in _paths(receipt, "transcript")),
        ],
        {"markers": sorted(markers), "missing": missing[:8]},
    )


def _judge_instrumentation_passed(receipt: dict[str, Any]) -> dict[str, Any]:
    outputs = [
        path.read_text(encoding="utf-8", errors="replace")
        for path in _paths(receipt, "instrumentation_output")
        if path.is_file()
    ]
    passed = bool(outputs) and all("FAILURES!!!" not in value and re.search(r"OK \(\d+ tests?\)", value) for value in outputs)
    return _result(
        passed,
        "device instrumentation completed without failures" if passed else "instrumentation output is missing or failed",
        (str(path) for path in _paths(receipt, "instrumentation_output")),
        {"outputs": len(outputs)},
    )


def _judge_terminal_ui(receipt: dict[str, Any]) -> dict[str, Any]:
    from terminal_acceptance_audit import verify
    # Bind every observation to one step; no borrowing another Run's DB/seal.
    kinds = ("terminal_capture", "terminal_frames", "terminal_actions", "terminal_metrics",
             "persistence_audit", "persistence_store")
    oracle = Path(os.environ.get("NOLA_TERMINAL_ORACLE_BIN", str(
        Path(__file__).resolve().parents[1] / "target/release/examples/terminal_acceptance_oracle")))
    errors = []
    for step in receipt.get("steps", []):
        by_kind = {kind: [Path(str(row["path"])) for row in step.get("observations", [])
                         if row.get("kind") == kind and row.get("path")] for kind in kinds}
        if not any(by_kind.values()):
            continue
        try:
            if any(len(paths) != 1 for paths in by_kind.values()):
                raise ValueError("one exact observation per terminal kind is required")
            paths = [by_kind[kind][0] for kind in kinds]
            metrics = verify(*paths, oracle)
            return _result(True, "standard VT100 replay, retained history and formal typed cancellation verified",
                           [str(path) for path in paths], metrics)
        except Exception as error:
            errors.append(f"{type(error).__name__}: {error}")
    return _result(False, "terminal observations do not prove the current lifecycle",
                   metrics={"errors": errors or ["no terminal observation step"]})


JUDGES = {
    "provider_real": _judge_provider_real,
    "event_terminal": _judge_event_terminal,
    "tool_pairing": _judge_tool_pairing,
    "assistant_nonempty": _judge_assistant_nonempty,
    "persistence_present": _judge_persistence_present,
    "http_api": _judge_http_api,
    "transcript_order": _judge_transcript_order,
    "ui_terminal": _judge_ui_terminal,
    "ui_progressive": _judge_ui_progressive,
    "ui_screenshot_valid": _judge_ui_screenshot,
    "accessibility_matches_transcript": _judge_accessibility_matches_transcript,
    "instrumentation_passed": _judge_instrumentation_passed,
    "terminal_ui": _judge_terminal_ui,
}


def evaluate_receipt(receipt: dict[str, Any], scenario: dict[str, Any]) -> dict[str, Any]:
    contract = scenario.get("_contract", {})
    scenario_evidence = scenario.get("evidence", {})
    validators = list(dict.fromkeys([
        *contract.get("validators", []),
        *scenario_evidence.get("validators", []),
    ]))
    assertions = {}
    errors = []
    for validator in validators:
        judge = JUDGES.get(validator)
        if judge is None:
            assertions[validator] = _result(False, "validator is not implemented")
        else:
            assertions[validator] = judge(receipt)
        if assertions[validator]["passed"] is not True:
            errors.append(f"central validator failed: {validator}: {assertions[validator]['detail']}")
    return {
        "schemaVersion": 1,
        "validators": validators,
        "passed": not errors,
        "assertions": assertions,
        "errors": errors,
    }
