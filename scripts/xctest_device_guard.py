#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Tracks and removes XCTest simulator clones created by one test invocation."""

from __future__ import annotations

import argparse
import errno
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import sys
import time
from typing import Callable


UUID_PATTERN = re.compile(
    r"^[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-"
    r"[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}$"
)


def device_paths(device_set: Path) -> dict[str, Path]:
    if not device_set.is_dir():
        return {}
    return {
        entry.name.upper(): entry
        for entry in device_set.iterdir()
        if entry.is_dir() and UUID_PATTERN.fullmatch(entry.name)
    }


def device_ids(device_set: Path) -> set[str]:
    return set(device_paths(device_set))


def write_snapshot(device_set: Path, output: Path) -> None:
    output.write_text("".join(f"{item}\n" for item in sorted(device_ids(device_set))))


def read_snapshot(path: Path) -> set[str]:
    if not path.is_file():
        return set()
    return {
        line.strip().upper()
        for line in path.read_text().splitlines()
        if UUID_PATTERN.fullmatch(line.strip())
    }


def delete_registered_device(device_set: Path, device_id: str) -> bool:
    subprocess.run(
        ["xcrun", "simctl", "--set", str(device_set), "shutdown", device_id],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    completed = subprocess.run(
        ["xcrun", "simctl", "--set", str(device_set), "delete", device_id],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    return completed.returncode == 0


def registered_device_name(device_path: Path) -> str | None:
    plist_path = device_path / "device.plist"
    if not plist_path.is_file():
        return None
    try:
        with plist_path.open("rb") as source:
            value = plistlib.load(source).get("name")
    except (OSError, plistlib.InvalidFileException):
        return None
    return value if isinstance(value, str) else None


def is_xctest_clone(device_path: Path) -> bool:
    name = registered_device_name(device_path)
    return name is not None and re.fullmatch(r"Clone \d+ of .+", name) is not None


def remove_partial_device(
    device_path: Path,
    attempts: int = 8,
    delay_seconds: float = 0.1,
    remover: Callable[[Path], None] = shutil.rmtree,
    sleeper: Callable[[float], None] = time.sleep,
) -> bool:
    """Remove an unregistered clone after XCTest finishes writing its data tree."""
    for attempt in range(attempts):
        try:
            remover(device_path)
            return True
        except FileNotFoundError:
            return True
        except OSError as error:
            retryable = error.errno in (errno.ENOTEMPTY, errno.EBUSY, errno.EACCES)
            if not retryable or attempt + 1 >= attempts:
                return False
            sleeper(delay_seconds)
    return False


def prune_stale_devices(
    device_set: Path,
    minimum_age_seconds: float,
    now: float | None = None,
    delete_device: Callable[[Path, str], bool] = delete_registered_device,
) -> tuple[list[str], list[str]]:
    """Removes abandoned XCTest clones while preserving fresh/unknown devices."""
    removed: list[str] = []
    retained: list[str] = []
    cutoff = (time.time() if now is None else now) - minimum_age_seconds
    for device_id, device_path in sorted(device_paths(device_set).items()):
        try:
            is_stale = device_path.stat().st_mtime <= cutoff
        except OSError:
            continue
        if not is_stale:
            continue
        if (device_path / "device.plist").is_file():
            if not is_xctest_clone(device_path):
                continue
            if delete_device(device_set, device_id):
                removed.append(device_id)
            else:
                retained.append(device_id)
            continue

        # A killed xcodebuild can leave a UUID data directory before the
        # device is registered. The strict UUID/direct-parent checks in
        # device_paths keep this deletion inside the XCTest-only device set.
        if remove_partial_device(device_path):
            removed.append(device_id)
        else:
            retained.append(device_id)
    return removed, retained


def cleanup_new_devices(
    device_set: Path,
    baseline: set[str],
    delete_device: Callable[[Path, str], bool] = delete_registered_device,
) -> tuple[list[str], list[str]]:
    removed: list[str] = []
    retained: list[str] = []
    paths = device_paths(device_set)
    for device_id in sorted(set(paths) - baseline):
        device_path = paths[device_id]
        if (device_path / "device.plist").is_file():
            if delete_device(device_set, device_id):
                removed.append(device_id)
            else:
                retained.append(device_id)
            continue

        # Xcode can be terminated between cloning the data directory and
        # registering device.plist. The UUID and direct-parent checks above
        # keep this fallback constrained to this run's incomplete clone.
        if remove_partial_device(device_path):
            removed.append(device_id)
        else:
            retained.append(device_id)
    return removed, retained


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=("snapshot", "cleanup", "prune-stale"))
    parser.add_argument("--device-set", required=True, type=Path)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--minimum-age-seconds", type=float, default=21_600)
    arguments = parser.parse_args()

    if arguments.command == "snapshot":
        if arguments.output is None:
            parser.error("snapshot requires --output")
        write_snapshot(arguments.device_set, arguments.output)
        return 0

    if arguments.command == "prune-stale":
        if arguments.minimum_age_seconds < 0:
            parser.error("--minimum-age-seconds must be non-negative")
        removed, retained = prune_stale_devices(
            arguments.device_set,
            arguments.minimum_age_seconds,
        )
        if removed:
            print(
                f"Removed {len(removed)} stale XCTest device clone(s).",
                file=sys.stderr,
            )
        if retained:
            print(
                "Warning: could not remove stale XCTest device clone(s): "
                + ", ".join(retained),
                file=sys.stderr,
            )
            return 1
        return 0

    if arguments.baseline is None:
        parser.error("cleanup requires --baseline")
    removed, retained = cleanup_new_devices(
        arguments.device_set,
        read_snapshot(arguments.baseline),
    )
    if removed:
        print(
            f"Removed {len(removed)} XCTest device clone(s) not present in the baseline.",
            file=sys.stderr,
        )
    if retained:
        print(
            "Warning: could not remove XCTest device clone(s): " + ", ".join(retained),
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
