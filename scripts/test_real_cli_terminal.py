#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Owned real-provider PTY: committed VT100 frames and formal Run receipts.

No ANSI stripping, HOME substitution, production data, or evidence deletion.
"""
from __future__ import annotations

import base64
import errno
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import re
import signal
import sqlite3
import struct
import subprocess
import sys
import tempfile
import termios
import time
import traceback

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts/e2e"))
from model_execution_fixture import fixture, runtime_capabilities


def exception_evidence(failure: BaseException, trace=None) -> dict:
    evidence = {"type": type(failure).__name__, "message": str(failure),
        "traceback": "".join(traceback.format_exception(type(failure), failure, trace or failure.__traceback__))}
    # These are the only credentials deliberately supplied to these runners.
    # Never read another configuration merely to redact failure diagnostics.
    for name in ("NOLA_REAL_TEST_API_KEY", "NOLA_REAL_E2E_API_KEY", "NOLA_LIVE_SKILL_API_KEY"):
        secret = os.environ.get(name)
        if secret:
            evidence = {key: value.replace(secret, "[redacted]") for key, value in evidence.items()}
    if isinstance(failure, OSError):
        evidence["errno"] = failure.errno
    return evidence


def process_group_snapshot(leader: int) -> list[dict]:
    """Read identities/states only, never command arguments or environment."""
    completed = subprocess.run(["/bin/ps", "-axo", "pid=,ppid=,pgid=,lstart=,stat="],
        check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=3,
        env={"PATH": os.defpath, "LC_ALL": "C"})
    members = []
    for line in completed.stdout.splitlines():
        fields = line.split()
        if len(fields) < 3 or not all(value.isdecimal() for value in fields[:3]):
            raise RuntimeError("process identity snapshot is malformed")
        pid, parent, group = map(int, fields[:3])
        if pid != leader and group != leader:
            continue
        if len(fields) != 9:
            raise RuntimeError("owned process identity/state is unavailable")
        members.append({"pid": pid, "ppid": parent, "pgid": group,
            "started": " ".join(fields[3:8]), "state": fields[8]})
    return sorted(members, key=lambda member: member["pid"])


class Oracle:
    def __init__(self, binary: Path):
        self.process = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.PIPE, text=True)

    def query(self, **request) -> dict:
        self.process.stdin.write(json.dumps(request) + "\n")
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        if not line:
            raise RuntimeError("terminal oracle terminated without a response")
        response = json.loads(line)
        if "error" in response:
            raise RuntimeError("terminal oracle: " + response["error"])
        return response

    def close(self):
        try:
            self.process.stdin.close()
        except BrokenPipeError:
            pass
        try:
            self.process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            try:
                self.process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=3)
        self.process.stdout.close()
        self.process.stderr.close()


class RealTerminal:
    def __init__(self, binary: Path, project: Path, environment: dict, oracle: Oracle, root: Path,
                 *, arguments: list[str] | None = None):
        self.oracle = oracle
        self.output = bytearray()
        self.frames, self.latest = [], {}
        self.reaped = False
        self.pty_closed = False
        self.parent_pid = os.getpid()
        self.leader_identity = None
        self.owned_group_members = {}
        self.cleanup_failures = []
        # Retain the waitable leader until *after* all group signals. A custom
        # SIGCHLD handler/auto-reaping disposition would invalidate that proof.
        if signal.getsignal(signal.SIGCHLD) != signal.SIG_DFL:
            raise RuntimeError("terminal validation requires a waitable, non-auto-reaped PTY leader")
        self.raw = (root / "terminal.raw").open("xb")
        self.frame_log = (root / "terminal-frames.jsonl").open("x", encoding="utf-8")
        self.actions = (root / "terminal-actions.jsonl").open("x", encoding="utf-8")
        self.process_log = (root / "terminal-processes.jsonl").open("x", encoding="utf-8")
        pid, master = pty.fork()
        if pid == 0:
            try:
                os.chdir(project)
                fcntl.ioctl(1, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 100, 0, 0))
                os.execvpe(str(binary), arguments if arguments is not None else
                           [str(binary), "--project", str(project), "--no-tools",
                            "--debug", "--lang", "en"], environment)
            except BaseException:
                # Never run the parent's credential/evidence cleanup in the
                # fork child if exec fails.
                os._exit(127)
        self.pid, self.master = pid, master
        os.set_blocking(master, False)
        self._process_event("process_spawn", pid=pid, parentPid=self.parent_pid, expectedProcessGroup=pid)
        try:
            self._owned_group("spawn")
        except Exception as failure:
            # The direct child has not been reaped. A later fresh snapshot may
            # establish its exact identity; no signal is allowed without it.
            self._process_event("process_observation_failed", stage="spawn", failure=exception_evidence(failure))

    def action(self, name: str, **facts):
        self.actions.write(json.dumps({"action": name, "monotonicNs": time.monotonic_ns(),
            "frameId": self.latest.get("frameId", 0), **facts}) + "\n")
        self.actions.flush()

    def _process_event(self, name: str, **facts):
        # Process lifecycle is not a visible-frame observation. In particular,
        # spawning precedes frame 1 and must not weaken the VT action oracle.
        self.process_log.write(json.dumps({"event": name, "monotonicNs": time.monotonic_ns(),
            **facts}) + "\n")
        self.process_log.flush()

    def write(self, value: bytes):
        os.write(self.master, value)

    def pump(self):
        try:
            chunk = os.read(self.master, 65_536)
        except BlockingIOError:
            time.sleep(0.01)
            return
        except OSError as error:
            if error.errno != errno.EIO:
                raise
            self.pty_closed = True
            return
        if not chunk:
            self.pty_closed = True
            return
        self.output.extend(chunk)
        self.raw.write(chunk)
        self.raw.flush()  # retain failure bytes before asking the oracle
        for frame in self.oracle.query(operation="feed", bytes_base64=base64.b64encode(chunk).decode())["frames"]:
            frame["observedMonotonicNs"] = time.monotonic_ns()
            self.frames.append(frame)
            self.latest = frame
            self.frame_log.write(json.dumps(frame, ensure_ascii=False) + "\n")
        self.frame_log.flush()

    def screen(self) -> str:
        return self.latest.get("screen", "")

    def wait(self, predicate, timeout: float, description: str):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.pump()
            if predicate():
                return
            # PTY EOF is enough to stop waiting, but is NOT permission to reap
            # the process-group leader before owned-group cleanup.
            if self.pty_closed:
                break
        raise RuntimeError(f"{description}: timeout/exit; committed screen:\n{self.screen()[-2000:]}")

    def _owned_group(self, stage: str) -> list[dict]:
        if self.reaped:
            raise RuntimeError("a reaped leader cannot prove process-group ownership")
        members = process_group_snapshot(self.pid)
        self._process_event("process_group_observed", stage=stage, members=members)
        leader = next((member for member in members if member["pid"] == self.pid), None)
        if leader is None or leader["ppid"] != self.parent_pid or leader["pgid"] != self.pid:
            raise RuntimeError("owned PTY leader identity/group is unavailable or changed")
        identity = {key: leader[key] for key in ("pid", "ppid", "pgid", "started")}
        if self.leader_identity is not None and identity != self.leader_identity:
            raise RuntimeError("owned PTY leader start identity changed")
        self.leader_identity = identity
        verified = {self.pid}
        pending = {member["pid"]: member for member in members if member["pid"] != self.pid}
        while pending:
            admitted = [pid for pid, member in pending.items() if member["ppid"] in verified
                or self.owned_group_members.get(pid) == {key: member[key] for key in ("pid", "pgid", "started")}]
            if not admitted:
                raise RuntimeError("process-group member ownership is unproven; refusing group signal/reap")
            for pid in admitted:
                verified.add(pid)
                pending.pop(pid)
        for member in members:
            self.owned_group_members[member["pid"]] = {key: member[key] for key in ("pid", "pgid", "started")}
        return members

    @staticmethod
    def _all_exited(members: list[dict]) -> bool:
        # Only explicit zombie states prove exit without consuming the child.
        return bool(members) and all(member["state"].startswith("Z") for member in members)

    def _signal_owned_group(self, signal_value: int) -> None:
        members = self._owned_group(f"before_signal_{signal_value}")
        if self._all_exited(members):
            return
        try:
            os.killpg(self.pid, signal_value)
            self._process_event("process_group_signal", signal=signal_value, processGroup=self.pid, accepted=True)
        except (ProcessLookupError, PermissionError) as failure:
            self._process_event("process_group_signal", signal=signal_value, processGroup=self.pid,
                accepted=False, failure=exception_evidence(failure))
            # EPERM/ESRCH is not a stop receipt. It is recoverable only when a
            # fresh, identity-checked snapshot proves no living group member.
            if not self._all_exited(self._owned_group(f"after_signal_error_{signal_value}")):
                raise
            self._process_event("process_group_exit_verified_after_signal_error", signal=signal_value)

    def close(self):
        failures = []
        pump_available = True
        try:
            if not self.reaped:
                self._owned_group("before_exit_request")
                try:
                    self.write(b"/exit\r")
                except OSError as failure:
                    self._process_event("process_exit_request_failed", failure=exception_evidence(failure))
                for signal_value, seconds in ((None, 2), (signal.SIGTERM, 2), (signal.SIGKILL, 3)):
                    if self._all_exited(self._owned_group("cleanup_wait")):
                        break
                    if signal_value is not None:
                        self._signal_owned_group(signal_value)
                    deadline = time.monotonic() + seconds
                    while time.monotonic() < deadline:
                        if self._all_exited(self._owned_group("cleanup_poll")):
                            break
                        if self.pty_closed or not pump_available:
                            # EOF only ends transport reads. The process may
                            # still be draining state, so retain its grace time.
                            time.sleep(0.05)
                        else:
                            try:
                                self.pump()
                            except Exception as failure:
                                failures.append(exception_evidence(failure))
                                pump_available = False
                if not self._all_exited(self._owned_group("before_reap")):
                    raise RuntimeError("owned process group still has live members after bounded cleanup")
                # All group signals precede this only consuming wait. The
                # unreaped leader reserves the numeric identity until here.
                deadline = time.monotonic() + 3
                while time.monotonic() < deadline:
                    try:
                        waited, status = os.waitpid(self.pid, os.WNOHANG)
                    except ChildProcessError as failure:
                        self.reaped = True
                        raise RuntimeError("PTY leader was reaped outside its owner") from failure
                    if waited == self.pid:
                        self.reaped = True
                        self._process_event("process_leader_reaped", pid=waited, waitStatus=status)
                        break
                    time.sleep(0.01)
                if not self.reaped:
                    raise RuntimeError("owned terminal process did not exit after bounded cleanup")
        except Exception as failure:
            failures.append(exception_evidence(failure))
        finally:
            self.cleanup_failures = failures
            try:
                self._process_event("process_cleanup_finished", reaped=self.reaped, failures=failures)
            except Exception as failure:
                failures.append(exception_evidence(failure))
            for close in (lambda: os.close(self.master), self.raw.close, self.frame_log.close,
                          self.actions.close, self.process_log.close):
                try:
                    close()
                except Exception as failure:
                    failures.append(exception_evidence(failure))
        if failures:
            raise RuntimeError("owned PTY cleanup failed; see terminal-processes.jsonl for exact process/error evidence")


def require(value, message):
    if not value:
        raise RuntimeError(message)


def readonly(database: Path):
    return sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True)


def settled(database: Path, expected: int) -> bool:
    if not database.is_file():
        return False
    with readonly(database) as connection:
        rows = connection.execute("SELECT status FROM runs ORDER BY created_at_ms,id").fetchall()
    return len(rows) == expected and all(row[0] == "sealed" for row in rows)


def formal(terminal, oracle, database, expected, kinds, timeout):
    terminal.wait(lambda: settled(database, expected), timeout, "formal Run settlement")
    audit = oracle.query(operation="audit", database=str(database), live=True)
    require([run["kind"] for run in audit["runs"]] == kinds, "unexpected typed Run outcomes")
    require(len({run["conversationId"] for run in audit["runs"]}) == 1, "follow-up changed canonical owner")
    terminal.action("formal_seal", runs=audit["runs"])
    return audit


def main() -> int:
    if len(sys.argv) != 3:
        raise SystemExit("usage: test_real_cli_terminal.py /path/to/nola /path/to/terminal_acceptance_oracle")
    binary, oracle_binary = (Path(argument).resolve() for argument in sys.argv[1:])
    require(binary.is_file() and oracle_binary.is_file(), "current CLI/oracle binary missing")
    source = Path(os.environ.get("NOLA_CONFIG_PATH", str(Path.home() / ".nola/config.json"))).resolve()
    source_bytes = source.read_bytes()
    configuration = json.loads(source_bytes)
    declaration = fixture(configuration, os.environ, require_tool=False)
    require(all(isinstance(configuration["llm"].get(key), str) and configuration["llm"][key]
                for key in ("endpoint", "api_key")), "explicit test endpoint/credential is missing")
    root = Path(tempfile.mkdtemp(prefix="nola-cli-terminal-real-")).resolve()
    os.chmod(root, 0o700)
    print(f"Terminal evidence retained: {root}", flush=True)
    project, home = root / "project", root / "profile"
    project.mkdir()
    (home / ".nola").mkdir(parents=True)
    database, config = project / "nola-real-terminal.sqlite", home / ".nola/config.json"
    provider_trace, provider_ready = root / "provider.ndjson", root / "provider-ready.json"
    (root / "model-execution.json").write_text(json.dumps(declaration, indent=2) + "\n")
    probe = terminal = oracle = None
    metrics = {"schemaVersion": 2, "passed": False}
    audit, error = {}, None
    def interrupted(signum, _frame):
        raise RuntimeError(f"terminal validation interrupted by signal {signum}")
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    try:
        probe = subprocess.Popen([sys.executable, str(ROOT / "scripts/e2e/provider_probe.py"),
            "--upstream", configuration["llm"]["endpoint"], "--log", str(provider_trace),
            "--ready", str(provider_ready)], stdout=subprocess.DEVNULL,
            stderr=(root / "provider-stderr.log").open("xb"))
        deadline = time.monotonic() + 5
        while not provider_ready.is_file() and time.monotonic() < deadline:
            require(probe.poll() is None, "provider probe exited")
            time.sleep(0.05)
        endpoint = json.loads(provider_ready.read_text())["endpoint"]
        provider = {"chat_completions": "openai-compatible", "responses": "openai-responses", "anthropic": "anthropic"}[declaration["wireProtocol"]]
        with config.open("x") as output:
            os.chmod(config, 0o600)
            json.dump({"llm": {"endpoint": endpoint, "api_key": configuration["llm"]["api_key"],
                "model": declaration["model"], "provider": provider,
                "capabilities": runtime_capabilities(declaration)}}, output)
        environment = {key: value for key, value in os.environ.items()
            if not key.startswith(("NOLA_", "nola_", "SIMCTL_CHILD_", "UPDATE_"))
            and key not in {"DATABASE_URL", "DB_PATH", "ROOT", "WEB_ROOT"}}
        environment.update(TERM="xterm-256color", COLUMNS="100", LINES="24", NOLA_HOME=str(home),
            NOLA_DB_PATH=str(database), NOLA_LLM_MAX_RETRIES="0", NOLA_SKIP_ONBOARDING="1",
            NOLA_PERMISSION_PROFILE="cautious", NOLA_MAC_APP_SUPPORT=str(root / "app-support"),
            NOLA_EXTERNAL_EXECUTION_HOME=str(root / "execution-home"))
        oracle = Oracle(oracle_binary)
        terminal = RealTerminal(binary, project, environment, oracle, root)
        terminal.wait(lambda: "Type a message" in terminal.screen(), 8, "initial composer")
        terminal.action("long_input")
        terminal.write(b"Output exactly 40 lines, numbered SCROLL-01 through SCROLL-40, each followed by one short word. No code block or explanation.\r")
        terminal.wait(lambda: any(re.search(r"(?m)^\s*SCROLL-01\s+\S", frame["screen"])
                                 for frame in terminal.frames), 60, "first visual response line")
        formal(terminal, oracle, database, 1, ["answered"], 60)
        terminal.wait(lambda: re.search(r"(?m)^\s*SCROLL-40\s+\S", terminal.screen())
                      and "Esc interrupt" not in terminal.screen(), 5, "last response line and idle composer")
        # Traverse the product-owned document, not vt100/native scrollback.
        for _ in range(4):
            terminal.action("page_up")
            previous = terminal.latest["frameId"]
            terminal.write(b"\x1b[5~")
            terminal.wait(lambda: terminal.latest["frameId"] > previous, 5, "PageUp frame")
            if re.search(r"(?m)^\s*SCROLL-01\s+\S", terminal.screen()):
                break
        require(re.search(r"(?m)^\s*SCROLL-01\s+\S", terminal.screen()), "PageUp lost first answer line")
        terminal.action("page_up_witness")
        for _ in range(4):
            terminal.action("page_down")
            previous = terminal.latest["frameId"]
            terminal.write(b"\x1b[6~")
            terminal.wait(lambda: terminal.latest["frameId"] > previous, 5, "PageDown frame")
            if re.search(r"(?m)^\s*SCROLL-40\s+\S", terminal.screen()):
                break
        require(re.search(r"(?m)^\s*SCROLL-40\s+\S", terminal.screen()), "PageDown lost answer tail")
        terminal.action("page_down_witness")
        numbers = {int(value) for frame in terminal.frames for value in
            re.findall(r"(?m)^\s*SCROLL-(\d{2})\s+\S", frame["screen"])}
        require(len(numbers) >= 38 and {1, 40}.issubset(numbers), "long response lost visible document rows")
        terminal.action("cancel_input")
        terminal.write(b"Output CANCEL-BEGIN-741 alone on its own line, then 300 lines CANCEL-LINE-1 through CANCEL-LINE-300.\r")
        terminal.wait(lambda: "Esc interrupt" in terminal.screen()
            and re.search(r"(?m)^\s*CANCEL-BEGIN-741\s*$", terminal.screen()), 45, "live cancellable output")
        before = terminal.latest["frameId"]
        cancelled_at = time.monotonic_ns()
        terminal.action("escape")
        terminal.write(b"\x1b")
        audit = formal(terminal, oracle, database, 2, ["answered", "cancelled"], 5)
        cancellation_ms = (time.monotonic_ns() - cancelled_at) // 1_000_000
        require(cancellation_ms <= 2_000, f"formal cancellation took {cancellation_ms}ms")
        continued = {value for frame in terminal.frames if frame["frameId"] > before
            for value in re.findall(r"(?m)^\s*CANCEL-LINE-(\d+)\s*$", frame["screen"])}
        require(len(continued) <= 20, "too many model lines after Escape")
        terminal.action("followup_input", priorRunId=audit["runs"][-1]["runId"])
        terminal.write(b"Return only FOLLOWUP-OK-741 on its own line.\r")
        audit = formal(terminal, oracle, database, 3, ["answered", "cancelled", "answered"], 45)
        terminal.wait(lambda: re.search(r"(?m)^\s*FOLLOWUP-OK-741\s*$", terminal.screen())
            and "Esc interrupt" not in terminal.screen(), 5, "idle follow-up answer")
        frames = [frame for frame in terminal.frames if "Type a message" in frame["screen"]]
        require(frames and all(frame["screen"].count("Type a message") == 1
            and frame["screen"].count("Context") == 1 for frame in frames), "duplicated/missing composer or footer")
        metrics.update(passed=True, longResponseUniqueLines=len(numbers), cancellationMs=cancellation_ms,
            continuedLinesAfterEsc=len(continued), followupSucceeded=True,
            formalRunKinds=[run["kind"] for run in audit["runs"]])
    except Exception as failure:
        error = f"{type(failure).__name__}: {failure}"
        metrics["failure"] = error
        metrics["primaryFailure"] = exception_evidence(failure)
    finally:
        # An outer timeout may interrupt the run, but cleanup must still scrub
        # the owned temporary credential material and preserve failed captures.
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        signal.signal(signal.SIGINT, signal.SIG_IGN)
        cleanup_errors = []
        if terminal:
            try:
                terminal.close()
            except Exception as failure:
                metrics["cleanupFailure"] = {"error": exception_evidence(failure),
                    "processErrors": terminal.cleanup_failures}
                metrics["passed"] = False
        if oracle:
            try:
                oracle.close()
            except Exception as failure:
                cleanup_errors.append(type(failure).__name__)
        if probe:
            try:
                probe.terminate()
                try:
                    probe.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    probe.kill()
                    probe.wait(timeout=3)
            except OSError as failure:
                cleanup_errors.append(type(failure).__name__)
        # Remove only this disposable run's credential files; keep audit data.
        try:
            if config.is_file():
                config.unlink()
            vault = database.parent / ".nola-host-secrets"
            if vault.is_dir() and not vault.is_symlink():
                for secret_file in vault.iterdir():
                    if secret_file.is_file() and not secret_file.is_symlink():
                        secret_file.unlink()
        except OSError as failure:
            cleanup_errors.append(type(failure).__name__)
        metrics["secretCleanupSucceeded"] = not cleanup_errors
        metrics["passed"] &= not cleanup_errors
        try:
            metrics["sourceConfigurationModified"] = hashlib.sha256(source.read_bytes()).digest() != hashlib.sha256(source_bytes).digest()
        except OSError:
            metrics["sourceConfigurationModified"] = True
        metrics["passed"] &= not metrics["sourceConfigurationModified"]
        if database.is_file():
            try:
                with readonly(database) as original, sqlite3.connect(root / "terminal-snapshot.sqlite") as snapshot:
                    original.backup(snapshot)
            except sqlite3.Error:
                metrics["passed"] = False
                metrics["snapshotFailure"] = True
        secret = configuration["llm"].get("api_key", "").encode()
        # Do not erase/rewrite evidence to hide a leak. Fail and keep it inside
        # this owner-only directory; never include the secret in diagnostics.
        leaked = False
        if secret:
            for artifact in root.rglob("*"):
                if artifact.is_file() and not artifact.is_symlink():
                    try:
                        with artifact.open("rb") as handle:
                            carry = b""
                            for chunk in iter(lambda: handle.read(65536), b""):
                                leaked |= secret in carry + chunk
                                carry = (carry + chunk)[-len(secret):]
                    except FileNotFoundError:
                        pass  # an already removed WAL cannot retain a secret
                    except OSError:
                        metrics.update(passed=False, evidenceScanFailed=True)
        metrics["retainedEvidenceContainsSecret"] = leaked
        metrics["passed"] &= not leaked
        (root / "terminal-audit.json").write_text(json.dumps(audit, indent=2) + "\n")
        (root / "terminal-metrics.json").write_text(json.dumps(metrics, indent=2) + "\n")
        if metrics["passed"]:
            from terminal_acceptance_audit import verify
            try:
                metrics["independentReplay"] = verify(root / "terminal.raw", root / "terminal-frames.jsonl",
                    root / "terminal-actions.jsonl", root / "terminal-metrics.json", root / "terminal-audit.json",
                    root / "terminal-snapshot.sqlite", oracle_binary)
            except Exception as failure:
                error = f"{type(failure).__name__}: {failure}"
                metrics.update(passed=False, independentReplayFailure=error)
            (root / "terminal-metrics.json").write_text(json.dumps(metrics, indent=2) + "\n")
        if os.environ.get("NOLA_E2E_RECEIPT_PATH"):
            from real_e2e_evidence import record_step
            assertions = {name: metrics["passed"] for name in ("long_response_preserved", "escape_cancelled_promptly",
                "follow_up_recovered", "terminal_layout_stable", "canonical_projection_valid")}
            observations = [{"id": identity, "kind": kind, "path": root / name} for identity, kind, name in (
                ("provider-traffic", "provider_trace", "provider.ndjson"),
                ("terminal-capture", "terminal_capture", "terminal.raw"),
                ("terminal-frames", "terminal_frames", "terminal-frames.jsonl"),
                ("terminal-actions", "terminal_actions", "terminal-actions.jsonl"),
                ("terminal-metrics", "terminal_metrics", "terminal-metrics.json"),
                ("terminal-audit", "persistence_audit", "terminal-audit.json"),
                ("terminal-database", "persistence_store", "terminal-snapshot.sqlite")) if (root / name).is_file()]
            record_step("interactive-terminal-turns", platforms=["cli"],
                modules=["chat", "provider", "terminal", "persistence", "cancellation"],
                boundaries=["cli-ui", "cli", "provider", "persistence"], assertions=assertions,
                assertion_evidence={name: [entry["id"] for entry in observations] for name in assertions},
                observations=observations, metrics=metrics)
    print(json.dumps({"passed": metrics["passed"], "evidenceRoot": str(root), "failure": error}), flush=True)
    return 0 if metrics["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
