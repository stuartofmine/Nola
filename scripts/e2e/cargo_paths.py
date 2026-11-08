#!/usr/bin/env python3
"""Canonical Cargo artifact resolution for repository test harnesses."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path


def cargo_target_directory(repository_root: Path, manifest_path: Path | None = None) -> Path:
    manifest = manifest_path or repository_root / "cli" / "Cargo.toml"
    completed = subprocess.run(
        [
            "cargo",
            "metadata",
            "--manifest-path",
            str(manifest),
            "--format-version",
            "1",
            "--no-deps",
        ],
        cwd=repository_root,
        check=True,
        capture_output=True,
        text=True,
    )
    return Path(json.loads(completed.stdout)["target_directory"])


def cargo_binary(
    repository_root: Path,
    *,
    profile: str = "debug",
    name: str = "nola",
    manifest_path: Path | None = None,
) -> Path:
    return cargo_target_directory(repository_root, manifest_path) / profile / name
