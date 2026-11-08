#!/usr/bin/env python3
"""Bounded, secret-safe real-model accuracy and latency evaluation."""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import statistics
import time
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONFIG = Path.home() / ".nola" / "config.json"
DEFAULT_OUTPUT = ROOT / "logs" / "fast-evaluation"


CASES: list[dict[str, Any]] = [
    {
        "id": "exact_english",
        "messages": [{"role": "user", "content": "Reply with exactly NOLA-EVAL-42 and nothing else."}],
        "expected": "NOLA-EVAL-42",
    },
    {
        "id": "exact_chinese",
        "messages": [{"role": "user", "content": "请只回复：评测通过。不要解释，不要使用 Markdown。"}],
        "expected": "评测通过",
    },
    {
        "id": "arithmetic",
        "messages": [{"role": "user", "content": "Compute 138 * 47. Return only the integer, without punctuation or explanation."}],
        "expected": "6486",
    },
    {
        "id": "structured_tool_call",
        "messages": [{"role": "user", "content": "Use lookup_fixture_weather to obtain the weather for Hangzhou. Do not guess."}],
        "tools": [{
            "type": "function",
            "function": {
                "name": "lookup_fixture_weather",
                "description": "Return fixture weather for a city.",
                "parameters": {
                    "type": "object",
                    "properties": {"city": {"type": "string"}},
                    "required": ["city"],
                    "additionalProperties": False,
                },
            },
        }],
        "expected_tool": "lookup_fixture_weather",
        "expected_city": "Hangzhou",
    },
]


def endpoint_url(endpoint: str) -> str:
    value = endpoint.rstrip("/")
    if value.endswith("/chat/completions"):
        return value
    return value + "/chat/completions"


def normalized(value: str) -> str:
    return value.strip().strip("`\"'。.").strip()


def evaluate_case(
    case: dict[str, Any],
    *,
    endpoint: str,
    api_key: str,
    model: str,
    timeout: float,
) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "model": model,
        "messages": case["messages"],
        "stream": True,
        "temperature": 0,
        "max_tokens": 128,
    }
    if case.get("tools"):
        payload["tools"] = case["tools"]
        payload["tool_choice"] = "auto"
    request = urllib.request.Request(
        endpoint_url(endpoint),
        data=json.dumps(payload, ensure_ascii=False).encode(),
        headers={
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
            "Accept": "text/event-stream",
            "User-Agent": "NolaEvaluation/1",
        },
        method="POST",
    )
    started = time.monotonic()
    first_delta: float | None = None
    content: list[str] = []
    tool_calls: dict[int, dict[str, str]] = {}
    status = 0
    error: str | None = None
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            status = response.status
            for raw_line in response:
                line = raw_line.decode(errors="replace").strip()
                if not line.startswith("data:"):
                    continue
                data = line[5:].strip()
                if not data or data == "[DONE]":
                    continue
                frame = json.loads(data)
                choices = frame.get("choices") or []
                if not choices:
                    continue
                delta = choices[0].get("delta") or {}
                visible = delta.get("content") or ""
                if visible:
                    content.append(visible)
                    first_delta = first_delta or (time.monotonic() - started)
                for call in delta.get("tool_calls") or []:
                    index = int(call.get("index", 0))
                    aggregate = tool_calls.setdefault(index, {"name": "", "arguments": ""})
                    function = call.get("function") or {}
                    aggregate["name"] += function.get("name") or ""
                    aggregate["arguments"] += function.get("arguments") or ""
                    first_delta = first_delta or (time.monotonic() - started)
    except Exception as caught:
        error = f"{type(caught).__name__}: {caught}"
    elapsed = time.monotonic() - started
    answer = "".join(content)
    passed = False
    detail = ""
    if error is None and case.get("expected_tool"):
        call = next(iter(tool_calls.values()), {"name": "", "arguments": ""})
        try:
            arguments = json.loads(call["arguments"] or "{}")
        except json.JSONDecodeError:
            arguments = {}
        city = str(arguments.get("city", ""))
        passed = call["name"] == case["expected_tool"] and city.casefold() == case["expected_city"].casefold()
        detail = json.dumps({"tool": call["name"], "arguments": arguments}, ensure_ascii=False)
    elif error is None:
        passed = normalized(answer) == normalized(str(case["expected"]))
        detail = normalized(answer)[:240]
    return {
        "id": case["id"],
        "passed": passed,
        "http_status": status,
        "first_delta_seconds": round(first_delta, 3) if first_delta is not None else None,
        "elapsed_seconds": round(elapsed, 3),
        "detail": detail,
        "error": error,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", type=Path, default=DEFAULT_CONFIG)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--budget-seconds", type=float, default=120)
    args = parser.parse_args()
    config = json.loads(args.config.read_text()).get("llm", {})
    endpoint = str(config.get("endpoint", "")).strip()
    api_key = str(config.get("api_key", "")).strip()
    model = str(config.get("model", "")).strip()
    if not endpoint or not api_key or not model:
        raise SystemExit("Config must provide llm.endpoint, llm.api_key, and llm.model")

    started = time.monotonic()
    timeout = min(90.0, args.budget_seconds)
    results: list[dict[str, Any]] = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as executor:
        futures = [
            executor.submit(
                evaluate_case,
                case,
                endpoint=endpoint,
                api_key=api_key,
                model=model,
                timeout=timeout,
            )
            for case in CASES
        ]
        for future in concurrent.futures.as_completed(futures, timeout=args.budget_seconds):
            results.append(future.result())
    results.sort(key=lambda item: item["id"])
    elapsed = time.monotonic() - started
    passed_count = sum(bool(result["passed"]) for result in results)
    ttfts = [result["first_delta_seconds"] for result in results if result["first_delta_seconds"] is not None]
    report = {
        "schema_version": 1,
        "status": "passed" if passed_count == len(CASES) and elapsed <= args.budget_seconds else "failed",
        "model": model,
        "endpoint_host": urllib.parse.urlparse(endpoint).hostname,
        "elapsed_seconds": round(elapsed, 3),
        "budget_seconds": args.budget_seconds,
        "accuracy_percent": round(100 * passed_count / len(CASES), 1),
        "median_first_delta_seconds": round(statistics.median(ttfts), 3) if ttfts else None,
        "cases": results,
    }
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "real-model.json").write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n")
    lines = [
        "# Nola real-model evaluation",
        "",
        f"- Status: **{report['status']}**",
        f"- Accuracy: **{report['accuracy_percent']}%**",
        f"- Elapsed: **{report['elapsed_seconds']}s** / {report['budget_seconds']}s",
        f"- Median first delta: **{report['median_first_delta_seconds']}s**",
        "",
        "| Case | Status | First delta | Total |",
        "|---|---:|---:|---:|",
    ]
    for result in results:
        lines.append(
            f"| {result['id']} | {'passed' if result['passed'] else 'failed'} | "
            f"{result['first_delta_seconds'] or 'n/a'} | {result['elapsed_seconds']} |"
        )
    (args.output / "real-model.md").write_text("\n".join(lines) + "\n")
    print(json.dumps({
        "status": report["status"],
        "accuracy_percent": report["accuracy_percent"],
        "elapsed_seconds": report["elapsed_seconds"],
        "median_first_delta_seconds": report["median_first_delta_seconds"],
    }, ensure_ascii=False))
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
