#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Bound one isolated evaluation process group before verifier handoff."""
from __future__ import annotations

import argparse
import json
import math
import os
import signal
import subprocess
import sys
import time


def terminate_group(process: subprocess.Popen) -> None:
    # The leader may have exited while a background child still holds output
    # pipes or can mutate artifacts. Always clean up the owned group, not only
    # a live leader. No unrelated container processes are selected by name.
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait(timeout=5)


def supervise(command: list[str], seconds: float, grace: float = 5) -> int:
    if not command or not math.isfinite(seconds) or seconds <= 0:
        raise ValueError('positive finite deadline and command required')
    environment = os.environ.copy()
    # The agent sees the same trial budget as the supervisor, not the task's
    # prose allowance. Enforcement still uses wait's monotonic timeout.
    environment['NOLA_EXECUTION_DEADLINE_UNIX_MS'] = str(time.time_ns() // 1_000_000 + math.ceil(seconds * 1000))
    environment['NOLA_EXECUTION_TIME_LIMIT_SECONDS'] = str(seconds)
    process = subprocess.Popen(command, start_new_session=True, env=environment)
    try:
        try:
            return process.wait(timeout=seconds)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGINT)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=grace)
            except subprocess.TimeoutExpired:
                pass
            return 124
    finally:
        terminate_group(process)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--seconds', type=float, required=True)
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ['--'] else args.command
    wall_start = time.time_ns()
    monotonic_start = time.monotonic_ns()
    code = supervise(command, args.seconds)
    monotonic_elapsed = time.monotonic_ns() - monotonic_start
    wall_elapsed = time.time_ns() - wall_start
    # Evaluation evidence only, never a canonical Nola result. Both clocks
    # bracket process execution and group cleanup; no arguments or secrets.
    print(json.dumps({
        'type': 'evaluation_process_timing',
        'wall_elapsed_ms': wall_elapsed / 1_000_000,
        'monotonic_elapsed_ms': monotonic_elapsed / 1_000_000,
        'clock_divergence_ms': (wall_elapsed - monotonic_elapsed) / 1_000_000,
        'supervisor_exit_code': code,
    }), file=sys.stderr, flush=True)
    raise SystemExit(code if code >= 0 else 128 - code)
