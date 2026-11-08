#!/usr/bin/env python3
"""PTY acceptance: modal lifecycle, resize and optional real-model follow-up.

Uses the installed model config without printing credentials. All conversation
state is isolated in a temporary project/database; HOME is never changed.
Run: python3 scripts/e2e/test_cli_modal_session.py target/debug/nola [--model]
"""
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import signal
import sqlite3
import struct
import sys
import tempfile
import termios
import time


def main():
    binary = str(Path(sys.argv[1]).resolve())
    project = tempfile.mkdtemp(prefix="nola-modal-acceptance-", dir="/tmp")
    print(f"Isolated project: {project}", flush=True)
    pid, master = pty.fork()
    if pid == 0:
        os.chdir(project)
        env = dict(os.environ, TERM="xterm-256color", NOLA_SKIP_ONBOARDING="1",
                   NOLA_DB_PATH=f"{project}/test.sqlite", NOLA_LLM_MAX_RETRIES="0")
        os.execve(binary, [binary, "--project", project, "--no-tools", "--lang", "en"], env)
    os.set_blocking(master, False)
    output = bytearray()

    def resize(rows, columns):
        fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))
        os.kill(pid, signal.SIGWINCH)

    def visible():
        return re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", output).decode("utf-8", "replace")

    def pump():
        try:
            output.extend(os.read(master, 65536))
        except BlockingIOError:
            time.sleep(0.02)
        except OSError:
            Path(project, "terminal.raw").write_bytes(output)
            raise AssertionError(f"CLI PTY closed unexpectedly: {project}/terminal.raw")

    def wait_for(predicate, timeout=15):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            pump()
            if predicate(visible()):
                return
        Path(project, "terminal.raw").write_bytes(output)
        raise AssertionError(f"PTY did not reach the expected state; capture at {project}/terminal.raw")

    def send(text):
        os.write(master, text.encode())

    def completed_reply(marker):
        # Durable run/message state, not repeated screen paint, is completion.
        try:
            with sqlite3.connect(f"file:{project}/test.sqlite?mode=ro", uri=True, timeout=0.2) as db:
                def message_text(message_id):
                    parts = db.execute("""
                        SELECT p.id, p.content_text, p.content_json FROM message_parts p
                        WHERE p.message_id=? AND p.kind='text' ORDER BY p.ordinal
                    """, (message_id,)).fetchall()
                    text = []
                    for part_id, inline, encoded in parts:
                        fields = {"content_text": inline, "content_json": encoded}
                        references = db.execute("SELECT field,reference_json FROM message_part_body_references WHERE part_id=?", (part_id,)).fetchall()
                        for field, reference in references:
                            content_id = json.loads(reference)["content_id"]
                            chunks = db.execute("SELECT text FROM canonical_text_chunks WHERE content_id=? ORDER BY offset_bytes", (content_id,))
                            fields[field] = "".join(row[0] for row in chunks)
                        # Canonical assistant parts store the typed Text part
                        # in content_json; user messages may use content_text.
                        body = fields["content_text"]
                        if not body and fields["content_json"]:
                            part = json.loads(fields["content_json"])
                            if part.get("kind") == "text":
                                body = part.get("text", "")
                        text.append(body)
                    return "".join(text)

                rows = db.execute("""
                    SELECT r.status,r.completed_at_ms,r.user_message_id,r.assistant_message_id
                    FROM runs r JOIN messages m ON m.id=r.assistant_message_id AND m.role='assistant'
                    ORDER BY r.created_at_ms DESC LIMIT 10
                """).fetchall()
                for status, completed, user_id, assistant_id in rows:
                    if marker not in message_text(user_id):
                        continue
                    if completed is not None and status != "completed":
                        raise AssertionError(f"Model run terminated without success: {status}")
                    return (status == "completed" and completed is not None
                            and marker in message_text(assistant_id))
                return False
        except sqlite3.OperationalError:
            return False

    try:
        resize(24, 80)
        wait_for(lambda value: "Type a message" in value)
        for rows, columns in [(12, 40), (30, 100), (24, 80)]:
            start = len(visible())
            send("/allow\r")
            wait_for(lambda value: "Permission Mode" in value[start:])
            resize(rows, columns)
            send("\x19")  # Ctrl+Y must not approve anything.
            send("\x1b")
            wait_for(lambda value: "Cancelled" in value[start:])
        print("PASS: /allow cancellation, Ctrl+Y and three terminal sizes", flush=True)
        if "--model" in sys.argv[2:]:
            for marker in ["MODAL-READY-741", "FOLLOWUP-READY-742"]:
                start = len(visible())
                send(f"请只回复 {marker}，不要调用工具。\r")
                wait_for(lambda _value: completed_reply(marker), 120)
            print("PASS: real-model response and natural follow-up", flush=True)
        print(f"Isolated evidence database: {project}/test.sqlite", flush=True)
    finally:
        Path(project, "terminal.raw").write_bytes(output)
        try:
            os.kill(pid, signal.SIGTERM)
            deadline = time.monotonic() + 2
            while time.monotonic() < deadline:
                if os.waitpid(pid, os.WNOHANG)[0]:
                    break
                time.sleep(0.02)
            else:
                os.kill(pid, signal.SIGKILL)
                # Kernel-stalled children can ignore even SIGKILL temporarily.
                # Never strand the acceptance runner in an unbounded waitpid.
                os.waitpid(pid, os.WNOHANG)
        except ProcessLookupError:
            pass
        os.close(master)


if __name__ == "__main__":
    main()
