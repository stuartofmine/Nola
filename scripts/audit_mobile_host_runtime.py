#!/usr/bin/env python3
"""Audit durable evidence produced by the real mobile-to-Web-Agent-Host chain."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import sqlite3
import sys


SCENARIOS = {
    "android-host-tool-": ("succeeded", "run.completed", ["system_time"]),
    "android-memory-write-": ("succeeded", "run.completed", ["memory_save"]),
    "android-memory-recall-": ("succeeded", "run.completed", ["memory_search"]),
    "android-host-permission-": ("succeeded", "run.completed", ["tool_search", "delete_file"]),
}


def rows(connection: sqlite3.Connection, query: str, parameters: tuple = ()) -> list[sqlite3.Row]:
    return list(connection.execute(query, parameters))


def check(condition: bool, message: str, failures: list[str]) -> None:
    if not condition:
        failures.append(message)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--db", type=Path, required=True)
    parser.add_argument("--protected-file", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    connection = sqlite3.connect(args.db)
    connection.row_factory = sqlite3.Row
    failures: list[str] = []
    evidence: dict[str, object] = {}

    active = rows(connection, "SELECT session_id FROM agent_run WHERE status = 'running'")
    check(not active, f"unfinished Agent runs remain: {[row['session_id'] for row in active]}", failures)

    for prefix, (expected_status, terminal_kind, expected_tools) in SCENARIOS.items():
        sessions = rows(
            connection,
            "SELECT id FROM session WHERE id LIKE ? ORDER BY created_at",
            (f"{prefix}%",),
        )
        check(len(sessions) == 1, f"{prefix} expected one durable session, found {len(sessions)}", failures)
        if len(sessions) != 1:
            continue
        session_id = sessions[0]["id"]
        run_rows = rows(
            connection,
            "SELECT trace_id, status, last_sequence, terminal_reason FROM agent_run WHERE session_id = ?",
            (session_id,),
        )
        check(len(run_rows) == 1, f"{session_id} expected one Agent run", failures)
        if len(run_rows) != 1:
            continue
        run = run_rows[0]
        check(run["status"] == expected_status, f"{session_id} status is {run['status']}", failures)

        trace_rows = rows(
            connection,
            "SELECT sequence, kind, status, event_json FROM trace_event "
            "WHERE session_id = ? AND trace_id = ? ORDER BY sequence",
            (session_id, run["trace_id"]),
        )
        sequences = [int(row["sequence"]) for row in trace_rows]
        check(bool(sequences), f"{session_id} has no durable trace", failures)
        check(
            all(before < after for before, after in zip(sequences, sequences[1:])),
            f"{session_id} trace order is not strictly increasing",
            failures,
        )
        kinds = [row["kind"] for row in trace_rows]
        check(kinds.count(terminal_kind) == 1, f"{session_id} lacks one {terminal_kind}", failures)

        started: dict[str, str] = {}
        completed: dict[str, str] = {}
        permission_requested: dict[str, str] = {}
        permission_resolved: dict[str, str] = {}
        for row in trace_rows:
            if row["kind"] not in {
                "tool.started",
                "tool.completed",
                "tool.permission.requested",
                "tool.permission.resolved",
            }:
                continue
            event = json.loads(row["event_json"])
            payload = event.get("payload") or {}
            call_id = str(payload.get("call_id") or "")
            tool_name = str(payload.get("tool_name") or "")
            check(bool(call_id and tool_name), f"{session_id} has an unidentifiable tool trace", failures)
            if row["kind"] == "tool.started":
                started[call_id] = tool_name
            elif row["kind"] == "tool.completed":
                completed[call_id] = tool_name
            elif row["kind"] == "tool.permission.requested":
                permission_requested[call_id] = tool_name
            else:
                permission_resolved[call_id] = row["status"]
        check(
            all(call_id in completed for call_id in started),
            f"{session_id} has a started tool without a terminal result",
            failures,
        )
        for call_id, tool_name in completed.items():
            if call_id in started:
                check(started[call_id] == tool_name, f"{session_id} tool identity changed", failures)
                continue
            check(
                permission_requested.get(call_id) == tool_name
                and permission_resolved.get(call_id) in {"failed", "cancelled"},
                f"{session_id} has an orphan terminal tool call {tool_name}",
                failures,
            )

        ledger = rows(
            connection,
            "SELECT tool_name, status FROM tool_event WHERE session_id = ? ORDER BY started_at",
            (session_id,),
        )
        ledger_names = [row["tool_name"] for row in ledger]
        check(len(ledger) == len(completed), f"{session_id} trace/tool ledger counts differ", failures)
        for tool in expected_tools:
            check(tool in ledger_names, f"{session_id} did not durably execute {tool}", failures)

        checkpoints = rows(
            connection,
            "SELECT revision, event_cursor, checkpoint_json FROM core_checkpoints JOIN core_checkpoint_bodies USING(conversation_id) "
            "WHERE conversation_id = ?",
            (session_id,),
        )
        check(len(checkpoints) == 1, f"{session_id} canonical checkpoint is missing", failures)
        checkpoint_reason = None
        if checkpoints:
            checkpoint = json.loads(checkpoints[0]["checkpoint_json"])
            checkpoint_reason = checkpoint.get("reason")
            check(
                (checkpoint.get("state") or {}).get("active_turn") is None,
                f"{session_id} checkpoint retains an active turn",
                failures,
            )

        evidence[prefix.rstrip("-")] = {
            "sessionId": session_id,
            "runStatus": run["status"],
            "terminalKind": terminal_kind,
            "lastSequence": run["last_sequence"],
            "traceEvents": len(trace_rows),
            "tools": ledger_names,
            "checkpointReason": checkpoint_reason,
        }

    cancel_sessions = rows(
        connection,
        "SELECT id FROM session WHERE id LIKE 'android-host-cancel-%' ORDER BY created_at",
    )
    check(len(cancel_sessions) == 1, f"android-host-cancel expected one durable session, found {len(cancel_sessions)}", failures)
    if len(cancel_sessions) == 1:
        session_id = cancel_sessions[0]["id"]
        cancel_runs = rows(
            connection,
            "SELECT trace_id, status, last_sequence, terminal_reason FROM agent_run "
            "WHERE session_id = ? ORDER BY started_at",
            (session_id,),
        )
        check(len(cancel_runs) == 2, f"{session_id} expected cancelled and recovery runs", failures)
        run_evidence = []
        expected_runs = [("cancelled", "run.cancelled"), ("succeeded", "run.completed")]
        for index, (expected_status, terminal_kind) in enumerate(expected_runs):
            if index >= len(cancel_runs):
                break
            run = cancel_runs[index]
            check(run["status"] == expected_status, f"{session_id} run {index + 1} is {run['status']}", failures)
            trace_rows = rows(
                connection,
                "SELECT sequence, kind, status, event_json FROM trace_event "
                "WHERE trace_id = ? ORDER BY sequence",
                (run["trace_id"],),
            )
            sequences = [int(row["sequence"]) for row in trace_rows]
            check(bool(sequences), f"{session_id} run {index + 1} has no durable trace", failures)
            check(
                all(before < after for before, after in zip(sequences, sequences[1:])),
                f"{session_id} run {index + 1} trace order is not strictly increasing",
                failures,
            )
            kinds = [row["kind"] for row in trace_rows]
            check(kinds.count(terminal_kind) == 1, f"{session_id} run {index + 1} lacks one {terminal_kind}", failures)
            tools = [
                json.loads(row["event_json"]).get("payload", {}).get("tool_name")
                for row in trace_rows
                if row["kind"] == "tool.completed"
            ]
            if index == 0:
                check(tools == ["bash"], f"{session_id} cancelled run did not terminate bash", failures)
            else:
                check(not tools, f"{session_id} recovery run unexpectedly executed tools", failures)
            run_evidence.append(
                {
                    "status": run["status"],
                    "terminalKind": terminal_kind,
                    "traceEvents": len(trace_rows),
                    "tools": tools,
                }
            )
        checkpoints = rows(
            connection,
            "SELECT revision, event_cursor, checkpoint_json FROM core_checkpoints JOIN core_checkpoint_bodies USING(conversation_id) "
            "WHERE conversation_id = ?",
            (session_id,),
        )
        check(len(checkpoints) == 1, f"{session_id} canonical checkpoint is missing", failures)
        checkpoint_reason = None
        if checkpoints:
            checkpoint = json.loads(checkpoints[0]["checkpoint_json"])
            checkpoint_reason = checkpoint.get("reason")
            check(
                (checkpoint.get("state") or {}).get("active_turn") is None,
                f"{session_id} checkpoint retains an active turn after recovery",
                failures,
            )
        evidence["android-host-cancel"] = {
            "sessionId": session_id,
            "runs": run_evidence,
            "checkpointReason": checkpoint_reason,
        }

    for backend in ("codex", "claude_code", "opencode"):
        prefix = f"android-external-{backend}"
        external_sessions = rows(
            connection,
            "SELECT id FROM session WHERE id LIKE ? ORDER BY created_at",
            (prefix + "-%",),
        )
        check(
            len(external_sessions) == 1,
            f"{prefix} expected one durable session, found {len(external_sessions)}",
            failures,
        )
        if len(external_sessions) != 1:
            continue
        session_id = external_sessions[0]["id"]
        messages = rows(
            connection,
            "SELECT position, role, content_json FROM message WHERE session_id = ? ORDER BY position",
            (session_id,),
        )
        check(len(messages) == 4, f"{session_id} expected four transcript messages", failures)
        check(
            [row["role"] for row in messages] == ["user", "assistant", "user", "assistant"],
            f"{session_id} external transcript order is invalid",
            failures,
        )
        check(
            all(json.loads(row["content_json"]).get("content") for row in messages),
            f"{session_id} external transcript contains empty content",
            failures,
        )
        external_runs = rows(
            connection,
            "SELECT trace_id, status FROM agent_run WHERE session_id = ? ORDER BY started_at",
            (session_id,),
        )
        check(len(external_runs) == 2, f"{session_id} expected two external runs", failures)
        run_evidence = []
        for run in external_runs:
            check(run["status"] == "succeeded", f"{session_id} external run is {run['status']}", failures)
            traces = rows(
                connection,
                "SELECT sequence, kind FROM trace_event WHERE trace_id = ? ORDER BY sequence",
                (run["trace_id"],),
            )
            sequences = [int(row["sequence"]) for row in traces]
            check(bool(sequences), f"{session_id} external run has no trace", failures)
            check(
                all(before < after for before, after in zip(sequences, sequences[1:])),
                f"{session_id} external trace order is invalid",
                failures,
            )
            check(
                [row["kind"] for row in traces].count("run.completed") == 1,
                f"{session_id} external run lacks one terminal event",
                failures,
            )
            run_evidence.append({"status": run["status"], "traceEvents": len(traces)})
        evidence[prefix] = {
            "sessionId": session_id,
            "messageCount": len(messages),
            "runs": run_evidence,
        }

    memories = rows(
        connection,
        "SELECT id, kind, content FROM memory WHERE content LIKE '%%ANDROID-REPORT-LABEL-%%'",
    )
    check(len(memories) == 1, f"expected one durable Android preference memory, found {len(memories)}", failures)
    check(
        bool(memories) and memories[0]["kind"] == "preference",
        "Android memory was not stored as a preference",
        failures,
    )

    permission_session = next(
        (value["sessionId"] for key, value in evidence.items() if key == "android-host-permission"),
        None,
    )
    if permission_session:
        permission_trace = rows(
            connection,
            "SELECT kind, status FROM trace_event WHERE session_id = ? "
            "AND kind IN ('tool.permission.requested', 'tool.permission.resolved') ORDER BY sequence",
            (permission_session,),
        )
        check(
            [(row["kind"], row["status"]) for row in permission_trace]
            == [("tool.permission.requested", "waiting"), ("tool.permission.resolved", "failed")],
            "permission deny lifecycle is incomplete",
            failures,
        )

    check(args.protected_file.is_file(), "permission denial did not preserve the protected file", failures)
    if args.protected_file.is_file():
        check(
            args.protected_file.read_text(encoding="utf-8") == "KEEP_UNTIL_PERMISSION_DENIED\n",
            "protected file content changed despite permission denial",
            failures,
        )

    audit_assertions = {
        "no_runtime_failures": not failures,
        "all_android_scenarios_verified": len(evidence) == len(SCENARIOS) + 2,
        "memory_persisted_once": len(memories) == 1,
        "protected_file_preserved": args.protected_file.is_file(),
    }
    receipt = {
        "schemaVersion": 1,
        "passed": all(audit_assertions.values()),
        "assertions": audit_assertions,
        "database": str(args.db.resolve()),
        "scenarioCount": len(evidence),
        "memoryCount": len(memories),
        "scenarios": evidence,
        "failures": failures,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(receipt, ensure_ascii=False, indent=2))
    return 0 if receipt["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
