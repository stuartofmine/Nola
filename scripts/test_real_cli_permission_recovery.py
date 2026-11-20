#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Real-provider CLI crash at permission wait, exact Run recovery and once/deny.

Only disposable files are targets. An isolated Python launcher establishes the
new CLI's controlling terminal before exec; no threaded-parent preexec hook or
provider key enters the child. Permission responses use the ordinary CLI UI.
"""
from __future__ import annotations

import argparse
import errno
import fcntl
import json
import shutil
import os
from pathlib import Path
import pty
import select
import signal
import sqlite3
import struct
import subprocess
import sys
import tempfile
import termios
import time
import uuid

from canonical_run_audit import audit_runs
from test_real_cli_long_tasks import apply_config, fixture, observed_provider, selected_route
from test_real_cli_hundreds_steps import provider_totals, write_json


def cold_projection(binary, project, env, session):
    result = subprocess.run([str(binary), "--json", "session", "show", session],
        cwd=project, env=env, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=15)
    if result.returncode:
        return None
    return json.loads(result.stdout)


def pending_requests(projection):
    return ((projection or {}).get("permission_projection") or {}).get("pending") or []


def run_rows(database, session):
    if not database.exists():
        return []
    with sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True, timeout=2) as db:
        return [dict(zip(("id", "status", "completed_at_ms"), row)) for row in db.execute(
            "SELECT id,status,completed_at_ms FROM runs WHERE conversation_id=? AND kind='conversation'", (session,))]


def drain(master, transcript):
    while select.select([master], [], [], 0)[0]:
        try:
            data = os.read(master, 65536)
        except OSError as error:
            if error.errno == errno.EIO:
                break
            raise
        if not data:
            break
        transcript.write(data)
        transcript.flush()


def terminal_process(binary, project, env, args):
    launcher = ("import fcntl,os,sys,termios; "
        "fcntl.ioctl(0,termios.TIOCSCTTY,0); os.chdir(sys.argv[2]); "
        "os.execve(sys.argv[1], [sys.argv[1]]+sys.argv[3:], dict(os.environ))")
    master, slave = pty.openpty()
    try:
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
        process = subprocess.Popen([sys.executable, "-I", "-c", launcher, str(binary), str(project), *args],
            env=env, stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
        return process, master
    except BaseException:
        os.close(master)
        raise
    finally:
        os.close(slave)


def stop_owned(process):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.kill()
            process.wait(timeout=20)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/nola"))
    parser.add_argument("--config", type=Path, default=Path.home() / ".nola/config.json")
    parser.add_argument("--timeout", type=int, default=240)
    args = parser.parse_args()
    binary = args.binary.resolve()
    upstream, model, secret = selected_route(args.config)
    root = Path(tempfile.mkdtemp(prefix="nola-real-permission-crash-"))
    root.chmod(0o700)
    (root / "bin").mkdir()
    pinned_binary = root / "bin/nola"
    try:
        os.link(binary, pinned_binary)
    except OSError:
        shutil.copy2(binary, pinned_binary)
    binary = pinned_binary
    receipt = {"passed": False, "model": model, "crash_injected": False, "provider_responses_injected": False}
    first = resumed = None
    master = None
    initial_input = None
    print(json.dumps({"evidence": str(root), "phase": "starting"}), flush=True)
    try:
        with observed_provider(upstream, model, secret, root / "provider.ndjson") as endpoint:
            project, env, config = fixture(root / "case", model, endpoint, profile="cautious")
            env["NOLA_CLI_INTERACTION_MODE"] = "always"
            env["TERM"] = "xterm-256color"
            apply_config(binary, project, env, config, root)
            database = Path(env["NOLA_DB_PATH"])
            target = project / "existing.txt"
            original, allowed, denied = (f"{label}-{uuid.uuid4().hex}\n" for label in ("ORIGINAL", "ALLOWED", "DENIED"))
            target.write_text(original)
            write_json(root / "independent-oracle.json", {"original": original, "allowed": allowed, "denied": denied})
            session = "permission-crash-a"
            prompt = ("请完成两次独立的文件覆盖：先读取 existing.txt，再将其全部内容替换为以下第一行（保留换行）：\n"
                + allowed + "读回确认第一次完成后，第二次独立调用覆盖为以下第二行（保留换行）：\n" + denied
                + "两次必须分开执行。每次需要的权限只适用于该次调用；第二次若被拒绝，立即停止并报告实际拒绝和当前文件状态。"
                "不得用其他工具、替代文件或更改权限绕过拒绝。只用文件读取/写入工具，不用 shell、代码、代理或网络。")
            argv = ["--session", session, "--project", str(project),
                "--allow-dir", str(project), "--lang", "zh", "--no-skills",
                "--network-policy", "deny", "--max-iterations", "16"]
            started = time.monotonic()
            with (root / "before-crash.log").open("wb") as out:
                first, initial_input = terminal_process(binary, project, env, argv)
                while time.monotonic() - started < args.timeout:
                    drain(initial_input, out)
                    if "输入消息".encode() in (root / "before-crash.log").read_bytes():
                        break
                    assert first.poll() is None, "CLI exited before opening the composer"
                    time.sleep(0.1)
                else:
                    raise AssertionError("interactive composer did not open")
                os.write(initial_input, b"\x1b[200~" + prompt.encode() + b"\x1b[201~")
                os.write(initial_input, b"\r")
                pending = []
                while time.monotonic() - started < args.timeout:
                    drain(initial_input, out)
                    if "仅允许本次".encode() not in (root / "before-crash.log").read_bytes():
                        assert first.poll() is None, "CLI ended before the first real provider response"
                        time.sleep(0.1)
                        continue
                    projection = cold_projection(binary, project, env, session)
                    pending = pending_requests(projection)
                    if pending:
                        break
                    assert first.poll() is None, "CLI ended before presenting the initial permission"
                    time.sleep(0.3)
                assert len(pending) == 1, "initial permission wait was not durable"
                rows = run_rows(database, session)
                assert len(rows) == 1 and rows[0]["status"] != "sealed" and rows[0]["completed_at_ms"] is None
                assert target.read_text() == original, "file changed before approval"
                original_run = rows[0]["id"]
                write_json(root / "before-crash-projection.json", projection)
                receipt["original_run"] = original_run
                receipt["original_pending"] = pending[0]
                # Crash only the CLI that owns the prompt, leaving its service to
                # observe the client disconnect through the normal lifecycle.
                assert first.poll() is None, "CLI exited before crash injection"
                drain(initial_input, out)
                first.kill()
                # Release the terminal before reaping a foreground process that
                # may still be blocked flushing its permission panel.
                os.close(initial_input)
                initial_input = None
                assert first.wait(timeout=30) == -signal.SIGKILL
                receipt["crash_injected"] = True
            after_crash = cold_projection(binary, project, env, session)
            assert pending_requests(after_crash), "crash discarded the pending authorization"
            assert target.read_text() == original
            write_json(root / "after-crash-projection.json", after_crash)
            print(json.dumps({"phase": "crashed_at_durable_permission", "run": original_run}), flush=True)

            # No --resume selector and no new user message: selecting the exact
            # saved session in the ordinary interactive CLI recovers its Run.
            resume_argv = ["--session", session, "--project", str(project), "--allow-dir", str(project),
                "--lang", "zh", "--no-skills", "--network-policy", "deny"]
            resumed, master = terminal_process(binary, project, env, resume_argv)
            responded = []
            seen = set()
            recovery_started = time.monotonic()
            checked_frame = 0
            with (root / "resumed-terminal.log").open("wb") as transcript:
                while time.monotonic() - recovery_started < args.timeout:
                    drain(master, transcript)
                    rows = run_rows(database, session)
                    assert len(rows) == 1 and rows[0]["id"] == original_run, "recovery admitted a replacement Run/input"
                    if rows[0]["status"] == "sealed":
                        break
                    assert resumed.poll() is None, "resumed CLI exited before settling the original Run"
                    with (root / "resumed-terminal.log").open("rb") as frames:
                        frames.seek(checked_frame)
                        if "仅允许本次".encode() not in frames.read():
                            time.sleep(0.1)
                            continue
                    projection = cold_projection(binary, project, env, session)
                    waiting = pending_requests(projection)
                    # Raw terminal mode is an OS observation that the product's
                    # actual keyboard permission prompt is ready. Send no extra
                    # Enter that might accidentally approve the next prompt.
                    raw_ready = not (termios.tcgetattr(master)[3] & termios.ICANON)
                    if len(waiting) == 1 and raw_ready:
                        request = waiting[0]
                        identity = request.get("request_id") or request.get("id")
                        assert identity, "pending request has no canonical identity"
                        if identity not in seen:
                            index = len(responded)
                            assert index < 2, "unexpected extra permission or denied-operation retry"
                            assert target.read_text() == (original if index == 0 else allowed), "permission boundary changed the file early"
                            write_json(root / f"permission-{index}.json", projection)
                            key = b"1" if index == 0 else b"n"
                            checked_frame = transcript.tell()
                            os.write(master, key)
                            responded.append({"request": request, "decision": "allow_once" if index == 0 else "deny"})
                            seen.add(identity)
                            print(json.dumps({"phase": "permission_response", "index": index, "decision": responded[-1]["decision"]}), flush=True)
                    time.sleep(0.3)
                else:
                    raise AssertionError("cold permission recovery did not settle before the observation deadline")
                drain(master, transcript)
            assert len(responded) == 2, "once-only permission did not require a separate second decision"
            assert target.read_text() == allowed, "denied second overwrite changed the file"
            receipt["decisions"] = responded
            with sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True) as db:
                receipt["canonical"] = audit_runs(db, session)
                evidence = [json.loads(raw)["evidence"] for (raw,) in db.execute(
                    "SELECT payload_json FROM run_events WHERE run_id=? AND event_kind='execution_evidence_observed' ORDER BY sequence", (original_run,))]
                mutations = [item for item in evidence if item["tool_name"] in {"write_file", "edit_file", "apply_patch"}]
                receipt["mutation_evidence"] = mutations
                assert sum(item["outcome"] == "succeeded" for item in mutations) == 1
                successful = next(item for item in mutations if item["outcome"] == "succeeded")
                attempts = [json.loads(raw) for (raw,) in db.execute(
                    "SELECT record_json FROM effect_delivery_attempts WHERE run_id=? AND effect_id=?", (original_run, successful["source"]["effect_id"]))]
                assert sum(item.get("execution_started_at_ms") is not None for item in attempts) == 1, "approved physical effect was dispatched more than once"
                receipt["approved_effect_delivery_attempts"] = attempts
            assert receipt["canonical"]["runCount"] == 1
            assert receipt["canonical"]["outcomes"][0] in {"answered", "blocked", "incomplete"}
            receipt["file_matches_only_approved_write"] = True
            receipt["provider"] = provider_totals(root / "provider.ndjson")
            assert receipt["provider"]["requests"] == receipt["provider"]["successful_requests"]
            assert not receipt["provider"]["transport_errors"] and not receipt["provider"]["injected_responses"]
            receipt["passed"] = True
    except Exception as error:
        receipt["error"] = f"{type(error).__name__}: {error}"
        receipt["provider"] = provider_totals(root / "provider.ndjson")
    finally:
        write_json(root / "receipt.json", receipt)
        if initial_input is not None:
            os.close(initial_input)
        if master is not None:
            # Closing the controlling terminal before reaping prevents its
            # final repaint/terminal restoration from blocking on a full PTY.
            os.close(master)
            master = None
        for process in (first, resumed):
            if process is not None:
                try:
                    stop_owned(process)
                except Exception as error:
                    receipt["cleanup_error"] = f"{type(error).__name__}: {error}"
                    receipt["passed"] = False
        write_json(root / "receipt.json", receipt)
    forbidden = {secret} | {value for key, value in os.environ.items()
        if value and (key.endswith("_API_KEY") or key == "FREE_MODEL_KEY")}
    needles = [value.encode() for value in forbidden if value]
    overlap = max((len(value) for value in needles), default=1) - 1
    for path in root.rglob("*"):
        if path == pinned_binary or not path.is_file():
            continue
        with path.open("rb") as source:
            tail = b""
            while chunk := source.read(1024 * 1024):
                window = tail + chunk
                if any(value in window for value in needles):
                    raise RuntimeError("Credential appeared in isolated evidence; do not publish artifacts")
                tail = window[-overlap:] if overlap else b""
    print(json.dumps({"passed": receipt["passed"], "error": receipt.get("error"),
        "provider": receipt.get("provider"), "evidence": str(root)}), flush=True)
    return 0 if receipt["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
