#!/usr/bin/env python3
"""PTY acceptance: modal lifecycle, resize and optional real-model follow-up.

The default run uses an isolated first-launch profile. --model uses the installed
model configuration with a temporary project and conversation database.
Build: cargo build -p nola_cli --bin nola --example terminal_acceptance_oracle
Run: python3 scripts/e2e/test_cli_modal_session.py target/debug/nola [--model]
"""
import fcntl
import json
import os
from pathlib import Path
import shutil
import signal
import sqlite3
import subprocess
import struct
import sys
import tempfile
import termios


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from test_real_cli_terminal import Oracle, RealTerminal


def main():
    binary = str(Path(sys.argv[1]).resolve())
    project = tempfile.mkdtemp(prefix="nola-modal-acceptance-", dir="/tmp")
    print(f"Isolated project: {project}", flush=True)
    live = "--model" in sys.argv[2:]
    # Keep model accounts and the Host credential vault inside a disposable
    # directory. The PTY transcript remains in project for diagnosis.
    with tempfile.TemporaryDirectory(prefix="nola-modal-state-") as state:
        database = Path(state, "test.sqlite") if live else Path(project, "test.sqlite")
        env = dict(os.environ, TERM="xterm-256color",
                   NOLA_DB_PATH=str(database), NOLA_LLM_MAX_RETRIES="0")
        env.pop("NOLA_SKIP_ONBOARDING", None)
        config_dir = Path(env.get("NOLA_HOME") or Path.home()) / ".nola"
        home = Path(state, "home")
        (home / ".nola").mkdir(parents=True)
        env.update(HOME=str(home), NOLA_HOME=str(home))
        if live:
            path = next((config_dir / name for name in ("config.json", "config.jsonc")
                         if (config_dir / name).is_file()), None)
            if path is None:
                raise RuntimeError("Live test requires a configured model in ~/.nola/config.json or config.jsonc")
            # Only the model test processes see this disposable profile.
            config = home / ".nola" / path.name
            shutil.copyfile(path, config)
            os.chmod(config, 0o600)
            # A catalog is opened through Core. Single-route configurations
            # continue to be read at CLI startup, including commented JSONC.
            result = subprocess.run([binary, "model", "apply-config", str(config)], env=env,
                                    cwd=project, stdin=subprocess.DEVNULL, capture_output=True,
                                    text=True, timeout=30)
            if result.returncode and not any(message in result.stderr for message in
                                             ("not single-route fields", "missing llm object")):
                raise RuntimeError(f"Could not prepare the isolated model catalog: {result.stderr.strip()}")
        else:
            (home / ".nola/config.json").write_text(json.dumps({
                "llm": {"endpoint": "http://127.0.0.1:9/v1", "model": "first-run-test"}
            }))
        exercise(binary, project, database, live, env)


def exercise(binary, project, database, live, env):
    oracle_path = Path(binary).parent / "examples/terminal_acceptance_oracle"
    if not oracle_path.is_file():
        raise RuntimeError("Build the VT100 reader with cargo build -p nola_cli --example terminal_acceptance_oracle")
    oracle = Oracle(oracle_path)
    try:
        terminal = RealTerminal(Path(binary), Path(project), env, oracle, Path(project),
                                arguments=[binary, "--project", project, "--no-tools", "--lang", "en"])
    except BaseException:
        oracle.close()
        raise

    def resize(rows, columns):
        oracle.query(operation="resize", rows=rows, columns=columns)
        fcntl.ioctl(terminal.master, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))
        os.kill(terminal.pid, signal.SIGWINCH)

    def act(keys, description, predicate, timeout=15):
        before = terminal.latest.get("frameId", 0)
        terminal.action(description)
        terminal.write(keys.encode())
        terminal.wait(lambda: terminal.latest.get("frameId", 0) > before and predicate(terminal.screen()),
                      timeout, description)

    def composer(screen):
        return "Type a message" in screen and "[Cautious]" in screen

    conversations = set()

    def completed_reply(prompt, expected):
        # Durable run/message state, not repeated screen paint, is completion.
        try:
            with sqlite3.connect(f"file:{database}?mode=ro", uri=True, timeout=0.2) as db:
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
                    SELECT r.status,r.completed_at_ms,r.user_message_id,r.assistant_message_id,m.status,r.conversation_id
                    FROM runs r JOIN messages m ON m.id=r.assistant_message_id AND m.role='assistant'
                    ORDER BY r.created_at_ms DESC LIMIT 10
                """).fetchall()
                for status, completed, user_id, assistant_id, assistant_status, conversation in rows:
                    if message_text(user_id).strip() != prompt:
                        continue
                    if completed is None:
                        return False
                    if status != "sealed" or assistant_status != "completed":
                        raise AssertionError(f"Model run terminated with run={status}, message={assistant_status}")
                    if message_text(assistant_id).strip() == expected:
                        conversations.add(conversation)
                        return True
                    return False
                return False
        except sqlite3.OperationalError:
            return False

    try:
        terminal.wait(lambda: "Type a message" in terminal.screen(), 15, "initial composer")
        act("/guide\r", "guide returns to composer",
            lambda screen: project in screen and "Catalog:" in screen and composer(screen))
        act("/goal status\r", "goal commands visible",
            lambda screen: "/goal pause" in screen and "/goal resume" in screen and composer(screen))
        act("/goal\r", "enter goal composer", lambda screen: "What should Nola accomplish?" in screen
            and "Esc return to chat" in screen)
        act("Unsaved goal draft", "type a goal without starting", lambda screen: "Unsaved goal draft" in screen)
        act("\x1b", "cancel goal draft", lambda screen: composer(screen) and "Unsaved goal draft" not in screen)
        with sqlite3.connect(Path(database).resolve().as_uri() + "?mode=ro", uri=True) as db:
            assert db.execute("SELECT count(*) FROM runs").fetchone()[0] == 0, "Cancelling started a run"
        print("PASS: workspace guide, goal entry and cancellation without starting a model run", flush=True)
        for rows, columns in [(12, 40), (30, 100), (24, 80)]:
            act("/allow\r", "open permission selector",
                lambda screen: "Current: Cautious" in screen and "› 1  Cautious" in screen)
            resize(rows, columns)
            act("\x1b[B", "inspect Allow All without applying",
                lambda screen: terminal.latest["rows"] == rows and terminal.latest["columns"] == columns
                and "› 2  Allow All" in screen and "[Cautious]" in screen)
            if columns == 40:
                act("\x1b[6~", "read the end of the permission description",
                    lambda screen: "still apply." in screen and "1/2 apply immediately" in screen)
                act("\x1b[5~", "scroll back to selected mode",
                    lambda screen: "Current: Cautious" in screen and "› 2  Allow All" in screen)
            act("\x19\x1b[A", "Ctrl+Y leaves permission dialog open",
                lambda screen: "› 1  Cautious" in screen and "Current: Cautious" in screen)
            act("\x1b[B", "highlight Allow All again", lambda screen: "› 2  Allow All" in screen)
            act("\x1b", "cancel and retain Cautious",
                lambda screen: "Cancelled" in screen and "Permission Mode" not in screen and composer(screen))
        act("/allow\r", "reopen after cancelling", lambda screen: "› 1  Cautious" in screen)
        act("2", "apply second mode", lambda screen: "Type a message" in screen and "[Allow All]" in screen)
        act("/allow\r", "read back applied mode", lambda screen: "Current: Allow All" in screen and "› 2  Allow All" in screen)
        act("1", "restore first mode", composer)
        print("PASS: permission selection, paging, cancellation and apply/readback on committed VT100 frames", flush=True)
        if live:
            turns = [
                ("请只回复 MODAL-READY-741，不要调用工具。", "MODAL-READY-741"),
                ("沿用你上一条回复，把开头换成 FOLLOWUP，把末尾数字加一。只回复结果。", "FOLLOWUP-READY-742"),
            ]
            for prompt, expected in turns:
                terminal.write((prompt + "\r").encode())
                terminal.wait(lambda: completed_reply(prompt, expected)
                              and any(line.strip() == expected for line in terminal.screen().splitlines()),
                              120, "completed model answer on screen and in conversation")
            assert len(conversations) == 1, "Follow-up opened another conversation"
            print("PASS: real-model response and context-dependent follow-up in the same conversation", flush=True)
        if not live:
            print(f"Isolated evidence database: {database}", flush=True)
    finally:
        try:
            terminal.close()
        finally:
            oracle.close()


if __name__ == "__main__":
    main()
