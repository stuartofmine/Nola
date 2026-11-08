#!/usr/bin/env python3
"""Run Nola's bounded architecture evaluation and emit JSON/Markdown evidence."""

from __future__ import annotations

import argparse
import json
import os
import platform
import re
import shlex
import signal
import subprocess
import sys
import time
from dataclasses import dataclass, asdict
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SUITE = ROOT / "scripts" / "fast_evaluation_suite.json"
DEFAULT_OUTPUT = ROOT / "logs" / "fast-evaluation"


@dataclass
class CaseResult:
    id: str
    category: str
    weight: float
    status: str
    elapsed_seconds: float
    peak_rss_bytes: int | None
    build_peak_rss_bytes: int | None
    command: list[str]
    output_tail: str


def _time_command(command: list[str]) -> tuple[list[str], str]:
    if not Path("/usr/bin/time").exists():
        return command, "none"
    if platform.system() == "Darwin":
        return ["/usr/bin/time", "-l", *command], "darwin"
    return ["/usr/bin/time", "-v", *command], "gnu"


def _peak_rss(stderr: str, flavor: str) -> int | None:
    if flavor == "darwin":
        match = re.search(r"^\s*(\d+)\s+maximum resident set size\s*$", stderr, re.MULTILINE)
        return int(match.group(1)) if match else None
    if flavor == "gnu":
        match = re.search(r"Maximum resident set size \(kbytes\):\s*(\d+)", stderr)
        return int(match.group(1)) * 1024 if match else None
    return None


def _tail(text: str, limit: int = 4000) -> str:
    return text[-limit:].strip()


def _test_binary(case_id: str) -> Path | None:
    candidates = [
        path for path in (ROOT / "target" / "debug" / "deps").glob(f"{case_id}-*")
        if path.is_file() and os.access(path, os.X_OK) and path.suffix != ".d"
    ]
    return max(candidates, key=lambda path: path.stat().st_mtime, default=None)


def _test_target(command: list[str]) -> str | None:
    try:
        index = command.index("--test")
        return command[index + 1]
    except (ValueError, IndexError):
        return None


def _reported_peak_rss(output: str) -> int | None:
    matches = re.findall(r"NOLA_PERF_PEAK_RSS_BYTES=(\d+)", output)
    return int(matches[-1]) if matches else None


def _case_metrics(case: dict[str, Any]) -> dict[str, Any]:
    raw_path = case.get("metrics_path")
    if not raw_path:
        return {}
    path = Path(str(raw_path))
    try:
        return json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        return {}


def _run_direct_test(binary: Path, timeout: float) -> tuple[int, float, int | None, str]:
    measured, flavor = _time_command([str(binary), "--quiet"])
    started = time.monotonic()
    completed = _run_bounded(
        measured,
        cwd=ROOT,
        timeout=timeout,
        env=os.environ.copy(),
    )
    elapsed = time.monotonic() - started
    output = "\n".join(part for part in [completed.stdout, completed.stderr] if part)
    return completed.returncode, elapsed, _peak_rss(completed.stderr, flavor), output


def _run_bounded(
    command: list[str],
    *,
    cwd: Path,
    timeout: float,
    env: dict[str, str],
) -> subprocess.CompletedProcess[str]:
    """Run one evaluation in its own process group and reap every child."""
    process = subprocess.Popen(
        command,
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env,
        start_new_session=True,
    )
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired as error:
        markers = _cleanup_markers(command)
        _terminate_process_tree(process.pid, signal.SIGTERM, markers)
        try:
            stdout, stderr = process.communicate(timeout=2)
        except subprocess.TimeoutExpired:
            _terminate_process_tree(process.pid, signal.SIGKILL, markers)
            stdout, stderr = process.communicate()
        _terminate_matching_processes(markers, signal.SIGKILL)
        prior_stdout = error.stdout.decode(errors="replace") if isinstance(error.stdout, bytes) else (error.stdout or "")
        prior_stderr = error.stderr.decode(errors="replace") if isinstance(error.stderr, bytes) else (error.stderr or "")
        raise subprocess.TimeoutExpired(
            command,
            timeout,
            output=prior_stdout + (stdout or ""),
            stderr=prior_stderr + (stderr or ""),
        ) from None
    return subprocess.CompletedProcess(command, process.returncode, stdout, stderr)


def _cleanup_markers(command: list[str]) -> list[str]:
    markers: list[str] = []
    for index, argument in enumerate(command[:-1]):
        if argument == "-derivedDataPath":
            markers.append(command[index + 1])
    return markers


def _process_rows() -> list[tuple[int, int, str]]:
    snapshot = subprocess.run(
        ["/bin/ps", "-axo", "pid=,ppid=,command="],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    rows: list[tuple[int, int, str]] = []
    for line in snapshot.stdout.splitlines():
        fields = line.strip().split(maxsplit=2)
        if len(fields) == 3 and fields[0].isdigit() and fields[1].isdigit():
            rows.append((int(fields[0]), int(fields[1]), fields[2]))
    return rows


def _descendant_pids(root_pid: int, rows: list[tuple[int, int, str]]) -> set[int]:
    children: dict[int, list[int]] = {}
    for pid, parent, _ in rows:
        children.setdefault(parent, []).append(pid)
    descendants: set[int] = set()
    pending = [root_pid]
    while pending:
        parent = pending.pop()
        for child in children.get(parent, []):
            if child not in descendants:
                descendants.add(child)
                pending.append(child)
    return descendants


def _signal_pids(pids: set[int], requested_signal: signal.Signals) -> None:
    for pid in sorted(pids, reverse=True):
        try:
            os.kill(pid, requested_signal)
        except (ProcessLookupError, PermissionError):
            pass


def _terminate_matching_processes(markers: list[str], requested_signal: signal.Signals) -> None:
    if not markers:
        return
    own_pid = os.getpid()
    matches = {
        pid for pid, _, command in _process_rows()
        if pid != own_pid and any(marker in command for marker in markers)
    }
    _signal_pids(matches, requested_signal)


def _terminate_process_tree(
    root_pid: int,
    requested_signal: signal.Signals,
    markers: list[str],
) -> None:
    rows = _process_rows()
    targets = _descendant_pids(root_pid, rows)
    targets.add(root_pid)
    targets.update(
        pid for pid, _, command in rows
        if any(marker in command for marker in markers)
    )
    _signal_pids(targets, requested_signal)
    try:
        os.killpg(root_pid, requested_signal)
    except (ProcessLookupError, PermissionError):
        pass


def run_case(case: dict[str, Any], remaining: float) -> CaseResult:
    command = [str(item) for item in case["command"]]
    timeout = max(1.0, min(float(case["timeout_seconds"]), remaining))
    measured, flavor = _time_command(command)
    working_directory = ROOT if case.get("cwd") == "root" else ROOT / "core"
    environment = {**os.environ, "CARGO_TERM_COLOR": "never"}
    metrics_path = Path(str(case["metrics_path"])) if case.get("metrics_path") else None
    if metrics_path:
        metrics_path.unlink(missing_ok=True)
    if case.get("cwd") == "root":
        environment.setdefault(
            "NOLA_AGENT_CARGO_TARGET_DIR",
            str(ROOT / "cli" / "target"),
        )
    started = time.monotonic()
    try:
        completed = _run_bounded(
            measured,
            cwd=working_directory,
            timeout=timeout,
            env=environment,
        )
        elapsed = time.monotonic() - started
        combined = "\n".join(part for part in [completed.stdout, completed.stderr] if part)
        status = "passed" if completed.returncode == 0 else "failed"
        build_peak = _peak_rss(completed.stderr, flavor)
        metrics = _case_metrics(case)
        runtime_peak = _reported_peak_rss(combined)
        if metrics.get("peak_rss_bytes") is not None:
            runtime_peak = int(metrics["peak_rss_bytes"])
            combined += "\nEvaluation metrics: " + json.dumps(metrics, sort_keys=True)
        if status == "passed" and command and command[0] == "cargo":
            target = _test_target(command)
            binary = _test_binary(target) if target else None
            if binary is None:
                status = "failed"
                combined += f"\nEvaluation integrity error: no test binary for {case['id']}"
            else:
                direct_timeout = max(1.0, timeout - elapsed)
                direct_code, direct_elapsed, runtime_peak, direct_output = _run_direct_test(
                    binary, direct_timeout
                )
                elapsed += direct_elapsed
                combined = "\n".join(part for part in [combined, direct_output] if part)
                if direct_code != 0:
                    status = "failed"
        return CaseResult(
            id=case["id"],
            category=case["category"],
            weight=float(case.get("weight", 1)),
            status=status,
            elapsed_seconds=round(elapsed, 3),
            peak_rss_bytes=runtime_peak,
            build_peak_rss_bytes=build_peak,
            command=command,
            output_tail=_tail(combined),
        )
    except subprocess.TimeoutExpired as error:
        elapsed = time.monotonic() - started
        output = "\n".join(
            part.decode(errors="replace") if isinstance(part, bytes) else (part or "")
            for part in [error.stdout, error.stderr]
        )
        return CaseResult(
            id=case["id"],
            category=case["category"],
            weight=float(case.get("weight", 1)),
            status="timed_out",
            elapsed_seconds=round(elapsed, 3),
            peak_rss_bytes=None,
            build_peak_rss_bytes=None,
            command=command,
            output_tail=_tail(output),
        )


def score(results: list[CaseResult], category: str) -> float | None:
    selected = [result for result in results if result.category == category]
    if not selected:
        return None
    total = sum(result.weight for result in selected)
    passed = sum(result.weight for result in selected if result.status == "passed")
    return round(100 * passed / total, 1)


def markdown(report: dict[str, Any]) -> str:
    lines = [
        "# Nola fast evaluation",
        "",
        f"- Status: **{report['status']}**",
        f"- Elapsed: **{report['elapsed_seconds']:.3f}s** / {report['budget_seconds']}s",
        f"- Reliability: **{report['scores'].get('reliability', 'n/a')}**",
        f"- Accuracy: **{report['scores'].get('accuracy', 'n/a')}**",
        f"- Performance: **{report['scores'].get('performance', 'n/a')}**",
        f"- Peak case RSS: **{report['peak_case_rss_bytes'] or 'unavailable'} bytes**",
        "",
        "| Case | Category | Status | Seconds | Runtime RSS | Build RSS |",
        "|---|---|---:|---:|---:|---:|",
    ]
    for result in report["cases"]:
        lines.append(
            f"| {result['id']} | {result['category']} | {result['status']} | "
            f"{result['elapsed_seconds']:.3f} | {result['peak_rss_bytes'] or 'n/a'} | "
            f"{result['build_peak_rss_bytes'] or 'n/a'} |"
        )
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--suite", type=Path, default=DEFAULT_SUITE)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--list", action="store_true")
    args = parser.parse_args()
    suite = json.loads(args.suite.read_text())
    if args.list:
        for case in suite["cases"]:
            print(f"{case['id']}: {shlex.join(case['command'])}")
        return 0

    budget = float(suite["budget_seconds"])
    started = time.monotonic()
    results: list[CaseResult] = []
    for case in suite["cases"]:
        remaining = budget - (time.monotonic() - started)
        if remaining <= 1:
            results.append(CaseResult(
                id=case["id"], category=case["category"],
                weight=float(case.get("weight", 1)), status="budget_exhausted",
                elapsed_seconds=0, peak_rss_bytes=None,
                build_peak_rss_bytes=None,
                command=case["command"], output_tail="Global evaluation budget exhausted",
            ))
            continue
        result = run_case(case, remaining)
        results.append(result)
        print(f"[{result.status:>16}] {result.id} {result.elapsed_seconds:.3f}s", flush=True)

    elapsed = time.monotonic() - started
    passed = all(result.status == "passed" for result in results)
    peak_values = [result.peak_rss_bytes for result in results if result.peak_rss_bytes is not None]
    build_peak_values = [
        result.build_peak_rss_bytes
        for result in results
        if result.build_peak_rss_bytes is not None
    ]
    report = {
        "schema_version": 1,
        "status": "passed" if passed and elapsed <= budget else "failed",
        "budget_seconds": budget,
        "elapsed_seconds": round(elapsed, 3),
        "scores": {
            category: score(results, category)
            for category in ["reliability", "accuracy", "performance"]
        },
        "peak_case_rss_bytes": max(peak_values) if peak_values else None,
        "peak_build_rss_bytes": max(build_peak_values) if build_peak_values else None,
        "cases": [asdict(result) for result in results],
    }
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "report.json").write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n")
    (args.output / "report.md").write_text(markdown(report))
    print(f"report: {args.output / 'report.md'}")
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
