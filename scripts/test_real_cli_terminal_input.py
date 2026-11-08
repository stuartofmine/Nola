#!/usr/bin/env python3
"""Real-model PTY input acceptance in a fresh workspace and database."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import tempfile
import uuid

from test_real_cli_dirty_audit import run_cli
from test_real_cli_commit_followup import assert_completed_turn


def verify_delivery(rows, token):
    expected = (token + "\n").encode()
    delivered = []
    for task_id, owner, status, raw in rows:
        assert owner == "terminal-input-acceptance", "Task escaped conversation ownership"
        assert status == "completed", f"Task {task_id} is not successfully terminal: {status}"
        metadata = json.loads(raw)
        for receipt in metadata.get("_nola_terminal_input_receipts", {}).values():
            assert receipt["phase"] == "delivered", "Unresolved or duplicate input is not successful delivery"
            if receipt["phase"] == "delivered":
                assert receipt["size_bytes"] == len(expected)
                assert receipt["acknowledged_bytes"] == len(expected)
                assert receipt["content_digest"] == hashlib.sha256(expected).hexdigest()
                assert receipt["identity"]["task_id"] == task_id
                assert receipt["identity"]["owner_session_id"] == owner
                delivered.append(receipt)
    assert len(delivered) == 1, "Expected exactly one persisted delivered input"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--long-output", action="store_true", help="Exercise PTY output beyond preview capacity and Unicode input")
    args = parser.parse_args()
    root = Path(tempfile.mkdtemp(prefix="nola-terminal-input-"))
    workspace = root / "workspace"
    workspace.mkdir()
    step = root / "turn-1"
    step.mkdir()
    token = ("terminal-中文🙂-" if args.long_output else "terminal-") + uuid.uuid4().hex
    # A bounded reader means an abandoned model run cannot leave an indefinite
    # waiting shell. The model must deliver through the terminal, not a pipe.
    command = "IFS= read -r -t 45 answer && printf '%s' \"$answer\" > terminal-result.txt"
    if args.long_output:
        command = "for ((i=0;i<256;i++)); do printf '%0512d\\n' 0; done; " + command
    prompt = (f"在当前目录测试一次交互终端。以后台PTY启动此bash命令，timeout_sec=60：{command}\n"
              f"启动后通过terminal_input向同一任务发送 {token} 加换行，随后查询直到任务完成。"
              "只启动一次。不要将输入拼进启动命令，不要用管道或其他文件写入工具代替终端输入。"
              "若调用失败，查询原任务并报告失败，不重启任务。")
    llm = json.loads((Path.home() / ".nola/config.json").read_text())["llm"]
    database = root / "fixture.db"
    env = dict(os.environ, NOLA_DB_PATH=str(database), NOLA_LONG_TERM_MEMORY_ENABLED="0",
               NOLA_CLI_INTERACTION_MODE="never")
    invocation = [str(args.binary.resolve()), prompt, "--project", str(workspace),
                  "--allow-dir", str(workspace), "--session", "terminal-input-acceptance",
                  "--yes", "--max-iterations", "20", "--json", "--endpoint", llm["endpoint"],
                  "--model", llm["model"], "--api-key", llm["api_key"]]
    print(f"Isolated evidence: {root}", flush=True)
    code = run_cli(Path(__file__).resolve().parents[1], step, invocation, env, 180, llm["endpoint"])
    assert code == 0, "CLI failed; preserve evidence"
    events = [json.loads(line) for line in (step / "events.ndjson").read_text().splitlines()]
    assert (workspace / "terminal-result.txt").read_bytes() == token.encode()
    with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as db:
        rows = db.execute("SELECT id,owner_session_id,status,metadata_json FROM durable_tasks WHERE kind='shell_process'").fetchall()
    assert len(rows) == 1, "Expected one original shell task, no restart"
    verify_delivery(rows, token)
    if args.long_output:
        metadata = json.loads(rows[0][3])
        output_path = Path(metadata["stdoutPath"])
        # Only inspect the capture belonging to this isolated fixture.
        assert output_path.resolve().is_relative_to(root.resolve())
        capture = output_path.read_bytes().replace(b"\r\n", b"\n")
        expected_line = b"0" * 512 + b"\n"
        assert capture.startswith(expected_line * 256), "PTY output was dropped or reordered"
        assert capture.count(expected_line) == 256, "PTY output was duplicated"
        assert len(capture) > 128 * 1024
    # Independent execution evidence determines success; explicit model
    # assessment is optional, but incomplete/failed runs remain unacceptable.
    print(json.dumps({"execution_postconditions_passed": True, "task_id": rows[0][0],
                      "evidence": str(root)}), flush=True)
    assert_completed_turn(events)
    print(json.dumps({"passed": True, "task_id": rows[0][0], "evidence": str(root)}))


if __name__ == "__main__":
    main()
