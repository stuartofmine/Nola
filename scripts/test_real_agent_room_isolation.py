#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Room Application IPC -> canonical contexts -> real CLI/Core/model isolation.

Uses an isolated database and the required LAN model. Never imports user history.
This verifies context isolation, cold-process recall, and real unaddressed room
dispatch through coordinator, frozen intent, and canonical Run seal.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import uuid
from urllib.parse import urlsplit

sys.path.insert(0, str(Path(__file__).resolve().parent / "e2e"))
from e2e.real_overwrite_permission import isolated_environment, scrub_case_credentials
from test_real_cli_commit_followup import assert_completed_turn


def command(sock, value):
    value = dict(value)
    envelope = {"protocol_version": 53, "request_id": str(uuid.uuid4()),
                "client_id": "room-isolation-acceptance", "expected_revision": None,
                "command": {"type": value.pop("type"), "arguments": value}}
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(30)
        connection.connect(str(sock))
        connection.sendall(json.dumps(envelope).encode() + b"\n")
        with connection.makefile("rb") as response:
            receipt = json.loads(response.readline())
    assert receipt["status"] == "committed", receipt
    return receipt["result"]


def room_command(sock, **fields):
    return command(sock, {"type": "agent_room", "command": fields})


def start_service(binary, root, environment):
    # Match ApplicationServiceClient::socket_path_for_database so independent
    # CLI hosts use this one explicit service owner, like mobile clients do.
    path_hash = 0xcbf29ce484222325
    for byte in str(root / "nola.db").encode():
        path_hash = ((path_hash ^ byte) * 0x100000001b3) & ((1 << 64) - 1)
    sock = Path(f"/tmp/nola-application-v53-{path_hash:016x}.sock")
    # CLI's child service relinquishes its database lock after its parent exits.
    # Retry startup only; never recreate or repair the database.
    for _ in range(20):
        with (root / "service.stderr").open("a") as errors:
            process = subprocess.Popen([str(binary), "--database", str(root / "nola.db"),
                                        "--socket", str(sock)], env=environment,
                                       stdout=subprocess.DEVNULL, stderr=errors)
        for _ in range(200):
            if process.poll() is not None:
                break
            if sock.exists():
                try:
                    command(sock, {"type": "health"})
                    # Do not mistake the previous CLI child's still-live
                    # socket for our service winning database ownership.
                    time.sleep(0.1)
                    if process.poll() is None:
                        return process, sock
                    break
                except (OSError, ValueError):
                    pass
            time.sleep(0.025)
        if process.poll() is None:
            process.terminate()
            process.wait()
        time.sleep(0.1)
    raise AssertionError("Application service startup timed out")


def stop_service(process):
    process.terminate()
    process.wait(timeout=15)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--service", type=Path, required=True)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--concurrent", action="store_true", help="Also prove two room Runs coexist and recover independently")
    parser.add_argument("--interactive-oracle", type=Path, help="Verify /room @ completion and automatic routing in a real PTY")
    args = parser.parse_args()
    llm = json.loads(args.config.read_text())["llm"]
    endpoint = urlsplit(llm["endpoint"])
    assert endpoint.hostname == "192.168.31.18" and endpoint.port == 8081
    root = Path(tempfile.mkdtemp(prefix="nola-room-isolation-"))
    workspace = root / "workspace"
    workspace.mkdir()
    environment = isolated_environment(root, "cautious")
    environment["NOLA_LONG_TERM_MEMORY_ENABLED"] = "0"
    environment["NOLA_APPLICATION_SERVICE_EXECUTABLE"] = str(args.service.resolve())
    config = root / "home/.nola/config.json"
    config.parent.mkdir(parents=True, exist_ok=True)
    with os.fdopen(os.open(config, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as output:
        json.dump({"llm": llm}, output)
    print(f"Isolated evidence: {root}", flush=True)
    service = None
    try:
        # Own the fixture service explicitly before the first CLI call. Do not
        # depend on a cold, newly linked binary winning autospawn's deadline.
        service, sock = start_service(args.service.resolve(), root, environment)
        # Register the model and credential through the normal CLI owner, then
        # capture only model defaults for new room contexts (never seed history).
        with (root / "seed.events").open("w") as out, (root / "seed.stderr").open("w") as err:
            seed = subprocess.run([str(args.binary.resolve()), "Reply only READY. Do not use tools.",
                "--project", str(workspace), "--session", "model-registration-seed", "--no-skills",
                "--max-iterations", "3", "--json", "--endpoint", llm["endpoint"],
                "--model", llm["model"], "--api-key", llm["api_key"]],
                env=environment, stdout=out, stderr=err, timeout=180)
        assert seed.returncode == 0, f"Model registration failed; inspect {root / 'seed.stderr'}"
        with sqlite3.connect((root / "nola.db").as_uri() + "?mode=ro", uri=True) as db:
            settings = json.loads(db.execute("SELECT settings_json FROM conversation_settings WHERE conversation_id=?",
                ("model-registration-seed",)).fetchone()[0])
        defaults = {key: settings[key] for key in ("modelSelectionID", "modelOverrides")}
        defaults["memoryEnabledOverride"] = False
        command(sock, {"type": "put_product_setting", "key": "chat.default_model",
            "value_json": json.dumps({"version": 1, "selection": {"kind": "profile",
                "profile_id": defaults["modelSelectionID"].removeprefix("configured:")}}), "updated_at_ms": 1})
        directory = room_command(sock, type="directory")
        for name in ("a", "b"):
            directory = room_command(sock, type="configure_agent", input={
                "id": name, "name": name, "expected_directory_revision": directory["revision"],
                "description": "Arithmetic specialist; handles addition and numeric calculations." if name == "a" else "Language specialist; handles translation and grammar, not arithmetic.",
                "expected_agent_revision": None, "defaults": defaults,
                "availability": "active", "updated_at_ms": 1})
        room = room_command(sock, type="create", title="Isolation", at_ms=1)
        for name in ("a", "b"):
            room = room_command(sock, type="join", input={"room_id": room["room"]["id"],
                "expected_room_revision": room["revision"], "agent_id": name,
                "expected_agent_revision": 1, "at_ms": 2})
        other = room_command(sock, type="create", title="Fresh room", at_ms=3)
        other = room_command(sock, type="join", input={"room_id": other["room"]["id"],
            "expected_room_revision": other["revision"], "agent_id": "a",
            "expected_agent_revision": 1, "at_ms": 4})
        members = room["room"]["participants"] + other["room"]["participants"]
        contexts = [member["context_conversation_id"] for member in members]
        assert len(set(contexts)) == 3
        for context in contexts:
            command(sock, {"type": "patch_conversation_configuration", "input": {
                "conversation_id": context, "expected_revision": 1,
                "patch_json": json.dumps({"permissionProfile": "cautious"}), "updated_at_ms": 5}})
        stop_service(service)
        service = None
        markers = ["ROOM_A_" + uuid.uuid4().hex, "ROOM_B_" + uuid.uuid4().hex]
        turns = [(0, f"For this conversation only, my private token is {markers[0]}. Reply only STORED. Do not use tools or persistent memory."),
                 (1, f"For this conversation only, my private token is {markers[1]}. Reply only STORED. Do not use tools or persistent memory."),
                 (0, "What is my private token? Reply with the exact token only. Do not use tools."),
                 (1, "What is my private token? Reply with the exact token only. Do not use tools."),
                 (2, "What is my private token? If none was provided in this conversation, reply only UNKNOWN. Do not use tools.")]
        def run_turn(label, member, prompt):
            step = root / label
            step.mkdir()
            with (step / "events.ndjson").open("w") as out, (step / "stderr.log").open("w") as err:
                result = subprocess.run([str(args.binary.resolve()), prompt, "--project", str(workspace),
                    "--session", contexts[member], "--no-skills", "--max-iterations", "3", "--json",
                    "--endpoint", llm["endpoint"], "--model", llm["model"], "--api-key", llm["api_key"]],
                    env=environment, stdout=out, stderr=err, timeout=180)
            assert result.returncode == 0, f"CLI {label} failed; inspect {step}"
            events = [json.loads(line) for line in (step / "events.ndjson").read_text().splitlines()]
            assert_completed_turn(events)
            assert not any(event.get("trace", {}).get("kind") in {"tool.started", "tool.completed"}
                           for event in events), "Tool use invalidates the pure context isolation sample"
            print(f"Completed canonical {label}", flush=True)
        for index, (member, prompt) in enumerate(turns):
            run_turn(f"turn-{index + 1}", member, prompt)
        if args.concurrent:
            service, sock = start_service(args.service.resolve(), root, environment)
            overlap = []
            # Both are actual CLI/Core Runs; a read-only database observer
            # proves simultaneous nonterminal ownership even if the provider
            # schedules its token generation serially.
            with ThreadPoolExecutor(max_workers=2) as workers:
                futures = [workers.submit(run_turn, f"concurrent-{member}", member,
                    "Output my private token, then write the integers 1 through 150 in order. "
                    "Do not use tools or persistent memory.") for member in (0, 1)]
                with sqlite3.connect((root / "nola.db").as_uri() + "?mode=ro", uri=True) as db:
                    while not all(future.done() for future in futures):
                        active = db.execute("SELECT conversation_id,id,status FROM runs "
                            "WHERE conversation_id IN (?,?) AND status<>'sealed' ORDER BY conversation_id",
                            contexts[:2]).fetchall()
                        if len({row[0] for row in active}) == 2 and not overlap:
                            overlap = active
                        time.sleep(0.025)
                for future in futures:
                    future.result()
            assert overlap, "No two simultaneous nonterminal Context Runs observed; concurrency not proven"
            with sqlite3.connect((root / "nola.db").as_uri() + "?mode=ro", uri=True) as db:
                for context, run, _ in overlap:
                    member = contexts.index(context)
                    terminal = db.execute("SELECT status FROM runs WHERE id=? AND conversation_id=?", (run, context)).fetchone()
                    assert terminal == ("sealed",), "Concurrent Run did not reach its own canonical terminal"
                    output = db.execute("SELECT COALESCE(json_extract(p.content_json,'$.text'),p.content_text) "
                        "FROM messages m JOIN message_parts p ON p.message_id=m.id "
                        "WHERE m.run_id=? AND m.conversation_id=? AND m.role='assistant' AND p.kind='text' "
                        "ORDER BY p.ordinal", (run, context)).fetchall()
                    text = "".join(row[0] or "" for row in output)
                    assert markers[member] in text and markers[1 - member] not in text, "Concurrent response lost its context identity"
            (root / "concurrent-evidence.json").write_text(json.dumps({"overlapping_runs": overlap}, indent=2))
            # Every CLI invocation is a new process. Verify the follow-up
            # independently restores each context after the overlapping Runs.
            for member in (0, 1):
                run_turn(f"cold-after-concurrent-{member}", member,
                    "What is my private token? Reply with the exact token only. Do not use tools.")
            stop_service(service)
            service = None
        with sqlite3.connect((root / "nola.db").as_uri() + "?mode=ro", uri=True) as db:
            for index, context in enumerate(contexts):
                rows = db.execute("SELECT m.role,COALESCE(json_extract(p.content_json,'$.text'),p.content_text) FROM messages m JOIN message_parts p "
                    "ON p.message_id=m.id WHERE m.conversation_id=? AND p.kind='text' ORDER BY m.ordinal,p.ordinal",
                    (context,)).fetchall()
                text = json.dumps(rows)
                if index < 2:
                    assert markers[index] in rows[-1][1], "Own token was not recalled"
                    assert markers[1 - index] not in text, "Foreign token leaked into context"
                else:
                    assert "UNKNOWN" in rows[-1][1], "Same Agent in fresh room inherited history"
                    assert all(marker not in text for marker in markers)
        service, sock = start_service(args.service.resolve(), root, environment)
        assert room_command(sock, type="read", room_id=room["room"]["id"]) == room
        assert room_command(sock, type="read", room_id=other["room"]["id"]) == other
        # Use the public CLI room entry, with no --to recipient. This must invoke
        # the actual coordinator, not the direct-context isolation path above.
        dispatch_dir = root / "unaddressed-dispatch"
        dispatch_dir.mkdir()
        with (dispatch_dir / "stdout.json").open("w") as out, (dispatch_dir / "stderr.log").open("w") as err:
            dispatched = subprocess.run([str(args.binary.resolve()), "room", "send", room["room"]["id"],
                "Calculate 19 + 23. Reply only 42. Do not use tools or memory.",
                "--project", str(workspace), "--no-skills", "--max-iterations", "3"],
                env=environment, stdout=out, stderr=err, timeout=240)
        assert dispatched.returncode == 0, f"Room dispatch failed; inspect {dispatch_dir}"
        submissions = room_command(sock, type="submissions", room_id=room["room"]["id"], limit=100, offset=0)
        assert len(submissions) == 1, "Expected exactly one room submission"
        submitted = submissions[0]
        assert submitted["input"]["participant_id"] is None, "Test accidentally selected a recipient"
        assert submitted["selected_participant_id"] == members[0]["id"], "Coordinator did not select arithmetic specialist"
        submitted = room_command(sock, type="reconcile_execution", room_id=room["room"]["id"],
            submission_id=submitted["input"]["submission_id"], at_ms=int(time.time() * 1000))
        assert submitted["state"] == "completed" and submitted["terminal"] is not None, submitted
        intent = room_command(sock, type="execution_intent", room_id=room["room"]["id"],
            submission_id=submitted["input"]["submission_id"])
        assert intent["execution_id"] == submitted["execution_id"] and intent["envelope"]["kind"] == "host"
        auxiliary = command(sock, {"type": "auxiliary_invocation", "command": {"type": "inspect",
            "operation_id": f"room-route:{room['room']['id']}:{submitted['input']['submission_id']}"}})
        assert auxiliary["attempt"]["state"] == "succeeded", auxiliary
        assert auxiliary["attempt"]["output"]["participant_id"] == submitted["selected_participant_id"]
        with sqlite3.connect((root / "nola.db").as_uri() + "?mode=ro", uri=True) as db:
            assert db.execute("SELECT conversation_id,status FROM runs WHERE id=?",
                (submitted["execution_id"],)).fetchone() == (contexts[0], "sealed")
            run_count = db.execute("SELECT COUNT(*) FROM runs").fetchone()[0]
            reply = db.execute("SELECT COALESCE(json_extract(p.content_json,'$.text'),p.content_text) "
                "FROM messages m JOIN message_parts p ON p.message_id=m.id WHERE m.run_id=? "
                "AND m.role='assistant' AND p.kind='text' ORDER BY p.ordinal",
                (submitted["execution_id"],)).fetchall()
            assert "42" in "".join(row[0] or "" for row in reply), "Completed room Run has no expected canonical reply"
        with (dispatch_dir / "recover.stdout").open("w") as out, (dispatch_dir / "recover.stderr").open("w") as err:
            recovered = subprocess.run([str(args.binary.resolve()), "room", "recover", room["room"]["id"]],
                env=environment, stdout=out, stderr=err, timeout=30)
        assert recovered.returncode == 0, "Cold room recovery failed"
        with sqlite3.connect((root / "nola.db").as_uri() + "?mode=ro", uri=True) as db:
            assert db.execute("SELECT COUNT(*) FROM runs").fetchone()[0] == run_count, "Terminal recovery dispatched another Run"
        (dispatch_dir / "evidence.json").write_text(json.dumps({"submission": submitted,
            "execution_id": intent["execution_id"], "coordinator_attempt": auxiliary}, indent=2))
        print("PASS: unaddressed room -> real coordinator -> selected recipient -> frozen intent -> canonical completed seal", flush=True)
        explicit_dir = root / "explicit-recipient"
        explicit_dir.mkdir()
        with (explicit_dir / "stdout.json").open("w") as out, (explicit_dir / "stderr.log").open("w") as err:
            explicit = subprocess.run([str(args.binary.resolve()), "room", "send", room["room"]["id"],
                "Translate bonjour into English. Reply only Hello. Do not use tools.", "--to", members[1]["id"],
                "--project", str(workspace), "--no-skills"], env=environment, stdout=out, stderr=err, timeout=180)
        assert explicit.returncode == 0, "Explicit recipient failed"
        explicit_rows = room_command(sock, type="submissions", room_id=room["room"]["id"], limit=100, offset=0)
        addressed = next(value for value in explicit_rows if value["input"]["submission_id"] != submitted["input"]["submission_id"])
        assert addressed["input"]["participant_id"] == addressed["selected_participant_id"] == members[1]["id"]
        assert addressed["context_conversation_id"] == contexts[1] and addressed["state"] == "completed"
        with sqlite3.connect((root / "nola.db").as_uri() + "?mode=ro", uri=True) as db:
            assert db.execute("SELECT conversation_id,status FROM runs WHERE id=?",
                (addressed["execution_id"],)).fetchone() == (contexts[1], "sealed")
        (explicit_dir / "evidence.json").write_text(json.dumps(addressed, indent=2))
        print("PASS: explicit recipient executes only the selected participant context", flush=True)
        if args.interactive_oracle:
            from test_real_cli_terminal import Oracle, RealTerminal
            pty_root = root / "room-pty"
            pty_root.mkdir()
            oracle = Oracle(args.interactive_oracle.resolve())
            terminal = None
            try:
                terminal = RealTerminal(args.binary.resolve(), workspace,
                    dict(environment, TERM="xterm-256color", COLUMNS="100", LINES="24"), oracle, pty_root,
                    arguments=[str(args.binary.resolve()), "--project", str(workspace), "--no-tools", "--no-skills", "--lang", "en"])
                terminal.wait(lambda: "Type a message" in terminal.screen(), 15, "room composer")
                terminal.action("room_invalid_candidate")
                terminal.write(f"/room {room['room']['id']} @missing hello\r".encode())
                terminal.wait(lambda: "Unknown or ambiguous @recipient" in terminal.screen()
                    and "Type a message" in terminal.screen(), 10, "recoverable room input error")
                terminal.action("room_mention_query")
                terminal.write(f"/room {room['room']['id']} @b".encode())
                terminal.wait(lambda: "tab @b" in terminal.screen(), 10, "named room candidate")
                terminal.action("room_mention_select")
                terminal.write(b"\t")
                terminal.write(b"Reply only ROOM_PTY_OK. Do not use tools.\r")
                terminal.wait(lambda: "ROOM_PTY_OK" in terminal.screen() and "@b" in terminal.screen()
                    and "Completed" in terminal.screen() and "Type a message" in terminal.screen(), 90, "explicit PTY room reply")
                pty_rows = room_command(sock, type="submissions", room_id=room["room"]["id"], limit=100, offset=0)
                selected = next(value for value in pty_rows if value["input"]["text"].startswith("Reply only ROOM_PTY_OK"))
                assert selected["input"]["participant_id"] == members[1]["id"] and selected["state"] == "completed"
                terminal.action("room_automatic_input")
                terminal.write(f"/room {room['room']['id']} Calculate 20 + 22. Reply only ROOM_AUTO_42. Do not use tools.\r".encode())
                terminal.wait(lambda: "ROOM_AUTO_42" in terminal.screen() and "@a" in terminal.screen()
                    and "Completed" in terminal.screen() and "Type a message" in terminal.screen(), 90, "automatic PTY room reply")
                pty_rows = room_command(sock, type="submissions", room_id=room["room"]["id"], limit=100, offset=0)
                automatic = next(value for value in pty_rows if "ROOM_AUTO_42" in value["input"]["text"])
                assert automatic["input"]["participant_id"] is None and automatic["selected_participant_id"] == members[0]["id"]
                assert automatic["state"] == "completed"
                idle = [frame for frame in terminal.frames if "Type a message" in frame["screen"]]
                assert idle and all(frame["screen"].count("Type a message") == 1 and frame["screen"].count("Context") == 1 for frame in idle)
                (pty_root / "evidence.json").write_text(json.dumps({"explicit": selected, "automatic": automatic,
                    "committed_frames": len(terminal.frames), "single_composer_footer": True}, indent=2))
                print("PASS: real PTY @ candidate+Tab, structured recipient, automatic room dispatch and single composer/footer", flush=True)
            finally:
                if terminal is not None:
                    terminal.close()
                oracle.close()
        if args.concurrent:
            print("PASS: simultaneous independent room Runs and cold-process recall after both complete", flush=True)
        print("PASS: Application-created rooms, distinct canonical contexts, real model recall, no cross-context tokens, cold room mapping", flush=True)
    finally:
        if service is not None:
            stop_service(service)
        scrub_case_credentials(root)


if __name__ == "__main__":
    main()
