#!/usr/bin/env python3
"""Cancel a real PTY task, then inspect it after restarting the CLI process."""
import argparse
import json
import os
from pathlib import Path
import sqlite3
import tempfile
from urllib.parse import urlsplit

from test_real_cli_dirty_audit import run_cli
from test_real_cli_commit_followup import assert_completed_turn


def execution_metadata(raw):
    metadata = json.loads(raw)
    # Notification delivery is not process execution state.
    metadata.pop("model_notified", None)
    return metadata


def task_state(database):
    with sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True) as db:
        rows = db.execute("SELECT id,status,metadata_json FROM durable_tasks WHERE kind='shell_process'").fetchall()
        assert len(rows) == 1, "Task was missing or restarted"
        attempts = db.execute("SELECT id FROM runtime_task_attempts WHERE task_id=?", (rows[0][0],)).fetchall()
    assert len(attempts) == 1, "Cancellation/recovery created another execution attempt"
    assert rows[0][1] == "cancelled", rows[0][1]
    metadata = execution_metadata(rows[0][2])
    assert metadata["shell_execution"]["process_tree_stop"]["state"] == "confirmed", "Cancellation lacks process-tree stop confirmation"
    return (rows[0][0], rows[0][1], metadata), attempts[0][0]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    args = parser.parse_args()
    root = Path(tempfile.mkdtemp(prefix="nola-terminal-cancel-"))
    workspace = root / "workspace"
    workspace.mkdir()
    database = root / "fixture.db"
    llm = json.loads((Path.home() / ".nola/config.json").read_text())["llm"]
    env = dict(os.environ, NOLA_DB_PATH=str(database), NOLA_LONG_TERM_MEMORY_ENABLED="0",
               NOLA_CLI_INTERACTION_MODE="never")
    prompt = ("测试一次后台PTY取消。仅启动一次命令 sleep 60; printf done > should-not-exist.txt，"
              "后台PTY模式，timeout_sec=90。查询确认running后立即使用后台停止工具取消原任务，"
              "查询直到终态。不重启，不发送终端输入，不执行其他命令或写文件。报告实际取消状态。")
    print(f"Isolated evidence: {root}", flush=True)
    port = 0
    before = None
    for index in range(2):
        step = root / f"turn-{index + 1}"
        step.mkdir()
        if index:
            prompt = f"继续检查之前取消的任务 {before[0][0]} 的持久化状态。只查询，不重启、不发送输入、不启动命令。"
        command = [str(args.binary.resolve()), prompt, "--project", str(workspace),
                   "--allow-dir", str(workspace), "--session", "terminal-cancel-acceptance",
                   "--yes", "--max-iterations", "16", "--json", "--endpoint", llm["endpoint"],
                   "--model", llm["model"], "--api-key", llm["api_key"]]
        assert run_cli(Path(__file__).resolve().parents[1], step, command, env, 180, llm["endpoint"], probe_port=port) == 0
        port = urlsplit(json.loads((step / "provider.endpoint").read_text())["endpoint"]).port
        events = [json.loads(line) for line in (step / "events.ndjson").read_text().splitlines()]
        assert_completed_turn(events)
        current = task_state(database)
        (step / "task-state.json").write_text(json.dumps(current, sort_keys=True, indent=2))
        assert not (workspace / "should-not-exist.txt").exists(), "Cancelled command continued to its side effect"
        if before:
            assert current == before, "Read-only recovery changed task or created another attempt"
        before = current
    print(json.dumps({"passed": True, "evidence": str(root), "task_id": before[0][0]}))


if __name__ == "__main__":
    main()
