#!/usr/bin/env python3
"""Reproduce and verify Nola's pinned 20-case LongMemEval-S benchmark."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import tempfile
import urllib.request
from collections import Counter
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[3]
BENCHMARK_DIR = Path(__file__).resolve().parent
CASES_DIR = BENCHMARK_DIR / "cases"
MANIFEST_PATH = BENCHMARK_DIR / "benchmark.json"
DEFAULT_SOURCE = (
    ROOT
    / ".artifacts"
    / "evals"
    / "longmemeval"
    / "downloads"
    / "longmemeval_s_cleaned.json"
)

DATASET_REPOSITORY = "https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned"
DATASET_REVISION = "98d7416c24c778c2fee6e6f3006e7a073259d48f"
DATASET_ARTIFACT = "longmemeval_s_cleaned.json"
DATASET_SHA256 = "d6f21ea9d60a0d56f34a05b609c79c88a451d2ae03597821ea3d5a9678c3a442"
DOWNLOAD_URL = (
    f"{DATASET_REPOSITORY}/resolve/{DATASET_REVISION}/{DATASET_ARTIFACT}"
)
CODE_REPOSITORY = "https://github.com/xiaowu0162/LongMemEval"
CODE_REVISION = "9e0b455f4ef0e2ab8f2e582289761153549043fc"
SEED = 20260911
LICENSE_SHA256 = "d3c4b9aa54759df6ded337978a6f3b55b75615e5e4525c3b82d7e2627d4b9732"
QUESTION_TYPES = (
    "knowledge-update",
    "multi-session",
    "single-session-assistant",
    "single-session-preference",
    "single-session-user",
    "temporal-reasoning",
)


def sha256_bytes(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_json(value: Any) -> bytes:
    return (
        json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
        + "\n"
    ).encode("utf-8")


def download_source(destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    request = urllib.request.Request(DOWNLOAD_URL, headers={"User-Agent": "NolaEval/1"})
    with tempfile.NamedTemporaryFile(dir=destination.parent, delete=False) as temporary:
        temporary_path = Path(temporary.name)
        try:
            with urllib.request.urlopen(request, timeout=120) as response:
                while chunk := response.read(1024 * 1024):
                    temporary.write(chunk)
            temporary.flush()
            os.fsync(temporary.fileno())
        except BaseException:
            temporary_path.unlink(missing_ok=True)
            raise
    actual = sha256_file(temporary_path)
    if actual != DATASET_SHA256:
        temporary_path.unlink(missing_ok=True)
        raise ValueError(f"download SHA-256 mismatch: expected {DATASET_SHA256}, got {actual}")
    temporary_path.replace(destination)


def load_source(path: Path, *, allow_download: bool) -> list[dict[str, Any]]:
    if not path.exists():
        if not allow_download:
            raise FileNotFoundError(f"LongMemEval source is missing: {path}")
        download_source(path)
    actual = sha256_file(path)
    if actual != DATASET_SHA256:
        raise ValueError(f"source SHA-256 mismatch: expected {DATASET_SHA256}, got {actual}")
    payload = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(payload, list) or len(payload) != 500:
        raise ValueError("pinned LongMemEval source must contain exactly 500 records")
    ids = [str(item.get("question_id", "")) for item in payload]
    if any(not item_id for item_id in ids) or len(set(ids)) != len(ids):
        raise ValueError("source question_id values must be non-empty and unique")
    observed_types = {str(item.get("question_type", "")) for item in payload}
    if observed_types != set(QUESTION_TYPES):
        raise ValueError(f"unexpected LongMemEval question types: {sorted(observed_types)}")
    return payload


def rank(record: dict[str, Any]) -> tuple[str, str]:
    question_id = str(record["question_id"])
    digest = hashlib.sha256(f"{SEED}\0{question_id}".encode("utf-8")).hexdigest()
    return digest, question_id


def select_cases(records: list[dict[str, Any]]) -> list[dict[str, Any]]:
    non_abstention = [
        record for record in records if not str(record["question_id"]).endswith("_abs")
    ]
    selected: list[dict[str, Any]] = []
    for question_type in QUESTION_TYPES:
        candidates = [
            record
            for record in non_abstention
            if record["question_type"] == question_type
        ]
        if len(candidates) < 3:
            raise ValueError(f"not enough non-abstention cases for {question_type}")
        selected.extend(sorted(candidates, key=rank)[:3])

    # Exercise abstention explicitly while avoiding two cases from the same category.
    abstention = sorted(
        (
            record
            for record in records
            if str(record["question_id"]).endswith("_abs")
        ),
        key=rank,
    )
    abstention_types: set[str] = set()
    for record in abstention:
        question_type = str(record["question_type"])
        if question_type in abstention_types:
            continue
        selected.append(record)
        abstention_types.add(question_type)
        if len(abstention_types) == 2:
            break
    if len(abstention_types) != 2:
        raise ValueError("not enough distinct abstention question types")
    return sorted(selected, key=lambda record: str(record["question_id"]))


def case_entry(record: dict[str, Any], payload: bytes) -> dict[str, Any]:
    question_id = str(record["question_id"])
    return {
        "id": question_id,
        "source_path": f"cases/{question_id}.json",
        "sha256": sha256_bytes(payload),
        "question_type": record["question_type"],
        "is_abstention": question_id.endswith("_abs"),
    }


def build_manifest(selected: list[dict[str, Any]]) -> tuple[dict[str, Any], dict[str, bytes]]:
    case_payloads = {
        str(record["question_id"]): canonical_json(record) for record in selected
    }
    by_type = Counter(str(record["question_type"]) for record in selected)
    abstention_count = sum(
        str(record["question_id"]).endswith("_abs") for record in selected
    )
    manifest = {
        "schema_version": 1,
        "name": "longmemeval",
        "variant": "s-cleaned-20",
        "upstream": {
            "repository": DATASET_REPOSITORY,
            "revision": DATASET_REVISION,
            "artifact_path": DATASET_ARTIFACT,
            "artifact_sha256": DATASET_SHA256,
            "download_url": DOWNLOAD_URL,
            "code_repository": CODE_REPOSITORY,
            "code_revision": CODE_REVISION,
            "license_spdx": "MIT",
            "license_files": [
                {"path": "LICENSE.longmemeval", "sha256": LICENSE_SHA256}
            ],
        },
        "selection": {
            "method": "sha256-stratified-v1",
            "seed": SEED,
            "count": len(selected),
            "non_abstention_per_question_type": 3,
            "abstention_count": abstention_count,
            "abstention_distinct_question_types": 2,
            "by_question_type": dict(sorted(by_type.items())),
        },
        "cases": [
            case_entry(record, case_payloads[str(record["question_id"])])
            for record in selected
        ],
    }
    return manifest, case_payloads


def write_benchmark(records: list[dict[str, Any]]) -> None:
    selected = select_cases(records)
    manifest, case_payloads = build_manifest(selected)
    CASES_DIR.mkdir(parents=True, exist_ok=True)
    selected_names = {f"{question_id}.json" for question_id in case_payloads}
    for stale in CASES_DIR.glob("*.json"):
        if stale.name not in selected_names:
            stale.unlink()
    for question_id, payload in case_payloads.items():
        (CASES_DIR / f"{question_id}.json").write_bytes(payload)
    MANIFEST_PATH.write_bytes(canonical_json(manifest))


def verify_benchmark(source: Path | None = None) -> None:
    manifest = json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))
    if (
        manifest.get("schema_version") != 1
        or manifest.get("name") != "longmemeval"
        or manifest.get("variant") != "s-cleaned-20"
    ):
        raise ValueError("unsupported LongMemEval benchmark manifest")
    upstream = manifest.get("upstream", {})
    expected_upstream = {
        "repository": DATASET_REPOSITORY,
        "revision": DATASET_REVISION,
        "artifact_path": DATASET_ARTIFACT,
        "artifact_sha256": DATASET_SHA256,
        "download_url": DOWNLOAD_URL,
        "code_repository": CODE_REPOSITORY,
        "code_revision": CODE_REVISION,
        "license_spdx": "MIT",
        "license_files": [
            {"path": "LICENSE.longmemeval", "sha256": LICENSE_SHA256}
        ],
    }
    if upstream != expected_upstream:
        raise ValueError("manifest upstream lock does not match prepare.py")
    if sha256_file(BENCHMARK_DIR / "LICENSE.longmemeval") != LICENSE_SHA256:
        raise ValueError("LongMemEval license SHA-256 mismatch")

    entries = manifest.get("cases")
    if not isinstance(entries, list) or len(entries) != 20:
        raise ValueError("benchmark must contain exactly 20 cases")
    ids: set[str] = set()
    records: list[dict[str, Any]] = []
    for entry in entries:
        question_id = str(entry.get("id", ""))
        if not question_id or question_id in ids:
            raise ValueError(f"duplicate or empty benchmark id: {question_id!r}")
        if re.fullmatch(r"[A-Za-z0-9_]+", question_id) is None:
            raise ValueError(f"unsafe benchmark id: {question_id!r}")
        ids.add(question_id)
        relative_path = str(entry.get("source_path", ""))
        expected_relative_path = f"cases/{question_id}.json"
        if relative_path != expected_relative_path:
            raise ValueError(f"unexpected source_path for {question_id}: {relative_path}")
        case_path = BENCHMARK_DIR / relative_path
        payload = case_path.read_bytes()
        if sha256_bytes(payload) != entry.get("sha256"):
            raise ValueError(f"case SHA-256 mismatch: {question_id}")
        record = json.loads(payload)
        if canonical_json(record) != payload:
            raise ValueError(f"case is not canonical JSON: {question_id}")
        if record.get("question_id") != question_id:
            raise ValueError(f"case id mismatch: {question_id}")
        if record.get("question_type") != entry.get("question_type"):
            raise ValueError(f"case type mismatch: {question_id}")
        if question_id.endswith("_abs") != entry.get("is_abstention"):
            raise ValueError(f"case abstention flag mismatch: {question_id}")
        required = {
            "question_id",
            "question_type",
            "question",
            "question_date",
            "answer",
            "answer_session_ids",
            "haystack_dates",
            "haystack_session_ids",
            "haystack_sessions",
        }
        if set(record) != required:
            raise ValueError(f"case fields differ from pinned upstream schema: {question_id}")
        session_count = len(record["haystack_sessions"])
        if len(record["haystack_dates"]) != session_count or len(record["haystack_session_ids"]) != session_count:
            raise ValueError(f"case session arrays are misaligned: {question_id}")
        records.append(record)

    observed_types = Counter(str(record["question_type"]) for record in records)
    if set(observed_types) != set(QUESTION_TYPES):
        raise ValueError("benchmark does not cover all six LongMemEval question types")
    selection = manifest.get("selection", {})
    expected_selection_fields = {
        "method": "sha256-stratified-v1",
        "seed": SEED,
        "count": 20,
        "non_abstention_per_question_type": 3,
        "abstention_count": 2,
        "abstention_distinct_question_types": 2,
    }
    for field, expected in expected_selection_fields.items():
        if selection.get(field) != expected:
            raise ValueError(f"selection {field} must be {expected!r}")
    if selection.get("by_question_type") != dict(sorted(observed_types.items())):
        raise ValueError("selection by_question_type does not match cases")
    abstention = [record for record in records if str(record["question_id"]).endswith("_abs")]
    if len(abstention) != 2 or len({record["question_type"] for record in abstention}) != 2:
        raise ValueError("benchmark must contain two abstentions from distinct types")
    if selection.get("abstention_count") != len(abstention):
        raise ValueError("selection abstention_count does not match cases")
    expected_case_names = {f"{question_id}.json" for question_id in ids}
    actual_case_names = {path.name for path in CASES_DIR.glob("*.json")}
    if actual_case_names != expected_case_names:
        raise ValueError("case directory does not exactly match manifest")

    if source is not None:
        source_records = load_source(source, allow_download=False)
        expected_manifest, expected_payloads = build_manifest(select_cases(source_records))
        if manifest != expected_manifest:
            raise ValueError("manifest is not the deterministic selection of pinned source")
        for question_id, payload in expected_payloads.items():
            if (CASES_DIR / f"{question_id}.json").read_bytes() != payload:
                raise ValueError(f"case differs from pinned source: {question_id}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("prepare", "verify"), nargs="?", default="prepare")
    parser.add_argument("--source", type=Path, default=DEFAULT_SOURCE)
    parser.add_argument(
        "--no-download",
        action="store_true",
        help="fail instead of downloading the pinned source when it is absent",
    )
    parser.add_argument(
        "--with-source",
        action="store_true",
        help="during verify, also compare every case with the pinned 500-case source",
    )
    args = parser.parse_args()

    if args.action == "prepare":
        source_records = load_source(args.source, allow_download=not args.no_download)
        write_benchmark(source_records)
        verify_benchmark(args.source)
    else:
        verify_benchmark(args.source if args.with_source else None)
    print(f"verified {MANIFEST_PATH.relative_to(ROOT)} (20 cases)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
