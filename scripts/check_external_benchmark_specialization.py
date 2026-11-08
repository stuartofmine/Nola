#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Reject pinned external-benchmark literals from Nola product sources."""

from __future__ import annotations

import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BENCHMARK_ROOT = ROOT / "evals" / "benchmarks"
PRODUCTION_ROOTS = (
    ROOT / "core" / "src",
    ROOT / "cli" / "src",
    ROOT / "Nola",
    ROOT / "NolaMac",
    ROOT / "android" / "app" / "src" / "main",
    ROOT / "web" / "app",
    ROOT / "backend" / "worker" / "src",
) + tuple((ROOT / "core").glob("*/src"))


def _manifest(name: str) -> dict:
    return json.loads(
        (BENCHMARK_ROOT / name / "benchmark.json").read_text(encoding="utf-8")
    )


def benchmark_literals() -> set[str]:
    longmem = _manifest("longmemeval")
    terminal = _manifest("terminal-bench")
    literals = {"evals/benchmarks"}
    literals.update(str(case["id"]) for case in longmem["cases"] + terminal["cases"])
    for entry in longmem["cases"]:
        case = json.loads(
            (BENCHMARK_ROOT / "longmemeval" / entry["source_path"]).read_text(
                encoding="utf-8"
            )
        )
        for key in ("question", "answer"):
            value = str(case.get(key) or "").strip()
            if len(value) >= 32:
                literals.add(value)
    for entry in terminal["cases"]:
        task = BENCHMARK_ROOT / "terminal-bench" / entry["source_path"]
        instruction = (task / "instruction.md").read_text(encoding="utf-8").strip()
        if len(instruction) >= 32:
            literals.add(instruction)
        for line in instruction.splitlines():
            if len(line.strip()) >= 120:
                literals.add(line.strip())
    return literals


def assert_no_benchmark_specialization() -> None:
    literals = benchmark_literals()
    violations: list[str] = []
    seen: set[Path] = set()
    for source_root in PRODUCTION_ROOTS:
        if not source_root.exists():
            continue
        for path in source_root.rglob("*"):
            if path in seen or not path.is_file():
                continue
            seen.add(path)
            try:
                source = path.read_text(encoding="utf-8")
            except (OSError, UnicodeError):
                continue
            match = next((literal for literal in literals if literal in source), None)
            if match:
                violations.append(f"{path.relative_to(ROOT)} contains {match[:80]!r}")
    if violations:
        raise RuntimeError(
            "benchmark specialization detected in product sources:\n"
            + "\n".join(violations)
        )


def main() -> int:
    assert_no_benchmark_specialization()
    print(f"anti-specialization: passed ({len(benchmark_literals())} literals)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
