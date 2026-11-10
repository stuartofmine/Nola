#!/usr/bin/env python3
"""Plan, execute, validate, and roll back bounded workspace file moves."""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
from pathlib import Path
from typing import Any


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def inside(path: Path, root: Path) -> bool:
    try:
        path.relative_to(root)
        return True
    except ValueError:
        return False


def versioned_destination(destination: Path) -> Path:
    if not destination.exists():
        return destination
    index = 1
    while True:
        candidate = destination.with_name(f"{destination.stem} ({index}){destination.suffix}")
        if not candidate.exists():
            return candidate
        index += 1


def build_manifest(spec: dict[str, Any]) -> dict[str, Any]:
    root = Path(str(spec["root"])).expanduser().resolve()
    operations: list[dict[str, Any]] = []
    for index, raw in enumerate(spec.get("operations", []), start=1):
        source = root / str(raw["source"])
        requested = root / str(raw["destination"])
        row: dict[str, Any] = {
            "operation_id": f"file-op-{index:03d}",
            "action": "move",
            "source": str(source),
            "requested_destination": str(requested),
            "final_destination": None,
            "source_sha256": None,
            "collision": "none",
            "reversible": True,
            "status": "planned",
            "error": None,
        }
        if source.is_symlink():
            row.update(status="rejected", error="symlink sources are not followed", reversible=False)
        elif not inside(source.resolve(strict=False), root) or not inside(requested.resolve(strict=False), root):
            row.update(status="rejected", error="path escapes approved root", reversible=False)
        elif not source.is_file():
            row.update(status="rejected", error="source is not a regular file", reversible=False)
        else:
            final_destination = versioned_destination(requested)
            row["final_destination"] = str(final_destination)
            row["source_sha256"] = digest(source)
            row["collision"] = "versioned_suffix" if final_destination != requested else "none"
        operations.append(row)
    return {
        "schema_version": 1,
        "mode": "dry-run",
        "root": str(root),
        "symlink_policy": "do not follow",
        "overwrite_policy": "never",
        "operations": operations,
        "counts": reconcile(operations),
    }


def reconcile(operations: list[dict[str, Any]]) -> dict[str, int]:
    states = ["planned", "executed", "rolled_back", "rejected", "failed"]
    result = {state: sum(row.get("status") == state for row in operations) for state in states}
    result["total"] = len(operations)
    return result


def execute(manifest: dict[str, Any]) -> None:
    root = Path(str(manifest["root"])).resolve()
    for row in manifest.get("operations", []):
        if row.get("status") != "planned":
            continue
        source = Path(str(row["source"]))
        destination = Path(str(row["final_destination"]))
        if source.is_symlink() or not inside(source.resolve(strict=False), root) or not inside(destination.resolve(strict=False), root):
            row.update(status="failed", error="scope changed after planning")
            break
        if digest(source) != row.get("source_sha256") or destination.exists():
            row.update(status="failed", error="source or collision state changed after planning")
            break
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)
        if digest(destination) != row["source_sha256"]:
            destination.unlink(missing_ok=True)
            row.update(status="failed", error="destination hash verification failed")
            break
        source.unlink()
        row.update(status="executed", error=None)
    manifest["mode"] = "executed"
    manifest["counts"] = reconcile(manifest["operations"])


def rollback(manifest: dict[str, Any]) -> None:
    for row in reversed(manifest.get("operations", [])):
        if row.get("status") != "executed":
            continue
        source = Path(str(row["source"]))
        destination = Path(str(row["final_destination"]))
        if source.exists() or not destination.is_file() or digest(destination) != row.get("source_sha256"):
            row.update(status="failed", error="rollback precondition failed")
            continue
        source.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(destination, source)
        if digest(source) != row["source_sha256"]:
            source.unlink(missing_ok=True)
            row.update(status="failed", error="rollback hash verification failed")
            continue
        destination.unlink()
        row.update(status="rolled_back", error=None)
    manifest["mode"] = "rolled_back"
    manifest["counts"] = reconcile(manifest["operations"])


def validate(manifest: dict[str, Any], expected_state: str | None) -> dict[str, Any]:
    errors: list[str] = []
    operations = manifest.get("operations", [])
    if expected_state and manifest.get("mode") != expected_state:
        errors.append(f"mode is not {expected_state}")
    if manifest.get("counts") != reconcile(operations):
        errors.append("operation counts do not reconcile")
    for row in operations:
        source = Path(str(row["source"]))
        destination = Path(str(row["final_destination"])) if row.get("final_destination") else None
        if row.get("status") == "rolled_back":
            if not source.is_file() or digest(source) != row.get("source_sha256"):
                errors.append(f"{row.get('operation_id')} source was not restored")
            if destination and destination.exists():
                errors.append(f"{row.get('operation_id')} rollback destination still exists")
        if row.get("status") == "rejected" and "symlink" in str(row.get("error", "")) and not source.is_symlink():
            errors.append(f"{row.get('operation_id')} rejected symlink was changed")
    return {"validated": not errors, "errors": errors, "mode": manifest.get("mode")}


def save(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser()
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument("--plan")
    action.add_argument("--execute", action="store_true")
    action.add_argument("--rollback", action="store_true")
    action.add_argument("--validate-only", action="store_true")
    parser.add_argument("--manifest", required=True)
    parser.add_argument("--expected-state")
    args = parser.parse_args()
    manifest_path = Path(args.manifest)
    if args.plan:
        manifest = build_manifest(json.loads(Path(args.plan).read_text(encoding="utf-8")))
        save(manifest_path, manifest)
        receipt = validate(manifest, "dry-run")
    else:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        if args.execute:
            execute(manifest)
            save(manifest_path, manifest)
            receipt = validate(manifest, "executed")
        elif args.rollback:
            rollback(manifest)
            save(manifest_path, manifest)
            receipt = validate(manifest, "rolled_back")
        else:
            receipt = validate(manifest, args.expected_state)
    print(json.dumps(receipt, ensure_ascii=False, sort_keys=True))
    return 0 if receipt["validated"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
