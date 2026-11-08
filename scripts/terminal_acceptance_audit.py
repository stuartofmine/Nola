#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Independent replay of current terminal acceptance observations.

Only the explicitly configured/repository-built validation example is executed;
an evidence document cannot select an executable. No ANSI text reconstruction.
"""
from __future__ import annotations
import base64
import json
from pathlib import Path
import re
import subprocess


def run_oracle(binary: Path, requests: list[dict]) -> list[dict]:
    if not binary.is_file():
        raise ValueError("current terminal VT100 validation example is unavailable")
    result = subprocess.run([str(binary)], input="".join(json.dumps(request) + "\n" for request in requests),
        text=True, capture_output=True, timeout=30, check=True)
    rows = [json.loads(line) for line in result.stdout.splitlines()]
    if len(rows) != len(requests) or any("error" in row for row in rows):
        raise ValueError("terminal oracle rejected the capture or typed receipt")
    return rows


def verify(capture: Path, frames_path: Path, actions_path: Path, metrics_path: Path,
           audit_path: Path, database: Path, oracle: Path) -> dict:
    for path in (capture, frames_path, actions_path, metrics_path, audit_path, database):
        if not path.is_file():
            raise ValueError("terminal observation file is missing")
    raw = capture.read_bytes()
    frames = [json.loads(line) for line in frames_path.read_text().splitlines()]
    actions = [json.loads(line) for line in actions_path.read_text().splitlines()]
    metrics, audit = json.loads(metrics_path.read_text()), json.loads(audit_path.read_text())
    if not raw or not frames or not actions or metrics.get("schemaVersion") != 2:
        raise ValueError("raw/committed frame/action observations are empty or obsolete")
    requests = [{"operation": "feed", "bytes_base64": base64.b64encode(raw[index:index + 65536]).decode()}
                for index in range(0, len(raw), 65536)]
    requests.append({"operation": "audit", "database": str(database.resolve()), "live": False})
    results = run_oracle(oracle, requests)
    replay = [frame for result in results[:-1] for frame in result["frames"]]
    keys = ("frameId", "rawOffset", "rows", "columns", "screen", "cursorRow", "cursorColumn")
    if [{key: frame[key] for key in keys} for frame in frames] != replay:
        raise ValueError("recorded frames disagree with standard VT100 replay")
    if not replay:
        raise ValueError("capture has no committed synchronized terminal frame")
    runs = results[-1]["runs"]
    kinds = ["answered", "cancelled", "answered"]
    if [run.get("kind") for run in runs] != kinds or any(run.get("status") != "sealed" for run in runs):
        raise ValueError("three exact typed sealed Run outcomes are required")
    if audit.get("formalServiceRead") is not True or audit.get("runs") != runs:
        raise ValueError("formal live Service receipt and frozen database disagree")
    if len({run["runId"] for run in runs}) != 3 or len({run["conversationId"] for run in runs}) != 1:
        raise ValueError("terminal turns do not share an exact canonical conversation")
    by_id = {frame["frameId"]: frame for frame in replay}
    if any(action.get("frameId") not in by_id for action in actions):
        raise ValueError("action references an unknown committed frame")
    times = [action["monotonicNs"] for action in actions]
    if times != sorted(times):
        raise ValueError("terminal action clock is not monotonic")

    def one(name):
        matching = [(index, action) for index, action in enumerate(actions) if action.get("action") == name]
        if len(matching) != 1:
            raise ValueError("missing/duplicate terminal lifecycle witness: " + name)
        return matching[0]

    long_at, long = one("long_input")
    up_at, up = one("page_up_witness")
    down_at, down = one("page_down_witness")
    cancel_at, cancel = one("cancel_input")
    escape_at, escape = one("escape")
    follow_at, follow = one("followup_input")
    seals = [(index, action) for index, action in enumerate(actions) if action.get("action") == "formal_seal"]
    if len(seals) != 3 or any(action.get("runs") != runs[:index + 1]
                              for index, (_, action) in enumerate(seals)):
        raise ValueError("formal seal timeline does not match frozen Run identities")
    if not (long_at < seals[0][0] < up_at < down_at < cancel_at < escape_at < seals[1][0]
            < follow_at < seals[2][0]):
        raise ValueError("follow-up input preceded formal cancellation/completion")
    if follow.get("priorRunId") != runs[1]["runId"]:
        raise ValueError("follow-up does not bind the cancelled Run")
    cancellation_ms = (seals[1][1]["monotonicNs"] - escape["monotonicNs"]) / 1_000_000
    if not 0 <= cancellation_ms <= 2000:
        raise ValueError("formal cancellation exceeded the 2000ms requirement")
    up_keys = [action for action in actions[seals[0][0]:up_at] if action.get("action") == "page_up"]
    down_keys = [action for action in actions[up_at:down_at] if action.get("action") == "page_down"]
    first = re.compile(r"(?m)^\s*SCROLL-01\s+\S")
    last = re.compile(r"(?m)^\s*SCROLL-40\s+\S")
    if not up_keys or not down_keys or not first.search(by_id[up["frameId"]]["screen"]) \
            or not last.search(by_id[down["frameId"]]["screen"]):
        raise ValueError("real PageUp/PageDown did not retain the document")
    long_frames = [frame for frame in replay if long["frameId"] < frame["frameId"] <= cancel["frameId"]]
    numbers = {int(number) for frame in long_frames for number in
        re.findall(r"(?m)^\s*SCROLL-(\d{2})\s+\S", frame["screen"])}
    initial = next((frame for frame in long_frames if first.search(frame["screen"])), None)
    if len(numbers) < 38 or not {1, 40}.issubset(numbers) or initial is None or last.search(initial["screen"]):
        raise ValueError("response prefix was skipped or retained history is incomplete")
    live_cancel = by_id[escape["frameId"]]["screen"]
    if "Esc interrupt" not in live_cancel or not re.search(r"(?m)^\s*CANCEL-BEGIN-741\s*$", live_cancel):
        raise ValueError("Escape did not interrupt visible live model output")
    active_frames = [frame for frame in replay if long["frameId"] <= frame["frameId"] <= seals[2][1]["frameId"]]
    if any(frame["screen"].count("Context") != 1 or frame["screen"].count("Type a message") > 1 for frame in active_frames):
        raise ValueError("composer/footer duplicated or disappeared from the viewport")
    # Final visual convergence can occur after the independent business seal.
    follow_frames = [frame for frame in replay if frame["frameId"] > follow["frameId"]]
    if not any(re.search(r"(?m)^\s*FOLLOWUP-OK-741\s*$", frame["screen"])
               and frame["screen"].count("Type a message") == 1
               and "Esc interrupt" not in frame["screen"] for frame in follow_frames):
        raise ValueError("ordinary post-cancellation input has no visible accepted answer")
    continued = {line for frame in replay if escape["frameId"] < frame["frameId"] <= follow["frameId"]
                 for line in re.findall(r"(?m)^\s*CANCEL-LINE-(\d+)\s*$", frame["screen"])}
    if (len(continued) > 20 or metrics.get("passed") is not True
            or metrics.get("sourceConfigurationModified") is not False
            or metrics.get("secretCleanupSucceeded") is not True
            or metrics.get("retainedEvidenceContainsSecret") is not False):
        raise ValueError("cancellation/source-isolation acceptance failed")
    return {"frames": len(replay), "visibleLongLines": len(numbers), "cancellationMs": cancellation_ms,
            "formalRunKinds": kinds, "pageUpDownVerified": True}
