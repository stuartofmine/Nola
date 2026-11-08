#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Validate vendored external benchmark selections without network access."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from collections import Counter
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ROOT = ROOT / "evals" / "benchmarks"
EXPECTED_BENCHMARKS = ("longmemeval", "terminal-bench")
LONGMEMEVAL_QUESTION_TYPES = {
    "single-session-user",
    "single-session-assistant",
    "single-session-preference",
    "multi-session",
    "knowledge-update",
    "temporal-reasoning",
}
TERMINAL_REQUIRED_ROLES = {"descriptor", "environment", "tests", "solution"}
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")


class ValidationError(ValueError):
    pass


def _load_object(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValidationError(f"{path}: cannot read JSON: {error}") from error
    if not isinstance(value, dict):
        raise ValidationError(f"{path}: root must be an object")
    return value


def _require_object(parent: dict[str, Any], key: str, location: str) -> dict[str, Any]:
    value = parent.get(key)
    if not isinstance(value, dict):
        raise ValidationError(f"{location}.{key}: must be an object")
    return value


def _require_string(parent: dict[str, Any], key: str, location: str) -> str:
    value = parent.get(key)
    if not isinstance(value, str) or not value.strip():
        raise ValidationError(f"{location}.{key}: must be a non-empty string")
    return value


def _require_sha256(value: Any, location: str) -> str:
    if not isinstance(value, str) or not SHA256_RE.fullmatch(value):
        raise ValidationError(f"{location}: must be a lowercase SHA-256 digest")
    return value


def _safe_child(root: Path, relative: Any, location: str) -> Path:
    if not isinstance(relative, str) or not relative or Path(relative).is_absolute():
        raise ValidationError(f"{location}: must be a non-empty relative path")
    candidate = (root / relative).resolve()
    try:
        candidate.relative_to(root.resolve())
    except ValueError as error:
        raise ValidationError(f"{location}: escapes benchmark directory") from error
    return candidate


def _file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _tree_sha256(root: Path) -> str:
    """Hash a directory as sorted UTF-8 relative paths and file digests."""
    digest = hashlib.sha256()
    files = sorted(
        (path for path in root.rglob("*") if path.is_file()),
        key=lambda path: path.relative_to(root).as_posix(),
    )
    if not files:
        raise ValidationError(f"{root}: task directory contains no files")
    for path in files:
        relative = path.relative_to(root).as_posix().encode("utf-8")
        digest.update(relative)
        digest.update(b"\0")
        digest.update(_file_sha256(path).encode("ascii"))
        digest.update(b"\n")
    return digest.hexdigest()


def _validate_upstream(root: Path, manifest: dict[str, Any]) -> None:
    upstream = _require_object(manifest, "upstream", "manifest")
    _require_string(upstream, "repository", "manifest.upstream")
    revision = _require_string(upstream, "revision", "manifest.upstream")
    if revision.lower() in {"head", "main", "master", "latest"}:
        raise ValidationError("manifest.upstream.revision: must pin a commit or release")
    _require_sha256(upstream.get("artifact_sha256"), "manifest.upstream.artifact_sha256")
    _require_string(upstream, "license_spdx", "manifest.upstream")
    license_files = upstream.get("license_files")
    if not isinstance(license_files, list) or not license_files:
        raise ValidationError("manifest.upstream.license_files: must archive at least one license")
    for index, item in enumerate(license_files):
        location = f"manifest.upstream.license_files[{index}]"
        if not isinstance(item, dict):
            raise ValidationError(f"{location}: must be an object")
        path = _safe_child(root, item.get("path"), f"{location}.path")
        expected = _require_sha256(item.get("sha256"), f"{location}.sha256")
        if not path.is_file():
            raise ValidationError(f"{location}.path: file does not exist: {path}")
        if _file_sha256(path) != expected:
            raise ValidationError(f"{location}: license file hash mismatch")


def _validate_common(root: Path, expected_name: str) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    manifest_path = root / "benchmark.json"
    manifest = _load_object(manifest_path)
    if manifest.get("schema_version") != 1:
        raise ValidationError(f"{manifest_path}: schema_version must be 1")
    if manifest.get("name") != expected_name:
        raise ValidationError(f"{manifest_path}: name must be {expected_name!r}")
    _require_string(manifest, "variant", "manifest")
    _validate_upstream(root, manifest)
    selection = _require_object(manifest, "selection", "manifest")
    _require_string(selection, "method", "manifest.selection")
    seed = selection.get("seed")
    if not isinstance(seed, (int, str)) or isinstance(seed, str) and not seed:
        raise ValidationError("manifest.selection.seed: must be a non-empty string or integer")
    if selection.get("count") != 20:
        raise ValidationError("manifest.selection.count: must be exactly 20")
    cases = manifest.get("cases")
    if not isinstance(cases, list) or len(cases) != 20:
        raise ValidationError("manifest.cases: must contain exactly 20 cases")
    typed_cases: list[dict[str, Any]] = []
    ids: list[str] = []
    source_paths: list[str] = []
    for index, item in enumerate(cases):
        if not isinstance(item, dict):
            raise ValidationError(f"manifest.cases[{index}]: must be an object")
        case_id = _require_string(item, "id", f"manifest.cases[{index}]")
        source_path = _require_string(item, "source_path", f"manifest.cases[{index}]")
        ids.append(case_id)
        source_paths.append(source_path)
        typed_cases.append(item)
    duplicates = sorted(case_id for case_id, count in Counter(ids).items() if count > 1)
    if duplicates:
        raise ValidationError(f"manifest.cases: duplicate ids: {', '.join(duplicates)}")
    duplicate_paths = sorted(path for path, count in Counter(source_paths).items() if count > 1)
    if duplicate_paths:
        raise ValidationError(
            f"manifest.cases: duplicate source paths: {', '.join(duplicate_paths)}"
        )
    return manifest, typed_cases


def _validate_longmemeval(root: Path) -> None:
    manifest, cases = _validate_common(root, "longmemeval")
    counts: Counter[str] = Counter()
    non_abstention_counts: Counter[str] = Counter()
    abstention_types: set[str] = set()
    abstention_count = 0
    for index, item in enumerate(cases):
        location = f"manifest.cases[{index}]"
        question_type = _require_string(item, "question_type", location)
        counts[question_type] += 1
        is_abstention = item.get("is_abstention")
        if not isinstance(is_abstention, bool):
            raise ValidationError(f"{location}.is_abstention: must be a boolean")
        if is_abstention != item["id"].endswith("_abs"):
            raise ValidationError(f"{location}.is_abstention: does not match _abs id suffix")
        abstention_count += int(is_abstention)
        if is_abstention:
            abstention_types.add(question_type)
        else:
            non_abstention_counts[question_type] += 1
        path = _safe_child(root, item.get("source_path"), f"{location}.source_path")
        expected = _require_sha256(item.get("sha256"), f"{location}.sha256")
        if not path.is_file():
            raise ValidationError(f"{location}.source_path: case file does not exist: {path}")
        if _file_sha256(path) != expected:
            raise ValidationError(f"{location}: case file hash mismatch")
        case = _load_object(path)
        upstream_id = case.get("question_id") or case.get("id")
        if upstream_id != item["id"]:
            raise ValidationError(f"{location}: case file id does not match manifest")
        if case.get("question_type") != question_type:
            raise ValidationError(f"{location}: case file question_type does not match manifest")
    missing = sorted(LONGMEMEVAL_QUESTION_TYPES - counts.keys())
    if missing:
        raise ValidationError(f"manifest.cases: missing LongMemEval question types: {', '.join(missing)}")
    selection = _require_object(manifest, "selection", "manifest")
    if selection.get("by_question_type") != dict(sorted(counts.items())):
        raise ValidationError("manifest.selection.by_question_type: does not match selected cases")
    if selection.get("abstention_count") != abstention_count:
        raise ValidationError("manifest.selection.abstention_count: does not match selected cases")
    if selection.get("abstention_distinct_question_types") != len(abstention_types):
        raise ValidationError(
            "manifest.selection.abstention_distinct_question_types: does not match selected cases"
        )
    non_abstention_targets = set(non_abstention_counts.values())
    if len(non_abstention_targets) != 1 or selection.get(
        "non_abstention_per_question_type"
    ) != next(iter(non_abstention_targets)):
        raise ValidationError(
            "manifest.selection.non_abstention_per_question_type: does not match selected cases"
        )


def _validate_terminal_bench(root: Path) -> None:
    manifest, cases = _validate_common(root, "terminal-bench")
    categories: Counter[str] = Counter()
    difficulties: Counter[str] = Counter()
    for index, item in enumerate(cases):
        location = f"manifest.cases[{index}]"
        categories[_require_string(item, "category", location)] += 1
        difficulties[_require_string(item, "difficulty", location)] += 1
        task_root = _safe_child(root, item.get("source_path"), f"{location}.source_path")
        expected_tree = _require_sha256(item.get("sha256"), f"{location}.sha256")
        if not task_root.is_dir():
            raise ValidationError(f"{location}.source_path: task directory does not exist: {task_root}")
        if _tree_sha256(task_root) != expected_tree:
            raise ValidationError(f"{location}: task tree hash mismatch")
        required_files = item.get("required_files")
        if not isinstance(required_files, list) or not required_files:
            raise ValidationError(f"{location}.required_files: must be a non-empty list")
        roles: set[str] = set()
        paths: set[str] = set()
        for file_index, file_item in enumerate(required_files):
            file_location = f"{location}.required_files[{file_index}]"
            if not isinstance(file_item, dict):
                raise ValidationError(f"{file_location}: must be an object")
            role = _require_string(file_item, "role", file_location)
            if role not in TERMINAL_REQUIRED_ROLES:
                raise ValidationError(f"{file_location}.role: unsupported role {role!r}")
            if role in roles:
                raise ValidationError(f"{location}.required_files: duplicate role {role!r}")
            roles.add(role)
            relative = _require_string(file_item, "path", file_location)
            if relative in paths:
                raise ValidationError(f"{location}.required_files: duplicate path {relative!r}")
            paths.add(relative)
            path = _safe_child(task_root, relative, f"{file_location}.path")
            expected = _require_sha256(file_item.get("sha256"), f"{file_location}.sha256")
            if not path.is_file():
                raise ValidationError(f"{file_location}.path: file does not exist: {path}")
            if _file_sha256(path) != expected:
                raise ValidationError(f"{file_location}: file hash mismatch")
        missing_roles = sorted(TERMINAL_REQUIRED_ROLES - roles)
        if missing_roles:
            raise ValidationError(f"{location}.required_files: missing roles: {', '.join(missing_roles)}")
    selection = _require_object(manifest, "selection", "manifest")
    if selection.get("by_category") != dict(sorted(categories.items())):
        raise ValidationError("manifest.selection.by_category: does not match selected cases")
    if selection.get("by_difficulty") != dict(sorted(difficulties.items())):
        raise ValidationError("manifest.selection.by_difficulty: does not match selected cases")
    if selection.get("category_quotas") != selection.get("by_category"):
        raise ValidationError("manifest.selection.category_quotas: must match by_category")
    if selection.get("source_pool_count") != 66:
        raise ValidationError("manifest.selection.source_pool_count: must be 66")


def validate(root: Path) -> None:
    _validate_longmemeval(root / "longmemeval")
    _validate_terminal_bench(root / "terminal-bench")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=DEFAULT_ROOT)
    args = parser.parse_args()
    try:
        validate(args.root.resolve())
    except ValidationError as error:
        print(f"external benchmark validation failed: {error}", file=sys.stderr)
        return 1
    print(f"external benchmark validation passed: 2 benchmarks, 40 cases ({args.root})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
