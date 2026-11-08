import json

from real_e2e_events import summarize
from real_e2e_judge import _judge_tool_pairing


def _persisted_row(kind: str, sequence: int, *, call_id: str = "", tool: str = "") -> dict:
    payload = {}
    if call_id:
        payload["call_id"] = call_id
    if tool:
        payload["tool_name"] = tool
    event = {
        "kind": kind,
        "sequence": sequence,
        "payload": payload,
    }
    return {
        "kind": kind,
        "sequence": sequence,
        "event_json": json.dumps(event),
    }


def _receipt(path) -> dict:
    return {
        "steps": [{
            "observations": [{
                "id": "events",
                "kind": "event_stream",
                "path": str(path),
                "exists": True,
            }],
        }],
    }


def test_sqlite_event_json_rows_preserve_terminal_and_tool_call_identity(tmp_path):
    path = tmp_path / "events.json"
    path.write_text(json.dumps([
        _persisted_row("run.started", 1),
        _persisted_row("tool.started", 2, call_id="call-1", tool="read_file"),
        _persisted_row("tool.completed", 3, call_id="call-1", tool="read_file"),
        _persisted_row("run.completed", 4),
    ]))

    summary = summarize(path)

    assert summary["doneCount"] == 1
    assert summary["toolCallsStarted"] == ["call-1"]
    assert summary["toolCallsCompleted"] == ["call-1"]
    assert _judge_tool_pairing(_receipt(path))["passed"] is True


def test_same_tool_name_cannot_hide_a_mismatched_call_id(tmp_path):
    path = tmp_path / "events.json"
    path.write_text(json.dumps([
        _persisted_row("tool.started", 1, call_id="call-started", tool="bash"),
        _persisted_row("tool.completed", 2, call_id="call-other", tool="bash"),
        _persisted_row("run.completed", 3),
    ]))

    result = _judge_tool_pairing(_receipt(path))

    assert result["passed"] is False
    assert result["metrics"]["pairingBasis"] == "call_id"
    assert result["metrics"]["missing"] == {"call-started": 1}
