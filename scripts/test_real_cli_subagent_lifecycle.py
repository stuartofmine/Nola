#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Real LAN-model delegation/cancellation, foreign owner denial and cold query.

This does NOT kill a running Host or prove running-crash recovery. Each CLI turn
exits normally; the last turn reopens an already cancelled task in a new process.
All databases/workspaces/evidence are isolated and deliberately retained.
"""
import argparse
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import sqlite3
import tempfile
from urllib.parse import urlsplit
from concurrent.futures import ThreadPoolExecutor
from contextlib import closing
import time

from test_real_cli_dirty_audit import run_cli
from test_real_cli_commit_followup import assert_completed_turn
from canonical_run_audit import audit_runs
from e2e.real_overwrite_permission import isolated_environment, scrub_case_credentials

OWNER = "subagent-lifecycle-owner"
FOREIGN = "subagent-lifecycle-foreign"


def verify_parent_scope(calls):
    # Optional completion metadata is neither delegation nor an external effect.
    # Permit it, but never require it to declare the acceptance successful.
    allowed = {"tool_search", "read_file", "read_task", "wait_task", "list_tasks",
               "spawn_agent", "stop_task", "tool_output_read", "tool_output_search",
               "requirements_update"}
    assert all(call["payload"].get("tool_name") in allowed for call in calls), \
        "Parent used an out-of-scope tool instead of delegating/querying the original task"


def resume_steps(root):
    """Only reuse a contiguous recorded prefix; each reused turn is revalidated."""
    assert root.is_dir() and (root / "fixture.db").is_file() and (root / "workspace").is_dir()
    seen_missing = False
    for index in range(3):
        step = root / f"turn-{index + 1}"
        if not step.exists():
            seen_missing = True
        else:
            assert not seen_missing, "Cannot resume an evidence directory with a missing earlier turn"
            assert (step / "events.ndjson").is_file() and (step / "provider.endpoint").is_file(), \
                "Incomplete prior turn must not be skipped or overwritten"


def completed_calls(events):
    return [event["trace"] for event in events
            if event.get("trace", {}).get("kind") == "tool.completed"]


def require_calls(calls, name, task_id=None, success=True):
    matches = [call for call in calls if call["payload"].get("tool_name") == name
               and (task_id is None or call["payload"].get("arguments", {}).get("task_id") == task_id)]
    assert matches, f"No actual {name} call for the observed task"
    assert all((call.get("status") == "succeeded") == success for call in matches), matches
    return matches


def terminal_snapshots(database, count):
    with closing(sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True)) as db:
        db.row_factory = sqlite3.Row
        db.execute("BEGIN")
        tasks = [dict(row) for row in db.execute("SELECT * FROM durable_tasks WHERE kind='agent' ORDER BY id")]
        assert len(tasks) == count, "Expected exact delegated agent count; no replay or replacement"
        snapshots = []
        for task in tasks:
            assert task["owner_session_id"] == OWNER and task["child_session_id"] not in (None, OWNER, FOREIGN)
            assert task["status"] == "sealed" and task["cancel_requested"], "Cancellation must be settled, not merely requested"
            receipts = db.execute("SELECT receipt_json FROM runtime_task_terminal_receipts WHERE task_id=?", (task["id"],)).fetchall()
            assert len(receipts) == 1, "Missing canonical terminal receipt"
            receipt = json.loads(receipts[0][0])
            verify_cancelled_receipt(receipt, task["id"])
            attempts = [dict(row) for row in db.execute("SELECT * FROM runtime_task_attempts WHERE task_id=? ORDER BY id", (task["id"],))]
            assert len(attempts) == 1, "A cancelled task must not acquire another execution attempt"
            snapshots.append({"task_id": task["id"], "owner": task["owner_session_id"], "child": task["child_session_id"],
                              "status": task["status"], "receipt": receipt, "attempts": attempts})
        assert len({task["child"] for task in snapshots}) == count
        return snapshots


def observe_running_children(database, workspace, prefixes, expected_count):
    if not database.exists() or not all((workspace / f"{prefix}-started.txt").exists() for prefix in prefixes):
        return None
    with closing(sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True)) as db:
        rows = db.execute("SELECT id,child_session_id,status,cancel_requested FROM durable_tasks WHERE kind='agent' ORDER BY id").fetchall()
    if len(rows) != expected_count or any(row[2] != "running" or row[3] for row in rows):
        return None
    assert not any((workspace / f"{prefix}-finished.txt").exists() for prefix in prefixes)
    return {"observed_at_ns": time.time_ns(), "tasks": rows, "started_markers": prefixes}


def verify_cancelled_receipt(receipt, task_id):
    value = receipt["journal_result"]["seal"]["result"]["value"]
    assert value["kind"] == "cancelled", "A sealed answer/failure is not cancellation"
    assert value["cancellation_id"] == f"task-user-input-cancel:{task_id}"
    assert not value["unknown_effect_refs"], "Cancellation with unresolved effects is not a confirmed stop"


def verify_foreign(calls, task_id, outputs):
    denied = require_calls(calls, "read_task", task_id, success=False)
    denied += require_calls(calls, "stop_task", task_id, success=False)
    for call in denied:
        output = outputs[call["payload"]["call_id"]]
        assert set(output) == {"contract", "data", "presentation", "execution", "full_output_ref", "memory_usage"}, \
            "Expected the current typed Host tool-result envelope"
        assert output["contract"] == "nola.tool_result.v2", "Unsupported tool-result contract"
        presentation = output["presentation"]
        assert isinstance(presentation, dict) and set(presentation) == {"text", "summary"}
        assert isinstance(presentation["text"], str) and isinstance(presentation["summary"], str)
        # task_error currently returns no business data and puts the exact
        # scope rejection in presentation.text; trace.status proves failure.
        # A transport failure with similar prose is not this tool rejection.
        assert output["data"] is None and presentation["text"] == f"Unknown task: {task_id}", \
            "Transport failure or generic error is not ownership isolation"
    assert not any(call["payload"].get("tool_name") in {"spawn_agent", "send_agent_message"} for call in calls)


def stored_outputs(database, session):
    with closing(sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True)) as db:
        outputs = {}
        for (raw,) in db.execute("SELECT payload_json FROM host_event_inbox"):
            event = json.loads(raw)
            payload = event.get("payload", {})
            if event.get("session_id") == session and payload.get("type") == "tool_completed":
                outputs[payload["call_id"]] = payload["output"]
        return outputs


def selected_route(config, environment):
    """Use only the current connection catalog; never infer capabilities."""
    llm = config["llm"]
    selected = llm["default_model"]
    connection = next(item for item in llm["connections"] if item["id"] == selected["connection"])
    model = next(item for item in connection["models"] if item["id"] == selected["model"])
    if connection["protocol"] != "chat_completions":
        raise ValueError("This streaming probe requires an explicitly configured chat_completions connection")
    key_name = connection.get("api_key_env")
    if key_name and not environment.get(key_name):
        raise ValueError("Selected connection credential environment is unavailable")
    return {"endpoint": connection["endpoint"], "model": model["id"],
            "api_key": environment.get(key_name, "") if key_name else "", "capabilities": model.get("capabilities")}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--resume-evidence", type=Path,
                        help="Revalidate recorded turns, then run only absent turns; never overwrite a failed turn")
    parser.add_argument("--agents", type=int, choices=(1, 2), default=1)
    parser.add_argument("--config", type=Path, default=Path.home() / ".nola/config.json")
    args = parser.parse_args()
    llm = selected_route(json.loads(args.config.read_text()), os.environ)
    host = urlsplit(llm["endpoint"]).hostname
    assert ipaddress.ip_address(host) in ipaddress.ip_network("192.168.0.0/16"), "Use the configured 192.168 LAN model"
    root = args.resume_evidence.resolve() if args.resume_evidence else Path(tempfile.mkdtemp(prefix="nola-subagent-lifecycle-"))
    if args.resume_evidence:
        resume_steps(root)
    root.chmod(0o700)
    workspace = root / "workspace"
    if not args.resume_evidence:
        workspace.mkdir()
    database = root / "fixture.db"
    env = isolated_environment(root, "allow_all")
    env.update(NOLA_DB_PATH=str(database), NOLA_LONG_TERM_MEMORY_ENABLED="0")
    if llm["capabilities"] is not None:
        (workspace / "nola.json").write_text(json.dumps({"model_capabilities": llm["capabilities"]}))
    prompt = (
        "在当前隔离目录验收一次真实子代理取消。只能spawn_agent一次，general，permission_mode=allow，"
        "max_turns=4，max_wall_time_seconds=150。给子代理任务：只运行一次前台bash命令 "
        "printf started > child-started.txt; sleep 90; printf finished > child-finished.txt，timeout_sec=120；"
        "不要改别的文件，不要派生更多代理。你自己不要运行命令或写文件。"
        "取得spawn_agent真实返回的task_id，用read_task及read_file确认child-started.txt已存在后，"
        "立即stop_task取消同一子代理，再read_task或有界wait_task直到取消终态。"
        "不要等它自然完成，不要重启或继续该任务。报告真实状态。"
    )
    prefixes = ["child"] if args.agents == 1 else ["child-0"]
    if args.agents == 2:
        chain = workspace / "read-chain"
        if not args.resume_evidence:
            chain.mkdir()
            for index in range(20):
                (chain / f"step-{index}.txt").write_text(
                    f"Record {index}: inspect this record, then read step-{index + 1}.txt next." if index < 19 else "Final record.")
        prompt = ("在当前隔离目录验收两个并发子代理取消。只调用 spawn_agent 两次，不能重复或替换。"
                  "第一个 general，permission_mode=allow，max_turns=4，max_wall_time_seconds=300。"
                  "它只运行一次前台 bash：printf started > child-0-started.txt; sleep 180; printf finished > child-0-finished.txt，timeout_sec=210。"
                  "第二个 explore，permission_mode=read_only，max_turns=20，max_wall_time_seconds=300。"
                  "它从 read-chain/step-0.txt 开始，每次仅 read_file 一个文件，依据文件里的下一条路径顺序读取，"
                  "逐项分析记录直至末尾；不能批量预读、用其他工具或派生代理。"
                  "同项目只能有一个可写子代理，第二个必须是 read_only。你自己不能执行命令或写文件。"
                  "先启动两个子代理，不要等待第一个结束才启动第二个。用 read_file 确认 child-0-started.txt 存在，"
                  "再 read_task 确认两个任务在运行，立即对两个真实 task_id 分别调用 stop_task。"
                  "最后 read_task 或有界 wait_task 确认两个取消终态，不等自然完成，不重启。报告真实状态。")
    print(f"Isolated evidence: {root}", flush=True)
    report = {"passed": False, "agents": args.agents, "completed_turns": [],
              "binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(),
              "running_host_crash_recovery_tested": False}
    try:
        baseline = None
        port = 0
        for index, session in enumerate([OWNER, FOREIGN, OWNER]):
            step = root / f"turn-{index + 1}"
            reuse = step.exists()
            if index == 1:
                prompt = (f"这是另一个会话的所有权隔离验收。上一轮真实结果的任务ID是 {', '.join(task['task_id'] for task in baseline)}。"
                          "请对每个任务分别实际调用read_task和stop_task各一次验证本会话无权读取和取消，报告工具拒绝。"
                          "不要用shell/数据库/文件绕过，不要启动代理或发消息，也不要猜其他ID。")
            elif index == 2:
                prompt = ("继续只读检查我们之前已取消的子代理。用list_tasks发现持久化任务，再read_task查询其真实终态。"
                          "不要启动、接续、取消、发消息或执行任何命令。报告原任务是否仍已取消。")
            command = [str(args.binary.resolve()), prompt, "--project", str(workspace), "--allow-dir", str(workspace),
                       "--session", session, "--yes", "--max-iterations", "20", "--json", "--endpoint", llm["endpoint"],
                       "--model", llm["model"], "--api-key", llm["api_key"]]
            if not reuse:
                step.mkdir()
                with ThreadPoolExecutor(max_workers=1) as executor:
                    run = executor.submit(run_cli, Path(__file__).resolve().parents[1], step, command, env,
                                          420 if args.agents == 2 else 240, llm["endpoint"], probe_port=port)
                    while not run.done():
                        if index == 0 and args.agents == 2 and not (step / "concurrent-running.json").exists():
                            observed = observe_running_children(database, workspace, prefixes, args.agents)
                            if observed:
                                (step / "concurrent-running.json").write_text(json.dumps(observed, indent=2))
                        time.sleep(0.1)
                    assert run.result() == 0
            port = urlsplit(json.loads((step / "provider.endpoint").read_text())["endpoint"]).port
            events = [json.loads(line) for line in (step / "events.ndjson").read_text().splitlines()]
            assert_completed_turn(events)
            with closing(sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True)) as connection:
                connection.execute("BEGIN")
                canonical = audit_runs(connection, session)
                assert canonical["runCount"] == (2 if session == OWNER and (root / "turn-3").exists() else 1)
                assert all(outcome in {"answered", "action_completed"} for outcome in canonical["outcomes"])
            calls = completed_calls(events)
            verify_parent_scope(calls)
            snapshot = terminal_snapshots(database, args.agents)
            for prefix in prefixes:
                assert (workspace / f"{prefix}-started.txt").read_text() == "started", "Child never actually executed its command"
                assert not (workspace / f"{prefix}-finished.txt").exists(), "Child continued after cancellation"
            if index == 0:
                assert len(require_calls(calls, "spawn_agent")) == args.agents
                if args.agents == 2:
                    observed = json.loads((step / "concurrent-running.json").read_text())
                    assert {row[0] for row in observed["tasks"]} == {task["task_id"] for task in snapshot}
                for task in snapshot:
                    require_calls(calls, "stop_task", task["task_id"])
                    require_calls(calls, "read_task", task["task_id"])
                baseline = snapshot
            else:
                assert snapshot == baseline, "Read-only cold query/foreign owner changed execution or dispatched again"
                if index == 1:
                    outputs = stored_outputs(database, FOREIGN)
                    for task in baseline:
                        verify_foreign(calls, task["task_id"], outputs)
                else:
                    require_calls(calls, "list_tasks")
                    for task in baseline:
                        require_calls(calls, "read_task", task["task_id"])
                    assert not any(call["payload"].get("tool_name") in {"spawn_agent", "stop_task", "send_agent_message"} for call in calls)
            state_file = step / "task-state.json"
            if state_file.exists():
                assert json.loads(state_file.read_text()) == snapshot, "Persisted acceptance baseline changed"
            else:
                state_file.write_text(json.dumps(snapshot, sort_keys=True, indent=2))
            report["completed_turns"].append(index + 1)
        report["passed"] = True
        report.update(evidence=str(root), task_ids=[task["task_id"] for task in baseline],
                      owner_isolation=True, cancelled_cold_query_no_redispatch=True)
    except Exception as error:
        report["failure_type"] = type(error).__name__
        raise
    finally:
        try:
            scrub_case_credentials(root)
        except Exception as error:
            report["passed"] = False
            report["failure_type"] = type(error).__name__
            raise
        finally:
            (root / "result.json").write_text(json.dumps(report, indent=2))
    print(json.dumps(report))


if __name__ == "__main__":
    main()
