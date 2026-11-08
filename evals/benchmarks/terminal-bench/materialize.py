#!/usr/bin/env python3
"""Materialize Nola's deterministic 20-case Terminal-Bench 4.0.0 baseline."""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import tomllib
from pathlib import Path


UPSTREAM_REPOSITORY = "https://github.com/harbor-framework/terminal-bench.git"
UPSTREAM_VERSION = "v4.0.0"
UPSTREAM_REVISION = "452bf305c6daa62fc59061d22133a7cbc7c1572e"
UPSTREAM_GIT_ARCHIVE_SHA256 = (
    "75a4c00bbe6d8a5405455d3f4ea9bda968a526609a74608e1926b9b3305aa2b5"
)
SEED = "nola-terminal-bench-v4.0.0-20"
CATEGORY_QUOTAS = {
    "Hardware": 2,
    "ML": 3,
    "Media": 2,
    "Operations": 3,
    "Science": 4,
    "Security": 2,
    "Software": 4,
}
TREE_HASH_ALGORITHM = "sha256(relpath_utf8 + NUL + file_sha256_ascii + LF), sorted by relpath"


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def regular_files(root: Path) -> list[Path]:
    files = []
    for path in root.rglob("*"):
        if path.is_symlink():
            raise ValueError(f"symlinks are not supported in benchmark snapshots: {path}")
        if path.is_file():
            files.append(path)
    return sorted(files, key=lambda path: path.relative_to(root).as_posix())


def tree_sha256(root: Path) -> str:
    digest = hashlib.sha256()
    for path in regular_files(root):
        relative = path.relative_to(root).as_posix()
        digest.update(relative.encode("utf-8"))
        digest.update(b"\0")
        digest.update(file_sha256(path).encode("ascii"))
        digest.update(b"\n")
    return digest.hexdigest()


def difficulty(expert_hours: float) -> str:
    # These bands describe relative expert effort within this release. Even the
    # "low" band contains frontier tasks; it is not a claim that a task is easy.
    if expert_hours <= 2:
        return "low"
    if expert_hours <= 6:
        return "medium"
    return "high"


def stable_tiebreak(task_id: str) -> str:
    return hashlib.sha256(f"{SEED}:{task_id}".encode()).hexdigest()


def evenly_spaced_indices(size: int, count: int) -> list[int]:
    if count < 2 or count > size:
        raise ValueError(f"invalid stratum size={size}, count={count}")
    # Integer half-up rounding avoids dependence on Python's bankers rounding.
    denominator = count - 1
    return [
        (2 * index * (size - 1) + denominator) // (2 * denominator)
        for index in range(count)
    ]


def load_pool(upstream: Path) -> list[dict[str, object]]:
    pool = []
    for task_dir in sorted((upstream / "tasks").iterdir()):
        descriptor = task_dir / "task.toml"
        if not task_dir.is_dir() or not descriptor.is_file():
            continue
        document = tomllib.loads(descriptor.read_text(encoding="utf-8"))
        metadata = document["metadata"]
        pool.append(
            {
                "id": task_dir.name,
                "category": metadata["category"],
                "subcategory": metadata["subcategory"],
                "expert_hours": float(metadata["expert_time_estimate_hours"]),
            }
        )
    return pool


def select_cases(pool: list[dict[str, object]]) -> list[dict[str, object]]:
    selected = []
    for category, quota in CATEGORY_QUOTAS.items():
        stratum = sorted(
            (case for case in pool if case["category"] == category),
            key=lambda case: (
                case["expert_hours"],
                stable_tiebreak(str(case["id"])),
                case["id"],
            ),
        )
        for index in evenly_spaced_indices(len(stratum), quota):
            selected.append(stratum[index])
    return sorted(selected, key=lambda case: str(case["id"]))


def verify_revision(upstream: Path) -> None:
    revision = subprocess.run(
        ["git", "-C", str(upstream), "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if revision != UPSTREAM_REVISION:
        raise ValueError(f"expected upstream revision {UPSTREAM_REVISION}, found {revision}")
    status = subprocess.run(
        ["git", "-C", str(upstream), "status", "--porcelain", "--untracked-files=all"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if status:
        raise ValueError("upstream checkout must be clean before materialization")


def required_files(task_dir: Path) -> list[dict[str, str]]:
    required = [
        ("task.toml", "descriptor"),
        ("environment/Dockerfile", "environment"),
        ("tests/test.sh", "tests"),
        ("solution/solve.sh", "solution"),
    ]
    result = []
    for relative, role in required:
        path = task_dir / relative
        if not path.is_file():
            raise ValueError(f"missing required {role} file: {path}")
        result.append({"path": relative, "sha256": file_sha256(path), "role": role})
    return result


def materialize(upstream: Path, output: Path) -> None:
    verify_revision(upstream)
    source_license = upstream / "LICENSE"
    if not source_license.is_file():
        raise ValueError("upstream LICENSE is missing")

    pool = load_pool(upstream)
    if len(pool) != 66:
        raise ValueError(f"expected 66 tasks in Terminal-Bench 4.0.0, found {len(pool)}")
    selected = select_cases(pool)
    if len(selected) != 20 or len({case["id"] for case in selected}) != 20:
        raise ValueError("selection must contain exactly 20 unique tasks")

    tasks_output = output / "tasks"
    tasks_output.mkdir(parents=True, exist_ok=True)
    selected_ids = {str(case["id"]) for case in selected}
    for existing in tasks_output.iterdir():
        if existing.name not in selected_ids:
            if existing.is_dir() and not existing.is_symlink():
                shutil.rmtree(existing)
            else:
                existing.unlink()

    cases = []
    for selected_case in selected:
        task_id = str(selected_case["id"])
        source = upstream / "tasks" / task_id
        destination = tasks_output / task_id
        if destination.exists():
            if destination.is_dir() and not destination.is_symlink():
                shutil.rmtree(destination)
            else:
                destination.unlink()
        shutil.copytree(source, destination)
        cases.append(
            {
                "id": task_id,
                "source_path": f"tasks/{task_id}",
                "sha256": tree_sha256(destination),
                "category": selected_case["category"],
                "difficulty": difficulty(float(selected_case["expert_hours"])),
                "required_files": required_files(destination),
            }
        )

    shutil.copy2(source_license, output / "LICENSE")
    license_sha256 = file_sha256(output / "LICENSE")
    manifest = {
        "schema_version": 1,
        "name": "terminal-bench",
        "variant": "v4.0.0-nola-20",
        "upstream": {
            "repository": UPSTREAM_REPOSITORY,
            "revision": UPSTREAM_REVISION,
            "artifact_sha256": UPSTREAM_GIT_ARCHIVE_SHA256,
            "license_spdx": "Apache-2.0",
            "license_files": [{"path": "LICENSE", "sha256": license_sha256}],
        },
        "selection": {
            "method": "category quotas with evenly spaced expert-effort ranks",
            "seed": SEED,
            "count": 20,
            "source_pool_count": len(pool),
            "category_quotas": CATEGORY_QUOTAS,
            "tree_hash_algorithm": TREE_HASH_ALGORITHM,
            "by_category": CATEGORY_QUOTAS,
            "by_difficulty": {
                band: sum(
                    difficulty(float(case["expert_hours"])) == band for case in selected
                )
                for band in ("low", "medium", "high")
            },
        },
        "cases": cases,
    }
    (output / "benchmark.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("upstream", type=Path, help="Terminal-Bench checkout at the pinned revision")
    parser.add_argument(
        "--output", type=Path, default=Path(__file__).resolve().parent, help="baseline directory"
    )
    args = parser.parse_args()
    materialize(args.upstream.resolve(), args.output.resolve())


if __name__ == "__main__":
    main()
