# SPDX-License-Identifier: Apache-2.0
"""No real processes: assert signal/reap ordering for the owned PTY leader."""
import io
import errno
import json
import signal

import pytest
import test_real_cli_terminal as runner


def terminal(monkeypatch):
    value = runner.RealTerminal.__new__(runner.RealTerminal)
    value.pid, value.master = 741, 742
    value.reaped, value.pty_closed = False, True
    value.parent_pid = 740
    value.leader_identity = {"pid":741,"ppid":740,"pgid":741,"started":"Sat Oct 17 02:00:00 2026"}
    value.owned_group_members = {}
    value.cleanup_failures = []
    value.evidence = []
    value.fixture_state = "S+"
    value._process_event = lambda name, **facts: value.evidence.append({"event":name, **facts})
    monkeypatch.setattr(runner, "process_group_snapshot", lambda pid:
        [{**value.leader_identity, "state":value.fixture_state}])
    value.latest = {"screen": "closed"}
    value.raw, value.frame_log, value.actions = io.BytesIO(), io.StringIO(), io.StringIO()
    value.process_log = io.StringIO()
    value.pump = lambda: None
    value.write = lambda value: None
    clock = iter(index * 0.5 for index in range(1000))
    monkeypatch.setattr(runner.time, "monotonic", lambda: next(clock))
    monkeypatch.setattr(runner.time, "sleep", lambda seconds: None)
    return value


def test_wait_never_reaps_the_leader_before_group_cleanup(monkeypatch):
    value = terminal(monkeypatch)
    monkeypatch.setattr(runner.os, "waitpid", lambda *args: pytest.fail("wait must retain leader identity"))
    with pytest.raises(RuntimeError, match="timeout/exit"):
        value.wait(lambda: False, 1, "fixture")
    assert not value.reaped


def test_owned_group_signals_all_precede_the_only_reap(monkeypatch):
    value = terminal(monkeypatch)
    events = []
    def signal_group(pid, sig):
        events.append(("signal", pid, sig))
        if sig == signal.SIGKILL:
            value.fixture_state = "Z+"
    monkeypatch.setattr(runner.os, "killpg", signal_group)
    monkeypatch.setattr(runner.os, "waitpid", lambda pid, flags: (events.append(("reap", pid)), (pid, 0))[1])
    monkeypatch.setattr(runner.os, "close", lambda fd: None)
    value.close()
    assert events == [("signal", 741, signal.SIGTERM), ("signal", 741, signal.SIGKILL), ("reap", 741)]
    assert value.reaped


def test_already_reaped_leader_can_never_authorize_numeric_group_signals(monkeypatch):
    value = terminal(monkeypatch)
    value.reaped = True
    monkeypatch.setattr(runner.os, "killpg", lambda *args: pytest.fail("old PGID cannot authorize a signal"))
    monkeypatch.setattr(runner.os, "waitpid", lambda *args: pytest.fail("leader is already reaped"))
    monkeypatch.setattr(runner.os, "close", lambda fd: None)
    value.close()


def test_verified_zombie_group_is_reaped_without_sending_an_unnecessary_signal(monkeypatch):
    value = terminal(monkeypatch)
    value.fixture_state = "Z+"
    monkeypatch.setattr(runner.os, "killpg", lambda *args: pytest.fail("dead group needs no signal"))
    monkeypatch.setattr(runner.os, "waitpid", lambda pid, flags: (pid, 0))
    monkeypatch.setattr(runner.os, "close", lambda fd: None)
    value.close()
    assert value.reaped
    assert any(record["event"] == "process_leader_reaped" for record in value.evidence)


def test_pty_eof_keeps_grace_until_later_process_exit_without_forcing_a_signal(monkeypatch):
    value = terminal(monkeypatch)
    polls, sleeps = [], []
    def observe(pid):
        polls.append(pid)
        return [{**value.leader_identity, "state":"Z+" if len(polls) >= 4 else "S+"}]
    monkeypatch.setattr(runner, "process_group_snapshot", observe)
    monkeypatch.setattr(runner.time, "sleep", sleeps.append)
    monkeypatch.setattr(runner.os, "killpg", lambda *args: pytest.fail("EOF is not zero grace"))
    monkeypatch.setattr(runner.os, "waitpid", lambda pid, flags: (pid, 0))
    monkeypatch.setattr(runner.os, "close", lambda fd: None)
    value.pump = lambda: pytest.fail("EOF must stop further PTY reads")
    value.close()
    assert sleeps and len(polls) >= 4 and value.reaped


def test_eperm_is_reconciled_only_after_exact_owned_group_is_proven_zombie(monkeypatch):
    value = terminal(monkeypatch)
    events = []
    def signal_group(pid, sig):
        events.append(("signal", pid, sig))
        value.fixture_state = "Z+"  # Exit raced the identity-checked signal.
        raise PermissionError(errno.EPERM, "fixture group no longer has living members")
    monkeypatch.setattr(runner.os, "killpg", signal_group)
    monkeypatch.setattr(runner.os, "waitpid", lambda pid, flags: (events.append(("reap", pid)), (pid, 0))[1])
    monkeypatch.setattr(runner.os, "close", lambda fd: None)
    value.close()
    assert events == [("signal", 741, signal.SIGTERM), ("reap", 741)]
    assert any(record["event"] == "process_group_exit_verified_after_signal_error" for record in value.evidence)
    denied = next(record for record in value.evidence if record["event"] == "process_group_signal")
    assert denied["accepted"] is False and denied["failure"]["errno"] == errno.EPERM


def test_eperm_with_a_living_member_remains_cleanup_failure_and_never_reaps(monkeypatch):
    value = terminal(monkeypatch)
    def signal_group(*args):
        raise PermissionError(errno.EPERM, "fixture signal denied")
    monkeypatch.setattr(runner.os, "killpg", signal_group)
    monkeypatch.setattr(runner.os, "waitpid", lambda *args: pytest.fail("living group is not a stop proof"))
    monkeypatch.setattr(runner.os, "close", lambda fd: None)
    with pytest.raises(RuntimeError, match="cleanup failed"):
        value.close()
    assert not value.reaped
    assert value.cleanup_failures[0]["errno"] == errno.EPERM


def test_unknown_group_member_cannot_authorize_signal_or_reap(monkeypatch):
    value = terminal(monkeypatch)
    monkeypatch.setattr(runner, "process_group_snapshot", lambda pid: [
        {**value.leader_identity,"state":"Z+"},
        {"pid":800,"ppid":999,"pgid":741,"started":"Sat Oct 17 02:00:01 2026","state":"Z"},
    ])
    monkeypatch.setattr(runner.os, "killpg", lambda *args: pytest.fail("foreign member cannot be signalled"))
    monkeypatch.setattr(runner.os, "waitpid", lambda *args: pytest.fail("foreign member cannot prove cleanup"))
    monkeypatch.setattr(runner.os, "close", lambda fd: None)
    with pytest.raises(RuntimeError, match="cleanup failed"):
        value.close()
    assert "ownership is unproven" in value.cleanup_failures[0]["message"]


def test_changed_leader_start_identity_never_receives_a_numeric_group_signal(monkeypatch):
    value = terminal(monkeypatch)
    monkeypatch.setattr(runner, "process_group_snapshot", lambda pid:
        [{**value.leader_identity,"started":"Sat Oct 17 02:00:02 2026","state":"S+"}])
    monkeypatch.setattr(runner.os, "killpg", lambda *args: pytest.fail("identity changed"))
    monkeypatch.setattr(runner.os, "waitpid", lambda *args: pytest.fail("identity changed"))
    monkeypatch.setattr(runner.os, "close", lambda fd: None)
    with pytest.raises(RuntimeError, match="cleanup failed"):
        value.close()
    assert "start identity changed" in value.cleanup_failures[0]["message"]


def test_process_snapshot_records_no_argv_or_environment_and_uses_fixed_local_ps(monkeypatch):
    calls = []
    def observe(command, **options):
        calls.append((command, options))
        return type("Result", (), {"stdout":
            "741 740 741 Sat Oct 17 02:00:00 2026 Z+\n999 1 999 Sat Oct 17 01:00:00 2026 S\n"})()
    monkeypatch.setattr(runner.subprocess, "run", observe)
    assert runner.process_group_snapshot(741) == [{"pid":741,"ppid":740,"pgid":741,
        "started":"Sat Oct 17 02:00:00 2026","state":"Z+"}]
    command, options = calls[0]
    assert command == ["/bin/ps", "-axo", "pid=,ppid=,pgid=,lstart=,stat="]
    assert options["env"] == {"PATH":runner.os.defpath,"LC_ALL":"C"}


def test_preframe_process_diagnostics_do_not_enter_the_committed_vt_action_log():
    value = runner.RealTerminal.__new__(runner.RealTerminal)
    value.process_log, value.actions = io.StringIO(), io.StringIO()
    value._process_event("process_spawn", pid=741, expectedProcessGroup=741)
    record = json.loads(value.process_log.getvalue())
    assert record["event"] == "process_spawn" and record["pid"] == 741
    assert "frameId" not in record
    assert value.actions.getvalue() == ""
