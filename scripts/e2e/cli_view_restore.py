"""Real PTY view restoration; the VT oracle only observes product output."""
import base64
import json
import signal
import time

import pexpect

from test_real_cli_terminal import Oracle


def exercise_view_restore(
    child, binary, session, project, env, root, log, oracle_binary, restore_columns=100
):
    oracle = Oracle(oracle_binary)

    class Capture:
        def __init__(self):
            self.latest = {}
            self.frames = []

        def write(self, text):
            frames = oracle.query(
                operation="feed", bytes_base64=base64.b64encode(text.encode()).decode()
            )["frames"]
            if frames:
                self.latest = frames[-1]
                self.frames.extend(frames)

        def flush(self):
            pass

    def settle():
        deadline = time.monotonic() + 0.5
        while time.monotonic() < deadline:
            try:
                child.read_nonblocking(65536, timeout=0.1)
            except pexpect.TIMEOUT:
                pass

    try:
        capture = Capture()
        child.logfile_read = capture
        # A complete repaint lets the observer attach after the initial screen.
        oracle.query(operation="resize", rows=25, columns=100)
        child.setwinsize(25, 100)
        settle()
        oracle.query(operation="resize", rows=24, columns=100)
        child.setwinsize(24, 100)
        settle()
        child.send("恢复草稿甲乙")
        child.send("\x1b[D\x1b[D\x1b[5~")
        child.expect("浏览历史")
        settle()
        before = capture.latest
        state_files = list(root.glob("CliViewState/*/*.json"))
        assert len(state_files) == 1
        saved = json.loads(state_files[0].read_text())["state"]
        assert saved["draft"] == "恢复草稿甲乙"
        assert saved["cursor"] == len("恢复草稿".encode())
        assert saved["viewport"]["reading"]
        (root / "view-before.json").write_text(json.dumps(before, ensure_ascii=False))
        # Close only this test's owned PTY process; no user process is touched.
        child.kill(signal.SIGHUP)
        child.expect(pexpect.EOF)
        child.close()
        oracle.close()
        oracle = Oracle(oracle_binary)
        oracle.query(operation="resize", rows=24, columns=restore_columns)
        capture = Capture()
        child = pexpect.spawn(
            binary,
            ["--session", session, "--project", project],
            env=env,
            encoding="utf-8",
            timeout=30,
            dimensions=(24, restore_columns),
        )
        child.logfile = log
        child.logfile_read = capture
        child.expect("恢复草稿甲乙")
        settle()
        after = capture.latest
        (root / "view-after.json").write_text(json.dumps(after, ensure_ascii=False))
        (root / "view-restore-frames.json").write_text(
            json.dumps(capture.frames, ensure_ascii=False)
        )
        assert after["cursorColumn"] == before["cursorColumn"]
        if restore_columns == 100:
            assert (
                after["screen"].splitlines()[:15] == before["screen"].splitlines()[:15]
            )
        else:
            normalize = lambda text: "".join(text.split())
            expected = next(
                line for line in before["screen"].splitlines() if line.strip()
            )
            assert normalize(expected) in normalize(
                "".join(after["screen"].splitlines()[:5])
            )
            assert "浏览历史" in after["screen"]
        child.send("X")
        settle()
        assert json.loads(state_files[0].read_text())["state"]["draft"] == "恢复草稿X甲乙"
        child.sendcontrol("c")
        child.expect("输入消息")
        child.logfile_read = None
        return child
    except BaseException:
        if child.isalive():
            child.terminate(force=True)
        raise
    finally:
        oracle.close()
