#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Bounded LAN-model child cancellation, owner rejection and cold observation.

This is NOT worker-kill recovery or cautious permission-reuse acceptance.
The CLI exits normally between turns; --yes cannot grant foreign task ownership.
"""
import argparse
import json
import os
from pathlib import Path
import sqlite3
import tempfile
import uuid
from urllib.parse import urlsplit

from test_real_cli_dirty_audit import run_cli
from test_real_cli_commit_followup import assert_completed_turn
from e2e.real_overwrite_permission import isolated_environment, scrub_case_credentials


def snapshot(database):
    with sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True) as db:
        tasks = db.execute("SELECT id,owner_session_id,status,child_session_id,cancel_requested "
                           "FROM durable_tasks WHERE kind='agent' ORDER BY id").fetchall()
        assert len(tasks) == 1, "Expected one original child, not replacement or resumed dispatch"
        attempts = db.execute("SELECT id FROM runtime_task_attempts WHERE task_id=? ORDER BY id",
                              (tasks[0][0],)).fetchall()
    assert len(attempts) == 1, "Child never ran or another attempt was dispatched"
    assert tasks[0][2] == "cancelled" and tasks[0][4], "No actual cancellation terminal"
    assert tasks[0][3], "Missing persisted child conversation identity"
    return {"task": list(tasks[0]), "attempts": [row[0] for row in attempts]}


def tool_events(events, name):
    return [event["trace"] for event in events if event.get("trace", {}).get("kind") == "tool.completed"
            and event["trace"].get("payload", {}).get("tool_name") == name]


def require_foreign_denial(database, events, foreign):
    call_ids = set()
    for name in ("read_task", "stop_task"):
        calls = tool_events(events, name)
        assert len(calls) == 1 and calls[0].get("status") == "failed", f"Foreign {name} was not rejected"
        call_ids.add(calls[0]["payload"]["call_id"])
    observed = set()
    with sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True) as db:
        for (raw,) in db.execute("SELECT payload_json FROM host_event_inbox"):
            event = json.loads(raw)
            payload = event.get("payload", {})
            if event.get("session_id") == foreign and payload.get("type") == "tool_completed" and payload.get("call_id") in call_ids:
                assert "Unknown task" in json.dumps(payload["output"]), "Denial was not the ownership boundary"
                observed.add(payload["call_id"])
    assert observed == call_ids, "Missing canonical Host denial evidence"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--config", type=Path, required=True, help="Read-only model config; 192.168 endpoint required")
    parser.add_argument("--timeout", type=int, default=180, choices=range(30, 241), metavar="30..240")
    args = parser.parse_args()
    llm = json.loads(args.config.read_text())["llm"]
    assert (urlsplit(llm["endpoint"]).hostname or "").startswith("192.168."), "LAN model required"
    root = Path(tempfile.mkdtemp(prefix="nola-agent-lifecycle-"))
    workspace = root / "workspace"
    workspace.mkdir()
    owner, foreign = "agent-owner-" + uuid.uuid4().hex, "agent-foreign-" + uuid.uuid4().hex
    # Sequential links provide bounded real child work without executing shell,
    # installing dependencies or fabricating task identity. Early completion is
    # a failed cancellation sample, not counted as cancellation success.
    files = [uuid.uuid4().hex + ".txt" for _ in range(16)]
    for index, name in enumerate(files):
        (workspace / name).write_text("next=" + (files[index + 1] if index + 1 < len(files) else "END") + "\n")
    original = {name: (workspace / name).read_bytes() for name in files}
    environment = isolated_environment(root, "allow_all")
    environment["NOLA_LONG_TERM_MEMORY_ENABLED"] = "0"
    database = Path(environment["NOLA_DB_PATH"])
    config = root / "home/.nola/config.json"
    config.parent.mkdir(parents=True, exist_ok=True)
    with os.fdopen(os.open(config, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as output:
        json.dump({"llm": llm}, output)
    print(f"Isolated evidence: {root}", flush=True)
    source, port, baseline = Path(__file__).resolve().parents[1], 0, None
    try:
        for index in range(3):
            if index == 0:
                prompt = (f"Start exactly one read_only explore sub-agent, max_turns=20, max_wall_time_seconds=90. "
                          f"Its task is to read {files[0]} and follow the next= file links in sequence to END, reporting the chain. "
                          "No shell, network, edits or additional agents. Use only the task_id returned by spawn_agent. "
                          "Read its state until running (bounded wait), then immediately stop that original task and wait at most 30 seconds "
                          "for its cancellation terminal. Do not restart, resume, replace or change its task. Report the actual state.")
            elif index == 1:
                prompt = (f"For the recorded task {baseline['task'][0]}, attempt read_task once and stop_task once. "
                          "This is an ownership rejection test in another conversation. Do not invent identity, create tasks, "
                          "read files or use another route after refusal. Report each actual rejection.")
            else:
                prompt = (f"Read the durable state of our cancelled child {baseline['task'][0]} using read_task once. "
                          "Only observe; do not spawn, resume, restart, stop, send messages or execute commands.")
            step = root / f"turn-{index + 1}"
            step.mkdir()
            session = foreign if index == 1 else owner
            command = [str(args.binary.resolve()), prompt, "--project", str(workspace), "--allow-dir", str(workspace),
                       "--session", session, "--no-skills", "--yes", "--max-iterations", "12", "--json",
                       "--endpoint", llm["endpoint"], "--model", llm["model"], "--api-key", llm["api_key"]]
            assert run_cli(source, step, command, environment, args.timeout, llm["endpoint"], probe_port=port) == 0
            port = urlsplit(json.loads((step / "provider.endpoint").read_text())["endpoint"]).port
            events = [json.loads(line) for line in (step / "events.ndjson").read_text().splitlines()]
            if index == 1:
                # An expected owner rejection may canonically block the turn;
                # this is not successful execution of the foreign operations.
                terminals = [event["trace"]["payload"].get("reason") for event in events
                             if event.get("trace", {}).get("kind") == "run.completed"]
                assert len(terminals) == 1 and terminals[0] in {"answered", "blocked"}, terminals
                assert not any(event.get("trace", {}).get("kind") in {"run.failed", "run.cancelled"} for event in events)
            else:
                assert_completed_turn(events)
            state = snapshot(database)
            assert state["task"][1] == owner
            if index == 0:
                assert len(tool_events(events, "spawn_agent")) == 1
                assert any(call.get("status") == "succeeded" for call in tool_events(events, "stop_task"))
            if index == 1:
                require_foreign_denial(database, events, foreign)
            if index == 2:
                assert any(call.get("status") == "succeeded" for call in tool_events(events, "read_task"))
            if baseline is not None:
                assert state == baseline, "Foreign control or cold observation changed execution identity/state"
                assert not tool_events(events, "spawn_agent"), "Observation re-dispatched work"
            baseline = state
            (step / "task-state.json").write_text(json.dumps(state, indent=2))
            assert all((workspace / name).read_bytes() == body for name, body in original.items())
        receipt = {"passed": True, "evidence": str(root), "task": baseline,
                   "covers": ["real_model_spawn", "owner_cancel", "foreign_read_stop_rejection", "cold_read_no_redispatch"],
                   "does_not_cover": ["worker_kill_recovery", "cautious_permission_reuse", "all_client_ui"]}
        (root / "receipt.json").write_text(json.dumps(receipt, indent=2))
        print(json.dumps(receipt))
    finally:
        scrub_case_credentials(root)


if __name__ == "__main__":
    main()
