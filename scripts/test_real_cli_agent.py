#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Real-provider acceptance for CLI Agent tools, policy, and compaction."""

from __future__ import annotations

# Executable acceptance scenario. The system E2E runner supplies binary and
# project arguments; pytest must not reinterpret them as fixture names.
__test__ = False

import collections
import fcntl
import json
import os
import pathlib
import pty
import re
import select
import signal
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
from typing import Any


ANSI_PATTERN = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def canonical_checkpoint(db_path: pathlib.Path, conversation_id: str) -> dict[str, Any]:
    with sqlite3.connect(db_path) as connection:
        row = connection.execute(
            "SELECT checkpoint_json FROM core_checkpoint_bodies WHERE conversation_id=?",
            (conversation_id,),
        ).fetchone()
    require(row is not None, f"checkpoint is missing for {conversation_id}")
    return json.loads(str(row[0]))


def event_name(event: dict[str, Any]) -> str:
    return str(event.get("name") or event.get("tool") or "")


def nested_integer(value: Any, key: str) -> int:
    if isinstance(value, dict):
        direct = value.get(key)
        if isinstance(direct, int):
            return direct
        return max((nested_integer(item, key) for item in value.values()), default=0)
    if isinstance(value, list):
        return max((nested_integer(item, key) for item in value), default=0)
    return 0


def flattened_text(value: Any) -> str:
    parts: list[str] = []

    def visit(item: Any) -> None:
        if isinstance(item, dict):
            for nested in item.values():
                visit(nested)
            return
        if isinstance(item, list):
            for nested in item:
                visit(nested)
            return
        if not isinstance(item, str):
            return
        parts.append(item)
        try:
            decoded = json.loads(item)
        except json.JSONDecodeError:
            return
        if decoded != item:
            visit(decoded)

    visit(value)
    return "\n".join(parts)


class JsonRun:
    def __init__(self, completed: subprocess.CompletedProcess[str]) -> None:
        self.completed = completed
        self.events: list[dict[str, Any]] = []
        for line in completed.stdout.splitlines():
            try:
                value = json.loads(line)
            except json.JSONDecodeError:
                continue
            if isinstance(value, dict):
                self.events.append(value)

    @property
    def assistant(self) -> str:
        text_parts: dict[str, str] = {}
        for event in self.events:
            if event.get("type") != "core_part_updated":
                continue
            part = event.get("part") or {}
            if part.get("kind") == "text":
                text_parts[str(event.get("part_id") or part.get("id") or "")] = str(
                    part.get("text") or ""
                )
        return "".join(text for text in text_parts.values() if text)

    @property
    def tool_results(self) -> list[dict[str, Any]]:
        canonical: list[dict[str, Any]] = []
        for event in self.events:
            if event.get("type") != "core_part_updated":
                continue
            part = event.get("part") or {}
            if part.get("kind") != "tool" or part.get("state") not in {
                "completed",
                "failed",
                "denied",
                "cancelled",
            }:
                continue
            result = dict(part)
            result["name"] = str(part.get("capability_id") or part.get("title") or "")
            result["tool"] = result["name"]
            result["error"] = part.get("state") != "completed"
            structured = part.get("structured_output")
            result["content"] = structured or part.get("output_preview")
            result["text"] = (
                structured.get("text")
                if isinstance(structured, dict) and isinstance(structured.get("text"), str)
                else part.get("output_preview") or ""
            )
            canonical.append(result)
        return canonical

    @property
    def tool_names(self) -> list[str]:
        return [event_name(event) for event in self.tool_results]

    def result_for(self, name: str) -> dict[str, Any] | None:
        return next(
            (
                event
                for event in reversed(self.tool_results)
                if event_name(event) == name
            ),
            None,
        )

    def assert_protocol(self, label: str) -> None:
        counts = collections.Counter(event.get("type") for event in self.events)
        require(
            self.completed.returncode == 0,
            f"{label}: CLI exited {self.completed.returncode}: "
            f"stderr={self.completed.stderr[-1000:]} "
            f"stdout={self.completed.stdout[-3000:]}",
        )
        canonical_events = [
            event for event in self.events if event.get("type") == "core_part_updated"
        ]
        require(canonical_events, f"{label}: missing canonical Core projection: {counts}")
        require(
            all(event.get("protocol_version") == 1 for event in canonical_events),
            f"{label}: invalid Core event protocol version: {counts}",
        )
        cursors = [int(event.get("cursor") or 0) for event in canonical_events]
        require(
            cursors == sorted(cursors) and len(cursors) == len(set(cursors)),
            f"{label}: unstable Core projection cursors: {cursors}",
        )
        require(
            any(
                (event.get("part") or {}).get("is_complete") is True
                or (event.get("part") or {}).get("state")
                in {"completed", "failed", "denied", "cancelled"}
                for event in canonical_events
            ),
            f"{label}: missing canonical terminal part: {counts}",
        )
        leaked = {"llm_delta", "provider_delta", "raw_provider_event"}.intersection(counts)
        require(not leaked, f"{label}: raw provider events leaked into JSON output: {leaked}")


def run_json(
    binary: pathlib.Path,
    project: pathlib.Path,
    environment: dict[str, str],
    prompt: str,
    *,
    extra: list[str] | None = None,
    session_id: str | None = None,
    timeout: float = 120,
) -> JsonRun:
    command = [
        str(binary),
        "--project",
        str(project),
        "--json",
        "--lang",
        "en",
    ]
    if session_id:
        command.extend(["--session", session_id])
    else:
        command.append("--no-save")
    command.extend(extra or [])
    command.append(prompt)
    try:
        completed = subprocess.run(
            command,
            env=environment,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=timeout,
            check=False,
        )
    except subprocess.TimeoutExpired as error:
        stdout = error.stdout or ""
        stderr = error.stderr or ""
        if isinstance(stdout, bytes):
            stdout = stdout.decode("utf-8", errors="replace")
        if isinstance(stderr, bytes):
            stderr = stderr.decode("utf-8", errors="replace")
        raise RuntimeError(
            f"CLI timed out after {timeout}s: {' '.join(command)}\n"
            f"stdout tail:\n{stdout[-6000:]}\n"
            f"stderr tail:\n{stderr[-3000:]}"
        ) from error
    return JsonRun(completed)


class InteractiveTerminal:
    def __init__(
        self,
        binary: pathlib.Path,
        project: pathlib.Path,
        environment: dict[str, str],
        session_id: str,
    ) -> None:
        self.pid, self.master = pty.fork()
        if self.pid == 0:
            os.chdir(project)
            os.execvpe(
                str(binary),
                [
                    str(binary),
                    "--project",
                    str(project),
                    "--session",
                    session_id,
                    "--lang",
                    "en",
                    "--debug",
                ],
                environment,
            )
        flags = fcntl.fcntl(self.master, fcntl.F_GETFL)
        fcntl.fcntl(self.master, fcntl.F_SETFL, flags | os.O_NONBLOCK)
        self.output = bytearray()

    def pump(self, timeout: float = 0.1) -> None:
        readable, _, _ = select.select([self.master], [], [], timeout)
        if not readable:
            return
        try:
            chunk = os.read(self.master, 65_536)
        except (BlockingIOError, OSError):
            return
        if chunk:
            self.output.extend(chunk)

    def visible(self) -> str:
        return ANSI_PATTERN.sub(
            "",
            self.output.decode("utf-8", errors="replace"),
        ).replace("\r", "")

    def wait_for(self, predicate, timeout: float, label: str) -> str:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.pump()
            visible = self.visible()
            if predicate(visible):
                return visible
            try:
                exited, _ = os.waitpid(self.pid, os.WNOHANG)
            except ChildProcessError:
                exited = self.pid
            if exited:
                break
        raise RuntimeError(f"{label} timed out; terminal tail:\n{self.visible()[-3000:]}")

    def write_line(self, value: str) -> None:
        os.write(self.master, value.encode("utf-8") + b"\r")

    def close(self) -> None:
        try:
            self.write_line("/exit")
        except OSError:
            pass
        for sig, timeout in ((signal.SIGTERM, 1.0), (signal.SIGKILL, 1.0)):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                try:
                    exited, _ = os.waitpid(self.pid, os.WNOHANG)
                except ChildProcessError:
                    exited = self.pid
                if exited:
                    break
                time.sleep(0.05)
            else:
                try:
                    os.killpg(self.pid, sig)
                except OSError:
                    break
                continue
            break
        try:
            os.close(self.master)
        except OSError:
            pass


def report(label: str, **values: Any) -> None:
    print(json.dumps({"case": label, **values}, ensure_ascii=False), flush=True)


def test_tools_and_permissions(
    binary: pathlib.Path,
    project: pathlib.Path,
    environment: dict[str, str],
) -> None:
    evidence = project / "evidence.txt"
    evidence.write_text("REAL_TOOL_MARKER_7319\n", encoding="utf-8")
    read_run = run_json(
        binary,
        project,
        environment,
        "Read evidence.txt with a file tool and return its exact content.",
    )
    read_run.assert_protocol("read")
    read_result = read_run.result_for("read_file")
    require(read_result is not None, f"read: read_file was not used: {read_run.tool_names}")
    require(read_result.get("error") is False, f"read: tool failed: {read_result}")
    require("REAL_TOOL_MARKER_7319" in read_run.assistant, "read: marker missing")
    report("read", tools=read_run.tool_names, assistant=read_run.assistant.strip())

    target = project / "result.txt"
    denied_run = run_json(
        binary,
        project,
        environment,
        "Create result.txt containing exactly PERMISSION_MARKER_8842 using write_file. "
        "Report whether the tool actually succeeded.",
    )
    denied_run.assert_protocol("write-denied")
    denied_result = denied_run.result_for("write_file")
    require(denied_result is not None, "write-denied: write_file was not attempted")
    require(denied_result.get("error") is True, "write-denied: tool was not denied")
    require(not target.exists(), "write-denied: file changed without approval")
    denial_answer = re.sub(r"[*_`]", "", denied_run.assistant.lower())
    require(
        "did not succeed" in denial_answer
        or "permission" in denial_answer
        or "was denied" in denial_answer
        or "not created" in denial_answer
        or "no file was written" in denial_answer,
        f"write-denied: model misreported denial: {denied_run.assistant}",
    )
    report("write-denied", fileExists=target.exists())

    approved_run = run_json(
        binary,
        project,
        environment,
        "Create result.txt containing exactly PERMISSION_MARKER_8842 using write_file. "
        "Report whether the tool actually succeeded.",
        extra=["--yes"],
    )
    approved_run.assert_protocol("write-approved")
    approved_result = approved_run.result_for("write_file")
    require(approved_result is not None, "write-approved: write_file was not used")
    require(approved_result.get("error") is False, "write-approved: tool failed")
    written_text = target.read_text(encoding="utf-8")
    require(
        written_text.strip() == "PERMISSION_MARKER_8842",
        f"write mismatch: {written_text!r}",
    )
    report("write-approved", fileText=written_text)


def test_session_permission(
    binary: pathlib.Path,
    project: pathlib.Path,
    environment: dict[str, str],
) -> None:
    session_project = project / "session-permission"
    session_project.mkdir()
    session_environment = environment.copy()
    session_environment["NOLA_DB_PATH"] = str(session_project / "nola.db")
    session_environment["NOLA_LLM_MAX_RETRIES"] = "0"
    terminal = InteractiveTerminal(
        binary,
        session_project,
        session_environment,
        "real_permission_7319",
    )
    try:
        terminal.wait_for(
            lambda text: "Type a message" in text,
            15,
            "session permission startup",
        )
        first_start = len(terminal.visible())
        terminal.write_line(
            "Use write_file to create first-session.txt containing exactly "
            "FIRST_SESSION_7319. Do not use bash."
        )
        terminal.wait_for(
            lambda text: "Allow for this session" in text[first_start:],
            120,
            "first session permission",
        )
        os.write(terminal.master, b"2")
        terminal.wait_for(
            lambda text: "Type a message" in text[first_start:],
            120,
            "first session write",
        )

        second_start = len(terminal.visible())
        terminal.write_line(
            "Use write_file to create second-session.txt containing exactly "
            "SECOND_SESSION_7319. Do not use bash."
        )
        terminal.wait_for(
            lambda text: "Type a message" in text[second_start:],
            120,
            "second session write",
        )
        permission_output = terminal.visible()[first_start:]
        prompt_count = permission_output.count("Allow for this session")
        require(
            prompt_count == 1,
            f"session permission: expected one prompt, observed {prompt_count}",
        )
        require(
            (session_project / "first-session.txt").read_text(encoding="utf-8")
            == "FIRST_SESSION_7319",
            "session permission: first write mismatch",
        )
        require(
            (session_project / "second-session.txt").read_text(encoding="utf-8")
            == "SECOND_SESSION_7319",
            "session permission: second write mismatch",
        )
        report(
            "session-permission",
            permissionPrompts=prompt_count,
            firstWritten=True,
            secondWritten=True,
        )
    finally:
        terminal.close()


def test_tool_failure_recovery(
    binary: pathlib.Path,
    project: pathlib.Path,
    environment: dict[str, str],
) -> None:
    recovery_project = project / "tool-failure-recovery"
    recovery_project.mkdir()
    marker = "RECOVERY_MARKER_7319"
    (recovery_project / "fallback.txt").write_text(marker + "\n", encoding="utf-8")
    recovery_environment = environment.copy()
    recovery_environment["NOLA_DB_PATH"] = str(recovery_project / "nola.db")
    run = run_json(
        binary,
        recovery_project,
        recovery_environment,
        "Call read_file on missing-first.txt first. After that tool fails, recover by "
        "calling read_file on fallback.txt. Return only the exact fallback file content.",
        timeout=120,
    )
    run.assert_protocol("tool-failure-recovery")
    reads = [
        result for result in run.tool_results if event_name(result) == "read_file"
    ]
    require(len(reads) >= 2, f"tool recovery did not retry read_file: {run.tool_names}")
    require(reads[0].get("error") is True, f"first read unexpectedly succeeded: {reads[0]}")
    require(
        any(result.get("error") is False for result in reads[1:]),
        f"tool recovery never succeeded: {reads}",
    )
    require(marker in run.assistant, f"tool recovery lost result: {run.assistant}")
    report(
        "tool-failure-recovery",
        failedThenRecovered=True,
        orderedToolResults=True,
        assistant=run.assistant.strip(),
    )


def test_escape_cancellation_and_follow_up(
    binary: pathlib.Path,
    project: pathlib.Path,
    environment: dict[str, str],
) -> None:
    cancel_project = project / "escape-cancellation"
    cancel_project.mkdir()
    cancel_environment = environment.copy()
    cancel_environment["NOLA_DB_PATH"] = str(cancel_project / "nola.db")
    cancel_environment["NOLA_LLM_MAX_RETRIES"] = "0"
    terminal = InteractiveTerminal(
        binary,
        cancel_project,
        cancel_environment,
        "real_escape_7319",
    )
    try:
        terminal.wait_for(lambda text: "Type a message" in text, 15, "escape startup")
        cancel_start = len(terminal.visible())
        terminal.write_line(
            "Write exactly 500 numbered lines, each with a different short sentence. "
            "Do not use tools."
        )
        terminal.wait_for(
            lambda text: (
                "thinking" in text[cancel_start:].lower()
                or re.search(r"(?m)^\s*1\.", text[cancel_start:]) is not None
                or re.search(
                    r"[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏]", text[cancel_start:]
                ) is not None
            ),
            30,
            "escape running state",
        )
        os.write(terminal.master, b"\x1b")
        cancelled = terminal.wait_for(
            lambda text: "Type a message" in text[cancel_start:],
            30,
            "escape cancellation",
        )[cancel_start:]
        numbered_lines = sum(
            1
            for line in cancelled.splitlines()
            if re.match(r"^\s*\d+\.", line)
        )
        require(
            numbered_lines < 100,
            f"escape cancellation arrived too late ({numbered_lines} answer lines)",
        )

        follow_start = len(terminal.visible())
        terminal.write_line("Return only AFTER_CANCEL_7319. Do not use tools.")
        follow_up = terminal.wait_for(
            lambda text: (
                "AFTER_CANCEL_7319" in text[follow_start:]
                and "Type a message" in text[follow_start:]
            ),
            90,
            "post-cancellation follow-up",
        )[follow_start:]
        require(
            follow_up.count("AFTER_CANCEL_7319") >= 1,
            "post-cancellation answer was missing",
        )
        report(
            "escape-cancellation",
            cancelledBeforeAnswer=True,
            numberedLinesBeforeStop=numbered_lines,
            followUpSucceeded=True,
        )
    finally:
        terminal.close()


def test_cross_process_tool_history(
    binary: pathlib.Path,
    project: pathlib.Path,
    environment: dict[str, str],
) -> None:
    history_project = project / "cross-process-history"
    history_project.mkdir()
    marker = "PERSISTED_TOOL_RESULT_7319"
    (history_project / "durable-fact.txt").write_text(marker + "\n", encoding="utf-8")
    history_environment = environment.copy()
    history_environment["NOLA_DB_PATH"] = str(history_project / "nola.db")
    session_id = "real_tool_history_7319"
    first = run_json(
        binary,
        history_project,
        history_environment,
        "Use read_file to read durable-fact.txt. For this first turn only, reply "
        "exactly READ_COMPLETE instead of repeating the file content.",
        session_id=session_id,
        timeout=120,
    )
    first.assert_protocol("cross-process-history-seed")
    seed_result = first.result_for("read_file")
    require(seed_result is not None, "history seed did not read the file")
    require(seed_result.get("error") is False, f"history seed read failed: {seed_result}")
    require(marker in flattened_text(seed_result), "history seed tool result lost the marker")
    checkpoint = canonical_checkpoint(
        pathlib.Path(history_environment["NOLA_DB_PATH"]), session_id
    )
    require(
        marker in json.dumps(checkpoint, ensure_ascii=False),
        "history seed checkpoint was not committed to canonical SQLite",
    )
    forbidden_state_files = [
        path
        for path in history_project.rglob("*.json")
        if path.name.endswith(".checkpoint.json")
        or path.name.endswith(".context.json")
        or path.name.endswith(".config.json")
    ]
    require(
        not forbidden_state_files,
        f"legacy per-session state files were created: {forbidden_state_files}",
    )

    recalled = run_json(
        binary,
        history_project,
        history_environment,
        "The first-turn display restriction has ended. Without calling any tool, return "
        "only the exact content previously read from durable-fact.txt.",
        extra=["--no-tools"],
        session_id=session_id,
        timeout=120,
    )
    recalled.assert_protocol("cross-process-history-recall")
    require(not recalled.tool_results, "history recall unexpectedly called a tool")
    require(marker in recalled.assistant, f"persisted tool result was lost: {recalled.assistant}")
    report(
        "cross-process-tool-history",
        restoredAcrossProcesses=True,
        toolResultPreserved=True,
        sqliteCheckpointOnly=True,
        recalledWithoutTool=True,
    )


def test_network_and_large_file(
    binary: pathlib.Path,
    project: pathlib.Path,
    environment: dict[str, str],
) -> None:
    query = "2026年7月伊朗局势"
    search_run = run_json(
        binary,
        project,
        environment,
        f"Call web_search with this exact opaque query: {query}. "
        "Return the number of ranked results and one source URL.",
        extra=["--network-policy", "allow"],
        timeout=150,
    )
    search_run.assert_protocol("web-search")
    search_result = search_run.result_for("web_search")
    require(search_result is not None, "web-search: web_search was not used")
    require(search_result.get("error") is False, f"web-search failed: {search_result}")
    search_text = str(search_result.get("text") or "")
    require(f"Query: {query}" in search_text, "web-search rewrote the original query")
    require("http://" in search_text or "https://" in search_text, "web-search returned no URL")
    report(
        "web-search",
        queryPreserved=True,
        hasUrl=True,
        assistant=search_run.assistant.strip()[:500],
    )

    huge = project / "large.log"
    marker_line = 12_345
    with huge.open("w", encoding="utf-8") as handle:
        for line_number in range(1, 20_001):
            value = (
                "LARGE_FILE_TARGET_5517"
                if line_number == marker_line
                else f"ordinary-line-{line_number}"
            )
            handle.write(value + "\n")
    large_run = run_json(
        binary,
        project,
        environment,
        "Find LARGE_FILE_TARGET_5517 in large.log efficiently without reading the whole "
        "file. Return only its 1-based line number.",
    )
    large_run.assert_protocol("large-file")
    require(
        any(name in {"grep", "code_search"} for name in large_run.tool_names),
        f"large-file: no targeted search tool used: {large_run.tool_names}",
    )
    search_evidence = "\n".join(
        flattened_text(result)
        for result in large_run.tool_results
        if event_name(result) in {"grep", "code_search"}
    )
    require(
        f":{marker_line}:" in search_evidence,
        f"large-file: tool evidence has the wrong line number: {search_evidence[-1000:]}",
    )
    normalized_answer = re.sub(r"[,，_\\s]", "", large_run.assistant)
    require(str(marker_line) in normalized_answer, "large-file: wrong line number")
    report("large-file", tools=large_run.tool_names, line=marker_line)


def test_compaction(
    binary: pathlib.Path,
    project: pathlib.Path,
    environment: dict[str, str],
) -> None:
    session_id = "real_compact_7319"
    marker = "HISTORICAL_MARKER_0_7319"
    value = "42017"
    facts = [
        f"The recovery code name is {marker} and its numeric value is {value}. "
        "Remember both exactly. Reply only ACK-0.",
        "The preferred deployment region is north-harbor-7. Remember it. Reply only ACK-1.",
        "The release train is cobalt-sparrow. Remember it. Reply only ACK-2.",
        "The unresolved owner is team-lantern. Remember it. Reply only ACK-3.",
    ]
    for index, prompt in enumerate(facts):
        seeded = run_json(
            binary,
            project,
            environment,
            prompt,
            extra=["--no-tools"],
            session_id=session_id,
        )
        seeded.assert_protocol(f"compact-seed-{index}")

    terminal = InteractiveTerminal(binary, project, environment, session_id)
    try:
        terminal.wait_for(
            lambda text: "Type a message" in text,
            15,
            "compact terminal startup",
        )
        compact_start = len(terminal.visible())
        terminal.write_line("/compact")
        compact_output = terminal.wait_for(
            lambda text: (
                "context compaction" in text[compact_start:].lower()
                and (
                    "summarized" in text[compact_start:].lower()
                    or "already compact" in text[compact_start:].lower()
                )
            ),
            150,
            "manual compaction",
        )[compact_start:]
        require("failed" not in compact_output.lower(), f"compact failed: {compact_output}")

        checkpoint_value = canonical_checkpoint(pathlib.Path(environment["NOLA_DB_PATH"]), session_id)
        checkpoint_text = json.dumps(checkpoint_value, ensure_ascii=False)
        require(marker in checkpoint_text, "compact: exact marker was lost")
        require(value in checkpoint_text, "compact: exact numeric value was lost")

        recall_start = len(terminal.visible())
        expected = f"{marker}={value}"
        terminal.write_line(
            f"Return only {expected}. The marker string and number are the complete fact."
        )
        recall_output = terminal.wait_for(
            lambda text: (
                "Type a message" in text[recall_start:]
                and expected
                in text[recall_start:].rsplit("thinking", 1)[-1]
            ),
            120,
            "post-compaction recall",
        )[recall_start:]
        require(
            expected in recall_output.rsplit("thinking", 1)[-1],
            f"compact: recall failed: {recall_output[-1500:]}",
        )
        report(
            "compact",
            checkpointPreserved=True,
            recalled=True,
            compactTail=compact_output[-500:],
        )
    finally:
        terminal.close()


def test_automatic_compaction(
    binary: pathlib.Path,
    project: pathlib.Path,
    environment: dict[str, str],
) -> None:
    auto_project = project / "auto-compaction"
    config_directory = auto_project / ".nola"
    config_directory.mkdir(parents=True)
    (config_directory / "config.json").write_text(
        json.dumps(
            {
                "compaction": {
                    "token_threshold": 512,
                    "context_window_tokens": 4096,
                    "keep_recent_messages": 2,
                }
            }
        ),
        encoding="utf-8",
    )
    auto_environment = environment.copy()
    auto_environment["NOLA_DB_PATH"] = str(auto_project / "nola.db")
    auto_environment["NOLA_LLM_MAX_RETRIES"] = "0"
    session_id = "real_auto_compact_7319"
    fact = "AUTO_COMPACT_FACT_7319=88241"
    prompts = [
        f"Remember this exact durable fact: {fact}. Reply only ACK-0.",
        "Remember that the secondary region is harbor-nine. "
        + ("alpha beta gamma delta " * 450)
        + "Reply only ACK-1.",
    ]
    for index, prompt in enumerate(prompts):
        seeded = run_json(
            binary,
            auto_project,
            auto_environment,
            prompt,
            extra=["--no-tools"],
            session_id=session_id,
            timeout=180,
        )
        seeded.assert_protocol(f"auto-compact-seed-{index}")

    checkpoint_value = canonical_checkpoint(
        pathlib.Path(auto_environment["NOLA_DB_PATH"]), session_id
    )
    generation = nested_integer(checkpoint_value, "compaction_generation")
    require(generation > 0, "auto-compact: Core never advanced compaction_generation")
    require(
        fact in json.dumps(checkpoint_value, ensure_ascii=False),
        "auto-compact: exact fact was lost from the checkpoint",
    )

    recall = run_json(
        binary,
        auto_project,
        auto_environment,
        f"Return only {fact}.",
        extra=["--no-tools"],
        session_id=session_id,
        timeout=180,
    )
    recall.assert_protocol("auto-compact-recall")
    require(fact in recall.assistant, f"auto-compact: recall failed: {recall.assistant}")
    report(
        "auto-compact",
        compactionGeneration=generation,
        checkpointPreserved=True,
        recalled=True,
    )


def test_background_task_lifecycle(
    binary: pathlib.Path,
    project: pathlib.Path,
    environment: dict[str, str],
) -> None:
    background_project = project / "background-lifecycle"
    background_project.mkdir()
    background_environment = environment.copy()
    background_environment["NOLA_DB_PATH"] = str(background_project / "nola.db")
    background_environment["NOLA_LLM_MAX_RETRIES"] = "0"
    session_id = "real_background_7319"
    marker = "BACKGROUND_STARTED_7319"
    start = run_json(
        binary,
        background_project,
        background_environment,
        "Call bash exactly once with this exact command: "
        f"printf '{marker}\\n'; sleep 90; printf 'BACKGROUND_DONE_7319\\n'. "
        "The runtime must background it. Do not inspect, wait for, or stop it in this turn. "
        "Return the task ID.",
        extra=["--yes"],
        session_id=session_id,
        timeout=150,
    )
    start.assert_protocol("background-start")
    start_result = start.result_for("bash")
    require(start_result is not None, f"background-start: bash not used: {start.tool_names}")
    require(start_result.get("error") is False, f"background-start failed: {start_result}")
    start_text = flattened_text(start_result)
    task_match = re.search(r"\b(bash-\d+-\d+)\b", start_text)
    require(task_match is not None, f"background-start: task id missing: {start_text}")
    task_id = task_match.group(1)

    status = run_json(
        binary,
        background_project,
        background_environment,
        f"Call read_task exactly once with task_id {task_id}. "
        "Return its current status and captured stdout.",
        session_id=session_id,
        timeout=120,
    )
    status.assert_protocol("background-status")
    status_result = status.result_for("read_task")
    require(status_result is not None, f"background-status tool missing: {status.tool_names}")
    require(status_result.get("error") is False, f"background-status failed: {status_result}")
    status_text = flattened_text(status_result)
    require(marker in status_text, f"background-status stdout missing: {status_text}")

    stop = run_json(
        binary,
        background_project,
        background_environment,
        f"Call stop_task exactly once with task_id {task_id}. "
        "Confirm that the task was stopped.",
        extra=["--yes"],
        session_id=session_id,
        timeout=120,
    )
    stop.assert_protocol("background-stop")
    stop_result = stop.result_for("stop_task")
    require(stop_result is not None, f"background-stop tool missing: {stop.tool_names}")
    require(stop_result.get("error") is False, f"background-stop failed: {stop_result}")
    stop_text = flattened_text(stop_result)
    require(
        task_id in stop_text and marker in stop_text,
        f"background-stop returned incomplete evidence: {stop_text}",
    )
    report(
        "background-lifecycle",
        taskId=task_id,
        statusReadWithoutApproval=True,
        stdoutRecovered=True,
        stopped=True,
    )


def test_goal_resume(
    binary: pathlib.Path,
    project: pathlib.Path,
    environment: dict[str, str],
) -> None:
    goal_project = project / "goal-resume"
    goal_project.mkdir()
    goal_environment = environment.copy()
    goal_environment["NOLA_DB_PATH"] = str(goal_project / "nola.db")
    goal_environment["NOLA_LLM_MAX_RETRIES"] = "0"
    session_id = "real_goal_resume_7319"
    marker = "GOAL_RESUME_MARKER_7319"
    target = (
        f"{marker}: immediately call goal_complete with status needs_user, "
        "summary waiting, blocker user-confirmation, and next_action ask-user. "
        "Do not call any other tool."
    )

    first = InteractiveTerminal(
        binary,
        goal_project,
        goal_environment,
        session_id,
    )
    try:
        first.wait_for(lambda text: "Type a message" in text, 15, "goal startup")
        start = len(first.visible())
        first.write_line(f"/goal {target}")
        first_output = first.wait_for(
            lambda text: "goal paused" in text[start:].lower(),
            150,
            "goal initial pause",
        )[start:]
        require(marker in first_output, "goal initial target was not displayed")
    finally:
        first.close()

    resumed = InteractiveTerminal(
        binary,
        goal_project,
        goal_environment,
        session_id,
    )
    try:
        startup = resumed.wait_for(
            lambda text: "Type a message" in text,
            20,
            "goal restored startup",
        )
        require(marker in startup, "goal was not restored on process restart")
        require(
            "waiting for user" in startup.lower() or "state: paused" in startup.lower(),
            f"goal restored with wrong state: {startup[-1500:]}",
        )
        resume_start = len(resumed.visible())
        resumed.write_line("/goal resume")
        resume_output = resumed.wait_for(
            lambda text: (
                "resuming goal:" in text[resume_start:].lower()
                and "goal paused" in text[resume_start:].lower()
            ),
            150,
            "goal resume execution",
        )[resume_start:]
        require(marker in resume_output, "goal resume lost the persisted target")
        require(
            resume_output.lower().count("resuming goal:") == 1,
            f"goal resume UI duplicated: {resume_output[-2000:]}",
        )
        report(
            "goal-resume",
            restoredAfterRestart=True,
            resumedWithRealModel=True,
            targetPreserved=True,
        )
    finally:
        resumed.close()


def main() -> int:
    if len(sys.argv) != 2:
        print(f"Usage: {sys.argv[0]} /path/to/nola", file=sys.stderr)
        return 2
    binary = pathlib.Path(sys.argv[1]).expanduser().resolve()
    require(binary.is_file(), f"CLI binary is missing: {binary}")
    source_config = pathlib.Path(
        os.environ.get("NOLA_CONFIG_PATH", pathlib.Path.home() / ".nola" / "config.json")
    ).expanduser()
    require(source_config.is_file(), f"Nola config is missing: {source_config}")
    with tempfile.TemporaryDirectory(prefix="nola-real-agent-", dir="/tmp") as temporary, \
        tempfile.TemporaryDirectory(prefix="nola-real-agent-runtime-", dir="/tmp") as runtime:
        project = pathlib.Path(temporary)
        runtime_home = pathlib.Path(runtime)
        runtime_config = runtime_home / ".nola" / "config.json"
        runtime_config.parent.mkdir(parents=True)
        shutil.copy2(source_config, runtime_config)
        if os.environ.get("NOLA_REAL_AGENT_CASE") == "responses":
            isolated_config = json.loads(runtime_config.read_text(encoding="utf-8"))
            isolated_config.setdefault("llm", {})["provider"] = "openai-responses"
            runtime_config.write_text(
                json.dumps(isolated_config, ensure_ascii=False), encoding="utf-8"
            )
        environment = os.environ.copy()
        environment.update(
            {
                "HOME": str(runtime_home),
                "XDG_CONFIG_HOME": str(runtime_home / ".config"),
                "XDG_DATA_HOME": str(runtime_home / ".local" / "share"),
                "XDG_CACHE_HOME": str(runtime_home / ".cache"),
                "NOLA_DB_PATH": str(project / "nola.db"),
                "NOLA_SKIP_ONBOARDING": "1",
            }
        )
        if os.environ.get("NOLA_REAL_AGENT_CASE") == "responses":
            run = run_json(
                binary, project, environment,
                "Reply with exactly READY and nothing else.",
                extra=["--no-tools"], session_id="responses-e2e", timeout=120,
            )
            run.assert_protocol("responses")
            require("READY" in run.assistant, f"responses: unexpected answer: {run.assistant[-500:]}")
            follow_up = run_json(
                binary, project, environment,
                "What exact single word did you answer in the preceding turn? Reply with only that word.",
                extra=["--no-tools"], session_id="responses-e2e", timeout=120,
            )
            follow_up.assert_protocol("responses-follow-up")
            require("READY" in follow_up.assistant,
                    f"responses: follow-up lost the prior turn: {follow_up.assistant[-500:]}")
            sentinel = "NOLA_RESPONSES_TOOL_SENTINEL_7E2B"
            (project / "sentinel.txt").write_text(sentinel + "\n", encoding="utf-8")
            tool_round = run_json(
                binary, project, environment,
                "Use read_file to read sentinel.txt in this project, then report its exact contents.",
                extra=["--yes"], session_id="responses-e2e", timeout=120,
            )
            tool_round.assert_protocol("responses-tool-round")
            require("read_file" in tool_round.tool_names,
                    f"responses: no read_file call: {tool_round.tool_names}")
            require(sentinel in tool_round.assistant,
                    f"responses: tool result missing from answer: {tool_round.assistant[-500:]}")
            after_tool = run_json(
                binary, project, environment,
                "What exact sentinel did you read in the preceding turn? Reply with only the sentinel.",
                extra=["--no-tools"], session_id="responses-e2e", timeout=120,
            )
            after_tool.assert_protocol("responses-after-tool")
            require(sentinel in after_tool.assistant,
                    f"responses: historical tool output was lost: {after_tool.assistant[-500:]}")
            report("responses", model=isolated_config["llm"].get("model"),
                   answer=run.assistant[-200:], followUp=follow_up.assistant[-200:],
                   tools=tool_round.tool_names, toolAnswer=tool_round.assistant[-200:],
                   afterTool=after_tool.assistant[-200:])
            return 0
        if os.environ.get("NOLA_REAL_AGENT_CASE") == "dijkstra":
            run = run_json(
                binary,
                project,
                environment,
                "Write a C++ Dijkstra implementation in dijkstra.cpp in this project, "
                "then compile it with c++ to dijkstra-check. Report the actual compiler result.",
                extra=["--yes"],
                session_id="dijkstra-e2e",
                timeout=240,
            )
            if run.completed.returncode != 0:
                evidence = []
                for database in [*project.rglob("*.db"), *runtime_home.rglob("*.db")]:
                    with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as connection:
                        tables = connection.execute(
                            "SELECT name FROM sqlite_master WHERE type='table' AND name='effect_delivery_attempts'"
                        ).fetchall()
                        leases = connection.execute(
                            "SELECT state, lease_expires_at_ms, revision FROM effect_delivery_attempts ORDER BY rowid DESC LIMIT 8"
                        ).fetchall() if tables else []
                    evidence.append({"path": str(database.relative_to(project) if database.is_relative_to(project) else database.relative_to(runtime_home)), "leases": leases})
                report("dijkstra-lease-diagnostic", nowMs=int(time.time() * 1000), evidence=evidence)
            run.assert_protocol("dijkstra")
            require("<dots_function_call" not in run.assistant.lower(),
                    "dijkstra: provider tool-call markup leaked into the answer")
            require((project / "dijkstra.cpp").is_file(),
                    f"dijkstra: source was not created in the project: {run.tool_names}")
            require((project / "dijkstra-check").is_file(),
                    f"dijkstra: compiler output missing: {run.tool_names}; {run.assistant[-600:]}")
            report("dijkstra", tools=run.tool_names, compiled=True, assistant=run.assistant[-400:])
            return 0
        if os.environ.get("NOLA_REAL_AGENT_CASE") == "background":
            test_background_task_lifecycle(binary, project, environment)
            report("complete", passed=True)
            return 0
        if os.environ.get("NOLA_REAL_AGENT_CASE") == "goal":
            test_goal_resume(binary, project, environment)
            report("complete", passed=True)
            return 0
        if os.environ.get("NOLA_REAL_AGENT_CASE") == "checkpoint":
            test_cross_process_tool_history(binary, project, environment)
            report("complete", passed=True)
            return 0
        test_tools_and_permissions(binary, project, environment)
        test_session_permission(binary, project, environment)
        test_tool_failure_recovery(binary, project, environment)
        test_escape_cancellation_and_follow_up(binary, project, environment)
        test_cross_process_tool_history(binary, project, environment)
        test_network_and_large_file(binary, project, environment)
        test_compaction(binary, project, environment)
        test_automatic_compaction(binary, project, environment)
        test_background_task_lifecycle(binary, project, environment)
        test_goal_resume(binary, project, environment)
    report("complete", passed=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
