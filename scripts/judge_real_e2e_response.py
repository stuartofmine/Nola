#!/usr/bin/env python3
"""Use a real model to judge a real persisted agent response against evidence."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import sqlite3
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any


def judge_provider(llm: dict[str, Any], environment: dict[str, str]) -> dict[str, str]:
    fields = {
        "endpoint": ("NOLA_JUDGE_ENDPOINT", "NOLA_REAL_E2E_ENDPOINT", "MODEL_API_URL"),
        "api_key": ("NOLA_JUDGE_API_KEY", "NOLA_REAL_E2E_API_KEY", "MODEL_API_KEY"),
        "model": ("NOLA_JUDGE_MODEL", "NOLA_REAL_E2E_MODEL", "MODEL_NAME"),
    }
    return {
        field: next((environment[name].strip() for name in names if environment.get(name, "").strip()),
                    str(llm.get(field) or "").strip())
        for field, names in fields.items()
    }


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", required=True, type=Path)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--db", type=Path)
    source.add_argument("--transcript", type=Path)
    parser.add_argument("--session-id")
    parser.add_argument("--task")
    parser.add_argument("--evidence-file", action="append", default=[], type=Path)
    parser.add_argument("--receipt", required=True, type=Path)
    parser.add_argument("--current-date", default=dt.date.today().isoformat())
    parser.add_argument("--minimum-score", type=float, default=3.0)
    return parser.parse_args()


def object_value(raw: str) -> dict[str, Any]:
    value = json.loads(raw)
    if not isinstance(value, dict):
        raise ValueError("expected JSON object")
    return value


def endpoint_url(base: str) -> str:
    normalized = base.rstrip("/")
    if normalized.endswith("/chat/completions"):
        return normalized
    return normalized + "/chat/completions"


def extract_json_object(text: str) -> dict[str, Any]:
    stripped = text.strip()
    if stripped.startswith("```"):
        lines = stripped.splitlines()
        if lines and lines[0].startswith("```"):
            lines = lines[1:]
        if lines and lines[-1].strip() == "```":
            lines = lines[:-1]
        stripped = "\n".join(lines).strip()
    try:
        return object_value(stripped)
    except (json.JSONDecodeError, ValueError):
        start = stripped.find("{")
        end = stripped.rfind("}")
        if start < 0 or end <= start:
            raise
        return object_value(stripped[start : end + 1])


def verdict_from_response(response_value: dict[str, Any]) -> tuple[dict[str, Any], str]:
    choices = response_value.get("choices")
    if not isinstance(choices, list) or not choices:
        raise ValueError("judge response has no choices")
    message = choices[0].get("message") if isinstance(choices[0], dict) else None
    if not isinstance(message, dict):
        raise ValueError("judge response has no message")
    errors: list[str] = []
    tool_calls = message.get("tool_calls")
    if isinstance(tool_calls, list):
        for tool_call in tool_calls:
            function = tool_call.get("function") if isinstance(tool_call, dict) else None
            if not isinstance(function, dict) or function.get("name") != "submit_verdict":
                continue
            arguments = function.get("arguments")
            try:
                verdict = arguments if isinstance(arguments, dict) else extract_json_object(str(arguments or ""))
            except (json.JSONDecodeError, ValueError) as error:
                errors.append(f"submit_verdict: {error}")
                continue
            if isinstance(verdict.get("scores"), dict) and isinstance(verdict.get("pass"), bool):
                return verdict, "submit_verdict"
            errors.append("submit_verdict: missing verdict fields")
    for field in ("content", "reasoning_content"):
        value = message.get(field)
        if not isinstance(value, str) or not value.strip():
            continue
        try:
            verdict = extract_json_object(value)
        except (json.JSONDecodeError, ValueError) as error:
            errors.append(f"{field}: {error}")
            continue
        if isinstance(verdict.get("scores"), dict) and isinstance(verdict.get("pass"), bool):
            return verdict, field
        errors.append(f"{field}: missing verdict fields")
    detail = "; ".join(errors) if errors else "content and reasoning_content are empty"
    raise ValueError(f"judge response has no parseable verdict: {detail}")


def request_judgement(endpoint: str, api_key: str, body: dict[str, Any]) -> dict[str, Any]:
    request = urllib.request.Request(
        endpoint_url(endpoint),
        data=json.dumps(body, ensure_ascii=False).encode("utf-8"),
        method="POST",
        headers={
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=180) as response:
            if body.get("stream") is not True:
                return object_value(response.read().decode("utf-8"))
            content: list[str] = []
            reasoning: list[str] = []
            tool_calls: dict[int, dict[str, str]] = {}
            raw_payload: list[str] = []
            for raw_line in response:
                line = raw_line.decode("utf-8", errors="replace").strip()
                if not line:
                    continue
                if not line.startswith("data:"):
                    raw_payload.append(line)
                    continue
                payload = line[5:].strip()
                if payload == "[DONE]":
                    break
                try:
                    event = object_value(payload)
                except (json.JSONDecodeError, ValueError):
                    continue
                choices = event.get("choices")
                if not isinstance(choices, list) or not choices:
                    continue
                choice = choices[0] if isinstance(choices[0], dict) else {}
                delta = choice.get("delta")
                if not isinstance(delta, dict):
                    delta = choice.get("message")
                if not isinstance(delta, dict):
                    continue
                if isinstance(delta.get("content"), str):
                    content.append(delta["content"])
                if isinstance(delta.get("reasoning_content"), str):
                    reasoning.append(delta["reasoning_content"])
                for tool_call in delta.get("tool_calls", []) if isinstance(delta.get("tool_calls"), list) else []:
                    if not isinstance(tool_call, dict):
                        continue
                    index = int(tool_call.get("index", 0))
                    accumulated = tool_calls.setdefault(index, {"name": "", "arguments": ""})
                    function = tool_call.get("function")
                    if isinstance(function, dict):
                        if isinstance(function.get("name"), str):
                            accumulated["name"] += function["name"]
                        if isinstance(function.get("arguments"), str):
                            accumulated["arguments"] += function["arguments"]
            if content or reasoning or tool_calls:
                return {
                    "choices": [
                        {
                            "message": {
                                "content": "".join(content),
                                "reasoning_content": "".join(reasoning),
                                "tool_calls": [
                                    {
                                        "type": "function",
                                        "function": {
                                            "name": value["name"],
                                            "arguments": value["arguments"],
                                        },
                                    }
                                    for _, value in sorted(tool_calls.items())
                                ],
                            }
                        }
                    ]
                }
            if raw_payload:
                return object_value("\n".join(raw_payload))
            raise ValueError("judge stream completed without content")
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", errors="replace")[:1000]
        raise RuntimeError(f"judge request failed with HTTP {error.code}: {detail}") from error


def request_judgement_with_retry(
    endpoint: str,
    api_key: str,
    body: dict[str, Any],
    maximum_attempts: int = 2,
) -> tuple[dict[str, Any], int]:
    last_error: BaseException | None = None
    for attempt in range(1, max(1, maximum_attempts) + 1):
        try:
            return request_judgement(endpoint, api_key, body), attempt
        except urllib.error.HTTPError:
            raise
        except (urllib.error.URLError, ConnectionError, TimeoutError, OSError) as error:
            last_error = error
    assert last_error is not None
    raise last_error


def judgement_continuation_messages(
    messages: list[dict[str, Any]], response_value: dict[str, Any]
) -> list[dict[str, Any]]:
    choices = response_value.get("choices")
    message = choices[0].get("message") if isinstance(choices, list) and choices and isinstance(choices[0], dict) else None
    if not isinstance(message, dict):
        return messages
    reasoning = message.get("reasoning_content")
    if not isinstance(reasoning, str) or not reasoning:
        return messages
    assistant = {
        "role": "assistant",
        "content": message.get("content") if isinstance(message.get("content"), str) else "",
        "reasoning_content": reasoning,
    }
    return messages + [
        assistant,
        {
            "role": "user",
            "content": "Continue from that completed reasoning and call submit_verdict exactly once now. Do not restart the evaluation.",
        },
    ]


def request_parseable_verdict(
    endpoint: str,
    api_key: str,
    body: dict[str, Any],
    *,
    maximum_verdict_attempts: int = 3,
) -> tuple[dict[str, Any], str, int]:
    current_body = dict(body)
    original_messages = list(body.get("messages", []))
    total_transport_attempts = 0
    parse_errors: list[str] = []
    for verdict_attempt in range(1, max(1, maximum_verdict_attempts) + 1):
        response_value, transport_attempts = request_judgement_with_retry(
            endpoint, api_key, current_body
        )
        total_transport_attempts += transport_attempts
        try:
            verdict, source = verdict_from_response(response_value)
            return verdict, source, total_transport_attempts
        except ValueError as error:
            parse_errors.append(f"attempt {verdict_attempt}: {error}")
            if verdict_attempt >= maximum_verdict_attempts:
                break
            continued = judgement_continuation_messages(original_messages, response_value)
            if continued == original_messages:
                continued = [
                    *original_messages,
                    {
                        "role": "user",
                        "content": (
                            "The previous verdict was incomplete. Re-evaluate from the supplied "
                            "evidence and call submit_verdict exactly once with complete JSON "
                            "arguments. Return no prose."
                        ),
                    },
                ]
            current_body = dict(body)
            current_body["messages"] = continued
    raise ValueError("judge produced no complete verdict after bounded retries: " + " | ".join(parse_errors))


def persisted_material(db: Path, session_id: str) -> tuple[str, str]:
    connection = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    rows = connection.execute(
        """SELECT m.id, m.role, p.kind, p.content_text, p.content_json
           FROM messages m JOIN message_parts p ON p.message_id = m.id
           WHERE m.conversation_id = ?
           ORDER BY m.ordinal, p.ordinal""",
        (session_id,),
    ).fetchall()
    assistant: dict[str, str] = {}
    tool_material: list[tuple[str, str]] = []
    for message_id, role, kind, text, raw in rows:
        if role == "assistant" and kind == "text" and text:
            assistant[message_id] = assistant.get(message_id, "") + text
        elif kind == "tool":
            value = object_value(raw)
            evidence = {key: value.get(key) for key in
                        ("call_id", "capability_id", "state", "input", "output_preview", "full_output_ref")}
            tool_material.append((str(value.get("capability_id") or "tool"),
                                  json.dumps(evidence, ensure_ascii=False)))
    connection.close()
    answer = max(assistant.values(), key=len, default="")
    return answer, bounded_tool_evidence(tool_material)


def transcript_material(path: Path) -> tuple[str, str]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, list):
        raise ValueError("transcript must be a JSON array")
    messages = [item for item in value if isinstance(item, dict)]
    candidates: list[tuple[int, str, str]] = []
    for index, item in enumerate(messages):
        role = str(item.get("role") or "").split(".")[-1].lower()
        answer = str(item.get("text") or item.get("content") or "").strip()
        if role != "assistant" or not answer:
            continue
        task = ""
        for previous in reversed(messages[:index]):
            previous_role = str(previous.get("role") or "").split(".")[-1].lower()
            if previous_role == "user":
                task = str(previous.get("text") or previous.get("content") or "").strip()
                break
        if task:
            candidates.append((len(answer), task, answer))
    if not candidates:
        raise ValueError("transcript has no user task followed by a nonempty assistant answer")
    _, task, answer = max(candidates, key=lambda item: item[0])
    return task, answer


def bounded_file_evidence(paths: list[Path], budget: int = 12_000) -> str:
    material: list[tuple[str, str]] = []
    for path in paths:
        try:
            content = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        structured = structured_event_evidence(content)
        if structured:
            content = structured
        if content.strip():
            material.append((path.name, content))
    return bounded_tool_evidence(material, budget=budget)


def structured_event_evidence(content: str) -> str:
    """Extract final typed tool inputs and outputs from a canonical NDJSON stream."""
    tools: dict[str, dict[str, Any]] = {}
    parsed = 0
    for line in content.splitlines():
        if not line.strip():
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(row, dict):
            continue
        parsed += 1
        part = row.get("part")
        if not isinstance(part, dict):
            update = row.get("update")
            part = update.get("part") if isinstance(update, dict) else None
        if not isinstance(part, dict) or part.get("kind") != "tool":
            continue
        call_id = str(part.get("call_id") or part.get("id") or "").strip()
        if not call_id:
            continue
        candidate = {
            "tool": part.get("capability_id") or part.get("title"),
            "input": part.get("input"),
            "state": part.get("state"),
            "output": part.get("structured_output") or part.get("output_preview"),
            "error": part.get("error"),
        }
        previous = tools.get(call_id)
        previous_output = previous.get("output") if isinstance(previous, dict) else None
        if previous is None or candidate["output"] not in (None, "") or previous_output in (None, ""):
            tools[call_id] = candidate
    if parsed == 0 or not tools:
        return ""
    return json.dumps(
        {"canonical_tool_calls": list(tools.values())},
        ensure_ascii=False,
        indent=2,
    )


def bounded_tool_evidence(
    tool_material: list[tuple[str, str]],
    budget: int = 12_000,
) -> str:
    if not tool_material or budget <= 0:
        return ""
    share = max(400, min(8_000, budget // len(tool_material)))
    evidence: list[str] = []
    for name, content in tool_material:
        if len(content) <= share:
            excerpt = content
        else:
            head = max(1, share // 2)
            tail = max(1, share - head)
            excerpt = content[:head] + "\n...[middle omitted by judge]...\n" + content[-tail:]
        evidence.append(f"[{name}]\n{excerpt}")
    return "\n\n".join(evidence)


def main() -> int:
    args = arguments()
    config = object_value(args.config.read_text(encoding="utf-8"))
    llm = config.get("llm")
    if not isinstance(llm, dict):
        raise RuntimeError("config.llm is required")
    provider = judge_provider(llm, dict(os.environ))
    endpoint = provider["endpoint"]
    api_key = provider["api_key"]
    model = provider["model"]
    if not endpoint or not api_key or not model:
        raise RuntimeError("llm endpoint, api_key, and model are required")

    if args.db is not None:
        if not args.session_id:
            raise RuntimeError("--session-id is required with --db")
        answer, evidence = persisted_material(args.db, args.session_id)
        task = str(args.task or "").strip()
        session_id = args.session_id
    else:
        assert args.transcript is not None
        derived_task, answer = transcript_material(args.transcript)
        task = str(args.task or derived_task).strip()
        evidence = bounded_file_evidence(args.evidence_file)
        session_id = args.transcript.stem
    if not task:
        raise RuntimeError("no user task to judge")
    if not answer:
        raise RuntimeError("no persisted assistant answer to judge")
    system = """You are an independent end-to-end quality judge. Evaluate only the supplied user task, final answer, current date, and actual tool evidence. Do not reward confident prose. A claim is supported only when the evidence establishes it. For time-sensitive recommendations, distinguish confirmed-current, uncertain, and expired items. Recommending an uncertain or expired item as currently available is a critical failure. When the task asks only for items satisfying a filter, listing rejected or ineligible alternatives is a task-fulfillment defect even if they appear in a separate section. Treat leaked private planning or meta narration in the final answer as a quality defect. Submit exactly one verdict through the submit_verdict tool with this shape:
{"pass":boolean,"scores":{"task_fulfillment":number,"evidence_grounding":number,"temporal_correctness":number,"language_and_presentation":number},"critical_failures":[string],"findings":[string],"summary":string}
Scores range from 0 to 4. Pass only if every score is at least 3 and critical_failures is empty."""
    system += " For code/design audits, a failed search for a guessed identifier does not establish missing functionality. Missing assertions in one test harness do not prove that production validation or repository-wide coverage is missing. Asynchronous code alone does not establish a race. Labelling an unsupported allegation 'static risk', 'possible', or 'coverage gap' does not ground it. Require a concrete relevant condition and consequence, or an honest statement that the area remains unassessed. Judge evidence may be truncated: do not invent missing support or award a pass based solely on precise-looking paths and line numbers."
    user = json.dumps(
        {
            "current_date": args.current_date,
            "task": task,
            "final_answer": answer,
            "tool_evidence": evidence,
        },
        ensure_ascii=False,
    )
    request_body = {
        "model": model,
        "stream": True,
        "temperature": 0,
        "max_tokens": 4000,
        "enable_thinking": False,
        "chat_template_kwargs": {"enable_thinking": False},
        "tools": [
            {
                "type": "function",
                "function": {
                    "name": "submit_verdict",
                    "description": "Submit the complete independent quality verdict.",
                    "parameters": {
                        "type": "object",
                        "additionalProperties": False,
                        "required": ["pass", "scores", "critical_failures", "findings", "summary"],
                        "properties": {
                            "pass": {"type": "boolean"},
                            "scores": {
                                "type": "object",
                                "additionalProperties": False,
                                "required": ["task_fulfillment", "evidence_grounding", "temporal_correctness", "language_and_presentation"],
                                "properties": {
                                    "task_fulfillment": {"type": "number", "minimum": 0, "maximum": 4},
                                    "evidence_grounding": {"type": "number", "minimum": 0, "maximum": 4},
                                    "temporal_correctness": {"type": "number", "minimum": 0, "maximum": 4},
                                    "language_and_presentation": {"type": "number", "minimum": 0, "maximum": 4},
                                },
                            },
                            "critical_failures": {"type": "array", "items": {"type": "string"}},
                            "findings": {"type": "array", "items": {"type": "string"}},
                            "summary": {"type": "string"},
                        },
                    },
                },
            }
        ],
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
    }
    verdict, verdict_source, attempts = request_parseable_verdict(
        endpoint, api_key, request_body
    )
    scores = verdict.get("scores") if isinstance(verdict.get("scores"), dict) else {}
    score_values = [
        float(scores.get(name, 0))
        for name in (
            "task_fulfillment",
            "evidence_grounding",
            "temporal_correctness",
            "language_and_presentation",
        )
    ]
    critical = verdict.get("critical_failures")
    critical_failures = critical if isinstance(critical, list) else ["invalid critical_failures"]
    passed = (
        verdict.get("pass") is True
        and all(score >= args.minimum_score for score in score_values)
        and not critical_failures
    )
    receipt = {
        "schema_version": 1,
        "kind": "real_model_response_judgement",
        "session_id": session_id,
        "judge_model": model,
        "judge_attempts": attempts,
        "verdict_source": verdict_source,
        "passed": passed,
        "verdict": verdict,
        "metrics": {
            "answer_characters": len(answer),
            "evidence_characters": len(evidence),
            "minimum_score": args.minimum_score,
        },
    }
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    args.receipt.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(receipt, ensure_ascii=False, indent=2))
    return 0 if passed else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"real response judge failed: {error}", file=sys.stderr)
        raise SystemExit(2)
