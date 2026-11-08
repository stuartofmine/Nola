#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Bounded real-LLM evidence-reference survival across manual compaction/restart.

No build, database seeding, synthetic model response, or product routing override.
Uses an isolated NOLA_HOME/database and a read-only copy of the configured 192.*
provider. A failed stage stops the run; no context-filling retry loop is used.
"""
from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import secrets
import select
import signal
import sqlite3
import struct
import subprocess
import termios
import time

# Finite historical notes, not instructions or an artificial context-filling
# loop. Externalized tool bodies are not model-context tokens; without these
# notes this fixture archives only ~378 tokens and cannot reasonably summarize
# its reference/provenance in a smaller budget. Distinct records collapse into
# one short provenance statement while the original tool source must survive.
ARCHIVE_BACKGROUND = """Historical audit notes (background only; no actions requested):
01. The evidence intake was scoped to one isolated workspace. Its owner chose a session-local receipt rather than a shared global scratch file so an unrelated conversation could not claim it.
02. The original content was recorded without Unicode normalization. This matters because the acceptance check compares byte identities, not merely visually similar text in the final answer.
03. An intake operator checked that the source file was ordinary UTF-8 text. The later reader is expected to recover the retained output, not rediscover or regenerate a replacement source.
04. The workflow distinguishes a short presentation preview from the complete retained body. A preview may help a reader navigate, but it is not sufficient proof that all evidence was recovered.
05. The receipt includes a durable source reference. The audit treats that reference as an address within its owning session, not as authority for another session to inspect the same file.
06. The owner approved a read-only verification. No edits to project files, credentials, settings, or other conversations are part of the evidence inspection being requested here.
07. A restart is an intentional boundary in this exercise. Memory retained by the original command process cannot count as persistent recovery once that process has exited.
08. The source body includes independent records and explicit boundary markers. Finding a marker alone does not establish that the intervening bytes were retained without truncation.
09. Pagination has an explicit byte offset and a next-offset receipt. A subsequent page must begin exactly where the preceding page ended, including when a character spans multiple bytes.
10. The completion condition is the final page reporting no more content. A conversational assertion that the reading is complete is not equivalent to the underlying tool receipt.
11. The audit records a whole-body digest before any context maintenance occurs. The same digest must be obtained from recovered pages after the new command process is started.
12. Conversation history can be summarized for efficient model input. That optimization must not erase the canonical source record used to prove where an evidence reference originated.
13. The summary may discard these operational notes as repetitive procedure. It must preserve the existence and ownership of the original evidence reference needed by the later read.
14. A different conversation is a negative control. Even possession of the exact reference string must not let that conversation obtain the original session's retained tool output.
15. Errors are evidence too: failure to read an unauthorized reference should produce a tool error, not an invented body or a successful empty result that obscures the denial.
16. The historical intake does not require a browser, a remote shell, or filesystem discovery. Its sole artifact is the local evidence body that the requested command will expose.
17. All temporary paths belong to this isolated test run. The audit must leave user databases and production credentials untouched regardless of whether a verification stage succeeds.
18. A failed maintenance stage is not a successful compression. The check requires a committed summary covering the original tool source before the restart phase may proceed.
19. The final report should retain enough receipts to independently verify ownership, byte coverage, and the content digest. It should not reproduce the entire body in ordinary assistant prose.
20. These notes describe one completed audit preparation, not twenty tasks. The immediate requested work remains the single read command followed by a brief acknowledgment.
"""
from urllib.parse import urlparse


def objects(value):
    """Inspect structured tool results, including JSON serialized in text."""
    if isinstance(value, dict):
        yield value
        for item in value.values():
            yield from objects(item)
    elif isinstance(value, list):
        for item in value:
            yield from objects(item)
    elif isinstance(value, str) and value.lstrip().startswith(("{", "[")):
        try:
            yield from objects(json.loads(value))
        except (ValueError, RecursionError):
            pass


def events(path):
    result = []
    for line in path.read_text(errors="replace").splitlines():
        try:
            result.append(json.loads(line))
        except ValueError:
            pass
    return result


def canonical_results(database, session, run_id=None):
    """Read-only auditor of real committed Tool sources, never a fixture writer."""
    results = []
    with sqlite3.connect(database.as_uri() + "?mode=ro", uri=True) as db:
        for raw_binding, in db.execute("SELECT binding_json FROM canonical_tool_sources "
                                       "WHERE conversation_id=? AND kind='tool_result' "
                                       "AND (? IS NULL OR run_id=?)", (session, run_id, run_id)):
            binding = json.loads(raw_binding)
            body = binding["body"]
            assert body["conversation_id"] == session == binding["owner"]["owner"]["conversation_id"]
            chunks = db.execute("SELECT offset_bytes,text FROM canonical_text_chunks "
                                "WHERE content_id=? ORDER BY offset_bytes", (body["content_id"],)).fetchall()
            assembled = bytearray()
            for offset, text in chunks:
                assert offset == len(assembled), "Canonical tool body has a gap"
                assembled.extend(text.encode())
            assert len(chunks) == int(body["chunk_count"])
            assert len(assembled) == int(body["total_utf8_bytes"])
            assert hashlib.sha256(assembled).hexdigest() == binding["body_sha256"]
            result = json.loads(assembled)
            assert result["id"] == binding["source_item_id"] and result["call_id"] == binding["call_id"]
            results.append(result)
    return results


def cold_result_slices(run_results, prior_ids, reference):
    fresh = [item for item in run_results if item["id"] not in prior_ids]
    return reconstruct_slices(fresh, reference)


def reconstruct_slices(stream, reference):
    pages = {}
    for event in stream:
        for value in objects(event):
            if value.get("type") != "tool_output_slice" or value.get("path") != reference:
                continue
            if value.get("json_pointer") is not None:
                raise AssertionError("Acceptance requires full evidence, not a JSON selection")
            raw = value["content"].encode()
            offset, end = int(value["offset"]), int(value["next_offset"])
            assert end - offset == len(raw) == int(value["bytes_returned"])
            assert offset not in pages or pages[offset] == value, "Conflicting repeated slice"
            pages[offset] = value
    assert pages, "No real tool_output_read result slices observed"
    cursor, assembled = 0, bytearray()
    while cursor in pages:
        value = pages[cursor]
        assembled.extend(value["content"].encode())
        next_cursor = int(value["next_offset"])
        assert next_cursor > cursor, "Slice pagination did not advance"
        cursor = next_cursor
        if not value["has_more"]:
            assert cursor == int(value["size_bytes"])
            return bytes(assembled), len(pages)
    raise AssertionError(f"Incomplete evidence coverage; missing byte offset {cursor}")


def compaction_count(database, session):
    with sqlite3.connect(database.as_uri() + "?mode=ro", uri=True) as db:
        return db.execute("SELECT count(*) FROM run_events e JOIN runs r ON r.id=e.run_id "
                          "WHERE r.conversation_id=? AND r.kind='maintenance' AND r.status='sealed' "
                          "AND e.event_kind='compaction_completed' "
                          "AND EXISTS(SELECT 1 FROM run_events t WHERE t.run_id=r.id AND t.event_kind='turn_completed')",
                          (session,)).fetchone()[0]


def require_compacted_source(semantic, target_ids):
    covered = set(semantic.get("retained_item_ids", []))
    assert target_ids and target_ids <= covered, "The original tool output was not covered by compaction"
    assert int(semantic.get("original_token_count", 0)) > 0, "Empty compaction input"
    assert int(semantic.get("compacted_token_count", 0)) > 0, "Empty compaction summary"
    assert semantic["compacted_token_count"] < semantic["original_token_count"], "Compaction did not reduce tokens"
    fingerprint = semantic.get("source_fingerprint", "")
    assert len(fingerprint) == 64 and all(char in "0123456789abcdef" for char in fingerprint)
    return {"covered_tool_result_ids": sorted(target_ids), "source_item_count": len(covered),
            "source_fingerprint": fingerprint, "original_token_count": semantic["original_token_count"],
            "compacted_token_count": semantic["compacted_token_count"]}


def require_maintenance_compaction(checkpoint, session, run_id, event, target_ids):
    state = checkpoint["state"]
    assert state["session_id"] == session
    turn = state["execution"]["active_turn"]
    assert turn["turn_id"] == run_id and turn["status"] == "completed"
    assert event["reason"] == "manual" and event["replaced_item_count"] > 0, "Manual compaction was a no-op"
    items = {item["id"]: item for item in state["conversation"]["model_context"]
             if item["type"] == "compaction"}
    current = items[event["compaction_item_id"]]
    assert current["id"].startswith(run_id + ":")
    assert current["source_fingerprint"] == event["source_fingerprint"]
    assert current["original_token_count"] == event["original_token_count"]
    assert len(current["source_item_ids"]) == event["source_item_count"]
    # Same scalar-character estimator as nola_kernel::estimated_item_tokens(Text).
    def semantic(item):
        return {"retained_item_ids": item["source_item_ids"],
                "source_fingerprint": item["source_fingerprint"],
                "original_token_count": item["original_token_count"],
                "compacted_token_count": (len(item["summary"]) + 3) // 4 + 4}
    require_compacted_source(semantic(current), set(current["source_item_ids"]))
    # Earlier automatic summaries may already own the target. They only count
    # while still retained in this successful new maintenance checkpoint;
    # their presence alone never satisfies the new-manual-operation assertion.
    covered = set()
    lineage = []
    for item in items.values():
        matching = target_ids & set(item["source_item_ids"])
        if matching:
            lineage.append({"compaction_item_id": item["id"],
                            **require_compacted_source(semantic(item), matching)})
            covered |= matching
    assert target_ids and covered == target_ids, "Current compressed history lost original tool source IDs"
    return {"maintenance_run_id": run_id, "compaction_item_id": current["id"],
            "source_item_count": event["source_item_count"],
            "original_token_count": current["original_token_count"],
            "compacted_token_count": semantic(current)["compacted_token_count"], "original_source_lineage": lineage}


def compaction_coverage(database, session, target_ids):
    with sqlite3.connect(database.as_uri() + "?mode=ro", uri=True) as db:
        row = db.execute("SELECT r.id,e.payload_json FROM run_events e JOIN runs r ON r.id=e.run_id "
                         "WHERE r.conversation_id=? AND r.kind='maintenance' AND r.status='sealed' "
                         "AND e.event_kind='compaction_completed' ORDER BY r.created_at_ms DESC LIMIT 1",
                         (session,)).fetchone()
        assert row, "No committed maintenance compaction"
        body = db.execute("SELECT checkpoint_json FROM core_checkpoint_bodies WHERE conversation_id=?",
                          (session,)).fetchone()
        assert body, "No persisted Core checkpoint"
        return require_maintenance_compaction(json.loads(body[0]), session, row[0], json.loads(row[1]), target_ids)


def stop(process):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)


def run_prompt(command, prompt, project, environment, output, seconds):
    with output.open("xb") as log:
        process = subprocess.Popen(command, cwd=project, env=environment, stdin=subprocess.PIPE,
                                   stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            process.communicate(prompt.encode(), timeout=seconds)
        finally:
            stop(process)
    assert process.returncode == 0, f"CLI failed; inspect {output.name}"
    return events(output)


def compact_with_product_command(command, project, environment, database, session, output, seconds):
    before = compaction_count(database, session)
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 35, 120, 0, 0))
    process = subprocess.Popen(command, cwd=project, env=environment, stdin=slave,
                               stdout=slave, stderr=slave, start_new_session=True)
    os.close(slave)
    started = time.monotonic()
    sent, completed, exit_sent = False, False, False
    try:
        with output.open("xb") as log:
            while time.monotonic() - started < seconds:
                if select.select([master], [], [], 0.25)[0]:
                    try:
                        chunk = os.read(master, 65536)
                    except OSError:
                        break
                    if not chunk:
                        break
                    log.write(chunk)
                if not sent and time.monotonic() - started >= 2:
                    os.write(master, b"/compact\r")
                    sent = True
                if sent and not completed:
                    completed = compaction_count(database, session) > before
                if completed and not exit_sent:
                    os.write(master, b"/exit\r")
                    exit_sent = True
                if process.poll() is not None:
                    break
        if not completed:
            with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as connection:
                failure = connection.execute(
                    "SELECT e.payload_json FROM run_events e JOIN runs r ON r.id=e.run_id "
                    "WHERE r.conversation_id=? AND r.kind='maintenance' "
                    "AND e.event_kind IN ('turn_failed','compaction_failed') "
                    "ORDER BY r.created_at_ms DESC,e.sequence DESC LIMIT 1", (session,)
                ).fetchone()
            if failure:
                raise AssertionError("Manual /compact failed: " + str(json.loads(failure[0]).get("error")))
            raise AssertionError("Manual /compact did not commit a completed maintenance Run; "
                                 f"process_exit={process.poll()}, command_sent={sent}")
    finally:
        stop(process)
        os.close(master)
    return {"before": before, "after": compaction_count(database, session),
            "trigger": "interactive /compact", "process_pid": process.pid}


def contains_fixture(raw, expected):
    if expected in raw:
        return True
    try:
        value = json.loads(raw)
    except ValueError:
        return False
    def strings(item):
        if isinstance(item, str):
            yield item
        elif isinstance(item, dict):
            for child in item.values():
                yield from strings(child)
        elif isinstance(item, list):
            for child in item:
                yield from strings(child)
    return any(expected.decode() in text for text in strings(value))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--config", type=Path, default=Path.home() / ".nola/config.json")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--stage-seconds", type=int, default=180)
    args = parser.parse_args()
    assert 10 <= args.stage_seconds <= 300, "Bound each stage to 10–300 seconds"
    binary = args.binary.resolve()
    assert binary.is_file() and (binary.parent / "nola-service").is_file(), "Build CLI and sibling service first"
    source = json.loads(args.config.read_text())
    llm = source["llm"]
    host = urlparse(llm["endpoint"]).hostname or ""
    assert host.startswith("192."), "Acceptance requires the configured 192.* local model endpoint"
    assert llm.get("model"), "Configure the real local model first"
    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    os.chmod(root, 0o700)
    project = root / "project"
    project.mkdir()
    (project / ".git").mkdir()
    task_home = root / "nola-home"
    (task_home / ".nola").mkdir(parents=True)
    config = task_home / ".nola/config.json"
    settings = {"llm": llm, "allowed_dirs": [str(project)], "max_iterations": 16,
                "network_policy": "deny", "compaction": {"context_window_tokens": 64000}}
    if "model_capabilities" in source:
        settings["model_capabilities"] = source["model_capabilities"]
    with open(config, "x", opener=lambda path, flags: os.open(path, flags, 0o600)) as handle:
        json.dump(settings, handle)
    database = root / "state.db"
    session = "reference-" + secrets.token_hex(8)
    token = secrets.token_hex(16)
    fixture = ("BEGIN-" + token + "\n" + "\n".join(secrets.token_hex(32) for _ in range(128))
               + "\nEND-" + token + "\n").encode()
    (project / "evidence.txt").write_bytes(fixture)
    environment = {**os.environ, "NOLA_HOME": str(task_home), "NOLA_DB_PATH": str(database),
                   "NOLA_SKIP_ONBOARDING": "1", "TERM": "xterm-256color"}
    command = [str(binary), "--project", str(project), "--session", session, "--yes"]
    json_command = [*command, "--json", "--max-iterations", "16"]
    report = {"passed": False, "session": session, "model": llm["model"], "endpoint_host": host,
              "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "fixture_sha256": hashlib.sha256(fixture).hexdigest(), "stages": {}}
    started = time.monotonic()
    try:
        first = run_prompt(json_command, ARCHIVE_BACKGROUND + "\nRun bash exactly once to print the entire local evidence.txt using cat. "
                           "Do not modify any file. Preserve the returned full_output_ref for later reading; "
                           "then reply ACK without copying the long output.", project, environment,
                           root / "01-produce.ndjson", args.stage_seconds)
        original_results = canonical_results(database, session)
        references = {obj["full_output_ref"] for event in original_results for obj in objects(event)
                      if isinstance(obj.get("full_output_ref"), str)}
        namespace = Path(str(database) + ".outputs") / hashlib.sha256(session.encode()).hexdigest()
        candidates = [(Path(ref), Path(ref).read_bytes()) for ref in references
                      if Path(ref).is_file() and Path(ref).resolve().parent == namespace.resolve()]
        selected = [(path, raw) for path, raw in candidates if contains_fixture(raw, fixture)]
        assert len(selected) == 1, "Expected one complete, session-owned persisted output reference"
        reference, saved = selected[0]
        assert len(saved) > 4000
        report["reference"] = str(reference)
        report["reference_sha256"] = hashlib.sha256(saved).hexdigest()
        report["reference_bytes"] = len(saved)
        compacted_targets = {item["id"] for item in original_results
                             if any(obj.get("full_output_ref") == str(reference) for obj in objects(item))}
        original_bindings = {item["id"]: item for item in original_results}
        run_prompt(json_command, "Retain the evidence reference from the previous turn. Reply only ACK.",
                   project, environment, root / "02-close-history.ndjson", args.stage_seconds)
        report["stages"]["compaction"] = compact_with_product_command(
            command, project, environment, database, session, root / "03-compact.terminal",
            args.stage_seconds)
        report["stages"]["compaction"].update(compaction_coverage(database, session, compacted_targets))
        # Remove only this generated source. Reopening it cannot satisfy recall.
        (project / "evidence.txt").unlink()
        before_cold = {item["id"]: item for item in canonical_results(database, session)}
        cold = run_prompt(json_command, "Read the complete saved tool evidence using tool_output_read at exact path "
                          + json.dumps(str(reference)) + ". Start at offset 0, max_bytes 2500; follow each returned "
                          "next_offset until has_more=false. Do not use bash/read_file or a JSON selector. "
                          "Do not rewrite files. After all pages reply VERIFIED.", project, environment,
                          root / "04-cold-read.ndjson", args.stage_seconds)
        after_results = canonical_results(database, session)
        for item in after_results:
            if item["id"] in before_cold:
                assert item == before_cold[item["id"]], "Cold read changed an existing tool source"
            if item["id"] in original_bindings:
                assert item == original_bindings[item["id"]], "Compaction changed original tool source"
        assert before_cold.keys() <= {item["id"] for item in after_results}, "Prior tool source lost"
        assert original_bindings.keys() <= before_cold.keys(), "Compaction lost original tool source"
        cold_runs = {event["run_id"] for event in cold
                     if event.get("trace", {}).get("kind") == "run.started"}
        assert len(cold_runs) == 1, "Cold read must have one exact admitted Run"
        cold_run = next(iter(cold_runs))
        restored, page_count = cold_result_slices(canonical_results(database, session, cold_run),
                                                 before_cold.keys(), str(reference))
        assert restored == saved, "Cold tool range reads differ from the original full output"
        assert hashlib.sha256(reference.read_bytes()).hexdigest() == report["reference_sha256"]
        report["stages"]["cold_read"] = {"run_id": cold_run, "complete_bytes": len(restored), "pages": page_count,
                                         "sha256": hashlib.sha256(restored).hexdigest()}
        stranger = [str(binary), "--project", str(project), "--session", session + "-other",
                    "--yes", "--json", "--max-iterations", "4"]
        denied = run_prompt(stranger, "Call tool_output_read exactly once for path " + json.dumps(str(reference))
                            + ". Do not use another tool or try another path. Report its result.",
                            project, environment, root / "05-owner-denial.ndjson", args.stage_seconds)
        denied_results = canonical_results(database, session + "-other")
        assert any(item["name"] == "tool_output_read" for item in denied_results), "Model did not exercise owner boundary"
        serialized = json.dumps(denied_results)
        assert "tool_output_outside_store" in serialized, "Cross-session read was not explicitly denied"
        assert not any(obj.get("type") == "tool_output_slice" and obj.get("path") == str(reference)
                       for event in denied_results for obj in objects(event)), "Foreign session received evidence"
        report["stages"]["owner_denial"] = True
        report["passed"] = True
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
    finally:
        config.unlink(missing_ok=True)
        report["elapsed_seconds"] = round(time.monotonic() - started, 3)
        (root / "summary.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(report, ensure_ascii=False))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
