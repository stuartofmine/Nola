#!/usr/bin/env python3
"""Opt-in real CLI lifecycle regression on a read-only backup of a recorded Store.
Requires pexpect. No model responses are mocked. The default scenario submits no
messages; --exercise-model-exit makes one real request using the selected account.
Temporary copied credentials are removed even on failure; source files stay intact.
"""
import argparse
from contextlib import closing
import pathlib, sqlite3, tempfile, hashlib, os, shutil, pexpect, json

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--source-db", type=pathlib.Path, required=True)
parser.add_argument("--credential-source-db", type=pathlib.Path)
parser.add_argument("--binary", type=pathlib.Path, required=True)
parser.add_argument("--session", required=True)
parser.add_argument("--project", type=pathlib.Path, required=True)
parser.add_argument("--exercise-model-exit", action="store_true")
parser.add_argument("--exercise-active-draft-restart", action="store_true")
parser.add_argument("--restart-signal", choices=("hup", "kill"), default="hup")
parser.add_argument("--exercise-view-restore", action="store_true")
parser.add_argument("--oracle", type=pathlib.Path)
parser.add_argument("--view-columns", type=int, default=100)
args = parser.parse_args()
if args.exercise_active_draft_restart and not args.exercise_model_exit:
    parser.error("--exercise-active-draft-restart requires --exercise-model-exit")
if args.restart_signal == "kill" and not args.exercise_active_draft_restart:
    parser.error("--restart-signal kill requires --exercise-active-draft-restart")
if args.exercise_view_restore and not args.oracle:
    parser.error(
        "--exercise-view-restore requires --oracle (terminal_acceptance_oracle)"
    )
root = pathlib.Path(tempfile.mkdtemp(prefix="nola-lifecycle-pty-"))
root.chmod(0o700)
results = {"status": "running"}
child = None
d = None
print(root, flush=True)
try:
    src = args.source_db.resolve()
    dst = root / "recorded.db"
    a = sqlite3.connect(f"file:{src}?mode=ro", uri=True)
    b = sqlite3.connect(dst)
    a.backup(b)
    a.close()
    assert b.execute("select 1 from conversations where id=?", (args.session,)).fetchone(), "Recorded conversation is absent"
    assert not b.execute("select 1 from runs where conversation_id=? and status!='sealed'", (args.session,)).fetchone(), "Use a settled recorded conversation for this navigation test"
    baseline_ids = {table: {row[0] for row in b.execute(f"select id from {table}")} for table in ("conversations", "runs")}
    baseline = {table: len(ids) for table, ids in baseline_ids.items()}
    b.close()
    original = (args.credential_source_db or args.source_db).resolve()


    def vault(p):
        return (
            p.parent
            / ".nola-host-secrets"
            / (hashlib.sha256(os.fsencode(p.resolve())).hexdigest() + ".sqlite")
        )


    d = vault(dst)
    d.parent.mkdir(mode=0o700)
    if vault(original).exists():
        with closing(sqlite3.connect(f"file:{vault(original)}?mode=ro", uri=True)) as credentials:
            with closing(sqlite3.connect(d)) as copy:
                credentials.backup(copy)
        d.chmod(0o600)
    elif args.exercise_model_exit:
        raise FileNotFoundError("Model test requires the source credential vault")
    env = dict(os.environ, NOLA_DB_PATH=str(dst))
    session = args.session
    project = str(args.project.resolve())
    binary = str(args.binary.resolve())
    with open(binary, "rb") as product:
        results["binary_sha256"] = hashlib.file_digest(product, "sha256").hexdigest()
    results["baseline"] = baseline
    with open(root / "terminal.log", "w") as log:
        child = pexpect.spawn(
            binary,
            ["--session", session, "--project", project, "--lang", "zh"],
            env=env,
            encoding="utf-8",
            timeout=30,
            dimensions=(24, 100),
        )
        child.logfile = log
        child.expect("输入消息")
        results["explicit_session_attached"] = True
        child.send("未提交草稿")
        child.expect("未提交草稿")
        child.sendcontrol("c")
        child.expect("输入消息")
        assert child.isalive()
        results["draft_ctrl_c_stays_open"] = True
        child.send("/resume\r")
        child.expect("最近会话")
        child.send("\x1b")
        child.expect("输入消息")
        assert child.isalive()
        results["picker_escape_stays_open"] = True
        if args.exercise_view_restore:
            from e2e.cli_view_restore import exercise_view_restore

            child = exercise_view_restore(
                child,
                binary,
                session,
                project,
                env,
                root,
                log,
                args.oracle.resolve(),
                args.view_columns,
            )
            results["draft_cursor_and_reading_view_restored"] = True
        if args.exercise_model_exit:
            child.send("这是退出测试。不要调用工具。请详细解释递归和迭代，给出十个例子。\r")
            child.expect("Esc 中断")
            import time

            probe = sqlite3.connect(dst)
            deadline = time.monotonic() + 20
            while not probe.execute(
                "select 1 from runs where conversation_id=? and status='running'",
                (session,),
            ).fetchone():
                assert time.monotonic() < deadline, "No active Run observed before exit"
                try:
                    child.read_nonblocking(65536, timeout=0.05)
                except pexpect.TIMEOUT:
                    pass
            # Real execution listener, not a synthetic key handler: require the
            # draft and UTF-8 cursor on disk while the model Run is still active.
            child.send("执行中草稿甲乙")
            child.send("\x1b[D\x1b[D")
            view_path = dst.parent / "CliViewState" / hashlib.sha256(os.fsencode(dst.resolve())).hexdigest() / (hashlib.sha256(session.encode()).hexdigest() + ".json")
            deadline = time.monotonic() + 10
            while True:
                record = json.loads(view_path.read_text())["state"]
                if record["draft"] == "执行中草稿甲乙" and record["cursor"] == len("执行中草稿".encode()):
                    break
                assert time.monotonic() < deadline, "Execution draft/cursor not persisted"
                try:
                    child.read_nonblocking(65536, timeout=0.05)
                except pexpect.TIMEOUT:
                    pass
            assert probe.execute("select 1 from runs where conversation_id=? and status='running'", (session,)).fetchone(), "Model completed before active-draft assertion"
            results["running_draft_and_cursor_persisted"] = True
            if args.exercise_active_draft_restart:
                import signal
                crash_signal = signal.SIGKILL if args.restart_signal == "kill" else signal.SIGHUP
                child.kill(crash_signal)
                child.expect(pexpect.EOF)
                child.close()
                if crash_signal == signal.SIGKILL:
                    assert child.signalstatus == signal.SIGKILL, "Host did not exit from the injected hard crash"
                results["restart_signal"] = args.restart_signal
                child = pexpect.spawn(binary, ["--session", session, "--project", project, "--lang", "zh"],
                    env=env, encoding="utf-8", timeout=60, dimensions=(24, 100))
                child.logfile = log
                child.expect("执行中草稿甲乙")
                results["active_draft_restored_after_terminal_close"] = True
            child.send("\x05\x15")  # End, then clear input; do not submit the draft.
            deadline = time.monotonic() + 10
            while json.loads(view_path.read_text())["state"]["draft"]:
                assert time.monotonic() < deadline, "Cleared execution draft remained on disk"
                try:
                    child.read_nonblocking(65536, timeout=0.05)
                except pexpect.TIMEOUT:
                    pass
            probe.close()
        child.send("/exit\r")
        child.expect(pexpect.EOF)
        child.close()
        assert child.exitstatus == 0
        text = (root / "terminal.log").read_text()
        assert "nola --session" in text and "--project '" in text
        results["exact_exit_hint"] = True
        child = None
    with open(root / "picker-exit.log", "w") as log:
        child = pexpect.spawn(
            binary,
            ["--resume", "--project", project, "--lang", "zh"],
            env=env,
            encoding="utf-8",
            timeout=30,
            dimensions=(24, 100),
        )
        child.logfile = log
        child.expect("最近会话")
        child.send("\x1b")
        child.expect(pexpect.EOF)
        child.close()
        assert child.exitstatus == 0
        child = None
    c = sqlite3.connect(dst)
    assert {row[0] for row in c.execute("select id from conversations")} == baseline_ids["conversations"]
    results["picker_does_not_create_conversation"] = True
    before = baseline["runs"]
    after_ids = {row[0] for row in c.execute("select id from runs")}
    assert baseline_ids["runs"] <= after_ids, "Recorded Runs were removed"
    new_run_ids = after_ids - baseline_ids["runs"]
    after = len(after_ids)
    if args.exercise_model_exit:
        assert after == before + 1
        assert len(new_run_ids) == 1
        run = c.execute(
            "select id,status from runs where id=? and conversation_id=?",
            (next(iter(new_run_ids)), session),
        ).fetchone()
        assert run, "New Run belongs to another conversation"
        results["run_id"] = run[0]
        result = c.execute(
            "select result_json from run_results where run_id=?", (run[0],)
        ).fetchone()
        assert (
            run[1] == "sealed"
            and result
            and json.loads(result[0])["value"]["kind"] == "cancelled"
        )
        assert (
            c.execute(
                "select count(*) from effect_outbox where run_id=? and status='pending'",
                (run[0],),
            ).fetchone()[0]
            == 0
        )
        results["real_model_exit_cancelled"] = True
    else:
        assert after == before
        results["no_new_runs"] = True
    c.close()
    results["status"] = "passed"
    print(results)
except BaseException as error:
    results["status"] = "failed"
    results["failure_type"] = type(error).__name__
    raise
finally:
    try:
        if child and child.isalive():
            child.terminate(force=True)
    finally:
        if d is not None and d.parent.exists():
            shutil.rmtree(d.parent)
        (root / "result.json").write_text(json.dumps(results, indent=2))
