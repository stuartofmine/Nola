import json
from pathlib import Path
import sqlite3
import struct
import zlib
import pytest

from real_e2e_judge import evaluate_receipt


def _write(path: Path, value: str) -> str:
    path.write_text(value, encoding="utf-8")
    return str(path)


def _observation(kind: str, path: str) -> dict:
    return {"kind": kind, "path": path}


def _scenario(*validators: str) -> dict:
    return {"_contract": {"validators": list(validators)}, "evidence": {}}


TERMINAL_KINDS = ("terminal_capture", "terminal_frames", "terminal_actions", "terminal_metrics",
                  "persistence_audit", "persistence_store")


def test_terminal_dispatch_passes_one_same_step_bundle_to_the_verifier(tmp_path, monkeypatch):
    """Dispatch unit only: real VT100/lifecycle proof has separate gates."""
    import terminal_acceptance_audit
    paths = [tmp_path / kind for kind in TERMINAL_KINDS]
    oracle = tmp_path / "trusted-current-oracle"
    monkeypatch.setenv("NOLA_TERMINAL_ORACLE_BIN", str(oracle))
    calls = []

    def verify(*arguments):
        calls.append(arguments)
        assert arguments == (*paths, oracle), "owner DB, frame cache and audit cannot be swapped"
        return {"unitDispatchOnly": True}

    monkeypatch.setattr(terminal_acceptance_audit, "verify", verify)
    observations = [_observation(kind, str(path)) for kind, path in zip(TERMINAL_KINDS, paths)]
    result = evaluate_receipt({"steps": [{"observations": observations}]}, _scenario("terminal_ui"))
    assert result["passed"] is True
    assert result["assertions"]["terminal_ui"]["metrics"] == {"unitDispatchOnly": True}
    assert len(calls) == 1


@pytest.mark.parametrize("kind", TERMINAL_KINDS)
@pytest.mark.parametrize("fault", ("missing", "duplicate", "split_step"))
def test_terminal_dispatch_rejects_nonunique_or_cross_step_evidence(tmp_path, monkeypatch, kind, fault):
    import terminal_acceptance_audit
    monkeypatch.setattr(terminal_acceptance_audit, "verify",
                        lambda *args: pytest.fail("invalid observation ownership must not reach verifier"))
    observations = [_observation(value, str(tmp_path / value)) for value in TERMINAL_KINDS]
    selected = next(row for row in observations if row["kind"] == kind)
    steps = [{"observations": observations}]
    if fault == "duplicate": observations.append(dict(selected))
    else:
        observations.remove(selected)
        if fault == "split_step": steps.append({"observations": [selected]})
    result = evaluate_receipt({"steps": steps}, _scenario("terminal_ui"))
    assert result["passed"] is False


@pytest.mark.parametrize("reason", ("foreign Run ownership", "frame cache disagrees with replay",
                                     "formal seal differs from retained database"))
def test_terminal_dispatch_preserves_real_verifier_rejection(tmp_path, monkeypatch, reason):
    import terminal_acceptance_audit

    def reject(*arguments):
        raise ValueError(reason)

    monkeypatch.setattr(terminal_acceptance_audit, "verify", reject)
    result = evaluate_receipt({"steps": [{"observations": [
        _observation(kind, str(tmp_path / kind)) for kind in TERMINAL_KINDS]}]}, _scenario("terminal_ui"))
    assert result["passed"] is False
    assert reason in result["assertions"]["terminal_ui"]["metrics"]["errors"][0]


def _terminal_fixture(database, transcript, kind="answered"):
    terminal = {"conversation_id": "conversation", "run_id": "run",
                "assistant_message_id": "assistant", "status": "sealed",
                "completed_at_ms": 7, "result_digest": "a" * 64,
                "result": {"value": {"kind": kind}}}
    seal = {"identity": {key: terminal[key] for key in
                         ("conversation_id", "run_id", "assistant_message_id")},
            **{key: terminal[key] for key in ("completed_at_ms", "result_digest", "result")}}
    database.executescript("""
        CREATE TABLE runs(id TEXT,conversation_id TEXT,assistant_message_id TEXT,status TEXT,
            completed_at_ms INTEGER,kind TEXT);
        CREATE TABLE assistant_run_terminal_seals(run_id TEXT,conversation_id TEXT,seal_json TEXT);
        INSERT INTO runs VALUES('run','conversation','assistant','sealed',7,'conversation');
    """)
    database.execute("INSERT INTO assistant_run_terminal_seals VALUES('run','conversation',?)",
                     (json.dumps(seal),))
    rows = json.loads(Path(transcript).read_text())
    rows[-1]["canonicalTerminal"] = terminal
    Path(transcript).write_text(json.dumps(rows))


def _png(path: Path, width: int = 640, height: int = 360) -> str:
    def chunk(kind: bytes, payload: bytes) -> bytes:
        return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF)
    rows = b"".join(
        b"\x00" + bytes(((x + y) % 251 for x in range(width * 3)))
        for y in range(height)
    )
    payload = b"\x89PNG\r\n\x1a\n"
    payload += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
    payload += chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")
    path.write_bytes(payload)
    return str(path)


def test_central_judge_accepts_complete_real_product_chain(tmp_path: Path) -> None:
    provider = _write(
        tmp_path / "provider.ndjson",
        json.dumps({
            "event": "provider.response",
            "status": 200,
            "responseBytes": 256,
            "upstreamPath": "/v1/chat/completions",
            "upstreamHost": "dashscope.aliyuncs.com",
            "model": "qwen",
        }) + "\n",
    )
    events = _write(
        tmp_path / "events.ndjson",
        "\n".join([
            json.dumps({"type": "agent_trace", "trace": {"kind": "tool.started", "sequence": 1, "payload": {"tool_name": "web_search", "call_id": "call-1"}}}),
            json.dumps({"type": "agent_trace", "trace": {"kind": "tool.completed", "sequence": 2, "payload": {"tool_name": "web_search", "call_id": "call-1"}}}),
            json.dumps({"type": "core_part_updated", "cursor": 3, "part_id": "answer", "part": {"kind": "text", "text": "NOLA-E2E-12345678", "is_complete": True}}),
            json.dumps({"type": "agent_trace", "trace": {"kind": "run.completed", "sequence": 4, "payload": {}}}),
        ]) + "\n",
    )
    transcript = _write(
        tmp_path / "transcript.json",
        json.dumps([
            {"role": "user", "text": "search"},
            {"role": "assistant", "text": "NOLA-E2E-12345678"},
        ]),
    )
    timeline = _write(
        tmp_path / "timeline.json",
        json.dumps([
            {"stage": "turn", "running": True, "assistantCharacters": 4},
            {"stage": "turn", "running": True, "assistantCharacters": 12},
            {"stage": "turn", "running": False, "assistantCharacters": 17},
        ]),
    )
    accessibility = _write(tmp_path / "accessibility.txt", "NOLA-E2E-12345678")
    persistence_path = tmp_path / "state.sqlite"
    database = sqlite3.connect(persistence_path)
    database.execute("CREATE TABLE messages(id INTEGER PRIMARY KEY, body TEXT NOT NULL)")
    database.execute("INSERT INTO messages(body) VALUES ('persisted')")
    _terminal_fixture(database, transcript)
    database.commit()
    database.close()
    persistence = str(persistence_path)
    screenshot = _png(tmp_path / "screen.png")
    receipt = {
        "steps": [{"observations": [
            _observation("provider_trace", provider),
            _observation("event_stream", events),
            _observation("transcript", transcript),
            _observation("ui_timeline", timeline),
            _observation("accessibility_tree", accessibility),
            _observation("ui_screenshot", screenshot),
            _observation("persistence_store", persistence),
        ]}]
    }

    result = evaluate_receipt(receipt, _scenario(
        "provider_real", "event_terminal", "tool_pairing", "assistant_nonempty",
        "persistence_present", "transcript_order", "ui_terminal", "ui_progressive",
        "accessibility_matches_transcript", "ui_screenshot_valid",
    ))

    assert result["passed"] is True
    assert all(value["passed"] for value in result["assertions"].values())


def test_self_reported_success_cannot_replace_raw_evidence() -> None:
    receipt = {
        "realProviderRequests": 99,
        "steps": [{
            "assertions": [{"id": "provider", "passed": True}],
            "observations": [],
        }],
    }

    result = evaluate_receipt(
        receipt,
        _scenario("provider_real", "event_terminal", "persistence_present"),
    )

    assert result["passed"] is False
    assert result["assertions"]["provider_real"]["passed"] is False
    assert result["assertions"]["event_terminal"]["passed"] is False
    assert result["assertions"]["persistence_present"]["passed"] is False


def test_persistence_judge_rejects_nonempty_fake_store_and_empty_audit(tmp_path: Path) -> None:
    store = _write(tmp_path / "fake.sqlite", "not a database but nonempty")
    audit = _write(tmp_path / "audit.json", json.dumps({"passed": True, "assertions": {}}))
    result = evaluate_receipt(
        {"steps": [{"observations": [
            _observation("persistence_store", store),
            _observation("persistence_audit", audit),
        ]}]},
        _scenario("persistence_present"),
    )
    assert result["passed"] is False


def test_semantic_audit_does_not_pollute_persistence_validation(tmp_path: Path) -> None:
    persistence_audit = _write(
        tmp_path / "persistence.json",
        json.dumps({
            "passed": True,
            "assertions": [{"name": "message_round_trip", "passed": True}],
        }),
    )
    semantic_audit = _write(
        tmp_path / "semantic.json",
        json.dumps({"passed": True, "verdict": {"pass": True}}),
    )
    result = evaluate_receipt(
        {"steps": [{"observations": [
            _observation("persistence_audit", persistence_audit),
            _observation("semantic_audit", semantic_audit),
        ]}]},
        _scenario("persistence_present"),
    )
    assert result["passed"] is True


def test_http_judge_requires_authenticated_identity_and_message_round_trip(tmp_path: Path) -> None:
    marker = "backend-e2e-marker"
    transcript = _write(
        tmp_path / "http.json",
        json.dumps({
            "marker": marker,
            "exchanges": [
                {"name": name, "status": 200}
                for name in (
                    "health", "register", "profile", "create_conversation",
                    "append_message", "list_messages",
                )
            ],
            "responses": {
                "register": {"userId": "user-1", "accessTokenIssued": True},
                "profile": {"userId": "user-1"},
                "list_messages": {"messages": [{"role": "user", "content": marker}]},
            },
        }),
    )
    result = evaluate_receipt(
        {"steps": [{"observations": [_observation("http_transcript", transcript)]}]},
        _scenario("http_api"),
    )
    assert result["passed"] is True


def test_http_judge_rejects_success_codes_without_round_trip(tmp_path: Path) -> None:
    transcript = _write(
        tmp_path / "http.json",
        json.dumps({
            "marker": "missing",
            "exchanges": [
                {"name": name, "status": 200}
                for name in (
                    "health", "register", "profile", "create_conversation",
                    "append_message", "list_messages",
                )
            ],
            "responses": {
                "register": {"userId": "user-1"},
                "profile": {"userId": "other-user"},
                "list_messages": {"messages": []},
            },
        }),
    )
    result = evaluate_receipt(
        {"steps": [{"observations": [_observation("http_transcript", transcript)]}]},
        _scenario("http_api"),
    )
    assert result["passed"] is False


def test_screenshot_judge_rejects_arbitrary_large_bytes(tmp_path: Path) -> None:
    fake = tmp_path / "fake.png"
    fake.write_bytes(b"not-a-png" * 500)
    result = evaluate_receipt(
        {"steps": [{"observations": [_observation("ui_screenshot", str(fake))]}]},
        _scenario("ui_screenshot_valid"),
    )
    assert result["passed"] is False


def test_progressive_judge_rejects_single_frame_and_regression(tmp_path: Path) -> None:
    single = _write(
        tmp_path / "single.json",
        json.dumps([{"stage": "turn", "running": False, "assistantCharacters": 120}]),
    )
    regressing = _write(
        tmp_path / "regressing.json",
        json.dumps([
            {"stage": "turn", "running": True, "assistantCharacters": 10},
            {"stage": "turn", "running": True, "assistantCharacters": 30},
            {"stage": "turn", "running": False, "assistantCharacters": 20},
        ]),
    )
    for path in (single, regressing):
        result = evaluate_receipt(
            {"steps": [{"observations": [_observation("ui_timeline", path)]}]},
            _scenario("ui_progressive"),
        )
        assert result["passed"] is False


def test_tool_judge_rejects_unfinished_call(tmp_path: Path) -> None:
    events = _write(
        tmp_path / "events.ndjson",
        json.dumps({"type": "agent_trace", "trace": {"kind": "tool.started", "sequence": 1, "payload": {"tool_name": "file_write", "call_id": "call-1"}}}) + "\n",
    )
    result = evaluate_receipt(
        {"steps": [{"observations": [_observation("event_stream", events)]}]},
        _scenario("tool_pairing"),
    )
    assert result["passed"] is False
    assert result["assertions"]["tool_pairing"]["metrics"]["missing"] == {"call-1": 1}


def test_accessibility_judge_rejects_missing_rendered_marker(tmp_path: Path) -> None:
    transcript = _write(
        tmp_path / "transcript.json",
        json.dumps([
            {"role": "user", "text": "create"},
            {"role": "assistant", "text": "Created NOLA-E2E-ABCDEFGH"},
        ]),
    )
    accessibility = _write(tmp_path / "accessibility.txt", "unrelated content")
    result = evaluate_receipt(
        {"steps": [{"observations": [
            _observation("transcript", transcript),
            _observation("accessibility_tree", accessibility),
        ]}]},
        _scenario("accessibility_matches_transcript"),
    )
    assert result["passed"] is False


def test_accessibility_judge_parses_json_escaped_multiline_text(tmp_path: Path) -> None:
    answer = (
        "The rendered answer begins on this line and continues with enough content "
        "to cross the semantic comparison boundary.\nA second visible line follows."
    )
    transcript = _write(
        tmp_path / "transcript.json",
        json.dumps([
            {"role": "user", "text": "show it"},
            {"role": "assistant", "text": answer},
        ]),
    )
    accessibility = _write(
        tmp_path / "accessibility.json",
        json.dumps({"text": answer, "nodes": [{"role": "text", "text": answer}]}),
    )

    result = evaluate_receipt(
        {"steps": [{"observations": [
            _observation("transcript", transcript),
            _observation("accessibility_tree", accessibility),
        ]}]},
        _scenario("accessibility_matches_transcript"),
    )

    assert result["passed"] is True


def test_accessibility_judge_matches_fragmented_swiftui_labels(tmp_path: Path) -> None:
    answer = (
        "Verified\n"
        "Source line: The product rendered this complete semantic line from a real response.\n"
        "Grounding: The visible result came from the observed tool output rather than an assumption."
    )
    transcript = _write(
        tmp_path / "transcript.json",
        json.dumps([
            {"role": "user", "text": "show it"},
            {"role": "assistant", "text": answer},
        ]),
    )
    accessibility = _write(
        tmp_path / "accessibility.txt",
        "\n".join([
            "StaticText, label: 'Verified'",
            "StaticText, label: 'Source line: The product rendered this complete semantic line from a real response.'",
            "StaticText, label: 'Grounding: The visible result came from the observed tool output rather than an assumption.'",
        ]),
    )
    result = evaluate_receipt(
        {"steps": [{"observations": [
            _observation("transcript", transcript),
            _observation("accessibility_tree", accessibility),
        ]}]},
        _scenario("accessibility_matches_transcript"),
    )
    assert result["passed"] is True


def test_mac_projection_events_are_judged_from_raw_fields(tmp_path: Path) -> None:
    events = _write(
        tmp_path / "mac-events.jsonl",
        "\n".join([
            json.dumps({
                "kind": "toolStarted", "trace_kind": "tool.started",
                "tool_name": "read_file", "tool_call_id": "call-1",
            }),
            json.dumps({
                "kind": "toolFinished", "trace_kind": "tool.completed",
                "tool_name": "read_file", "tool_call_id": "call-1",
            }),
            json.dumps({
                "kind": "assistantDelta", "trace_kind": "assistant.delta",
                "text": "A real Mac answer",
            }),
            json.dumps({"kind": "finished", "trace_kind": "run.completed"}),
        ]) + "\n",
    )
    result = evaluate_receipt(
        {"steps": [{"observations": [_observation("event_stream", events)]}]},
        _scenario("event_terminal", "tool_pairing", "assistant_nonempty"),
    )
    assert result["passed"] is False
    assert result["assertions"]["event_terminal"]["passed"] is False
    assert result["assertions"]["tool_pairing"]["passed"] is True
    assert result["assertions"]["assistant_nonempty"]["passed"] is True


def test_terminal_gate_preserves_outcomes_without_trace(tmp_path: Path) -> None:
    for kind in ("answered", "action_completed", "incomplete", "blocked", "failed", "cancelled"):
        transcript = _write(tmp_path / f"{kind}.json", json.dumps([{"role": "assistant"}]))
        store = tmp_path / f"{kind}.sqlite"
        with sqlite3.connect(store) as database:
            _terminal_fixture(database, transcript, kind)
        receipt = {"steps": [{"observations": [
            _observation("transcript", transcript), _observation("persistence_store", str(store))]}]}
        result = evaluate_receipt(receipt, _scenario("event_terminal"))
        assert result["passed"] is True
        audit = result["assertions"]["event_terminal"]["metrics"]["audits"][0]
        assert audit["outcomes"] == {kind: 1}
        assert audit["canonicalSealedSuccessfulRuns"] == int(kind in ("answered", "action_completed"))
        # A persisted seal without the exact production-readback identity is insufficient.
        rows = json.loads(Path(transcript).read_text())
        rows[0]["canonicalTerminal"]["run_id"] = "foreign"
        Path(transcript).write_text(json.dumps(rows))
        assert evaluate_receipt(receipt, _scenario("event_terminal"))["passed"] is False


def test_terminal_evidence_cannot_be_borrowed_across_steps(tmp_path: Path) -> None:
    transcript = _write(tmp_path / "transcript.json", json.dumps([{"role": "assistant"}]))
    store = tmp_path / "store.sqlite"
    with sqlite3.connect(store) as database:
        _terminal_fixture(database, transcript)
    receipt = {"steps": [
        {"observations": [_observation("transcript", transcript)]},
        {"observations": [_observation("persistence_store", str(store))]},
    ]}
    assert evaluate_receipt(receipt, _scenario("event_terminal"))["passed"] is False
