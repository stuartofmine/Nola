#!/usr/bin/env python3
"""Real-provider multi-turn review/clarification/commit in a disposable Git repo.

Only this generated repository may be mutated. Git postconditions, not a model
judge or printed commands, determine whether the commit request succeeded.
"""
import argparse
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
from urllib.parse import urlsplit

from test_real_cli_dirty_audit import run_cli
from canonical_run_audit import audit_runs


def assert_completed_turn(events):
    reasons = [event["trace"]["payload"].get("reason") for event in events
               if event.get("trace", {}).get("kind") == "run.completed"]
    assert len(reasons) == 1, "Expected exactly one canonical completion trace"
    # Assessment is optional. Neither label proves task success: callers must
    # independently verify the requested effects and preservation constraints.
    allowed = {"answered", "action_completed"}
    assert reasons[-1] in allowed, reasons
    assert not any(event.get("trace", {}).get("kind") in {"run.failed", "run.cancelled"}
                   for event in events), "Run also emitted a failure/cancellation"


def assert_existing_files_unchanged(before, after):
    for path, content in before.items():
        assert path in after, f"Review deleted an existing file: {path}"
        assert after[path] == content, f"Review modified an existing file: {path}"


def pending_scope_question(connection, events, conversation_id):
    """Recognize a committed open question, never prose or an inferred timeout."""
    records = [event["record"] for event in events
               if event.get("type") == "user_input_wait_changed"]
    if not records:
        return None
    record = records[-1]
    wait = record["wait"]
    if wait.get("state", {}).get("type") != "open":
        return None
    starts = [event["trace"].get("run_id") for event in events
              if event.get("trace", {}).get("kind") == "run.started"]
    assert starts == [record["run_id"]], "Question must belong to this Run"
    assert record["conversation_id"] == conversation_id, "Question owner mismatch"
    assert not any(event.get("trace", {}).get("kind") in
                   {"run.completed", "run.failed", "run.cancelled"} for event in events)
    row = connection.execute("""SELECT w.revision,w.state,w.response_schema_digest,
        r.status,r.completed_at_ms,e.payload_json
        FROM user_input_wait_heads w JOIN runs r ON r.id=w.run_id
        JOIN run_events e ON e.event_id=w.source_event_id
        WHERE w.wait_id=? AND w.conversation_id=? AND w.run_id=?
          AND r.conversation_id=w.conversation_id AND e.run_id=w.run_id
          AND e.event_kind='user_input_wait_changed'""",
        (wait["wait_id"], conversation_id, record["run_id"])).fetchone()
    assert row is not None, "Question lacks persisted owner-bound evidence"
    revision, state, digest, status, completed, raw = row
    assert (revision, state, digest, status, completed) == (
        wait["revision"], "open", wait["response_schema_digest"], "waiting_input", None)
    assert json.loads(raw)["wait"] == wait, "Question differs from committed event"
    questions = wait.get("question_set", {}).get("questions", [])
    assert questions and all(isinstance(q.get("prompt"), str) and q["prompt"].strip()
                             for q in questions), "Missing actual question"
    return {"run_id": record["run_id"], "wait_id": wait["wait_id"],
            "question_ids": [q["id"] for q in questions]}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=240)
    parser.add_argument("--commit-only", action="store_true", help="Focused commit/verification acceptance from the repository root; not a three-turn review acceptance")
    parser.add_argument("--split-commits", action="store_true", help="Independent composition fixture: multiple commits with an unrelated staged source file preserved")
    args = parser.parse_args()
    source = Path(__file__).resolve().parents[1]
    run = Path(tempfile.mkdtemp(prefix="nola-commit-followup-"))
    workspace = run / "workspace"
    subdir = workspace / "apps/cli" / "scripts"
    subdir.mkdir(parents=True)

    def git(*arguments):
        return subprocess.check_output(["git", "-C", str(workspace), *arguments], stderr=subprocess.STDOUT).decode()

    git("init", "--quiet")
    git("config", "user.name", "Nola isolated test")
    git("config", "user.email", "fixture@localhost")
    git("config", "commit.gpgsign", "false")
    (workspace / "README.md").write_text("# Fixture\nInitial description.\n")
    (subdir / "note.txt").write_text("Initial note.\n")
    git("add", "-A")
    git("commit", "--quiet", "-m", "fixture baseline")
    baseline = git("rev-parse", "HEAD").strip()
    (workspace / "README.md").write_text("# Fixture\nUpdated description.\n")
    (subdir / "note.txt").write_text("Updated note.\n")
    new_source = workspace / "native_host_output.rs"
    new_source.write_text("pub fn output() -> &'static str { \"fixture\" }\n")
    excluded = {"cli/scripts/nola": b"\xcf\xfa\xed\xfe" + b"fixture executable",
                ".env": b"FIXTURE_SECRET=not-a-real-credential\n"}
    for path, content in excluded.items():
        (workspace / path).write_bytes(content)
    expected = {str(path.relative_to(workspace)): path.read_bytes()
                for path in [workspace / "README.md", subdir / "note.txt", new_source]}
    preserved_index = None
    if args.split_commits:
        (workspace / "unrelated.rs").write_text("// staged user work\n")
        git("add", "--", "unrelated.rs")
        preserved_index = git("ls-files", "--stage", "--", "unrelated.rs")
        (workspace / "unrelated.rs").write_text("// further unstaged user work\n")
    llm = json.loads((Path.home() / ".nola" / "config.json").read_text())["llm"]
    (run / "provider.json").write_text(json.dumps({key: llm[key] for key in ["endpoint", "model"]}))
    database = run / "fixture.db"
    env = dict(os.environ, NOLA_DB_PATH=str(database), NOLA_LONG_TERM_MEMORY_ENABLED="0",
               NOLA_CLI_INTERACTION_MODE="never")
    print(f"Isolated evidence: {run}; provider: {llm['model']}", flush=True)
    prompts = ["审计下当前git目录未提交的变更。不要修改或删除已有文件；允许新增审计记录，但不要暂存或提交。",
               "我指的是git根目录。请检查根目录与子目录的变更。不要修改或删除已有文件；允许新增审计记录，但不要暂存或提交。",
               "提交下，把审计开始前已有的代码和文档变更放在一次提交里，排除构建产物、本地环境文件及审计期间新增的记录，提交信息你拟定。不要创建或修改文件，不要 push。"]
    tasks = list(enumerate(prompts))
    if args.split_commits:
        tasks = [(2, "把 README.md、cli/scripts/note.txt 和 native_host_output.rs 的变更分多次提交，怎么分你判断，提交信息你拟定。其他文件和已有的其他暂存内容保持原样，不要 push。")]
    elif args.commit_only:
        tasks = tasks[2:]
    # Session configuration owns its endpoint. Reuse the probe address across
    # turns instead of silently pointing restored sessions at a dead listener.
    probe_port = 0
    # User-owned protection is task-scoped, not redefined at each follow-up.
    task_baseline = {str(path.relative_to(workspace)): path.read_bytes()
                     for path in workspace.rglob("*")
                     if path.is_file() and ".git" not in path.relative_to(workspace).parts}
    for index, prompt in tasks:
        before_index = git("ls-files", "--stage")
        before_files = {str(path.relative_to(workspace)): path.read_bytes()
                        for path in workspace.rglob("*")
                        if path.is_file() and ".git" not in path.relative_to(workspace).parts}
        step = run / f"turn-{index + 1}"
        step.mkdir()
        command = [str(args.binary.resolve()), prompt, "--project", str(workspace if args.commit_only or args.split_commits else subdir),
                   "--allow-dir", str(workspace), "--session", "commit-followup", "--yes",
                   "--max-iterations", "20", "--json", "--endpoint", llm["endpoint"],
                   "--model", llm["model"], "--api-key", llm["api_key"]]
        code = run_cli(source, step, command, env, args.timeout, llm["endpoint"], probe_port=probe_port)
        probe_port = urlsplit(json.loads((step / "provider.endpoint").read_text())["endpoint"]).port
        assert code == 0, f"Turn {index + 1} failed; evidence: {step}"
        provider_events = [json.loads(line) for line in (step / "provider.ndjson").read_text().splitlines()]
        for event in provider_events:
            if event.get("toolChoice") == "none":
                assert "user" in event.get("messageRoles", []), "Host dropped review evidence"
        events = [json.loads(line) for line in (step / "events.ndjson").read_text().splitlines()]
        with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as db:
            pending = pending_scope_question(db, events, "commit-followup") if index == 2 else None
            if pending is None:
                assert_completed_turn(events)
                canonical = audit_runs(db, "commit-followup")
                assert all(kind in {"answered", "action_completed"} for kind in canonical["outcomes"]), canonical
        for path, content in expected.items():
            assert (workspace / path).read_bytes() == content, "Review/commit changed file content"
        for path, content in excluded.items():
            assert (workspace / path).read_bytes() == content, "Excluded files must remain untouched"
            assert not git("ls-files", "--", path).strip(), "Excluded file entered the index"
        if preserved_index is not None:
            assert git("ls-files", "--stage", "--", "unrelated.rs") == preserved_index, "Unrelated staged entry changed"
            assert (workspace / "unrelated.rs").read_text() == "// further unstaged user work\n", "Unrelated working-tree content changed"
        if index < 2:
            assert git("rev-parse", "HEAD").strip() == baseline, "Read-only review committed changes"
            assert git("ls-files", "--stage") == before_index, "Read-only review changed the index"
            after_files = {str(path.relative_to(workspace)): path.read_bytes()
                           for path in workspace.rglob("*")
                           if path.is_file() and ".git" not in path.relative_to(workspace).parts}
            assert_existing_files_unchanged(task_baseline, after_files)
        else:
            after_files = {str(path.relative_to(workspace)): path.read_bytes()
                           for path in workspace.rglob("*")
                           if path.is_file() and ".git" not in path.relative_to(workspace).parts}
            assert after_files == before_files, "Commit phase changed working-tree files"
        print(f"Turn {index + 1} returned; external state checked", flush=True)
        if pending is not None:
            commits = git("rev-list", "--reverse", f"{baseline}..HEAD").splitlines()
            if not args.split_commits:
                assert len(commits) <= 1, "Waiting does not waive the single-commit constraint"
            committed = set(git("diff", "--name-only", "--no-renames", baseline, "HEAD").splitlines())
            assert committed <= set(expected), "Waiting does not authorize unrelated commits"
            receipt = {"pass": None, "accepted": True, "task_complete": False,
                       "status": "awaiting_user", "pending": pending,
                       "committed_paths": sorted(committed),
                       "remaining_paths": sorted(set(expected) - committed),
                       "confirmation_before_commit": not commits,
                       "protection_checks_passed": True,
                       "limitations": ["Question semantics require review; an open wait is not task completion."]}
            (run / "pending-receipt.json").write_text(json.dumps(receipt, indent=2))
            print(json.dumps(receipt), flush=True)
            return
    head = git("rev-parse", "HEAD").strip()
    assert head != baseline, "No commit created: command prose is not execution"
    commits = git("rev-list", "--reverse", f"{baseline}..HEAD").splitlines()
    if args.split_commits:
        assert len(commits) >= 2, "User requested multiple commits"
        observed = set()
        parent = baseline
        for commit in commits:
            assert git("rev-parse", f"{commit}^").strip() == parent, "History must be a simple chain"
            paths = set(git("diff-tree", "--no-commit-id", "--name-only", "--no-renames", "-r", commit).splitlines())
            assert paths and not paths & observed, "Each commit must contribute a disjoint nonempty selection"
            observed.update(paths)
            parent = commit
        assert observed == set(expected), "Composed operations missed or added task resources"
    else:
        assert git("rev-parse", "HEAD^").strip() == baseline, "Unexpected additional commits"
    assert not git("diff", "HEAD", "--", *expected).strip(), "Selected changes were not fully committed"
    assert set(git("diff", "--name-only", "--no-renames", baseline, "HEAD").splitlines()) == set(expected)
    for path, content in expected.items():
        assert git("show", f"HEAD:{path}").encode() == content
    receipt = {"pass": True, "scenario": "composed_selections" if args.split_commits else "commit_only" if args.commit_only else "three_turn_followup", "baseline": baseline, "commit": head,
               "commits": commits, "unrelated_index_preserved": True if preserved_index is not None else None,
               "committed_paths": sorted(expected), "excluded_paths": sorted(excluded), "model": llm["model"]}
    (run / "receipt.json").write_text(json.dumps(receipt, indent=2))
    print(json.dumps(receipt), flush=True)


if __name__ == "__main__":
    main()
