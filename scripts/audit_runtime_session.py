#!/usr/bin/env python3
"""Audit a persisted Nola session using production runtime evidence.

This is intentionally independent from client acceptance harness counters.  A
real end-to-end scenario passes only when its durable message, run, trace, and
tool ledgers agree and the persisted response was delivered incrementally.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sqlite3
import subprocess
import sys
import tempfile
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


@dataclass
class Assertion:
    name: str
    passed: bool
    detail: str


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--db", required=True, type=Path)
    parser.add_argument("--session-id")
    parser.add_argument("--expected-result", default="success",
                        choices=("success", "answered", "action_completed", "blocked", "incomplete", "failed", "cancelled"))
    parser.add_argument("--receipt", type=Path)
    parser.add_argument("--presentation-receipt", type=Path)
    parser.add_argument("--provider-trace", type=Path)
    parser.add_argument("--canonical-reader", type=Path,
                        default=os.environ.get("NOLA_AUDIT_CANONICAL_READER"))
    parser.add_argument("--large-response-chars", type=int, default=512)
    parser.add_argument("--min-large-response-parts", type=int, default=3)
    parser.add_argument("--max-single-part-ratio", type=float, default=0.90)
    parser.add_argument("--allow-batched-message-timestamps", action="store_true")
    parser.add_argument("--allow-missing-tool-ledger", action="store_true")
    parser.add_argument("--allow-serialized-model-tool-batch", action="store_true")
    return parser.parse_args()


def decode_json(raw: str | None) -> dict[str, Any]:
    if not raw:
        return {}
    try:
        value = json.loads(raw)
    except json.JSONDecodeError:
        return {}
    return value if isinstance(value, dict) else {}


def event_payload(raw: str) -> dict[str, Any]:
    event = decode_json(raw)
    payload = event.get("payload")
    return payload if isinstance(payload, dict) else event


def table_names(connection: sqlite3.Connection) -> set[str]:
    rows = connection.execute(
        "SELECT name FROM sqlite_master WHERE type = 'table'"
    ).fetchall()
    return {str(row[0]) for row in rows}


def choose_session(connection: sqlite3.Connection, requested: str | None) -> str:
    if requested:
        row = connection.execute(
            "SELECT id FROM conversations WHERE id = ?", (requested,)
        ).fetchone()
    else:
        row = connection.execute(
            "SELECT id FROM conversations ORDER BY updated_at_ms DESC LIMIT 1"
        ).fetchone()
    if row is None:
        raise RuntimeError("No matching persisted session")
    return str(row[0])


def extract_final_content(messages: list[dict[str, Any]]) -> str:
    for message in reversed(messages):
        if message["role"] != "assistant":
            continue
        content = decode_json(message["content_json"]).get("content")
        # The newest assistant record owns the result, including a failed or
        # empty result. An older answer must never fill in for a later failure.
        return content if isinstance(content, str) else ""
    return ""


def assistant_contents(messages: list[dict[str, Any]]) -> list[str]:
    contents: list[str] = []
    for message in messages:
        if message["role"] != "assistant":
            continue
        content = decode_json(message["content_json"]).get("content")
        if isinstance(content, str) and content:
            contents.append(content)
    return contents


def strict_json_object(raw: str) -> dict[str, Any]:
    def unique_fields(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"duplicate JSON field: {key}")
            result[key] = value
        return result

    value = json.loads(raw, object_pairs_hook=unique_fields)
    if not isinstance(value, dict):
        raise ValueError("text JSON is not an object")
    return value


class ApplicationCanonicalTextReader:
    """Use the actual Application reader on this pinned snapshot's private copy.

    The adapter validates range framing and identity only. It never interprets
    SQL manifests, assembles storage chunks, or accepts a display/trace cache.
    The live database is opened read-only by main and never handed to Runtime.
    """

    RANGE_BYTES = 32768
    MAXIMUM_FRAME_BYTES = 262144
    MAXIMUM_PART_BYTES = 16 * 1024 * 1024
    MAXIMUM_TOTAL_BYTES = 64 * 1024 * 1024

    def __init__(self, connection: sqlite3.Connection, executable: Path):
        self.connection = connection
        self.executable = executable.resolve()
        self.directory: tempfile.TemporaryDirectory | None = None
        self.snapshot: Path | None = None
        self.query_count = 0
        self.total_bytes = 0
        self.binary_sha256: str | None = None

    def _prepare(self) -> None:
        if self.snapshot is not None:
            return
        if not self.executable.is_file() or not os.access(self.executable, os.X_OK):
            raise ValueError("Application canonical reader executable is unavailable")
        digest = hashlib.sha256()
        with self.executable.open("rb") as source:
            while chunk := source.read(65536):
                digest.update(chunk)
        self.binary_sha256 = digest.hexdigest()
        self.directory = tempfile.TemporaryDirectory(prefix="nola-canonical-audit-")
        snapshot = Path(self.directory.name) / "snapshot.db"
        with sqlite3.connect(snapshot) as destination:
            # The source BEGIN is already pinned by choose_session/load_messages.
            # SQLite backup copies that one revision-consistent read transaction.
            self.connection.backup(destination)
        snapshot.chmod(0o600)
        self.snapshot = snapshot

    def close(self) -> None:
        if self.directory is not None:
            self.directory.cleanup()
            self.directory = None
            self.snapshot = None

    def read_part(self, conversation_id: str, message_id: str, part_id: str) -> str:
        self._prepare()
        assert self.snapshot is not None
        environment = dict(os.environ)
        environment.update({"NOLA_DB_PATH": str(self.snapshot),
                            "NOLA_HOME": str(self.snapshot.parent / "home"),
                            "NOLA_MAC_APP_SUPPORT": str(self.snapshot.parent / "support"),
                            "NOLA_MAC_CACHE_ROOT": str(self.snapshot.parent / "cache")})
        offset = 0
        total: int | None = None
        pieces: list[str] = []
        while True:
            request = {"type": "read_conversation_text_range", "conversation_id": conversation_id,
                       "message_id": message_id, "part_id": part_id,
                       "offset_bytes": offset, "limit_bytes": self.RANGE_BYTES}
            try:
                result = subprocess.run(
                    [str(self.executable), "--snapshot-database", str(self.snapshot)],
                    input=json.dumps(request) + "\n", text=True, encoding="utf-8",
                    capture_output=True, timeout=30, check=False, env=environment,
                )
            except (OSError, subprocess.TimeoutExpired) as error:
                raise ValueError("Application canonical reader could not finish the bounded query") from error
            self.query_count += 1
            if result.returncode != 0 or len(result.stdout.encode("utf-8")) > self.MAXIMUM_FRAME_BYTES:
                raise ValueError("Application canonical reader failed or exceeded its frame budget")
            response = strict_json_object(result.stdout)
            if response.get("ok") is not True or set(response) != {"ok", "range"}:
                raise ValueError("Application canonical reader rejected the original body")
            value = response["range"]
            fields = {"conversation_id", "message_id", "part_id", "offset_bytes", "next_offset_bytes",
                      "total_utf8_bytes", "text", "end_of_content"}
            if not isinstance(value, dict) or set(value) != fields:
                raise ValueError("invalid Application canonical range schema")
            if (value["conversation_id"] != conversation_id or value["message_id"] != message_id
                    or value["part_id"] != part_id or value["offset_bytes"] != offset):
                raise ValueError("Application canonical range owner or offset mismatch")
            if (any(type(value[key]) is not int or value[key] < 0 for key in (
                    "offset_bytes", "next_offset_bytes", "total_utf8_bytes"))
                    or not isinstance(value["text"], str) or type(value["end_of_content"]) is not bool):
                raise ValueError("invalid Application canonical range types")
            length = len(value["text"].encode("utf-8"))
            expected_next = offset + length
            if (length > self.RANGE_BYTES or value["next_offset_bytes"] != expected_next
                    or expected_next > value["total_utf8_bytes"]
                    or value["end_of_content"] != (expected_next == value["total_utf8_bytes"])
                    or (not length and not value["end_of_content"])):
                raise ValueError("Application canonical range bounds do not advance consistently")
            if total is None:
                total = value["total_utf8_bytes"]
                if total > self.MAXIMUM_PART_BYTES or self.total_bytes + total > self.MAXIMUM_TOTAL_BYTES:
                    raise ValueError("original canonical body exceeds the explicit audit read budget")
            elif total != value["total_utf8_bytes"]:
                raise ValueError("Application canonical body length changed across bounded ranges")
            pieces.append(value["text"])
            offset = expected_next
            if value["end_of_content"]:
                self.total_bytes += total
                return "".join(pieces)


def inline_part_text(part: sqlite3.Row, run_ids: set[str]) -> tuple[str, bool]:
    """Read known inline text schemas, not arbitrary private JSON prose."""
    scalar = part["content_text"]
    raw = part["content_json"]
    if not isinstance(scalar, str):
        raise ValueError("stored scalar text is not a string")
    if raw is None:
        return scalar, part["state"] in {"complete", "completed"}
    value = strict_json_object(raw)
    if value.get("kind") != "text":
        raise ValueError("stored text Part and JSON kind disagree")
    if "id" in value:
        required = {"kind", "id", "message_id", "turn_id", "text", "is_complete"}
        if not required.issubset(value) or set(value) - required - {"review"}:
            raise ValueError("unsupported AgentPart text schema")
        if (not all(isinstance(value[key], str) for key in ("id", "message_id", "turn_id"))
                or value["id"] != part["id"] or value["message_id"] != part["message_id"]
                or value["turn_id"] not in run_ids):
            raise ValueError("text JSON owner differs from the persisted Part/message/run")
        if value.get("review") is not None and value["review"] != "approved":
            raise ValueError("private or rejected review text is not a public answer")
        text, complete = value["text"], value["is_complete"]
    elif "part_id" in value:
        required = {"kind", "part_id", "revision", "markdown", "is_final"}
        if set(value) != required or value["part_id"] != part["id"]:
            raise ValueError("unsupported or unbound MessagePartView text schema")
        if type(value["revision"]) is not int or not 0 <= value["revision"] < 2 ** 64:
            raise ValueError("invalid text Part revision")
        text, complete = value["markdown"], value["is_final"]
    else:
        raise ValueError("private JSON has no canonical text Part identity")
    if not isinstance(text, str) or type(complete) is not bool:
        raise ValueError("invalid typed text or finality")
    if scalar and scalar != text:
        raise ValueError("scalar and typed JSON text conflict")
    finalized = part["state"] in {"complete", "completed"}
    if complete != finalized:
        raise ValueError("typed text finality differs from its persisted Part state")
    return text, complete


def load_messages(connection: sqlite3.Connection, session_id: str,
                  canonical_reader: ApplicationCanonicalTextReader | None = None) -> list[dict[str, Any]]:
    """Pin original fields; unsupported backing is explicit incomplete evidence.

    Canonical stream ancestry and immutable body ownership are validated by the
    Application reader, not duplicated here. A caller must use that reader to
    audit those backings; their empty inline slots are never treated as content.
    """
    tables = table_names(connection)
    records = connection.execute(
        "SELECT id, run_id, ordinal - 1 AS position, role, status, created_at_ms "
        "FROM messages WHERE conversation_id = ? ORDER BY ordinal", (session_id,)
    ).fetchall()
    result: list[dict[str, Any]] = []
    for record in records:
        run_ids = {str(record["run_id"])} if record["run_id"] is not None else set()
        if "canonical_message_run_owners" in tables and run_ids:
            owners = {str(row[0]) for row in connection.execute(
                "SELECT run_id FROM canonical_message_run_owners "
                "WHERE conversation_id = ? AND message_id = ?", (session_id, record["id"])
            )}
            if run_ids.issubset(owners):
                run_ids.update(owners)
        parts = connection.execute(
            "SELECT id, message_id, ordinal, state, content_text, content_json "
            "FROM message_parts WHERE message_id = ? AND kind = 'text' ORDER BY ordinal",
            (record["id"],)
        ).fetchall()
        text_parts: list[str] = []
        errors: list[str] = []
        complete = bool(parts)
        for part in parts:
            try:
                external_backing = False
                text_backing = False
                for table, description in (
                    ("message_part_body_references", "immutable canonical body"),
                    ("canonical_stream_heads", "canonical stream body"),
                ):
                    if table in tables:
                        if connection.execute(
                            f"SELECT 1 FROM {table} WHERE part_id = ? LIMIT 1", (part["id"],)
                        ).fetchone():
                            external_backing = True
                            if canonical_reader is None:
                                raise ValueError(f"{description} requires the Application canonical reader")
                            if table == "canonical_stream_heads" or connection.execute(
                                "SELECT 1 FROM message_part_body_references WHERE part_id = ? AND field = 'content_text'",
                                (part["id"],),
                            ).fetchone():
                                text_backing = True
                if external_backing:
                    if not text_backing and not part["content_text"]:
                        raise ValueError("JSON-only body requires the Application original-JSON reader")
                    text = canonical_reader.read_part(session_id, record["id"], part["id"])
                    finalized = part["state"] in {"complete", "completed"}
                else:
                    text, finalized = inline_part_text(part, run_ids)
                text_parts.append(text)
                complete = complete and finalized
            except (ValueError, TypeError) as error:
                errors.append(f"{part['id']}: {error}")
        # Fail closed for the complete message, not a partial/scalar fallback.
        content = "".join(text_parts) if not errors else ""
        result.append({
            "id": record["id"], "run_id": record["run_id"],
            "position": record["position"], "role": record["role"],
            "status": record["status"], "created_at": record["created_at_ms"],
            "content_json": json.dumps({"content": content}, ensure_ascii=False),
            "content_errors": errors, "content_complete": complete and not errors,
        })
    return result


def parse_timestamp(value: str | None) -> datetime | None:
    if not value:
        return None
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
        if parsed.tzinfo is None:
            return parsed.replace(tzinfo=timezone.utc)
        return parsed.astimezone(timezone.utc)
    except ValueError:
        return None


def provider_rows(path: Path | None) -> list[dict[str, Any]]:
    if path is None or not path.is_file():
        return []
    rows: list[dict[str, Any]] = []
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict) and value.get("event") == "provider.response":
            rows.append(value)
    return rows


def model_tool_batches(messages: list[dict[str, Any]]) -> list[list[str]]:
    batches: list[list[str]] = []
    for message in messages:
        if message["role"] != "assistant":
            continue
        calls = decode_json(message["content_json"]).get("tool_calls")
        if not isinstance(calls, list) or len(calls) < 2:
            continue
        call_ids = [
            str(call.get("id"))
            for call in calls
            if isinstance(call, dict) and call.get("id")
        ]
        if len(call_ids) >= 2:
            batches.append(call_ids)
    return batches


def main() -> int:
    args = parse_args()
    if not args.db.is_file():
        print(f"runtime audit: database not found: {args.db}", file=sys.stderr)
        return 2

    connection = sqlite3.connect(f"file:{args.db}?mode=ro", uri=True)
    connection.row_factory = sqlite3.Row
    required_tables = {
        "conversations",
        "messages",
        "message_parts",
        "trace_journal_runs",
        "trace_journal_events",
    }
    missing_tables = required_tables - table_names(connection)
    if missing_tables:
        print(
            "runtime audit: missing production tables: " + ", ".join(sorted(missing_tables)),
            file=sys.stderr,
        )
        return 2

    connection.execute("BEGIN")
    session_id = choose_session(connection, args.session_id)
    canonical_reader = ApplicationCanonicalTextReader(connection, args.canonical_reader) if args.canonical_reader else None
    try:
        messages = load_messages(connection, session_id, canonical_reader)
    finally:
        if canonical_reader is not None:
            canonical_reader.close()
    runs = connection.execute(
        "SELECT trace_id, status, terminal_reason, started_at, finished_at "
        "FROM trace_journal_runs WHERE conversation_id = ? ORDER BY started_at",
        (session_id,),
    ).fetchall()
    traces = connection.execute(
        "SELECT trace_id, sequence, kind, status, event_json, emitted_at "
        "FROM trace_journal_events "
        "WHERE conversation_id = ? ORDER BY trace_id, sequence",
        (session_id,),
    ).fetchall()
    tool_ledger = connection.execute(
        "SELECT kind AS tool_name, status, event_json AS args_json, "
        "event_json AS result_json, emitted_at AS started_at, emitted_at AS finished_at "
        "FROM trace_journal_events WHERE conversation_id = ? "
        "AND kind = 'tool.completed' ORDER BY trace_id, sequence",
        (session_id,),
    ).fetchall()

    assertions: list[Assertion] = []

    def check(name: str, passed: bool, detail: str) -> None:
        assertions.append(Assertion(name, bool(passed), detail))

    content_errors = [error for message in messages for error in message["content_errors"]]
    check("canonical_content_read_complete", not content_errors,
          "; ".join(content_errors) or (
              f"owned inline Parts and Application canonical ranges; queries={canonical_reader.query_count} "
              f"utf8_bytes={canonical_reader.total_bytes}" if canonical_reader and canonical_reader.query_count
              else "all text Parts read from supported owned inline fields"))

    positions = [int(row["position"]) for row in messages]
    check("messages_present", bool(messages), f"count={len(messages)}")
    check(
        "message_positions_contiguous",
        positions == list(range(len(positions))),
        f"positions={positions}",
    )
    timestamps = {str(row["created_at"]) for row in messages}
    timestamps_ok = len(messages) <= 2 or len(timestamps) > 1
    check(
        "message_timeline_preserved",
        args.allow_batched_message_timestamps or timestamps_ok,
        f"messages={len(messages)} distinct_created_at={len(timestamps)}",
    )

    final_content = extract_final_content(messages)
    persisted_assistant_contents = assistant_contents(messages)
    longest_assistant_content = max(persisted_assistant_contents, key=len, default="")
    last_assistant = next((message for message in reversed(messages)
                           if message["role"] == "assistant"), None)
    check(
        "final_assistant_message_present",
        bool(final_content.strip()),
        "canonical content unavailable" if last_assistant and last_assistant["content_errors"]
        else f"characters={len(final_content)}",
    )
    last_user_position = max(
        (int(row["position"]) for row in messages if row["role"] == "user"),
        default=-1,
    )
    latest_turn_assistants = [
        row
        for row in messages
        if row["role"] == "assistant" and int(row["position"]) > last_user_position
    ]
    latest_turn_content = extract_final_content(latest_turn_assistants)
    check(
        "latest_turn_assistant_present",
        bool(latest_turn_content.strip()),
        "canonical content unavailable" if any(row["content_errors"] for row in latest_turn_assistants)
        else f"last_user_position={last_user_position} assistant_count={len(latest_turn_assistants)} characters={len(latest_turn_content)}",
    )
    latest_assistant = latest_turn_assistants[-1] if latest_turn_assistants else None
    check("latest_turn_assistant_completed",
          latest_assistant is not None and latest_assistant["status"] == "completed"
          and latest_assistant["content_complete"],
          f"status={latest_assistant['status'] if latest_assistant else 'missing'}")
    from canonical_run_audit import audit_runs
    try:
        canonical = audit_runs(connection, session_id)
        check("canonical_run_result_seal_consistent", True, json.dumps(canonical))
        allowed_results = ("answered", "action_completed") if args.expected_result == "success" else (args.expected_result,)
        check("canonical_runs_match_expected_result", all(kind in allowed_results
              for kind in canonical["outcomes"]), json.dumps(canonical["outcomes"]))
    except (ValueError, sqlite3.Error) as error:
        check("canonical_run_result_seal_consistent", False, str(error))

    sequences_by_trace: dict[str, list[int]] = {}
    for row in traces:
        sequences_by_trace.setdefault(str(row["trace_id"]), []).append(int(row["sequence"]))
    sequence_groups_valid = all(
        len(sequences) == len(set(sequences)) and sequences == sorted(sequences)
        for sequences in sequences_by_trace.values()
    )
    # Trace is optional diagnostics. Mandatory lifecycle evidence is checked
    # above from Run decisions and immutable seals, never inferred from Trace.
    check(
        "trace_sequences_strictly_increasing_per_run",
        sequence_groups_valid,
        " ".join(
            f"{trace_id}:count={len(sequences)},unique={len(set(sequences))}"
            for trace_id, sequences in sequences_by_trace.items()
        ),
    )

    started: dict[str, sqlite3.Row] = {}
    completed: dict[str, sqlite3.Row] = {}
    denied_before_execution: set[str] = set()
    body_parts: list[str] = []
    latest_trace_id = str(runs[-1]["trace_id"]) if runs else ""
    latest_trace_body = ""
    latest_terminal_sequences: list[int] = []
    for row in traces:
        payload = event_payload(str(row["event_json"]))
        if row["kind"] == "tool.started" and payload.get("call_id"):
            started[str(payload["call_id"])] = row
        elif row["kind"] == "tool.completed" and payload.get("call_id"):
            completed[str(payload["call_id"])] = row
        elif (
            row["kind"] == "tool.permission.resolved"
            and row["status"] == "failed"
            and payload.get("call_id")
        ):
            denied_before_execution.add(str(payload["call_id"]))
        elif row["kind"] in {"assistant.delta", "core_part_updated"}:
            text = payload.get("delta") or payload.get("text") or payload.get("content")
            if isinstance(text, str) and text:
                body_parts.append(text)
        if str(row["trace_id"]) == latest_trace_id:
            if row["kind"] == "assistant.reset":
                value = payload.get("delta") or payload.get("text") or ""
                latest_trace_body = value if isinstance(value, str) else ""
            elif row["kind"] == "assistant.delta":
                value = payload.get("delta") or payload.get("text") or ""
                if isinstance(value, str):
                    latest_trace_body += value
            elif row["kind"] in {"run.completed", "run.failed", "run.cancelled"}:
                latest_terminal_sequences.append(int(row["sequence"]))

    latest_trace_sequences = [
        int(row["sequence"])
        for row in traces
        if str(row["trace_id"]) == latest_trace_id
    ]
    check(
        "diagnostic_terminal_not_duplicated",
        len(latest_terminal_sequences) <= 1,
        f"trace_id={latest_trace_id} terminals={latest_terminal_sequences}",
    )
    check(
        "no_trace_events_after_terminal",
        not latest_terminal_sequences
        or max(latest_trace_sequences, default=-1) == latest_terminal_sequences[-1],
        f"terminal={latest_terminal_sequences[-1:] or None} max_sequence={max(latest_trace_sequences, default=-1)}",
    )
    check(
        "latest_trace_matches_persisted_assistant",
        not any(row["content_errors"] for row in latest_turn_assistants)
        and (not latest_trace_body or latest_trace_body == latest_turn_content),
        "canonical content unavailable" if any(row["content_errors"] for row in latest_turn_assistants)
        else f"trace_characters={len(latest_trace_body)} persisted_characters={len(latest_turn_content)}",
    )

    completed_without_start = set(completed) - set(started)
    check(
        "tool_trace_lifecycle_balanced",
        set(started).issubset(completed)
        and completed_without_start.issubset(denied_before_execution),
        f"started={len(started)} completed={len(completed)} "
        f"missing_completed={sorted(set(started) - set(completed))} "
        f"completed_without_start={sorted(completed_without_start)} "
        f"denied_before_execution={sorted(denied_before_execution)}",
    )
    check(
        "denied_tools_not_executed",
        not set(started).intersection(denied_before_execution),
        f"started_after_denial={sorted(set(started).intersection(denied_before_execution))}",
    )
    ledger_ok = not completed or len(tool_ledger) == len(completed)
    check(
        "tool_ledger_matches_trace",
        args.allow_missing_tool_ledger or ledger_ok,
        f"trace_completed={len(completed)} ledger_rows={len(tool_ledger)}",
    )

    presentation_metrics: dict[str, Any] = {}
    presentation_follow_up_text = ""
    if args.presentation_receipt and args.presentation_receipt.is_file():
        presentation = decode_json(args.presentation_receipt.read_text(encoding="utf-8"))
        candidate = presentation.get("metrics")
        if not isinstance(candidate, dict):
            candidate = presentation.get("render_probe")
        if isinstance(candidate, dict):
            presentation_metrics = candidate
        follow_up = presentation.get("follow_up")
        if isinstance(follow_up, dict) and follow_up.get("finished"):
            candidate_text = follow_up.get("assistant_text")
            if isinstance(candidate_text, str):
                presentation_follow_up_text = candidate_text
    presentation_text = presentation_follow_up_text or presentation_metrics.get("raw_text")
    if isinstance(presentation_text, str) and presentation_text:
        check(
            "presentation_matches_persisted_assistant",
            presentation_text == latest_turn_content,
            f"presentation_characters={len(presentation_text)} persisted_characters={len(latest_turn_content)}",
        )
    else:
        check(
            "presentation_matches_persisted_assistant",
            args.presentation_receipt is None,
            "presentation receipt did not contain render_probe.raw_text",
        )

    provider = provider_rows(args.provider_trace)
    boundary = connection.execute("""SELECT created_at_ms,completed_at_ms FROM runs
        WHERE conversation_id=? AND kind='conversation' ORDER BY created_at_ms DESC,id DESC LIMIT 1""",
        (session_id,)).fetchone()
    latest_run_started = datetime.fromtimestamp(boundary[0] / 1000, timezone.utc) if boundary else None
    latest_run_finished = datetime.fromtimestamp(boundary[1] / 1000, timezone.utc) if boundary and boundary[1] is not None else None
    provider_for_latest_run: list[dict[str, Any]] = []
    for row in provider:
        observed = parse_timestamp(str(row.get("startedAt") or ""))
        if observed is None:
            continue
        if latest_run_started and observed < latest_run_started:
            continue
        if latest_run_finished and observed > latest_run_finished:
            continue
        provider_for_latest_run.append(row)
    terminal_provider_indices = [
        index
        for index, row in enumerate(provider_for_latest_run)
        if set(row.get("finishReasons") or []) & {"stop", "completed", "end_turn", "tool_calls"}
        and not row.get("error")
        and (int(row.get("responseTextBytes") or 0) > 0
             or "requirements_update" in (row.get("responseToolNames") or []))
    ]
    if args.provider_trace is not None:
        check(
            "provider_trace_covers_latest_run",
            bool(provider_for_latest_run),
            f"provider_rows={len(provider)} latest_run_rows={len(provider_for_latest_run)}",
        )
        check(
            "provider_delivery_present",
            bool(terminal_provider_indices),
            f"terminal_text_responses={terminal_provider_indices} latest_run_requests={len(provider_for_latest_run)}",
        )
        check(
            "final_provider_response_delivers_content_or_control",
            bool(terminal_provider_indices)
            and terminal_provider_indices[-1] == len(provider_for_latest_run) - 1,
            f"terminal_indices={terminal_provider_indices} request_count={len(provider_for_latest_run)}",
        )
    if any(row["content_errors"] for row in messages if row["role"] == "assistant"):
        check("presentation_evidence_present", bool(presentation_metrics),
              "response size cannot be assessed: canonical content unavailable")
        check("large_response_delivered_incrementally", False,
              "not assessed: canonical content unavailable")
    elif len(longest_assistant_content) >= args.large_response_chars:
        mutation_count = int(
            presentation_metrics.get("body_mutation_count")
            or presentation_metrics.get("bodyMutations")
            or presentation_metrics.get("core_text_projection_count")
            or presentation_metrics.get("coreTextProjections")
            or 0
        )
        check(
            "presentation_evidence_present",
            bool(presentation_metrics),
            "receipt=" + (str(args.presentation_receipt) if args.presentation_receipt else "none"),
        )
        check(
            "large_response_delivered_incrementally",
            mutation_count >= args.min_large_response_parts,
            f"longest_assistant_chars={len(longest_assistant_content)} "
            f"presentation_mutations={mutation_count} "
            f"terminal_trace_body_parts={len(body_parts)}",
        )
    else:
        check("presentation_evidence_present", True, "not_applicable short_response")
        check(
            "large_response_delivered_incrementally",
            True,
            f"not_applicable final_chars={len(final_content)}",
        )

    multi_call_batches = model_tool_batches(messages)
    overlapping_batches = 0
    serialized_batches = 0
    for batch in multi_call_batches:
        intervals = []
        for call_id in batch:
            start = started.get(call_id)
            finish = completed.get(call_id)
            if start is not None and finish is not None:
                intervals.append((str(start["emitted_at"]), str(finish["emitted_at"])))
        if len(intervals) < 2:
            continue
        intervals.sort()
        overlap = any(intervals[index + 1][0] < intervals[index][1] for index in range(len(intervals) - 1))
        if overlap:
            overlapping_batches += 1
        else:
            serialized_batches += 1
    check(
        "model_tool_batches_overlap",
        args.allow_serialized_model_tool_batch or not multi_call_batches or overlapping_batches > 0,
        f"multi_call_batches={len(multi_call_batches)} "
        f"overlapping_batches={overlapping_batches} "
        f"serialized_batches={serialized_batches}",
    )

    metrics = {
        "canonical_reader_query_count": canonical_reader.query_count if canonical_reader else 0,
        "canonical_reader_utf8_bytes": canonical_reader.total_bytes if canonical_reader else 0,
        "canonical_reader_sha256": canonical_reader.binary_sha256 if canonical_reader else None,
        "message_count": len(messages),
        "distinct_message_timestamps": len(timestamps),
        "run_count": len(runs),
        "trace_count": len(traces),
        "tool_started_count": len(started),
        "tool_completed_count": len(completed),
        "tool_ledger_count": len(tool_ledger),
        "body_part_count": len(body_parts),
        "presentation_metrics": presentation_metrics,
        "final_character_count": len(final_content),
        "latest_turn_character_count": len(latest_turn_content),
        "latest_trace_character_count": len(latest_trace_body),
        "latest_run_provider_request_count": len(provider_for_latest_run),
        "latest_run_terminal_provider_answer_count": len(terminal_provider_indices),
        "longest_assistant_character_count": len(longest_assistant_content),
        "final_prefix": final_content[:240],
        "longest_assistant_prefix": longest_assistant_content[:240],
    }
    failed = [assertion for assertion in assertions if not assertion.passed]
    receipt = {
        "schema_version": 1,
        "kind": "persisted_runtime_session_audit",
        "database": str(args.db),
        "session_id": session_id,
        "passed": not failed,
        "assertions": [asdict(assertion) for assertion in assertions],
        "metrics": metrics,
    }
    rendered = json.dumps(receipt, ensure_ascii=False, indent=2)
    if args.receipt:
        args.receipt.parent.mkdir(parents=True, exist_ok=True)
        args.receipt.write_text(rendered + "\n", encoding="utf-8")
    print(rendered)
    return 0 if not failed else 1


if __name__ == "__main__":
    raise SystemExit(main())
