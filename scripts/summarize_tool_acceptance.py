#!/usr/bin/env python3
"""Read preserved acceptance evidence; print aggregate metrics without content."""
import argparse
from collections import Counter
import json
import math
from pathlib import Path
import sqlite3


def distribution(values):
    values = sorted(values)
    if not values:
        return {"samples": 0, "p50": None, "p95": None}
    return {"samples": len(values),
            "p50": values[math.ceil(len(values) * .50) - 1],
            "p95": values[math.ceil(len(values) * .95) - 1]}


def summarize(root):
    latencies, exposed_counts, outcomes = [], [], Counter()
    discovery_us, schema_bytes, output_bytes = [], [], []
    request_schema_bytes, request_schema_estimates, read_bytes = [], [], []
    duplicate_calls = 0
    comparable_calls = 0
    rejected_repeats = 0
    classified_results = 0
    turns = []
    for turn in sorted(root.glob("turn-*")):
        seen_calls = set()
        seen_signatures = set()
        terminal = []
        providers = turn / "provider.ndjson"
        if providers.exists():
            for line in providers.read_text().splitlines():
                row = json.loads(line)
                # Capability probes (/models) have no prompt or tool schemas.
                if not row.get("requestInputPresent"):
                    continue
                elapsed = row.get("durationMilliseconds")
                if type(elapsed) in (int, float) and elapsed >= 0:
                    latencies.append(elapsed)
                if isinstance(row.get("tools"), list):
                    exposed_counts.append(len(row["tools"]))
                size = row.get("toolSchemaUtf8Bytes")
                if type(size) is int and size >= 0:
                    request_schema_bytes.append(size)
                estimate = row.get("toolSchemaEstimatedTokens")
                if row.get("toolSchemaTokenMethod") == "ascii4_nonascii1_json" and type(estimate) is int and estimate >= 0:
                    request_schema_estimates.append(estimate)
        events = turn / "events.ndjson"
        if events.exists():
            for line in events.read_text().splitlines():
                trace = json.loads(line).get("trace", {})
                payload = trace.get("payload", {})
                if trace.get("kind") in ("run.completed", "run.failed", "run.cancelled"):
                    terminal.append({"kind": trace["kind"], "status": trace.get("status")})
                if trace.get("kind") != "tool.completed":
                    continue
                call_id = payload.get("call_id")
                if not call_id or call_id in seen_calls:
                    continue
                seen_calls.add(call_id)
                outcomes[trace.get("status", "unknown")] += 1
                try:
                    detail = json.loads(payload.get("detail", ""))
                except (ValueError, TypeError):
                    detail = None
                if isinstance(detail, dict) and detail.get("tool_execution_skipped") is True:
                    classified_results += 1
                    if trace.get("status") == "failed" and detail.get("error") == "repeated_tool_batch":
                        rejected_repeats += 1
                # Missing arguments are not an empty invocation. UI previews can
                # omit them; never classify two unknown payloads as a retry.
                if isinstance(payload.get("tool_name"), str) and isinstance(payload.get("arguments"), dict):
                    comparable_calls += 1
                    signature = json.dumps([payload["tool_name"], payload["arguments"]], sort_keys=True)
                    duplicate_calls += signature in seen_signatures
                    seen_signatures.add(signature)
        turns.append({"turn": turn.name, "terminal": terminal})
    database = root / "fixture.db"
    if database.is_file():
        # Host facts preserve numeric metrics omitted from UI trace previews.
        # Read only; never create or upgrade a user's database to obtain them.
        with sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True) as db:
            table = db.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='host_event_inbox'").fetchone()
            if table:
                seen = set()
                for (raw,) in db.execute("SELECT payload_json FROM host_event_inbox"):
                    event = json.loads(raw)
                    payload = event.get("payload", {})
                    call_id = payload.get("call_id")
                    key = (event.get("turn_id"), call_id)
                    if payload.get("type") != "tool_completed" or not call_id or key in seen:
                        continue
                    seen.add(key)
                    data = payload.get("output", {}).get("data") or {}
                    if isinstance(data, dict) and data.get("type") == "tool_output_slice":
                        count = data.get("bytes_returned")
                        if type(count) is int and count >= 0 and isinstance(data.get("content"), str) and len(data["content"].encode("utf-8")) == count:
                            read_bytes.append(count)
                    metrics = data.get("tool_discovery_metrics", {}) if isinstance(data, dict) else {}
                    for field, values in [("duration_us", discovery_us), ("returned_schema_utf8_bytes", schema_bytes),
                                          ("output_utf8_bytes", output_bytes)]:
                        value = metrics.get(field)
                        if type(value) in (int, float) and value >= 0:
                            values.append(value)
    known = outcomes["succeeded"] + outcomes["failed"]
    return {"schemaVersion": 1, "turns": turns,
            "providerRequestLatencyMs": distribution(latencies),
            "exposedToolCount": distribution(exposed_counts),
            "discoveryLatencyUs": distribution(discovery_us),
            "discoveredSchemaUtf8Bytes": distribution(schema_bytes),
            "discoveryOutputUtf8Bytes": distribution(output_bytes),
            "requestToolSchemaUtf8Bytes": distribution(request_schema_bytes),
            "requestToolSchemaEstimatedTokens": {"method":"ascii4_nonascii1_json", **distribution(request_schema_estimates)},
            "outputReadReturnedBytes": {"total": sum(read_bytes) if read_bytes else None, **distribution(read_bytes)},
            "toolOutcomes": dict(outcomes),
            "toolFailureRateKnownOutcomes": outcomes["failed"] / known if known else None,
            "sameArgumentRepeatCount": duplicate_calls if comparable_calls else None,
            "sameArgumentComparableCalls": comparable_calls,
            "reportedNoProgressRejections": {"count": rejected_repeats if classified_results else None,
                                             "structuredSkippedResults": classified_results},
            "invalidRetryCount": None, "toolSchemaTokens": None,
            "limitations": ["Repeats are not necessarily invalid retries.",
                            "Reported no-progress rejections are a subset, not all invalid retries; missing structured detail is unknown.",
                            "Request latency includes generation; it is not discovery latency.",
                            "Missing tokenizer or retry classification is unknown, not zero.",
                            "Schema bytes are not tokens; discovery output bytes are not recalled output bytes.",
                            "Schema token estimates exclude provider framing and are not billed token counts.",
                            "Terminal events do not prove task postconditions."]}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("evidence", type=Path)
    args = parser.parse_args()
    print(json.dumps(summarize(args.evidence), ensure_ascii=False, indent=2))
