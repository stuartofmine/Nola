#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Real LAN model -> Room -> live permission -> numeric PTY approval.

Only a disposable workspace file is requested. No mocked model/tool/permission
events; a model that does not ask for approval leaves this test uncovered.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import uuid
from urllib.parse import urlsplit

from test_real_agent_room_isolation import command, room_command, start_service, stop_service
from test_real_cli_terminal import Oracle, RealTerminal
from e2e.real_overwrite_permission import isolated_environment, scrub_case_credentials
from e2e.model_execution_fixture import fixture, runtime_capabilities


def main():
    parser = argparse.ArgumentParser()
    for name in ("binary", "service", "oracle"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    parser.add_argument("--config", type=Path)
    parser.add_argument("--synthetic-provider", action="store_true", help="Loopback fixture only; NOT real-model acceptance")
    args = parser.parse_args()
    root = Path(tempfile.mkdtemp(prefix="nola-room-permission-pty-"))
    workspace = root / "workspace"
    workspace.mkdir()
    target = workspace / "existing.txt"
    original, replacement = "ORIGINAL\n", f"ROOM_APPROVED_{uuid.uuid4().hex}\n"
    target.write_text(original)
    synthetic = None
    if args.synthetic_provider:
        assert args.config is None, "Synthetic mode must not read user configuration"
        from e2e.room_permission_fixture import RoomPermissionFixture
        synthetic = RoomPermissionFixture(target, replacement)
        source, source_bytes = synthetic.configuration, None
    else:
        assert args.config is not None, "Real acceptance requires an explicit authorized configuration"
        source_bytes = args.config.read_bytes()
        source = json.loads(source_bytes)
        endpoint = urlsplit(source["llm"]["endpoint"])
        assert endpoint.hostname == "192.168.31.18" and endpoint.port == 8081
    declaration = fixture(source, {}, require_tool=True) if synthetic else fixture(source, os.environ, require_tool=True)
    llm = dict(source["llm"], capabilities=runtime_capabilities(declaration))
    env = isolated_environment(root, "cautious")
    env.update(NOLA_APPLICATION_SERVICE_EXECUTABLE=str(args.service.resolve()),
        NOLA_CLI_INTERACTION_MODE="always", NOLA_LONG_TERM_MEMORY_ENABLED="0",
        TERM="xterm-256color", COLUMNS="100", LINES="24")
    config = root / "home/.nola/config.json"
    config.parent.mkdir(parents=True, exist_ok=True)
    with os.fdopen(os.open(config, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as output:
        json.dump({"llm": llm}, output)
    print(f"Isolated evidence: {root}", flush=True)
    service = terminal = oracle = None
    permission_seen = False
    try:
        service, sock = start_service(args.service.resolve(), root, env)
        with (root / "seed.events").open("w") as out, (root / "seed.stderr").open("w") as err:
            seed = subprocess.run([str(args.binary.resolve()), "Reply only READY. Do not use tools.",
                "--project", str(workspace), "--session", "permission-seed", "--no-skills", "--no-tools",
                "--max-iterations", "3", "--json", "--endpoint", llm["endpoint"],
                "--model", llm["model"], "--api-key", llm["api_key"]],
                env=env, stdout=out, stderr=err, timeout=180)
        assert seed.returncode == 0, "Seed failed; inspect fixture seed.stderr"
        with sqlite3.connect((root / "nola.db").as_uri() + "?mode=ro", uri=True) as db:
            settings = json.loads(db.execute("SELECT settings_json FROM conversation_settings WHERE conversation_id='permission-seed'").fetchone()[0])
        defaults = {key: settings[key] for key in ("modelSelectionID", "modelOverrides")}
        defaults.update(memoryEnabledOverride=False)
        directory = room_command(sock, type="directory")
        room_command(sock, type="configure_agent", input={"id":"files", "name":"Files",
            "description":"Performs explicitly requested file operations inside the disposable workspace.",
            "expected_directory_revision":directory["revision"], "expected_agent_revision":None,
            "defaults":defaults, "availability":"active", "updated_at_ms":1})
        room = room_command(sock, type="create", title="Permission fixture", at_ms=1)
        room = room_command(sock, type="join", input={"room_id":room["room"]["id"],
            "expected_room_revision":room["revision"], "agent_id":"files", "expected_agent_revision":1, "at_ms":2})
        member = room["room"]["participants"][0]
        oracle = Oracle(args.oracle.resolve())
        terminal = RealTerminal(args.binary.resolve(), workspace, env, oracle, root,
            arguments=[str(args.binary.resolve()), "--project", str(workspace), "--no-skills", "--lang", "en"])
        terminal.wait(lambda: "Type a message" in terminal.screen(), 20, "composer")
        prompt = (f"Replace the entire contents of {target} with exactly {replacement.strip()} and a final newline. "
            "Use an appropriate available file tool. This overwrites an existing disposable fixture. "
            "Do not access other paths, use shell commands, network, browser, devices, or alternative methods. "
            "Retain any required approval; after executing read this file back and report its exact content.")
        terminal.action("submit_room_file_request")
        terminal.write(f"/room {room['room']['id']} @Files {prompt}\r".encode())
        terminal.wait(lambda: "awaiting permission" in terminal.screen(), 150, "live room permission panel (not covered if absent)")
        permission_seen = True
        assert target.read_text() == original, "Write occurred before approval"
        pending = room_command(sock, type="submissions", room_id=room["room"]["id"], limit=10, offset=0)[0]
        assert pending["state"] == "executing" and pending["execution_id"]
        assert pending["context_conversation_id"] == member["context_conversation_id"]
        panel = terminal.screen()
        assert "existing.txt" in panel and "write_file" in panel, "Only the fixture write is authorized"
        assert "replace full file" in panel, "The approval must explicitly describe replacement"
        for text in ("Mode:", "UTF-8 bytes:", "Content SHA-256:"):
            line = next(line for line in panel.splitlines() if text in line)
            assert line.count("│") == 2, "Every safe-review line must remain inside the panel"
        assert hashlib.sha256(replacement.encode()).hexdigest() in "".join(panel.split()), "Only exact fixture bytes are authorized"
        terminal.action("approve_once_digit_only", runId=pending["execution_id"])
        terminal.write(b"1")  # No Enter: numeric choice is the confirmation.
        if synthetic:
            terminal.wait(lambda: target.read_text() == replacement, 30, "real write after synthetic-provider approval")
            terminal.action("cancel_after_observed_write")
            terminal.write(b"\x03")
            terminal.wait(lambda: "Cancelled" in terminal.screen() and "Type a message" in terminal.screen(), 30, "canonical cancelled receipt and composer after cancellation")
            observed = room_command(sock, type="submissions", room_id=room["room"]["id"], limit=10, offset=0)[0]
            assert observed["execution_id"] == pending["execution_id"]
            assert observed["state"] == "cancelled", "Only canonical cancellation confirms cancellation"
            idle = [frame for frame in terminal.frames if "Type a message" in frame["screen"] and "awaiting permission" not in frame["screen"]]
            assert idle and all(frame["screen"].count("Type a message") == 1 and frame["screen"].count("Context") == 1 for frame in idle)
            (root / "evidence.json").write_text(json.dumps({"passed":True,"mode":"synthetic_provider_real_core_host_pty",
                "real_model_verified":False,"completion_verified":False,"permission_panel":panel,
                "pending":pending,"observed":observed,"numeric_choice_without_enter":True,
                "unchanged_before_approval":True,"fixture_bytes_match":True,"single_idle_footer":True,
                "committed_frames":len(terminal.frames)}, indent=2))
            print("PASS synthetic provider only: real Room approval/write/PTY; no real-model or completion claim", flush=True)
            return
        terminal.wait(lambda: "Completed" in terminal.screen() and "Type a message" in terminal.screen(),
            150, "original room execution after digit approval")
        completed = room_command(sock, type="submissions", room_id=room["room"]["id"], limit=10, offset=0)[0]
        assert completed["execution_id"] == pending["execution_id"] and completed["state"] == "completed"
        assert target.read_text() == replacement
        with sqlite3.connect((root / "nola.db").as_uri() + "?mode=ro", uri=True) as db:
            runs = db.execute("SELECT id,status FROM runs WHERE conversation_id=?", (member["context_conversation_id"],)).fetchall()
        assert runs == [(pending["execution_id"], "sealed")], runs
        idle = [frame for frame in terminal.frames if "Type a message" in frame["screen"]
            and "awaiting permission" not in frame["screen"]]
        assert idle and all(frame["screen"].count("Type a message") == 1 and frame["screen"].count("Context") == 1 for frame in idle)
        (root / "evidence.json").write_text(json.dumps({"passed":True, "permission_panel":panel,
            "pending":pending, "completed":completed, "room_runs":runs, "numeric_choice_without_enter":True,
            "unchanged_before_approval":True, "fixture_bytes_match":True, "single_idle_footer":True,
            "committed_frames":len(terminal.frames)}, indent=2))
        print("PASS: live Room permission, digit-only approval, original sealed Run, fixture bytes, single footer", flush=True)
    except Exception as failure:
        (root / "evidence.json").write_text(json.dumps({"passed":False,
            "permission_panel_observed":permission_seen, "failure_type":type(failure).__name__,
            "fixture_unchanged":target.read_text() == original}, indent=2))
        raise
    finally:
        cleanup_errors = []
        for cleanup in (
            terminal.close if terminal is not None else None,
            oracle.close if oracle is not None else None,
            (lambda: stop_service(service)) if service is not None else None,
            lambda: scrub_case_credentials(root),
            synthetic.close if synthetic is not None else None,
        ):
            if cleanup is not None:
                try:
                    cleanup()
                except Exception as failure:
                    cleanup_errors.append(type(failure).__name__)
        evidence_path = root / "evidence.json"
        if evidence_path.exists():
            evidence = json.loads(evidence_path.read_text())
            evidence["interaction_assertions_passed"] = evidence.get("passed", False)
            evidence["cleanup_verified"] = not cleanup_errors
            evidence["cleanup_errors"] = cleanup_errors
            evidence["passed"] = evidence.get("passed", False) and not cleanup_errors
            evidence_path.write_text(json.dumps(evidence, indent=2))
        if synthetic is None:
            assert hashlib.sha256(args.config.read_bytes()).digest() == hashlib.sha256(source_bytes).digest()
        if cleanup_errors:
            raise RuntimeError("Isolated cleanup failed; inspect terminal-processes.jsonl")


if __name__ == "__main__":
    main()
