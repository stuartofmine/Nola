import json
import sqlite3

import pytest

from judge_real_e2e_response import (
    bounded_tool_evidence,
    judge_provider,
    persisted_material,
    judgement_continuation_messages,
    request_parseable_verdict,
    request_judgement,
    request_judgement_with_retry,
    structured_event_evidence,
    transcript_material,
    verdict_from_response,
)


VERDICT = '{"pass":true,"scores":{"task_fulfillment":4},"critical_failures":[],"findings":[],"summary":"ok"}'


def test_judge_provider_honors_explicit_then_runtime_then_config_overrides() -> None:
    config = {"endpoint": "configured-url", "api_key": "fixture-key", "model": "configured-model"}
    assert judge_provider(config, {}) == config
    assert judge_provider(config, {
        "MODEL_NAME": "runtime-model", "NOLA_REAL_E2E_MODEL": "acceptance-model",
        "NOLA_JUDGE_MODEL": "independent-model", "NOLA_JUDGE_ENDPOINT": " ",
    }) == {**config, "model": "independent-model"}
    assert judge_provider(config, {"MODEL_NAME": "runtime-model"})["model"] == "runtime-model"


def test_persisted_judge_reads_canonical_parts_not_legacy_message_json(tmp_path) -> None:
    database = tmp_path / "fixture.db"
    with sqlite3.connect(database) as db:
        db.executescript("""
            CREATE TABLE messages(id TEXT, conversation_id TEXT, role TEXT, ordinal INTEGER);
            CREATE TABLE message_parts(message_id TEXT, kind TEXT, content_text TEXT, content_json TEXT, ordinal INTEGER);
            INSERT INTO messages VALUES('answer','session','assistant',1);
            INSERT INTO messages VALUES('foreign','other','assistant',1);
            INSERT INTO message_parts VALUES('answer','text','confirmed ',NULL,1);
            INSERT INTO message_parts VALUES('answer','text','result',NULL,3);
            INSERT INTO message_parts VALUES('foreign','text','private foreign content',NULL,1);
        """)
        db.execute("INSERT INTO message_parts VALUES(?,?,?,?,?)", (
            "answer", "tool", "", json.dumps({"call_id":"call-1", "capability_id":"read_file",
                "state":"completed", "input":{"path":"file.rs"}, "output_preview":"actual evidence"}), 2))
    answer, evidence = persisted_material(database, "session")
    assert answer == "confirmed result"
    assert "actual evidence" in evidence and "file.rs" in evidence
    assert "private foreign content" not in answer + evidence


def test_verdict_uses_standard_content() -> None:
    verdict, source = verdict_from_response({
        "choices": [{"message": {"content": VERDICT}}],
    })
    assert verdict["pass"] is True
    assert source == "content"


def test_verdict_accepts_parseable_reasoning_when_content_is_empty() -> None:
    verdict, source = verdict_from_response({
        "choices": [{"message": {"content": "", "reasoning_content": VERDICT}}],
    })
    assert verdict["pass"] is True
    assert source == "reasoning_content"


def test_verdict_prefers_structured_submit_verdict_call() -> None:
    verdict, source = verdict_from_response({
        "choices": [{"message": {"content": "", "tool_calls": [{
            "type": "function",
            "function": {"name": "submit_verdict", "arguments": VERDICT},
        }]}}],
    })
    assert verdict["pass"] is True
    assert source == "submit_verdict"


def test_verdict_rejects_unstructured_reasoning() -> None:
    with pytest.raises(ValueError, match="no parseable verdict"):
        verdict_from_response({
            "choices": [{"message": {"content": "", "reasoning_content": "still thinking"}}],
        })


def test_evidence_budget_represents_every_tool_result() -> None:
    material = [(f"tool-{index}", f"head-{index}-" + "x" * 2_000 + f"-tail-{index}") for index in range(12)]
    evidence = bounded_tool_evidence(material, budget=6_000)
    for index in range(12):
        assert f"[tool-{index}]" in evidence
        assert f"head-{index}" in evidence
        assert f"tail-{index}" in evidence
    assert len(evidence) < 7_500


def test_transcript_judge_selects_longest_delivery_and_its_user_task(tmp_path) -> None:
    transcript = tmp_path / "transcript.json"
    transcript.write_text(json.dumps([
        {"role": "user", "text": "read both files"},
        {"role": "assistant", "text": "The complete grounded result from both files."},
        {"role": "user", "text": "hello"},
        {"role": "assistant", "text": "Hi."},
    ]))
    task, answer = transcript_material(transcript)
    assert task == "read both files"
    assert answer == "The complete grounded result from both files."


def test_transcript_judge_rejects_orphan_assistant_output(tmp_path) -> None:
    transcript = tmp_path / "transcript.json"
    transcript.write_text(json.dumps([{"role": "assistant", "text": "orphan"}]))
    with pytest.raises(ValueError, match="no user task"):
        transcript_material(transcript)


def test_structured_event_evidence_keeps_tool_outputs_from_middle_of_stream() -> None:
    rows = [
        {"type": "context_usage", "used_tokens": 10},
        {
            "type": "core_part_updated",
            "part": {
                "kind": "tool",
                "call_id": "call-alpha",
                "capability_id": "read_file",
                "input": {"path": "alpha.txt"},
                "state": "completed",
                "output_preview": "alpha verification value: ALPHA-31415",
            },
        },
        {"type": "context_usage", "used_tokens": 20},
    ]
    evidence = structured_event_evidence("\n".join(json.dumps(row) for row in rows))
    assert "read_file" in evidence
    assert "alpha.txt" in evidence
    assert "ALPHA-31415" in evidence
    assert "context_usage" not in evidence


def test_streamed_judge_response_reassembles_content_and_reasoning(monkeypatch) -> None:
    class Response:
        def __enter__(self):
            return self

        def __exit__(self, *_args):
            return False

        def __iter__(self):
            events = [
                {"choices": [{"delta": {"reasoning_content": "checked"}}]},
                {"choices": [{"delta": {"content": VERDICT[:25]}}]},
                {"choices": [{"delta": {"content": VERDICT[25:]}}]},
            ]
            return iter(
                [f"data: {json.dumps(event)}\n".encode() for event in events]
                + [b"data: [DONE]\n"]
            )

    monkeypatch.setattr("urllib.request.urlopen", lambda *_args, **_kwargs: Response())
    response = request_judgement(
        "https://example.test/v1",
        "secret",
        {"model": "judge", "stream": True, "messages": []},
    )
    verdict, source = verdict_from_response(response)
    assert verdict["pass"] is True
    assert source == "content"
    assert response["choices"][0]["message"]["reasoning_content"] == "checked"


def test_streamed_judge_response_reassembles_tool_arguments(monkeypatch) -> None:
    class Response:
        def __enter__(self):
            return self

        def __exit__(self, *_args):
            return False

        def __iter__(self):
            events = [
                {"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"name": "submit_verdict", "arguments": VERDICT[:30]}}]}}]},
                {"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": VERDICT[30:]}}]}}]},
            ]
            return iter([f"data: {json.dumps(event)}\n".encode() for event in events] + [b"data: [DONE]\n"])

    monkeypatch.setattr("urllib.request.urlopen", lambda *_args, **_kwargs: Response())
    response = request_judgement(
        "https://example.test/v1",
        "secret",
        {"model": "judge", "stream": True, "messages": []},
    )
    verdict, source = verdict_from_response(response)
    assert verdict["pass"] is True
    assert source == "submit_verdict"


def test_transport_reset_retries_once(monkeypatch) -> None:
    calls = 0

    def request(*_args, **_kwargs):
        nonlocal calls
        calls += 1
        if calls == 1:
            raise ConnectionResetError("reset")
        return {"choices": [{"message": {"content": VERDICT}}]}

    monkeypatch.setattr("judge_real_e2e_response.request_judgement", request)
    response, attempts = request_judgement_with_retry(
        "https://example.test/v1",
        "secret",
        {"stream": True},
    )
    assert attempts == 2
    assert calls == 2
    assert response["choices"][0]["message"]["content"] == VERDICT


def test_thinking_continuation_passes_reasoning_back_to_provider() -> None:
    messages = [{"role": "user", "content": "judge this"}]
    continued = judgement_continuation_messages(
        messages,
        {"choices": [{"message": {"content": "", "reasoning_content": "private analysis"}}]},
    )
    assert continued[:1] == messages
    assert continued[1] == {
        "role": "assistant",
        "content": "",
        "reasoning_content": "private analysis",
    }
    assert continued[2]["role"] == "user"
    assert "submit_verdict" in continued[2]["content"]


def test_parseable_verdict_retries_two_truncated_tool_calls(monkeypatch) -> None:
    responses = [
        {"choices": [{"message": {"content": "", "tool_calls": [{
            "function": {"name": "submit_verdict", "arguments": '{"pass":true'},
        }]}}]},
        {"choices": [{"message": {"content": "", "tool_calls": [{
            "function": {"name": "submit_verdict", "arguments": '{"pass":true,"scores":'},
        }]}}]},
        {"choices": [{"message": {"content": VERDICT}}]},
    ]
    calls = []

    def request(_endpoint, _api_key, body):
        calls.append(body)
        return responses[len(calls) - 1], 1

    monkeypatch.setattr("judge_real_e2e_response.request_judgement_with_retry", request)
    verdict, source, attempts = request_parseable_verdict(
        "https://example.test/v1",
        "secret",
        {"stream": True, "messages": [{"role": "user", "content": "judge"}]},
    )
    assert verdict["pass"] is True
    assert source == "content"
    assert attempts == 3
    assert len(calls) == 3
