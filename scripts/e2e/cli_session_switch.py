# SPDX-License-Identifier: Apache-2.0
"""Observe real CLI session switching. No model response or view state is seeded."""
import hashlib
import json
import signal

from test_real_cli_terminal import Oracle, RealTerminal


def exercise_session_switch(binary, project, env, root, oracle_binary):
    database = root / "nola.db"
    scope = hashlib.sha256(str(database.resolve()).encode()).hexdigest()
    view_root = root / "CliViewState" / scope

    def view(session):
        path = view_root / (hashlib.sha256(session.encode()).hexdigest() + ".json")
        return json.loads(path.read_text())["state"] if path.exists() else None

    terminal = oracle = None
    try:
        # Populate each draft through its actual terminal, then close without
        # submitting it. No synthetic ViewStore JSON can make this test pass.
        for index in range(2):
            case = root / f"draft-seed-{index}"
            case.mkdir()
            oracle = Oracle(oracle_binary)
            terminal = RealTerminal(binary, project, env, oracle, case, arguments=[str(binary),
                "--session", f"offline-session-{index}", "--project", str(project), "--lang", "en"])
            terminal.wait(lambda: "Type a message" in terminal.screen(), 15, "empty session editor")
            draft = f"SESSION_{index}_草稿甲乙"
            terminal.write(draft.encode() + b"\x1b[D\x1b[D")
            terminal.wait(lambda: draft in terminal.screen() and
                (view(f"offline-session-{index}") or {}).get("cursor") == len(f"SESSION_{index}_草稿".encode()),
                10, "owned draft and UTF-8 cursor persisted")
            terminal._signal_owned_group(signal.SIGHUP)
            terminal.wait(lambda: terminal.pty_closed, 10, "owned terminal closed")
            terminal.close()
            terminal = None
            oracle.close()
            oracle = None
        case = root / "session-switch"
        case.mkdir()
        oracle = Oracle(oracle_binary)
        terminal = RealTerminal(binary, project, env, oracle, case, arguments=[str(binary),
            "--session", "offline-session-0", "--project", str(project), "--lang", "en"])
        terminal.wait(lambda: "SESSION_0_草稿甲乙" in terminal.screen(), 15, "first draft restored")
        assert "SESSION_1_" not in terminal.screen()
        # Inserting at the restored cursor verifies the cursor itself, not only
        # a JSON field. Clear is an explicit user action before slash navigation.
        terminal.write(b"X")
        terminal.wait(lambda: "SESSION_0_草稿X甲乙" in terminal.screen(), 5, "restored cursor edits right position")
        terminal.write(b"\x05\x15/resume\r")
        before = len(terminal.output)
        terminal.wait(lambda: b"Recent sessions" in terminal.output[before:], 10, "in-process picker")
        terminal.write(b"\r")  # The untouched second archive has the newer timestamp.
        terminal.wait(lambda: "SESSION_1_草稿甲乙" in terminal.screen(), 15, "second session owns second draft")
        assert "SESSION_0_" not in terminal.screen(), "Other conversation draft leaked into screen"
        assert view("offline-session-0")["draft"] == ""
        assert view("offline-session-1")["draft"] == "SESSION_1_草稿甲乙"
        terminal.write(b"Y")
        terminal.wait(lambda: "SESSION_1_草稿Y甲乙" in terminal.screen(), 5, "second restored cursor")
        terminal.write(b"\x05\x15/resume\r")
        before = len(terminal.output)
        terminal.wait(lambda: b"Recent sessions" in terminal.output[before:], 10, "second picker")
        terminal.write(b"\x1b[B\r")
        terminal.wait(lambda: "Type a message" in terminal.screen() and "Archived offline fixture 0" in terminal.screen(),
                      15, "return to cleared first conversation")
        assert "SESSION_1_" not in terminal.screen() and "SESSION_0_" not in terminal.screen()
        assert view("offline-session-1")["draft"] == ""
        return {"two_drafts_restored": True, "in_process_switch_isolated": True,
                "both_utf8_cursors_verified_by_edit": True, "cleared_draft_not_resurrected": True}
    finally:
        try:
            if terminal:
                # Cleanup must never submit the unsent draft as a model request.
                terminal._signal_owned_group(signal.SIGHUP)
                terminal.close()
        finally:
            if oracle:
                oracle.close()
