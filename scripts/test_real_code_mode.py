#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Real LAN-model Code Mode dependency/parallel read acceptance, isolated state."""
import argparse
import json
import os
import re
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time
import uuid
from urllib.parse import urlsplit

sys.path.insert(0, str(Path(__file__).resolve().parent / "e2e"))
from e2e.real_overwrite_permission import isolated_environment, scrub_case_credentials
from real_e2e_events import summarize


def emits_computed_total(chunk, expected):
    """Accept an explicit scalar/total, not an incidental number in source data."""
    try:
        value = json.loads(chunk)
    except (ValueError, TypeError):
        return re.fullmatch(r"\s*(?:total|sum)\s*[:=]\s*" + str(expected) + r"\s*", chunk,
                            flags=re.IGNORECASE) is not None
    if type(value) in (int, float):
        return value == expected
    return isinstance(value, dict) and any(
        type(value.get(key)) in (int, float) and value[key] == expected
        for key in ("total", "sum"))


def model_cost_metrics(db):
    """Canonical completions only; progress copies must not double-count usage."""
    requests = {}
    for effect, raw in db.execute("SELECT effect_id, payload_json FROM effect_outbox_payloads"):
        payload = json.loads(raw)["payload"]
        if payload.get("type") == "model_stream":
            requests[effect] = payload.get("settings", {}).get("step_context", {}).get(
                "tool_projection", {}).get("schema_token_estimate")
    completions = []
    for effect, raw in db.execute("SELECT effect_id, payload_json FROM host_event_inbox ORDER BY received_at_ms, sequence"):
        payload = json.loads(raw)["payload"]
        if payload.get("type") != "model_completed":
            continue
        usage = payload.get("usage", {})
        completions.append({"effect_id": effect,
                            "schema_tokens_estimated": requests.get(effect),
                            "input_tokens": usage.get("input_tokens"),
                            "output_tokens": usage.get("output_tokens"),
                            "cached_input_tokens": usage.get("cached_input_tokens")
                            if usage.get("cached_input_tokens_reported") else None})
    totals = {}
    for key in ("input_tokens", "output_tokens", "cached_input_tokens", "schema_tokens_estimated"):
        values = [row[key] for row in completions]
        # Unknown is not zero; preserve missing provider measurements.
        totals[key] = sum(values) if values and all(type(v) is int for v in values) else None
    return {"completed_requests": len(completions), "totals": totals, "requests": completions}


def has_dynamic_cell_chain(parts, observations, child_names, expected):
    for observation in observations:
        if not any(emits_computed_total(chunk["text"], expected) for chunk in observation["output"]):
            continue
        children = [part for part in parts
                    if part.get("execution_source", {}).get("cell", {}).get("cell_id") == observation["cell_id"]
                    and part.get("state") == "completed" and part.get("capability_id") == "read_file"]
        by_name = {Path(part["input"]["path"]).name: part for part in children}
        if not {"index.json", *child_names}.issubset(by_name):
            continue
        parent = next((part for part in parts if part.get("call_id") == observation["parent_call_id"]), None)
        source = (parent or {}).get("input", {}).get("source", "")
        # Unpredictable child names must be obtained from the index at runtime,
        # not copied from a preceding model turn into a replacement cell.
        if "Promise.all" not in source or any(name in source for name in child_names):
            continue
        index_end = by_name["index.json"].get("finished_at_ms")
        if index_end is not None and all(
            by_name[name].get("started_at_ms", -1) >= index_end for name in child_names
        ):
            return True
    return False


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--config", type=Path, required=True)
    args = parser.parse_args()
    config = json.loads(args.config.read_text())
    llm = config["llm"]
    assert (urlsplit(llm["endpoint"]).hostname or "").startswith("192.168.")
    root = Path(tempfile.mkdtemp(prefix="nola-real-code-mode-"))
    print(f"Evidence: {root}", flush=True)
    failures = []
    child_names = [f"data-{uuid.uuid4().hex}.json" for _ in range(2)]
    for mode in ("code", "ordinary"):
        case = root / mode
        case.mkdir()
        env = isolated_environment(case, "allow_all")
        env["NOLA_LONG_TERM_MEMORY_ENABLED"] = "0"
        project = case / "project"
        project.mkdir()
        files = {"index.json": json.dumps({"files": child_names}),
                 child_names[0]: json.dumps({"values": [17, 23]}),
                 child_names[1]: json.dumps({"values": [31, 47]})}
        for name, body in files.items():
            (project / name).write_text(body)
        local = case / "home/.nola/config.json"
        local.parent.mkdir(parents=True, exist_ok=True)
        with os.fdopen(os.open(local, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as output:
            json.dump({"llm": llm, "network_policy": "allow"}, output)
        prompt = ("Read index.json, then read exactly the files named in its files array. "
                  "Sum all their values and report the total. Do not modify any files or use shell, network, or subagents. ")
        prompt += ("Use one code_mode_exec cell for the complete dependency chain: first read the index through a catalog file tool, then use Promise.all "
                   "for the dependent reads and compute the sum in JavaScript. Discover exact tool names and schemas "
                   "from ALL_TOOLS. Use text to return the sum; wait for the original cell if needed."
                   if mode == "code" else "Use ordinary file tools directly, not Code Mode.")
        started = time.monotonic()
        wall_started = time.time()
        try:
            with (case / "events.ndjson").open("w") as output, (case / "stderr.log").open("w") as error:
                result = subprocess.run([str(args.binary.resolve()), prompt, "--project", str(project),
                    "--allow-dir", str(project), "--session", mode, "--no-skills", "--json",
                    "--max-iterations", "10"], env=env, stdout=output, stderr=error, timeout=180)
            summary = summarize(case / "events.ndjson", case / "nola.db")
            with sqlite3.connect((case / "nola.db").as_uri() + "?mode=ro", uri=True) as db:
                parts = [json.loads(row[0]) for row in db.execute(
                    "SELECT content_json FROM message_parts WHERE kind='tool' AND content_json IS NOT NULL")]
                cells = [json.loads(row[0]) for row in db.execute(
                    "SELECT record_json FROM code_cell_observations")]
                cost_metrics = model_cost_metrics(db)
            nested = [part for part in parts if part.get("execution_source", {}).get("kind") == "code_cell"]
            events = [json.loads(line) for line in (case / "events.ndjson").read_text().splitlines()
                      if line.strip()]
            model_requests = {
                event["trace"]["span_id"] for event in events
                if event.get("trace", {}).get("kind") == "tool.progress"
                and event["trace"].get("payload", {}).get("summary") == "model_generation_started"
            }
            observations = [cell["observation"] for cell in cells]
            elapsed = time.monotonic() - started
            wall_elapsed = time.time() - wall_started
            report = {"exit_code": result.returncode, "elapsed_seconds": elapsed,
                      "wall_elapsed_seconds": wall_elapsed,
                      "wall_minus_monotonic_seconds": wall_elapsed - elapsed,
                      "model_requests": len(model_requests),
                      "model_cost": cost_metrics,
                      "nested_calls": len(nested), "tools": len(parts), "answer": summary["assistant"],
                      "cell_observations": observations,
                      "execution_errors": summary["errors"]}
            (case / "result.json").write_text(json.dumps(report, ensure_ascii=False, indent=2))
            print(json.dumps(report, ensure_ascii=False), flush=True)
            assert result.returncode == 0, "CLI failed; inspect stderr.log and execution_errors"
            assert "118" in summary["assistant"], "Actual answer omitted computed total"
            assert all((project / name).read_text() == body for name, body in files.items())
            assert len(nested) >= 3 if mode == "code" else not nested
            if mode == "code":
                emitted = [chunk["text"] for cell in cells
                           for chunk in cell["observation"]["output"]]
                assert any(emits_computed_total(chunk, 118) for chunk in emitted), "Cell never emitted the computed total"
                assert has_dynamic_cell_chain(parts, observations, child_names, 118), "No single cell dynamically read the index, dependent files and emitted the computed total"
                assert all(cell["observation"]["settled"] for cell in cells), "Unsettled cell remains"
            assert not summary["errors"], "Canonical execution contained errors"
        except (AssertionError, subprocess.TimeoutExpired) as error:
            failures.append({"mode": mode, "error": str(error) or "acceptance assertion failed"})
        finally:
            scrub_case_credentials(case)
    (root / "failures.json").write_text(json.dumps(failures, indent=2))
    assert not failures, failures


if __name__ == "__main__":
    main()
