#!/usr/bin/env python3
"""Summarize canonical Nola Part and Trace events for real E2E assertions."""

from __future__ import annotations

import json
import sqlite3
from pathlib import Path
import sys


def load_events(path: Path) -> list[dict]:
    text = path.read_text(encoding="utf-8")
    stripped = text.lstrip()
    if stripped.startswith("[") or stripped.startswith("{"):
        try:
            value = json.loads(text)
        except json.JSONDecodeError:
            value = None
        if isinstance(value, list):
            return [item for item in value if isinstance(item, dict)]
        if isinstance(value, dict):
            nested = []
            for item in value.values():
                if isinstance(item, list):
                    nested.extend(entry for entry in item if isinstance(entry, dict))
            if nested:
                return nested
    result = []
    for line_number, line in enumerate(text.splitlines(), 1):
        if not line.strip():
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError as error:
            raise ValueError(f"invalid NDJSON at {path}:{line_number}: {error}") from error
        if isinstance(value, dict):
            result.append(value)
    return result


def committed_journal_parts(events: list[dict], database: Path) -> list[dict]:
    """Read full immutable bodies at the notified durable boundary, never previews.

    This is an isolated acceptance reader, not a cryptographic seal verifier.
    Refuse newer Store heads instead of mixing a notification with future data.
    """
    notifications = {}
    for event in events:
        if event.get("type") == "assistant_journal_committed_v1":
            boundary = event["through"]
            key = json.dumps(boundary["identity"], sort_keys=True)
            if int(boundary["sequence"]) >= int(notifications.get(key, {}).get("sequence", 0)):
                notifications[key] = boundary
    parts = []
    with sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True) as connection:
        connection.execute("BEGIN")
        owners = dict(connection.execute("SELECT DISTINCT owner, owner FROM assistant_journal_records"))
        for boundary in notifications.values():
            owner = next((owner for owner in owners if json.loads(owner) == boundary["identity"]), None)
            if owner is None:
                raise ValueError("notified journal owner is missing")
            rows = connection.execute("SELECT sequence, header_json FROM assistant_journal_records WHERE owner=? ORDER BY sequence", (owner,)).fetchall()
            if [row[0] for row in rows] != list(range(1, int(boundary["sequence"]) + 1)):
                raise ValueError("journal notification is not the complete current Store boundary")
            previous = None
            for sequence, header_json in rows:
                header = json.loads(header_json)
                if header["identity"] != boundary["identity"] or int(header["sequence"]) != sequence:
                    raise ValueError("journal header identity mismatch")
                if previous is not None and header["previous_record_digest"] != previous:
                    raise ValueError("journal record chain mismatch")
                previous = header["record_digest"]
            if previous != boundary["record_digest"]:
                raise ValueError("journal notification digest mismatch")
            for (raw,) in connection.execute("SELECT head_json FROM assistant_journal_parts WHERE owner=?", (owner,)):
                head = json.loads(raw)
                if head["identity"] != boundary["identity"]:
                    raise ValueError("journal part identity mismatch")
                bodies = {}
                for body in head["bodies"]:
                    root = body["content_root"]
                    chunks, seen = [], set()
                    count, size = int(body["chunk_count"]), int(body["utf8_bytes"])
                    # Empty content roots are derived identities, not rows.
                    while count > 0:
                        if root in seen:
                            raise ValueError("cyclic journal manifest")
                        seen.add(root)
                        row = connection.execute("SELECT owner,part_id,field,generation,byte_count,chunk_count,previous_root,chunk_digest FROM assistant_journal_manifests WHERE root=?", (root,)).fetchone()
                        if (row is None or row[:2] != (owner, head["part_id"])
                                or json.loads(row[2]) != body["field"]
                                or row[3] != int(body["content_generation"])
                                or row[4:6] != (size, count)):
                            raise ValueError("journal manifest identity or size mismatch")
                        if row[7] is not None:
                            chunk = connection.execute("SELECT bytes FROM assistant_journal_chunks WHERE digest=?", (row[7],)).fetchone()
                            if chunk is None:
                                raise ValueError("missing journal body chunk")
                            data = bytes(chunk[0])
                            chunks.append(data)
                            size -= len(data)
                            count -= 1
                        root = row[6]
                    if size or count:
                        raise ValueError("incomplete journal body")
                    bodies[body["field"]] = b"".join(reversed(chunks)).decode("utf-8")
                parts.append({"head": head, "bodies": bodies})
    return sorted(parts, key=lambda part: int(part["head"]["order_key"]["position"]))


def summarize(path: Path, database: Path | None = None) -> dict:
    events = load_events(path)
    completed_parts: list[tuple[int, str]] = []
    canonical_deltas: list[str] = []
    started: list[str] = []
    completed: list[str] = []
    started_calls: list[str] = []
    completed_calls: list[str] = []
    permissions = 0
    errors: list[str] = []
    done = 0
    last_tool_sequence = 0
    first_assistant_sequence: int | None = None

    for event in events:
        event_type = str(event.get("type", ""))
        sequence = int(event.get("sequence", 0) or 0)
        trace = event.get("trace") if isinstance(event.get("trace"), dict) else {}
        if isinstance(event.get("event_json"), str):
            try:
                persisted_trace = json.loads(event["event_json"])
            except json.JSONDecodeError:
                persisted_trace = None
            if isinstance(persisted_trace, dict):
                trace = persisted_trace
        if not trace and isinstance(event.get("payload"), dict) and event.get("kind"):
            trace = event
        if not trace and event.get("trace_kind"):
            trace = {
                "kind": event.get("trace_kind"),
                "sequence": event.get("sequence", 0),
                "payload": {
                    "tool_name": event.get("tool_name"),
                    "call_id": event.get("tool_call_id"),
                    "delta": event.get("text"),
                    "detail": event.get("detail"),
                },
            }
        trace_kind = str(trace.get("kind", ""))
        trace_sequence = int(trace.get("sequence", sequence) or 0)
        payload = trace.get("payload") if isinstance(trace.get("payload"), dict) else {}

        if trace_kind == "assistant.delta":
            canonical_deltas.append(str(payload.get("delta", "")))
            if first_assistant_sequence is None:
                first_assistant_sequence = trace_sequence

        if event_type == "core_part_updated":
            part = event.get("part") if isinstance(event.get("part"), dict) else {}
            if part.get("kind") == "text" and part.get("is_complete") is True:
                text = str(part.get("text", ""))
                if text:
                    completed_parts.append((int(event.get("cursor", 0) or 0), text))

        if trace_kind == "tool.started":
            started.append(str(payload.get("tool_name", "")))
            started_calls.append(str(payload.get("call_id", "")))
            last_tool_sequence = max(last_tool_sequence, trace_sequence)
        elif trace_kind == "tool.completed":
            completed.append(str(payload.get("tool_name", "")))
            completed_calls.append(str(payload.get("call_id", "")))
            last_tool_sequence = max(last_tool_sequence, trace_sequence)

        if trace_kind == "permission.requested":
            permissions += 1

        if trace_kind == "run.failed":
            errors.append(
                str(event.get("message") or payload.get("detail") or payload.get("summary") or trace_kind)
            )
        if trace_kind in {"run.completed", "run.cancelled"}:
            done += 1

    if completed_parts:
        # A canonical part may be projected repeatedly. Keep only the final text
        # for each part identity/cursor ordering rather than concatenating frames.
        assistant = completed_parts[-1][1]
    elif canonical_deltas:
        assistant = "".join(canonical_deltas)
    else:
        assistant = ""

    journal_parts = committed_journal_parts(events, database) if database is not None else []
    if journal_parts:
        assistant = "\n".join(part["bodies"]["markdown"] for part in journal_parts
                              if part["head"]["metadata"]["kind"] == "text"
                              and part["head"]["metadata"]["data"].get("is_final") is True
                              and "markdown" in part["bodies"])

    return {
        "assistant": assistant.strip().strip("`"),
        "toolsStarted": [name for name in started if name],
        "toolsCompleted": [name for name in completed if name],
        "toolCallsStarted": [call_id for call_id in started_calls if call_id],
        "toolCallsCompleted": [call_id for call_id in completed_calls if call_id],
        "permissionCount": permissions,
        "errors": errors,
        "doneCount": done,
        "terminalKinds": [
            str((event.get("trace") or event).get("kind"))
            for event in events
            if str((event.get("trace") or event).get("kind"))
            in {"run.completed", "run.cancelled", "run.failed"}
        ],
        "lastToolSequence": last_tool_sequence,
        "firstAssistantSequence": first_assistant_sequence,
        "eventCount": len(events),
        "journalParts": journal_parts,
    }


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: real_e2e_events.py EVENTS.ndjson", file=sys.stderr)
        return 2
    try:
        print(json.dumps(summarize(Path(sys.argv[1])), ensure_ascii=False, separators=(",", ":")))
    except (OSError, ValueError) as error:
        print(str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
