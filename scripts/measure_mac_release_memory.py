#!/usr/bin/env python3
"""Measure standalone Nola Release processes without XCTest or build RSS."""

from __future__ import annotations

import argparse
import json
import os
import re
import signal
import subprocess
import time
from pathlib import Path


UNITS = {"KB": 1024, "MB": 1024**2, "GB": 1024**3}


def physical_footprint(pid: int) -> int:
    output = subprocess.run(
        ["/usr/bin/footprint", "-p", str(pid)],
        check=True,
        capture_output=True,
        text=True,
        timeout=15,
    ).stdout
    match = re.search(r"phys_footprint:\s*([0-9.]+)\s*(KB|MB|GB)", output)
    if not match:
        raise RuntimeError(f"footprint did not report phys_footprint for pid {pid}")
    return int(float(match.group(1)) * UNITS[match.group(2)])


def process_rows(root_pid: int, *, include_footprint: bool = True) -> list[dict[str, object]]:
    """Snapshot descendants; RSS-only mode avoids inspecting short-lived children.

    Detailed footprint remains strict: a failed inspection must not become a
    zero-valued process that silently passes the standalone memory budget.
    """
    output = subprocess.run(
        ["/bin/ps", "-axo", "pid=,ppid=,rss=,comm="],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    rows: dict[int, tuple[int, int, str]] = {}
    for line in output.splitlines():
        fields = line.strip().split(maxsplit=3)
        if len(fields) != 4:
            continue
        pid, ppid, rss = map(int, fields[:3])
        rows[pid] = (ppid, rss * 1024, fields[3])

    selected = {root_pid}
    changed = True
    while changed:
        changed = False
        for pid, (ppid, _, _) in rows.items():
            if ppid in selected and pid not in selected:
                selected.add(pid)
                changed = True

    result = []
    for pid in sorted(selected):
        if pid not in rows:
            continue
        ppid, rss, command = rows[pid]
        row = {
            "pid": pid,
            "ppid": ppid,
            "name": Path(command).name,
            "rss_bytes": rss,
        }
        if include_footprint:
            row["phys_footprint_bytes"] = physical_footprint(pid)
        result.append(row)
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--stabilize-seconds", type=float, default=10)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--main-budget-mib", type=float, default=250)
    parser.add_argument("--idle-total-budget-mib", type=float, default=350)
    arguments = parser.parse_args()

    executable = arguments.app / "Contents" / "MacOS" / "Nola"
    if not executable.is_file():
        parser.error(f"Release executable is missing: {executable}")

    environment = os.environ.copy()
    environment["NOLA_SKIP_ONBOARDING"] = "1"
    process = subprocess.Popen(
        [str(executable)],
        env=environment,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
    try:
        time.sleep(arguments.stabilize_seconds)
        if process.poll() is not None:
            raise RuntimeError(f"Nola exited before sampling: {process.returncode}")
        rows = process_rows(process.pid)
        main_row = next(row for row in rows if row["pid"] == process.pid)
        total = sum(int(row["phys_footprint_bytes"]) for row in rows)
        forbidden = [
            str(row["name"])
            for row in rows
            if str(row["name"]) in {"nola-agent-host", "nola-mlx-runtime"}
        ]
        main_budget = int(arguments.main_budget_mib * 1024**2)
        total_budget = int(arguments.idle_total_budget_mib * 1024**2)
        passed = (
            int(main_row["phys_footprint_bytes"]) <= main_budget
            and total <= total_budget
            and not forbidden
        )
        report = {
            "status": "passed" if passed else "failed",
            "scenario": "standalone_release_idle",
            "stabilize_seconds": arguments.stabilize_seconds,
            "main_budget_bytes": main_budget,
            "idle_total_budget_bytes": total_budget,
            "total_phys_footprint_bytes": total,
            "unexpected_idle_helpers": forbidden,
            "processes": rows,
        }
        encoded = json.dumps(report, indent=2, sort_keys=True) + "\n"
        if arguments.output:
            arguments.output.parent.mkdir(parents=True, exist_ok=True)
            arguments.output.write_text(encoded)
        print(encoded, end="")
        return 0 if passed else 1
    finally:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)


if __name__ == "__main__":
    raise SystemExit(main())
