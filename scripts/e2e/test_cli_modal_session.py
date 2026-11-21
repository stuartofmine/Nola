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
        act("/help\r", "compact help fits the initial viewport", lambda screen:
            "/help all" in screen and "/edit" in screen and "/recover retry" not in screen and composer(screen))
        act("/help /permissions\r", "look up an alias", lambda screen:
            sum(row.strip().startswith("/allow ") for row in screen.splitlines()) >= 2 and composer(screen))
        act("/help goal\r", "read goal start and pause syntax", lambda screen:
            "/goal pause" in screen and "--constraints" in screen and composer(screen))
        act("/help all\r", "show the full command directory", lambda screen:
            "/recover" in screen and "/quit" in screen and composer(screen))
        if not live:
            def editing(text):
                return lambda screen: any(row.strip() == text for row in screen.splitlines())
            def highlighted(screen, token):
                return any(row.strip().startswith("› " + token + "  ") for row in screen.splitlines())
            act("/m", "show command names and descriptions", lambda s: editing("/m")(s) and highlighted(s, "/model"))
            act("\x1b[B", "preview MCP without overwriting the filter", lambda s: editing("/m")(s) and highlighted(s, "/mcp"))
            act("\x1b", "dismiss while retaining the filter", lambda s: editing("/m")(s) and not highlighted(s, "/mcp"))
            resize(22, 62)
            terminal.wait(lambda: terminal.latest["columns"] == 62 and editing("/m")(terminal.screen())
                          and "Esc close" not in terminal.screen(), 10, "dismissal survives resize")
            act("c", "typing reopens filtered suggestions", lambda s: editing("/mc")(s) and highlighted(s, "/mcp"))
            act("\tlist", "Tab completes and leaves room for an argument", editing("/mcp list"))
            terminal.write(b"\x01\x0b")
            resize(24, 80)
            act("unsent draft", "type a draft", editing("unsent draft"))
            act("\x1b[B!", "down preserves a draft outside history", editing("unsent draft!"))
            terminal.write(b"\x01\x0b")
            for name in ("sample-a.md", "sample-b.md"):
                Path(project, name).write_text("local fixture")
            act("@sample-", "show workspace completions", lambda screen: "@sample-a.md" in screen and "@sample-b.md" in screen)
            act("\x1b[B\r", "insert the second file without submitting a turn", editing("@sample-b.md"))
            with sqlite3.connect(Path(database).resolve().as_uri() + "?mode=ro", uri=True) as db:
                assert db.execute("SELECT count(*) FROM runs").fetchone()[0] == 0
            terminal.write(b"\x01\x0b")
            print("PASS: compact help, slash and file suggestions, draft preservation, and no run on file insertion", flush=True)
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
                    lambda screen: "still apply." in screen and "1/2 select · Enter applies" in screen)
                act("\x1b[5~", "scroll back to selected mode",
                    lambda screen: "Current: Cautious" in screen and "› 2  Allow All" in screen)
            act("\x19\x1b[A", "Ctrl+Y leaves permission dialog open",
                lambda screen: "› 1  Cautious" in screen and "Current: Cautious" in screen)
            act("\x1b[B", "highlight Allow All again", lambda screen: "› 2  Allow All" in screen)
            act("\x1b", "cancel and retain Cautious",
                lambda screen: "Cancelled" in screen and "Permission Mode" not in screen and composer(screen))
        act("/allow\r", "reopen after cancelling", lambda screen: "› 1  Cautious" in screen)
        act("2", "number previews second mode", lambda s: "› 2  Allow All" in s and "Current: Cautious" in s)
        act("\x1b", "cancel numbered choice", composer)
        act("/allow\r", "cancelled digit was not saved", lambda s: "› 1  Cautious" in s and "Current: Cautious" in s)
        act("2\r", "confirm second mode", lambda screen: "Type a message" in screen and "[Allow All]" in screen)
        act("/allow\r", "read back applied mode", lambda screen: "Current: Allow All" in screen and "› 2  Allow All" in screen)
        act("1\r", "restore first mode", composer)
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
            # The offline route intentionally fails. A failed message must still be
            # editable, including replacements whose text matches a CLI command.
            terminal.write(b"Original modal test message\r")
            def sealed_conversation():
                with sqlite3.connect(database) as db:
                    return db.execute("SELECT conversation_id FROM runs WHERE completed_at_ms IS NOT NULL").fetchone()
            def failed_turn_is_idle():
                screen = terminal.screen()
                return sealed_conversation() and composer(screen) and "error:" in screen and "Esc interrupt" not in screen
            terminal.wait(failed_turn_is_idle, 45, "failed turn ready for editing")
            conversation = sealed_conversation()[0]
            act("retain-me", "type a new draft", editing("retain-me"))
            act("\x1b[D\x1b[A", "recall the preceding message",
                editing("Original modal test message"))
            act("\x01\x0b/m", "filter commands while editing a recalled message",
                lambda s: editing("/m")(s) and highlighted(s, "/model"))
            act("\x1b[B", "navigate suggestions inside the recalled draft",
                lambda s: editing("/m")(s) and highlighted(s, "/mcp"))
            act("\x1b", "dismiss suggestions before leaving history", editing("/m"))
            act("\x1b[BX", "return to the saved draft and caret", editing("retain-mXe"))
            terminal.write(b"\x01\x0b")
            act("/edit", "type edit command", lambda s: any(row.strip() == "/edit" for row in s.splitlines()))
            act("\r", "open sent message in composer", lambda s: "Esc to cancel" in s and "Original modal test message" in s)
            act("\x01\x0b/status", "replace sent text with slash command", lambda s: "/status" in s and "Enter to resend" in s)
            terminal.write(b"\r")
            def edited_turn_finished():
                with sqlite3.connect(database) as db:
                    return db.execute("SELECT count(*) FROM runs WHERE conversation_id=? AND completed_at_ms IS NOT NULL",
                                      (conversation,)).fetchone()[0] >= 2
            terminal.wait(lambda: edited_turn_finished() and composer(terminal.screen()), 45, "resend finished")
            saved = subprocess.run([binary, "--json", "session", "show", conversation], env=env, cwd=project,
                                   check=True, text=True, capture_output=True, timeout=20)
            messages = json.loads(saved.stdout)["messages"]
            assert [m["content"] for m in messages if m["role"] == "user"] == ["/status"], \
                "Reopening the same conversation must show only the replacement"
            print("PASS: editing a failed message resends slash text and rewinds the same conversation", flush=True)
            first = "甲" * 320
            second = "乙" * 320
            def paste(text):
                terminal.write(b"\x1b[200~" + text.encode() + b"\x1b[201~")
            # A sealed run can still be handing the composer back to the idle
            # editor. Its command palette identifies the active keyboard owner.
            act("/m", "idle editor owns the keyboard after resend",
                lambda s: editing("/m")(s) and highlighted(s, "/model"))
            act("\x01\x0b", "clear the command filter", composer)
            paste(first)
            terminal.wait(lambda: "[Pasted Content 320 chars]" in terminal.screen(), 10, "long paste collapsed")
            terminal.write(b"\x01")  # insert the second block before the first
            paste(second)
            terminal.write(b"\x01")
            literal = "[Pasted Content 320 chars]"
            paste(literal)
            terminal.write(b"\r")
            with sqlite3.connect(database) as db:
                terminal.wait(lambda: db.execute("SELECT count(*) FROM runs WHERE conversation_id=? AND completed_at_ms IS NOT NULL",
                                                  (conversation,)).fetchone()[0] >= 3, 45, "pasted message sealed")
            saved = subprocess.run([binary, "--json", "session", "show", conversation], env=env, cwd=project,
                                   check=True, text=True, capture_output=True, timeout=20)
            messages = [m["content"] for m in json.loads(saved.stdout)["messages"] if m["role"] == "user"]
            assert messages[-1] == literal + second + first, "Large pastes must preserve insertion order and literal text"
            print("PASS: same-size pastes inserted out of order and literal marker survived session reopen", flush=True)
            print(f"Isolated evidence database: {database}", flush=True)
    finally:
        try:
            terminal.close()
        finally:
            oracle.close()


if __name__ == "__main__":
    main()
