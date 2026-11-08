from __future__ import annotations

import json
import sqlite3
import subprocess
import sys
from pathlib import Path

import pytest
import audit_runtime_session as audit


SCRIPT = Path(__file__).with_name("audit_runtime_session.py")
SESSION_ID = "audit-contract-session"
TRACE_ID = "audit-contract-trace"
RUN_ID = "audit-contract-run"
STARTED_AT = "2026-08-16T00:00:00"
FINISHED_AT = "2026-08-16T00:00:10"
STARTED_AT_MS = 1786838400000


def create_database(path: Path, messages: list[tuple[int, str, str]]) -> None:
    """Use the current production tables, not legacy message JSON fixtures."""
    connection = sqlite3.connect(path)
    connection.executescript(
        """
        CREATE TABLE conversations (
            id TEXT PRIMARY KEY, title TEXT NOT NULL, revision INTEGER NOT NULL,
            created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL
        );
        CREATE TABLE runs (
            id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL,
            kind TEXT NOT NULL, created_at_ms INTEGER NOT NULL,
            completed_at_ms INTEGER, status TEXT NOT NULL,
            assistant_message_id TEXT NOT NULL
        );
        CREATE TABLE assistant_run_terminal_seals (
            run_id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL,
            seal_json TEXT NOT NULL
        );
        CREATE TABLE run_results (
            run_id TEXT PRIMARY KEY, result_json TEXT NOT NULL
        );
        CREATE TABLE run_events (
            run_id TEXT NOT NULL, sequence INTEGER NOT NULL,
            event_kind TEXT NOT NULL, payload_json TEXT NOT NULL
        );
        CREATE TABLE messages (
            id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL, run_id TEXT,
            role TEXT NOT NULL, status TEXT NOT NULL, ordinal INTEGER NOT NULL,
            created_at_ms INTEGER NOT NULL, completed_at_ms INTEGER,
            UNIQUE(conversation_id, ordinal)
        );
        CREATE TABLE message_parts (
            id TEXT PRIMARY KEY, message_id TEXT NOT NULL, kind TEXT NOT NULL,
            ordinal INTEGER NOT NULL, state TEXT NOT NULL,
            content_text TEXT NOT NULL DEFAULT '', content_json TEXT,
            content_digest TEXT, created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL, UNIQUE(message_id, ordinal)
        );
        CREATE TABLE trace_journal_runs (
            trace_id TEXT PRIMARY KEY, run_id TEXT NOT NULL UNIQUE,
            conversation_id TEXT NOT NULL, message_id TEXT, turn_id TEXT NOT NULL,
            status TEXT NOT NULL, last_sequence INTEGER NOT NULL,
            terminal_reason TEXT, started_at TEXT NOT NULL, updated_at TEXT NOT NULL,
            finished_at TEXT
        );
        CREATE TABLE trace_journal_events (
            event_id TEXT PRIMARY KEY, trace_id TEXT NOT NULL,
            conversation_id TEXT NOT NULL, turn_id TEXT NOT NULL,
            sequence INTEGER NOT NULL, kind TEXT NOT NULL, status TEXT NOT NULL,
            visibility TEXT NOT NULL, span_id TEXT, parent_span_id TEXT,
            emitted_at TEXT NOT NULL, event_json TEXT NOT NULL, envelope_json TEXT,
            UNIQUE(trace_id, sequence)
        );
        CREATE TABLE canonical_message_run_owners (
            conversation_id TEXT NOT NULL, message_id TEXT NOT NULL,
            run_id TEXT NOT NULL, PRIMARY KEY(conversation_id, message_id, run_id)
        );
        CREATE TABLE message_part_body_references (
            part_id TEXT NOT NULL, field TEXT NOT NULL, reference_json TEXT NOT NULL,
            PRIMARY KEY(part_id, field)
        );
        CREATE TABLE canonical_stream_heads (
            part_id TEXT PRIMARY KEY, head_json TEXT NOT NULL
        );
        """
    )
    connection.execute(
        "INSERT INTO conversations VALUES (?, 'Audit fixture', 1, ?, ?)",
        (SESSION_ID, STARTED_AT_MS, STARTED_AT_MS + 10000),
    )
    for position, role, content in messages:
        message_id = f"message-{position}"
        timestamp = STARTED_AT_MS + position * 1000
        connection.execute(
            "INSERT INTO messages VALUES (?, ?, ?, ?, 'completed', ?, ?, ?)",
            (message_id, SESSION_ID, RUN_ID, role, position + 1, timestamp, timestamp),
        )
        connection.execute(
            "INSERT INTO message_parts VALUES (?, ?, 'text', 1, 'completed', ?, NULL, NULL, ?, ?)",
            (f"part-{position}", message_id, content, timestamp, timestamp),
        )
    connection.execute(
        "INSERT INTO runs VALUES (?, ?, 'conversation', ?, ?, 'sealed', 'message-1')",
        (RUN_ID, SESSION_ID, STARTED_AT_MS, STARTED_AT_MS + 10000),
    )
    canonical_result = {
        "result_id": "result-1", "run_id": RUN_ID, "conversation_id": SESSION_ID,
        "value": {"kind": "answered", "answer_part_refs": ["part-1"]},
    }
    canonical_seal = {
        "identity": {"run_id": RUN_ID, "conversation_id": SESSION_ID,
                     "assistant_message_id": "message-1"},
        "completed_at_ms": STARTED_AT_MS + 10000, "result": canonical_result,
    }
    connection.execute("INSERT INTO run_results VALUES (?, ?)",
                       (RUN_ID, json.dumps(canonical_result)))
    connection.execute("INSERT INTO assistant_run_terminal_seals VALUES (?, ?, ?)",
                       (RUN_ID, SESSION_ID, json.dumps(canonical_seal)))
    connection.executemany("INSERT INTO run_events VALUES (?, ?, ?, ?)", [
        (RUN_ID, 1, "run_result_decided", json.dumps({"decision": canonical_result})),
        (RUN_ID, 2, "turn_completed", json.dumps({"turn_id": RUN_ID})),
    ])
    connection.execute(
        "INSERT INTO trace_journal_runs VALUES (?, ?, ?, 'message-1', ?, 'succeeded', 2, 'completed', ?, ?, ?)",
        (TRACE_ID, RUN_ID, SESSION_ID, RUN_ID, STARTED_AT, FINISHED_AT, FINISHED_AT),
    )
    insert_traces(connection, [
        (1, "assistant.delta", "running", {"delta": "latest answer"}, "2026-08-16T00:00:05"),
        (2, "run.completed", "succeeded", {"reason": "completed"}, FINISHED_AT),
    ])
    connection.commit()
    connection.close()


def insert_traces(connection: sqlite3.Connection, rows: list[tuple]) -> None:
    connection.executemany(
        "INSERT INTO trace_journal_events VALUES (?, ?, ?, ?, ?, ?, ?, 'public', NULL, NULL, ?, ?, NULL)",
        [
            (f"event-{sequence}", TRACE_ID, SESSION_ID, RUN_ID, sequence, kind,
             status, emitted_at, json.dumps({"payload": payload}))
            for sequence, kind, status, payload, emitted_at in rows
        ],
    )


def typed_text(text: str = "latest answer", **overrides: object) -> dict:
    value = {
        "kind": "text", "id": "part-1", "message_id": "message-1",
        "turn_id": RUN_ID, "text": text, "is_complete": True,
    }
    value.update(overrides)
    return value


def replace_assistant_part(database: Path, scalar: str, content_json: str | None) -> None:
    with sqlite3.connect(database) as connection:
        connection.execute(
            "UPDATE message_parts SET content_text = ?, content_json = ? WHERE id = 'part-1'",
            (scalar, content_json),
        )


def write_presentation(path: Path, text: str) -> None:
    path.write_text(
        json.dumps({"render_probe": {"raw_text": text, "body_mutation_count": 3}}),
        encoding="utf-8",
    )


def write_provider_trace(path: Path, terminal_answers: int) -> None:
    # Mix production SQLite UTC timestamps without an offset with the provider's
    # RFC 3339 offset: both must be compared on the same UTC timeline.
    rows = [
        {"event": "provider.response", "startedAt": f"2026-08-16T00:00:0{6 + index}+00:00",
         "finishReasons": ["stop"], "responseTextBytes": 20}
        for index in range(terminal_answers)
    ]
    path.write_text("".join(json.dumps(row) + "\n" for row in rows), encoding="utf-8")


def prepare_audit(tmp_path: Path) -> Path:
    database = tmp_path / "runtime.db"
    create_database(database, [(0, "user", "question"), (1, "assistant", "latest answer")])
    write_presentation(tmp_path / "presentation.json", "latest answer")
    write_provider_trace(tmp_path / "provider.ndjson", 1)
    return database


def run_audit(tmp_path: Path) -> tuple[subprocess.CompletedProcess[str], dict]:
    receipt = tmp_path / "receipt.json"
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "--db", str(tmp_path / "runtime.db"),
         "--session-id", SESSION_ID, "--presentation-receipt", str(tmp_path / "presentation.json"),
         "--provider-trace", str(tmp_path / "provider.ndjson"), "--receipt", str(receipt)],
        text=True, capture_output=True, check=False,
    )
    assert receipt.is_file(), result.stdout + result.stderr
    return result, json.loads(receipt.read_text(encoding="utf-8"))


def failed_assertions(receipt: dict) -> set[str]:
    return {assertion["name"] for assertion in receipt["assertions"] if not assertion["passed"]}


def assertion_detail(receipt: dict, name: str) -> str:
    return next(assertion["detail"] for assertion in receipt["assertions"] if assertion["name"] == name)


def test_complete_cross_layer_evidence_passes(tmp_path: Path) -> None:
    prepare_audit(tmp_path)
    result, receipt = run_audit(tmp_path)
    assert result.returncode == 0, result.stdout + result.stderr
    assert receipt["passed"] is True


@pytest.mark.parametrize("scalar", ["", "latest answer"])
def test_owned_typed_json_is_read_without_duplication(tmp_path: Path, scalar: str) -> None:
    database = prepare_audit(tmp_path)
    replace_assistant_part(database, scalar, json.dumps(typed_text()))
    result, receipt = run_audit(tmp_path)
    assert result.returncode == 0, result.stdout + result.stderr
    assert receipt["metrics"]["latest_turn_character_count"] == len("latest answer")


def test_part_view_schema_requires_bound_identity(tmp_path: Path) -> None:
    database = prepare_audit(tmp_path)
    replace_assistant_part(database, "", json.dumps({
        "kind": "text", "part_id": "part-1", "revision": 4,
        "markdown": "latest answer", "is_final": True,
    }))
    result, receipt = run_audit(tmp_path)
    assert result.returncode == 0, result.stdout + result.stderr
    assert receipt["passed"] is True


def test_text_parts_keep_exact_ordinal_and_ignore_nontext(tmp_path: Path) -> None:
    database = prepare_audit(tmp_path)
    replace_assistant_part(database, "latest ", None)
    with sqlite3.connect(database) as connection:
        connection.executemany(
            "INSERT INTO message_parts VALUES (?, 'message-1', ?, ?, 'completed', ?, ?, NULL, ?, ?)",
            [
                ("third", "text", 3, "answer", None, STARTED_AT_MS, STARTED_AT_MS),
                ("second", "reasoning", 2, "PRIVATE REASONING", '{"text":"not public"}', STARTED_AT_MS, STARTED_AT_MS),
            ],
        )
    result, receipt = run_audit(tmp_path)
    assert result.returncode == 0, result.stdout + result.stderr
    assert receipt["metrics"]["final_character_count"] == len("latest answer")


def test_inline_text_preserves_spaces_newlines_and_unicode(tmp_path: Path) -> None:
    database = prepare_audit(tmp_path)
    text = "  最终结果\n\n✓ 完成  \n"
    replace_assistant_part(database, "", json.dumps(typed_text(text), ensure_ascii=False))
    with sqlite3.connect(database) as connection:
        connection.execute(
            "UPDATE trace_journal_events SET event_json = ? WHERE kind = 'assistant.delta'",
            (json.dumps({"payload": {"delta": text}}, ensure_ascii=False),),
        )
    write_presentation(tmp_path / "presentation.json", text)
    result, receipt = run_audit(tmp_path)
    assert result.returncode == 0, result.stdout + result.stderr
    assert receipt["metrics"]["final_character_count"] == len(text)


@pytest.mark.parametrize("content_json, reason", [
    (json.dumps(typed_text(text="different body")), "scalar and typed JSON text conflict"),
    (json.dumps(typed_text(id="other-part")), "owner differs"),
    (json.dumps(typed_text(message_id="other-message")), "owner differs"),
    (json.dumps(typed_text(turn_id="other-run")), "owner differs"),
    (json.dumps(typed_text(turn_id={})), "owner differs"),
    (json.dumps(typed_text(kind="reasoning")), "kind disagree"),
    (json.dumps(typed_text(is_complete=False)), "finality differs"),
    (json.dumps(typed_text(is_complete=1)), "invalid typed text or finality"),
    (json.dumps(typed_text(text={})), "invalid typed text or finality"),
    (json.dumps(typed_text(review="pending")), "private or rejected review"),
    (json.dumps(typed_text(review="rejected")), "private or rejected review"),
    (json.dumps(typed_text(review={})), "private or rejected review"),
    (json.dumps({"kind": "text", "text": "latest answer"}), "no canonical text Part identity"),
    (json.dumps({"private_candidate": {"text": "latest answer"}}), "kind disagree"),
    ('{"kind":"text","kind":"text"}', "duplicate JSON field"),
    ('{"kind":"text","\\u006bind":"text"}', "duplicate JSON field"),
    ("{bad JSON}", "Expecting property name"),
    ("[]", "not an object"),
])
def test_conflicting_or_private_json_never_falls_back_to_scalar(
    tmp_path: Path, content_json: str, reason: str,
) -> None:
    database = prepare_audit(tmp_path)
    replace_assistant_part(database, "latest answer", content_json)
    result, receipt = run_audit(tmp_path)
    assert result.returncode == 1
    assert "canonical_content_read_complete" in failed_assertions(receipt)
    assert reason in assertion_detail(receipt, "canonical_content_read_complete")
    assert receipt["metrics"]["latest_turn_character_count"] == 0
    assert assertion_detail(receipt, "latest_turn_assistant_present") == "canonical content unavailable"


@pytest.mark.parametrize("current_owner_present", [False, True])
def test_historical_owner_requires_current_owner_membership(tmp_path: Path, current_owner_present: bool) -> None:
    database = prepare_audit(tmp_path)
    replace_assistant_part(database, "", json.dumps(typed_text(turn_id="prior-run")))
    with sqlite3.connect(database) as connection:
        connection.execute(
            "INSERT INTO canonical_message_run_owners VALUES (?, 'message-1', 'prior-run')", (SESSION_ID,),
        )
        if current_owner_present:
            connection.execute(
                "INSERT INTO canonical_message_run_owners VALUES (?, 'message-1', ?)", (SESSION_ID, RUN_ID),
            )
    result, receipt = run_audit(tmp_path)
    assert result.returncode == (0 if current_owner_present else 1), result.stdout + result.stderr
    assert ("canonical_content_read_complete" not in failed_assertions(receipt)) == current_owner_present


@pytest.mark.parametrize("table, statement", [
    ("immutable canonical body", "INSERT INTO message_part_body_references VALUES ('part-1', 'content_text', '{}')"),
    ("canonical stream body", "INSERT INTO canonical_stream_heads VALUES ('part-1', '{}')"),
])
def test_noninline_body_requires_application_reader_not_empty_or_scalar_fallback(
    tmp_path: Path, table: str, statement: str,
) -> None:
    database = prepare_audit(tmp_path)
    with sqlite3.connect(database) as connection:
        connection.execute(statement)
    result, receipt = run_audit(tmp_path)
    assert result.returncode == 1
    assert "canonical_content_read_complete" in failed_assertions(receipt)
    assert f"{table} requires the Application canonical reader" in assertion_detail(receipt, "canonical_content_read_complete")
    assert assertion_detail(receipt, "latest_turn_assistant_present") == "canonical content unavailable"
    assert "latest_trace_matches_persisted_assistant" in failed_assertions(receipt)
    assert "large_response_delivered_incrementally" in failed_assertions(receipt)
    assert "not_applicable" not in assertion_detail(receipt, "large_response_delivered_incrementally")


def test_old_assistant_cannot_mask_missing_latest_turn_answer(tmp_path: Path) -> None:
    create_database(tmp_path / "runtime.db", [
        (0, "user", "old question"), (1, "assistant", "old answer"), (2, "user", "latest question"),
    ])
    write_presentation(tmp_path / "presentation.json", "latest answer")
    write_provider_trace(tmp_path / "provider.ndjson", 1)
    result, receipt = run_audit(tmp_path)
    assert result.returncode == 1
    assert "latest_turn_assistant_present" in failed_assertions(receipt)
    assert "latest_trace_matches_persisted_assistant" in failed_assertions(receipt)


def test_old_answer_cannot_fill_empty_failed_assistant(tmp_path: Path) -> None:
    create_database(tmp_path / "runtime.db", [
        (0, "user", "old question"), (1, "assistant", "old answer"),
        (2, "user", "latest question"), (3, "assistant", ""),
    ])
    with sqlite3.connect(tmp_path / "runtime.db") as connection:
        connection.execute("UPDATE messages SET status = 'failed' WHERE id = 'message-3'")
        connection.execute("UPDATE message_parts SET state = 'failed' WHERE id = 'part-3'")
    write_presentation(tmp_path / "presentation.json", "old answer")
    write_provider_trace(tmp_path / "provider.ndjson", 1)
    result, receipt = run_audit(tmp_path)
    assert result.returncode == 1
    assert "final_assistant_message_present" in failed_assertions(receipt)
    assert "latest_turn_assistant_completed" in failed_assertions(receipt)
    assert receipt["metrics"]["final_character_count"] == 0


def test_cancelled_partial_answer_is_not_completed_acceptance(tmp_path: Path) -> None:
    database = prepare_audit(tmp_path)
    with sqlite3.connect(database) as connection:
        connection.execute("UPDATE messages SET status = 'cancelled' WHERE id = 'message-1'")
        connection.execute("UPDATE message_parts SET state = 'interrupted' WHERE id = 'part-1'")
        connection.execute("UPDATE trace_journal_runs SET status = 'cancelled', terminal_reason = 'cancelled'")
        connection.execute("UPDATE trace_journal_events SET kind = 'run.cancelled', status = 'cancelled' WHERE sequence = 2")
        result = json.loads(connection.execute("SELECT result_json FROM run_results").fetchone()[0])
        result["value"] = {"kind": "cancelled", "cancellation_id": "cancel-1",
                           "observed_effect_refs": [], "unknown_effect_refs": []}
        seal = json.loads(connection.execute("SELECT seal_json FROM assistant_run_terminal_seals").fetchone()[0])
        seal["result"] = result
        connection.execute("UPDATE run_results SET result_json=?", (json.dumps(result),))
        connection.execute("UPDATE assistant_run_terminal_seals SET seal_json=?", (json.dumps(seal),))
        connection.execute("UPDATE run_events SET payload_json=? WHERE event_kind='run_result_decided'",
                           (json.dumps({"decision": result}),))
        connection.execute("UPDATE run_events SET event_kind='turn_cancelled' WHERE event_kind='turn_completed'")
    result, receipt = run_audit(tmp_path)
    assert result.returncode == 1
    failures = failed_assertions(receipt)
    assert "latest_turn_assistant_completed" in failures
    assert "canonical_runs_match_expected_result" in failures
    assert "canonical_run_result_seal_consistent" not in failures


def test_presentation_must_match_durable_latest_answer(tmp_path: Path) -> None:
    prepare_audit(tmp_path)
    write_presentation(tmp_path / "presentation.json", "different UI answer")
    result, receipt = run_audit(tmp_path)
    assert result.returncode == 1
    assert "presentation_matches_persisted_assistant" in failed_assertions(receipt)


def test_multiple_provider_completions_do_not_duplicate_canonical_run_result(tmp_path: Path) -> None:
    prepare_audit(tmp_path)
    write_provider_trace(tmp_path / "provider.ndjson", 2)
    result, receipt = run_audit(tmp_path)
    # A model can finish multiple tool/recovery rounds within one Run. Only
    # committed Core decisions determine whether the Run completed twice.
    assert result.returncode == 0, result.stdout + result.stderr
    assert receipt["metrics"]["latest_run_terminal_provider_answer_count"] == 2
    assert "canonical_run_result_seal_consistent" not in failed_assertions(receipt)


def test_duplicate_canonical_terminal_is_rejected(tmp_path: Path) -> None:
    database = prepare_audit(tmp_path)
    with sqlite3.connect(database) as connection:
        connection.execute("INSERT INTO run_events VALUES (?, 3, 'turn_completed', ?)",
                           (RUN_ID, json.dumps({"turn_id": RUN_ID})))
    result, receipt = run_audit(tmp_path)
    assert result.returncode == 1
    assert "canonical_run_result_seal_consistent" in failed_assertions(receipt)


@pytest.mark.parametrize("executes_denied_call", [False, True])
def test_permission_denial_has_no_execution(tmp_path: Path, executes_denied_call: bool) -> None:
    database = prepare_audit(tmp_path)
    replace_assistant_part(database, "permission denied", None)
    with sqlite3.connect(database) as connection:
        connection.execute("DELETE FROM trace_journal_events")
        rows = [
            (1, "run.started", "running", {}, "2026-08-16T00:00:00.100Z"),
            (2, "tool.permission.resolved", "failed",
             {"call_id": "denied-call", "permission_decision": "deny"}, "2026-08-16T00:00:05.100Z"),
        ]
        if executes_denied_call:
            rows.append((3, "tool.started", "running", {"call_id": "denied-call"}, "2026-08-16T00:00:05.101Z"))
        sequence = len(rows) + 1
        rows.extend([
            (sequence, "tool.completed", "failed", {"call_id": "denied-call", "detail": "Permission denied"}, "2026-08-16T00:00:05.102Z"),
            (sequence + 1, "assistant.delta", "running", {"delta": "permission denied"}, "2026-08-16T00:00:06.000Z"),
            (sequence + 2, "run.completed", "succeeded", {}, "2026-08-16T00:00:06.100Z"),
        ])
        insert_traces(connection, rows)
    write_presentation(tmp_path / "presentation.json", "permission denied")
    result, receipt = run_audit(tmp_path)
    assert result.returncode == (1 if executes_denied_call else 0), result.stdout + result.stderr
    assert ("denied_tools_not_executed" in failed_assertions(receipt)) == executes_denied_call


def canonical_range(text: str = "latest answer", **changes: object) -> dict:
    value = {"conversation_id": SESSION_ID, "message_id": "message-1", "part_id": "part-1",
             "offset_bytes": 0, "next_offset_bytes": len(text.encode("utf-8")),
             "total_utf8_bytes": len(text.encode("utf-8")), "text": text, "end_of_content": True}
    value.update(changes)
    return value


def reader_fixture(tmp_path: Path) -> tuple[sqlite3.Connection, audit.ApplicationCanonicalTextReader]:
    database = prepare_audit(tmp_path)
    connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
    connection.row_factory = sqlite3.Row
    connection.execute("BEGIN")
    connection.execute("SELECT id FROM conversations").fetchall()
    # The executable is not called in these protocol-unit tests. Its subprocess
    # transport is mocked below; real canonical semantics require the Rust bridge.
    return connection, audit.ApplicationCanonicalTextReader(connection, Path(sys.executable))


def mock_range_transport(monkeypatch: pytest.MonkeyPatch, response: dict) -> None:
    def invoke(arguments: list[str], **options: object) -> subprocess.CompletedProcess:
        return subprocess.CompletedProcess(arguments, 0, json.dumps({"ok": True, "range": response}), "")
    monkeypatch.setattr(audit.subprocess, "run", invoke)


def test_application_bridge_uses_private_snapshot_not_live_database(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    connection, reader = reader_fixture(tmp_path)
    snapshot_paths = []

    def invoke(arguments: list[str], **options: object) -> subprocess.CompletedProcess:
        snapshot = Path(arguments[2])
        snapshot_paths.append(snapshot)
        assert arguments[1] == "--snapshot-database"
        assert snapshot != tmp_path / "runtime.db"
        assert snapshot.name == "snapshot.db"
        assert snapshot.parent.name.startswith("nola-canonical-audit-")
        assert snapshot.stat().st_mode & 0o777 == 0o600
        assert snapshot.parent.stat().st_mode & 0o777 == 0o700
        assert options["env"]["NOLA_DB_PATH"] == str(snapshot)
        assert options["env"].get("HOME") == audit.os.environ.get("HOME")
        with sqlite3.connect(snapshot) as copied:
            copied.execute("UPDATE message_parts SET content_text = 'snapshot-only-change' WHERE id = 'part-1'")
        return subprocess.CompletedProcess(arguments, 0, json.dumps({"ok": True, "range": canonical_range()}), "")

    monkeypatch.setattr(audit.subprocess, "run", invoke)
    try:
        assert reader.read_part(SESSION_ID, "message-1", "part-1") == "latest answer"
        assert connection.execute("SELECT content_text FROM message_parts WHERE id = 'part-1'").fetchone()[0] == "latest answer"
        assert reader.query_count == 1
        assert reader.total_bytes == len("latest answer")
        assert reader.binary_sha256 is not None
    finally:
        reader.close()
        connection.close()
    assert not snapshot_paths[0].parent.exists()


def test_backup_preserves_the_already_pinned_revision(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    database = prepare_audit(tmp_path)
    with sqlite3.connect(database) as writer:
        writer.execute("PRAGMA journal_mode = WAL")
    connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
    connection.row_factory = sqlite3.Row
    connection.execute("BEGIN")
    assert connection.execute("SELECT content_text FROM message_parts WHERE id='part-1'").fetchone()[0] == "latest answer"
    with sqlite3.connect(database) as writer:
        writer.execute("UPDATE message_parts SET content_text='newer live answer' WHERE id='part-1'")
    reader = audit.ApplicationCanonicalTextReader(connection, Path(sys.executable))

    def invoke(arguments: list[str], **options: object) -> subprocess.CompletedProcess:
        with sqlite3.connect(f"file:{arguments[2]}?mode=ro", uri=True) as copied:
            assert copied.execute("SELECT content_text FROM message_parts WHERE id='part-1'").fetchone()[0] == "latest answer"
        return subprocess.CompletedProcess(arguments, 0, json.dumps({"ok": True, "range": canonical_range()}), "")

    monkeypatch.setattr(audit.subprocess, "run", invoke)
    try:
        assert reader.read_part(SESSION_ID, "message-1", "part-1") == "latest answer"
    finally:
        reader.close()
        connection.close()


def test_canonical_bridge_ranges_use_utf8_bytes_not_characters(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    connection, reader = reader_fixture(tmp_path)
    reader.RANGE_BYTES = 4
    requests = []

    def invoke(arguments: list[str], **options: object) -> subprocess.CompletedProcess:
        request = json.loads(options["input"])
        requests.append(request)
        offset = request["offset_bytes"]
        text = "🙂" if offset == 0 else "a✓"
        value = canonical_range(text, offset_bytes=offset, next_offset_bytes=offset + 4,
                                total_utf8_bytes=8, end_of_content=offset == 4)
        return subprocess.CompletedProcess(arguments, 0, json.dumps({"ok": True, "range": value}), "")

    monkeypatch.setattr(audit.subprocess, "run", invoke)
    try:
        assert reader.read_part(SESSION_ID, "message-1", "part-1") == "🙂a✓"
        assert [request["offset_bytes"] for request in requests] == [0, 4]
        assert reader.total_bytes == 8
    finally:
        reader.close()
        connection.close()


@pytest.mark.parametrize("changes", [
    {"conversation_id": "other"}, {"message_id": "other"}, {"part_id": "other"},
    {"offset_bytes": 1}, {"offset_bytes": False}, {"next_offset_bytes": 1},
    {"total_utf8_bytes": 1}, {"total_utf8_bytes": True}, {"text": {}},
    {"end_of_content": 1}, {"end_of_content": False}, {"extra": "UI cache"},
    {"text": "", "next_offset_bytes": 0, "total_utf8_bytes": 1, "end_of_content": False},
])
def test_canonical_bridge_rejects_wrong_owner_bounds_or_untyped_response(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, changes: dict,
) -> None:
    connection, reader = reader_fixture(tmp_path)
    response = canonical_range()
    response.update(changes)
    mock_range_transport(monkeypatch, response)
    try:
        with pytest.raises(ValueError):
            reader.read_part(SESSION_ID, "message-1", "part-1")
        assert reader.total_bytes == 0
    finally:
        reader.close()
        connection.close()


def test_canonical_bridge_rejects_changed_total_and_explicit_budget_overflow(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    connection, reader = reader_fixture(tmp_path)
    reader.RANGE_BYTES = 4
    reader.MAXIMUM_PART_BYTES = 8
    mock_range_transport(monkeypatch, canonical_range("abcd", total_utf8_bytes=9, end_of_content=False))
    try:
        with pytest.raises(ValueError, match="explicit audit read budget"):
            reader.read_part(SESSION_ID, "message-1", "part-1")
        calls = 0

        def invoke(arguments: list[str], **options: object) -> subprocess.CompletedProcess:
            nonlocal calls
            offset = json.loads(options["input"])["offset_bytes"]
            calls += 1
            value = canonical_range("abcd", offset_bytes=offset, next_offset_bytes=offset + 4,
                                    total_utf8_bytes=8 if calls == 1 else 9, end_of_content=False)
            return subprocess.CompletedProcess(arguments, 0, json.dumps({"ok": True, "range": value}), "")

        monkeypatch.setattr(audit.subprocess, "run", invoke)
        with pytest.raises(ValueError, match="length changed"):
            reader.read_part(SESSION_ID, "message-1", "part-1")
    finally:
        reader.close()
        connection.close()


def test_chunk_backing_can_only_use_actual_reader_boundary_not_inline_fallback(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    database = prepare_audit(tmp_path)
    with sqlite3.connect(database) as writer:
        writer.execute("INSERT INTO canonical_stream_heads VALUES ('part-1', '{}')")
        writer.execute("UPDATE message_parts SET content_text='wrong scalar fallback' WHERE id='part-1'")
    connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
    connection.row_factory = sqlite3.Row
    connection.execute("BEGIN")
    reader = audit.ApplicationCanonicalTextReader(connection, Path(sys.executable))
    mock_range_transport(monkeypatch, canonical_range())
    try:
        messages = audit.load_messages(connection, SESSION_ID, reader)
        assert json.loads(messages[-1]["content_json"])["content"] == "latest answer"
        assert messages[-1]["content_complete"]
        assert not messages[-1]["content_errors"]
        monkeypatch.setattr(audit.subprocess, "run", lambda *args, **options: subprocess.CompletedProcess(args, 0, '{"ok":false,"error":"rejected"}', ""))
        rejected = audit.load_messages(connection, SESSION_ID, reader)[-1]
        assert rejected["content_errors"]
        assert json.loads(rejected["content_json"])["content"] == ""
    finally:
        reader.close()
        connection.close()
