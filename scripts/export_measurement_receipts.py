#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Export only hashed receipt identities/statuses from an isolated experiment DB.

Usage: export_measurement_receipts.py DATABASE EXPERIMENT_INPUT_JSON
Input contains build_digest and samples[{client_id, request_id, command}].
Output goes to stdout; never exports command/result/error payloads or DB paths.
"""
import argparse
import hashlib
import json
import pathlib
import re
import sqlite3


def command_identity(client, request):
    digest = hashlib.sha256(b"nola.command.v1\0")
    for part in (client, request):
        raw = part.encode("utf-8")
        digest.update(len(raw).to_bytes(8, "big"))
        digest.update(raw)
    return digest.hexdigest()


def identity(value):
    return hashlib.sha256(value.encode("utf-8")).hexdigest() if isinstance(value, str) and value else None


def export_attempts(connection, run_ids):
    attempts = []
    for run in sorted(run_ids):
        rows = connection.execute("""
            SELECT a.id,a.run_id,r.conversation_id,r.kind,a.effect_id,a.predecessor_id,a.state,
                   e.effect_kind,d.request_id
            FROM run_attempts a JOIN runs r ON r.id=a.run_id
            LEFT JOIN effect_outbox e ON e.effect_id=a.effect_id AND e.run_id=a.run_id
            LEFT JOIN (
                SELECT a2.id AS attempt_id,d2.request_id,a2.run_id,a2.effect_id
                FROM run_attempt_deliveries d2 JOIN run_attempts a2 ON a2.id=d2.attempt_id
                UNION
                SELECT json_extract(h.payload_json,'$.attempt_id'),h.request_id,h.run_id,h.effect_id
                FROM host_event_inbox h WHERE h.run_id=?
            ) d ON d.attempt_id=a.id AND d.run_id=a.run_id AND d.effect_id=a.effect_id
            WHERE a.run_id=? ORDER BY a.id,d.request_id
        """, (run, run))
        for attempt, owner, session, run_kind, effect, predecessor, state, kind, request in rows:
            attempts.append({"attempt": identity(attempt), "run": identity(owner), "session": identity(session),
                "run_kind": run_kind if run_kind in ("conversation", "maintenance") else "unknown",
                "effect": identity(effect), "predecessor": identity(predecessor), "state": state,
                "purpose": "conversation" if kind == "model_stream" else "compaction" if kind == "compact" else "unknown",
                "request": identity(request)})
    return attempts


def export_receipts(connection, experiment):
    build = experiment.get("build_digest")
    if not isinstance(build, str) or re.fullmatch(r"[0-9a-f]{64}", build) is None:
        raise ValueError("fixed build_digest is required")
    samples = experiment.get("samples")
    if not isinstance(samples, list):
        raise ValueError("samples must be a list")
    receipts, seen, run_ids = [], set(), set()
    # One read snapshot, never N independent snapshots while a run is updating.
    connection.execute("BEGIN")
    try:
        for sample in samples:
            client, request = sample.get("client_id"), sample.get("request_id")
            if not isinstance(client, str) or not isinstance(request, str):
                raise ValueError("receipt tuple is required for every sample")
            command = command_identity(client, request)
            if sample.get("command") != command or command in seen:
                raise ValueError("duplicate or inconsistent sample command identity")
            seen.add(command)
            row = connection.execute("""
                SELECT status,command_type,
                    json_extract(command_json,'$.input.conversation_id'),
                    json_extract(command_json,'$.input.run_id'),
                    json_extract(result_json,'$.begin.run_id')
                FROM command_receipts WHERE client_id=? AND request_id=?""",
                (client, request)).fetchone()
            status = row[0] if row else "absent"
            if status == "accepted":
                status = "recovery_required"
            if status not in ("committed", "rejected", "conflict", "recovery_required", "absent"):
                raise ValueError("unknown canonical receipt status")
            receipt = {"command": command, "status": status}
            if row and status == "committed":
                _, kind, session, input_run, new_run = row
                starts = ("begin_assistant_turn", "begin_task_assistant_turn", "begin_automation_assistant_turn", "promote_queued_assistant_turn")
                run = new_run if kind in starts else input_run if kind == "reduce_and_commit_active_submission" else None
                if isinstance(run, str) and run and isinstance(session, str) and session:
                    owner = connection.execute("SELECT conversation_id FROM runs WHERE id=?", (run,)).fetchone()
                    if owner is None or owner[0] != session:
                        receipt["run_binding"] = "unresolved"
                    else:
                        receipt.update(run=identity(run), session=identity(session),
                            run_binding="opened_run" if kind in starts else "existing_run")
                        run_ids.add(run)
                else:
                    receipt["run_binding"] = "not_available"
            receipts.append(receipt)
        return {"schema_version": 1, "build_digest": build, "receipts": receipts,
            "attempts": export_attempts(connection, run_ids)}
    finally:
        connection.rollback()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("database")
    parser.add_argument("experiment")
    args = parser.parse_args()
    experiment = json.loads(pathlib.Path(args.experiment).read_bytes())
    uri = pathlib.Path(args.database).resolve().as_uri() + "?mode=ro"
    with sqlite3.connect(uri, uri=True) as connection:
        connection.execute("PRAGMA query_only=ON")
        print(json.dumps(export_receipts(connection, experiment), sort_keys=True, indent=2))


if __name__ == "__main__":
    main()
