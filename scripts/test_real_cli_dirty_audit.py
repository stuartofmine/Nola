#!/usr/bin/env python3
"""Real-model audit of an isolated copy of the actual dirty worktree.

Unlike the five-function smoke fixture, retain HEAD and all non-ignored local
changes. Never execute the model against the user's original worktree/database.
Credentials are read from existing configuration and are never printed.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import shutil
import signal
import sqlite3
import subprocess
import tempfile
import time
from canonical_run_audit import audit_runs


def git(root: pathlib.Path, *args: str) -> bytes:
    return subprocess.check_output(["git", "-C", str(root), *args])


def fingerprint(root: pathlib.Path) -> dict[str, str]:
    names = git(root, "ls-files", "-z", "--cached", "--others", "--exclude-standard").split(b"\0")
    result = {}
    for raw in names:
        if not raw:
            continue
        name = os.fsdecode(raw)
        path = root / name
        if path.is_symlink():
            result[name] = "link:" + os.readlink(path)
        elif path.is_file():
            result[name] = hashlib.sha256(path.read_bytes()).hexdigest()
        else:
            result[name] = "absent"
    return result


def run_cli(source, run, command, env, timeout, upstream, probe_port=0):
    ready = run / "provider.endpoint"
    with (run / "provider-probe.log").open("w") as probe_log:
        probe = subprocess.Popen(["python3", str(source / "scripts/e2e/provider_probe.py"),
            "--upstream", upstream, "--log", str(run / "provider.ndjson"), "--ready", str(ready),
            "--upstream-timeout-seconds", "60", "--port", str(probe_port)], stdout=probe_log, stderr=probe_log)
        try:
            for _ in range(100):
                if ready.exists() or probe.poll() is not None:
                    break
                time.sleep(0.05)
            assert ready.exists(), f"Provider probe failed: {run}"
            command[command.index("--endpoint") + 1] = json.loads(ready.read_text())["endpoint"]
            with (run / "events.ndjson").open("w") as out, (run / "stderr.log").open("w") as err:
                process = subprocess.Popen(command, env=env, stdout=out, stderr=err)
                try:
                    return process.wait(timeout=timeout)
                except subprocess.TimeoutExpired:
                    process.send_signal(signal.SIGINT)
                    try:
                        process.wait(timeout=15)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
                    raise AssertionError(f"Audit did not converge; preserved evidence: {run}") from None
        finally:
            probe.terminate()
            probe.wait(timeout=5)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=pathlib.Path, required=True)
    parser.add_argument("--timeout", type=int, default=480)
    parser.add_argument("--prompt", default="只读审计当前 Nola 仓库的未提交变更，重点检查 harness 的工具协议、队列、参数纠正、结果保存和读取，以及 Mac 对话展示。不要修改文件或运行测试。给出有代码证据的问题和审计覆盖范围，不要声称未检查的文件已通过。")
    args = parser.parse_args()
    source = pathlib.Path(__file__).resolve().parents[1]
    run = pathlib.Path(tempfile.mkdtemp(prefix="nola-real-dirty-audit-"))
    workspace = run / "workspace"
    workspace.mkdir()
    print(f"Preserved audit evidence: {run}", flush=True)
    archive = subprocess.Popen(["git", "-C", str(source), "archive", "HEAD"], stdout=subprocess.PIPE)
    subprocess.run(["tar", "-x", "-C", str(workspace)], stdin=archive.stdout, check=True)
    archive.stdout.close()
    assert archive.wait() == 0
    git(workspace, "init", "--quiet")
    git(workspace, "add", "--all")
    git(workspace, "-c", "user.name=Nola fixture", "-c", "user.email=fixture@localhost",
        "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "isolated baseline")
    changed = set(git(source, "diff", "HEAD", "--name-only", "-z").split(b"\0"))
    changed.update(git(source, "ls-files", "--others", "--exclude-standard", "-z").split(b"\0"))
    for raw in sorted(changed - {b""}):
        relative = pathlib.Path(os.fsdecode(raw))
        assert not relative.is_absolute() and ".." not in relative.parts
        original, target = source / relative, workspace / relative
        if not original.exists() and not original.is_symlink():
            if target.is_file() or target.is_symlink():
                target.unlink()  # Only the generated fixture, never source data.
            continue
        target.parent.mkdir(parents=True, exist_ok=True)
        if original.is_file() or original.is_symlink():
            shutil.copy2(original, target, follow_symlinks=False)
    before = fingerprint(workspace)
    (run / "workspace-before.json").write_text(json.dumps(before, sort_keys=True))
    database = run / "fixture.db"
    config_path = pathlib.Path(os.environ.get("NOLA_CONFIG_PATH", pathlib.Path.home() / ".nola/config.json"))
    llm = json.loads(config_path.read_text())["llm"]
    # Match the Mac acceptance entrypoint's explicit provider overrides.
    llm = dict(llm)
    for field, names in {"endpoint": ("NOLA_REAL_E2E_ENDPOINT", "MODEL_API_URL"),
                         "api_key": ("NOLA_REAL_E2E_API_KEY", "MODEL_API_KEY"),
                         "model": ("NOLA_REAL_E2E_MODEL", "MODEL_NAME")}.items():
        llm[field] = next((os.environ[name] for name in names if os.environ.get(name)), llm[field])
    (run / "provider.json").write_text(json.dumps({"endpoint": llm["endpoint"], "model": llm["model"]}))
    env = dict(os.environ, NOLA_DB_PATH=str(database), NOLA_LONG_TERM_MEMORY_ENABLED="0",
               NOLA_CLI_INTERACTION_MODE="never")
    prompt = args.prompt
    command = [str(args.binary.resolve()), prompt,
               "--project", str(workspace), "--session", "dirty-audit", "--max-iterations", "35", "--json",
               "--endpoint", llm["endpoint"], "--model", llm["model"], "--api-key", llm["api_key"]]
    exit_code = run_cli(source, run, command, env, args.timeout, llm["endpoint"])
    after = fingerprint(workspace)
    (run / "workspace-after.json").write_text(json.dumps(after, sort_keys=True))
    assert before == after, f"Read-only audit modified its fixture; inspect {run}"
    db = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
    runs = db.execute("SELECT id,status FROM runs").fetchall()
    parts = db.execute("SELECT content_json FROM message_parts WHERE kind='tool' ORDER BY rowid").fetchall()
    calls = [json.loads(row[0]) for row in parts]
    summary = {"exit_code": exit_code, "runs": runs, "changed_files": len(changed - {b""}),
               "tool_parts": len(calls), "read_only": True,
               "tools": [{"capability": call.get("capability_id"), "state": call.get("state"),
                          "input": call.get("input"), "summary": call.get("summary")} for call in calls]}
    (run / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2))
    print(json.dumps({key: value for key, value in summary.items() if key != "tools"}, ensure_ascii=False), flush=True)
    assert exit_code == 0, f"Audit process failed: {run}"
    canonical = audit_runs(db, "dirty-audit")
    assert all(kind in {"answered", "action_completed"} for kind in canonical["outcomes"]), canonical
    verdict = subprocess.run(["python3", str(source / "scripts/judge_real_e2e_response.py"),
        "--config", str(config_path), "--db", str(database), "--session-id", "dirty-audit",
        "--task", prompt, "--receipt", str(run / "semantic-audit.json")])
    assert verdict.returncode == 0, f"Completed process is not sufficient: semantic audit failed; inspect {run}"


if __name__ == "__main__":
    main()
