#!/usr/bin/env python3
"""Run deterministic and real-model evaluations under one 10-minute budget."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / "scripts"
sys.path.insert(0, str(SCRIPTS))
from run_fast_evaluation import _run_bounded  # noqa: E402


def run(command: list[str], remaining: float) -> tuple[int, float]:
    started = time.monotonic()
    completed = _run_bounded(
        command,
        cwd=ROOT,
        timeout=max(1, remaining),
        env=os.environ.copy(),
    )
    sys.stdout.write(completed.stdout)
    sys.stderr.write(completed.stderr)
    return completed.returncode, time.monotonic() - started


def main() -> int:
    budget = 600.0
    started = time.monotonic()
    deterministic_code, _ = run(
        [sys.executable, str(SCRIPTS / "run_fast_evaluation.py")],
        min(480.0, budget),
    )
    remaining = budget - (time.monotonic() - started)
    if deterministic_code != 0 or remaining <= 1:
        return 1
    real_code, _ = run(
        [
            sys.executable,
            str(SCRIPTS / "run_real_model_evaluation.py"),
            "--budget-seconds",
            str(min(120.0, remaining)),
        ],
        remaining,
    )
    elapsed = time.monotonic() - started
    summary = {
        "status": "passed" if real_code == 0 and elapsed <= budget else "failed",
        "elapsed_seconds": round(elapsed, 3),
        "budget_seconds": budget,
    }
    output = ROOT / "logs" / "fast-evaluation" / "initial-evaluation.json"
    output.write_text(json.dumps(summary, indent=2) + "\n")
    print(f"initial evaluation: {summary['status']} in {summary['elapsed_seconds']}s")
    return 0 if summary["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
