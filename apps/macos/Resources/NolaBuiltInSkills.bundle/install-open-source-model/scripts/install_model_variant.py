#!/usr/bin/env python3
"""Install a complete model snapshot and atomically register it with Nola."""

import argparse
import hashlib
import json
import os
import re
import shutil
import signal
import tempfile
import time
import urllib.request
from pathlib import Path, PurePosixPath
from urllib.parse import urlparse


CANCELLED = False
REPORT_INTERVAL_SECONDS = 1.0
LOCK_STALE_SECONDS = 90


def handle_stop(_signum, _frame):
    global CANCELLED
    CANCELLED = True


def load_object(path):
    value = json.loads(Path(path).read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def atomic_json(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(value, handle, ensure_ascii=False, indent=2, sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def safe_id(value):
    normalized = re.sub(r"[^A-Za-z0-9._-]+", "-", str(value)).strip("-.")
    if not normalized:
        raise ValueError("model id is empty after normalization")
    return normalized[:160]


def safe_relative_path(value):
    if value is None or not str(value).strip():
        raise ValueError("artifact path is required")
    path = PurePosixPath(str(value))
    if path.is_absolute() or not path.parts or any(part in {"", ".", ".."} for part in path.parts):
        raise ValueError(f"artifact path must be a safe relative path: {value!r}")
    return Path(*path.parts)


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as handle:
        while chunk := handle.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def process_alive(pid):
    try:
        os.kill(int(pid), 0)
        return True
    except (OSError, TypeError, ValueError):
        return False


def acquire_lock(lock, task_id):
    for _ in range(2):
        try:
            lock.mkdir()
            owner = {
                "schemaVersion": 1,
                "taskID": task_id,
                "pid": os.getpid(),
                "startedEpoch": int(time.time()),
                "heartbeatEpoch": int(time.time()),
            }
            atomic_json(lock / "owner.json", owner)
            return
        except FileExistsError:
            owner_path = lock / "owner.json"
            owner = load_object(owner_path) if owner_path.exists() else {}
            heartbeat = int(owner.get("heartbeatEpoch") or owner.get("startedEpoch") or 0)
            owner_pid = owner.get("pid")
            live = process_alive(owner_pid)
            fresh = time.time() - heartbeat < LOCK_STALE_SECONDS
            if live or (not owner_pid and fresh):
                raise RuntimeError(f"another installation owns this model: {owner}")
            stale = lock.with_name(f".install.lock.stale-{int(time.time())}")
            try:
                os.replace(lock, stale)
            except FileNotFoundError:
                continue
            shutil.rmtree(stale, ignore_errors=True)
    raise RuntimeError("unable to acquire model installation lock")


def heartbeat(lock, task_id):
    atomic_json(lock / "owner.json", {
        "schemaVersion": 1,
        "taskID": task_id,
        "pid": os.getpid(),
        "heartbeatEpoch": int(time.time()),
    })


def emit_state(path, lock, task_id, status, completed, total, started, attempts,
               artifact=None, artifact_bytes=None, artifact_total=None, error=None):
    elapsed = max(time.monotonic() - started, 0.001)
    speed = int(completed / elapsed)
    remaining = max(total - completed, 0) if total else None
    value = {
        "schemaVersion": 1,
        "taskID": task_id,
        "status": status,
        "downloadedBytes": completed,
        "totalBytes": total or None,
        "progress": completed / total if total else None,
        "bytesPerSecond": speed,
        "etaSeconds": int(remaining / speed) if remaining is not None and speed else None,
        "attempts": attempts,
        "pid": os.getpid(),
        "heartbeatEpoch": int(time.time()),
        "currentArtifact": artifact,
        "artifactDownloadedBytes": artifact_bytes,
        "artifactTotalBytes": artifact_total,
        "error": error,
    }
    atomic_json(path, value)
    heartbeat(lock, task_id)
    print(json.dumps(value, ensure_ascii=False), flush=True)


def normalize_artifacts(args, selected):
    if args.artifacts_json:
        document = load_object(args.artifacts_json)
        values = document.get("artifacts")
        revision = document.get("revision")
        if revision and selected.get("revision") and revision != selected.get("revision"):
            raise ValueError("artifact manifest revision does not match selected model revision")
    else:
        if not args.artifact_url or not args.filename:
            raise ValueError("provide --artifacts-json or both --artifact-url and --filename")
        values = [{
            "url": args.artifact_url,
            "path": args.filename,
            "sha256": args.sha256,
            "size_bytes": selected.get("size_bytes"),
        }]
    if not isinstance(values, list) or not values:
        raise ValueError("artifact manifest must contain a non-empty artifacts array")
    artifacts = []
    seen = set()
    for index, raw in enumerate(values):
        if not isinstance(raw, dict):
            raise ValueError(f"artifact {index} must be an object")
        relative = safe_relative_path(raw.get("path") or raw.get("filename"))
        key = relative.as_posix()
        if key in seen:
            raise ValueError(f"duplicate artifact path: {key}")
        seen.add(key)
        size = int(raw.get("size_bytes") or raw.get("sizeBytes") or 0)
        if size <= 0:
            raise ValueError(f"artifact {key} requires a positive size_bytes")
        url = str(raw.get("url") or "")
        if urlparse(url).scheme not in {"https", "file"}:
            raise ValueError(f"artifact {key} URL must use https or file")
        artifacts.append({
            "url": url,
            "path": key,
            "sizeBytes": size,
            "sha256": raw.get("sha256"),
            "etag": raw.get("etag"),
        })
    return artifacts


def open_remote(artifact, partial, retries, retry_delay):
    source = artifact["url"]
    parsed = urlparse(source)
    if parsed.scheme == "file":
        source_path = Path(parsed.path)
        if source_path.stat().st_size != artifact["sizeBytes"]:
            raise ValueError(f"source size does not match manifest for {artifact['path']}")
        existing = partial.stat().st_size if partial.exists() else 0
        if existing > artifact["sizeBytes"]:
            partial.unlink()
            existing = 0
        handle = source_path.open("rb")
        handle.seek(existing)
        return handle, existing, 1

    metadata_path = partial.with_suffix(partial.suffix + ".remote.json")
    last_error = None
    for attempt in range(1, retries + 2):
        try:
            existing = partial.stat().st_size if partial.exists() else 0
            if existing == artifact["sizeBytes"]:
                return None, existing, attempt
            prior = load_object(metadata_path) if metadata_path.exists() else {}
            headers = {"User-Agent": "NolaModelInstaller/2"}
            if existing:
                validator = prior.get("etag") or prior.get("lastModified")
                if not validator:
                    partial.unlink(missing_ok=True)
                    existing = 0
                else:
                    headers["Range"] = f"bytes={existing}-"
                    headers["If-Range"] = validator
            response = urllib.request.urlopen(urllib.request.Request(source, headers=headers), timeout=120)
            status = getattr(response, "status", 200)
            etag = response.headers.get("ETag")
            modified = response.headers.get("Last-Modified")
            manifest_etag = artifact.get("etag")
            if manifest_etag and etag and manifest_etag != etag:
                response.close()
                raise RuntimeError(f"ETag does not match manifest for {artifact['path']}")
            if existing and (status != 206 or (prior.get("etag") and etag and prior["etag"] != etag)):
                response.close()
                partial.unlink(missing_ok=True)
                existing = 0
                response = urllib.request.urlopen(
                    urllib.request.Request(source, headers={"User-Agent": "NolaModelInstaller/2"}),
                    timeout=120,
                )
                status = getattr(response, "status", 200)
                etag = response.headers.get("ETag")
                modified = response.headers.get("Last-Modified")
            if existing:
                content_range = response.headers.get("Content-Range", "")
                if status != 206 or not content_range.startswith(f"bytes {existing}-"):
                    response.close()
                    raise RuntimeError(f"invalid Content-Range for resume: {content_range!r}")
            atomic_json(metadata_path, {
                "url": source,
                "etag": etag,
                "lastModified": modified,
                "sizeBytes": artifact["sizeBytes"],
            })
            return response, existing, attempt
        except Exception as error:
            last_error = error
            if attempt > retries:
                raise RuntimeError(f"download remains incomplete: {error}") from error
            time.sleep(min(retry_delay * (2 ** (attempt - 1)), 300))
    raise RuntimeError(f"download remains incomplete: {last_error}")


def download_artifact(artifact, partial, completed_before, aggregate_total, state_path,
                      lock, task_id, started, retries, retry_delay):
    source, existing, attempt = open_remote(artifact, partial, retries, retry_delay)
    if source is None:
        return existing
    mode = "ab" if existing else "wb"
    downloaded = existing
    last_report = 0.0
    partial.parent.mkdir(parents=True, exist_ok=True)
    with source, partial.open(mode) as output:
        while True:
            if CANCELLED:
                output.flush()
                os.fsync(output.fileno())
                emit_state(state_path, lock, task_id, "paused", completed_before + downloaded,
                           aggregate_total, started, attempt, artifact["path"], downloaded,
                           artifact["sizeBytes"], "cancelled; verified partial data retained")
                raise SystemExit(130)
            chunk = source.read(1024 * 1024)
            if not chunk:
                break
            output.write(chunk)
            downloaded += len(chunk)
            if downloaded > artifact["sizeBytes"]:
                raise ValueError(f"artifact exceeds manifest size: {artifact['path']}")
            now = time.monotonic()
            if now - last_report >= REPORT_INTERVAL_SECONDS:
                output.flush()
                emit_state(state_path, lock, task_id, "downloading", completed_before + downloaded,
                           aggregate_total, started, attempt, artifact["path"], downloaded,
                           artifact["sizeBytes"])
                last_report = now
        output.flush()
        os.fsync(output.fileno())
    if downloaded != artifact["sizeBytes"]:
        raise ValueError(f"artifact size mismatch for {artifact['path']}: {downloaded} != {artifact['sizeBytes']}")
    return downloaded


def transfer_with_retries(artifact, partial, completed_before, aggregate_total, state_path,
                          lock, task_id, started, retries, retry_delay):
    last_error = None
    for attempt in range(1, retries + 2):
        try:
            return download_artifact(
                artifact, partial, completed_before, aggregate_total, state_path, lock,
                task_id, started, 0, retry_delay,
            )
        except SystemExit:
            raise
        except Exception as error:
            last_error = error
            partial_bytes = partial.stat().st_size if partial.exists() else 0
            if attempt > retries:
                break
            emit_state(
                state_path, lock, task_id, "retrying", completed_before + partial_bytes,
                aggregate_total, started, attempt, artifact["path"], partial_bytes,
                artifact["sizeBytes"], str(error),
            )
            time.sleep(min(retry_delay * (2 ** (attempt - 1)), 300))
    raise RuntimeError(f"artifact {artifact['path']} remains incomplete: {last_error}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--plan", required=True)
    parser.add_argument("--models-root")
    parser.add_argument("--artifacts-json")
    parser.add_argument("--artifact-url")
    parser.add_argument("--filename")
    parser.add_argument("--sha256")
    parser.add_argument("--max-retries", type=int, default=20)
    parser.add_argument("--retry-delay", type=int, default=15)
    parser.add_argument("--task-id")
    args = parser.parse_args()

    plan = load_object(args.plan)
    selected = plan.get("selected")
    if plan.get("status") != "recommended" or not isinstance(selected, dict):
        raise ValueError("plan has no compatible selected model")
    model_id = safe_id(selected.get("repo_id") or plan.get("model"))
    task_id = safe_id(args.task_id or f"model-install-{model_id}")
    artifacts = normalize_artifacts(args, selected)
    total_bytes = sum(item["sizeBytes"] for item in artifacts)
    declared_bytes = int(selected.get("size_bytes") or total_bytes)
    if declared_bytes != total_bytes:
        raise ValueError(f"artifact manifest total {total_bytes} does not match selected size {declared_bytes}")

    models_root = args.models_root or os.environ.get("NOLA_MODELS_ROOT")
    if not models_root:
        raise ValueError("models root was not provided by the active Nola Host")
    root = Path(models_root).expanduser().resolve()
    model_dir = root / model_id
    snapshot_id = safe_id(selected.get("revision") or "unversioned")
    snapshot_dir = model_dir / "snapshots" / f"{snapshot_id}-{task_id}"
    staging = model_dir / ".staging" / f"{snapshot_id}-{task_id}"
    task_path = model_dir / "task.json"
    lock = model_dir / ".install.lock"
    model_dir.mkdir(parents=True, exist_ok=True)
    acquire_lock(lock, task_id)
    started = time.monotonic()
    signal.signal(signal.SIGINT, handle_stop)
    signal.signal(signal.SIGTERM, handle_stop)

    try:
        existing = sum(
            min((staging / item["path"]).with_suffix((staging / item["path"]).suffix + ".part").stat().st_size, item["sizeBytes"])
            for item in artifacts
            if (staging / item["path"]).with_suffix((staging / item["path"]).suffix + ".part").exists()
        )
        free = shutil.disk_usage(model_dir).free
        reserve = max(512 * 1024 * 1024, int(total_bytes * 0.05))
        if total_bytes - existing + reserve > free:
            emit_state(task_path, lock, task_id, "insufficient_disk", existing, total_bytes,
                       started, 0, error=f"need {total_bytes - existing + reserve} bytes; have {free}")
            raise RuntimeError("insufficient disk space for complete model snapshot")

        completed = 0
        observed_artifacts = []
        for artifact in artifacts:
            destination = staging / artifact["path"]
            partial = destination.with_suffix(destination.suffix + ".part")
            if destination.exists() and destination.stat().st_size == artifact["sizeBytes"]:
                expected_sha256 = artifact.get("sha256")
                observed = sha256(destination) if expected_sha256 else None
                if not expected_sha256 or observed.lower() == str(expected_sha256).lower():
                    downloaded = artifact["sizeBytes"]
                    completed += downloaded
                    observed_artifacts.append({
                        "path": artifact["path"],
                        "bytes": downloaded,
                        "sha256": observed,
                        "sourceURL": artifact["url"],
                    })
                    emit_state(task_path, lock, task_id, "resuming", completed, total_bytes,
                               started, 0, artifact["path"], downloaded, artifact["sizeBytes"])
                    continue
                destination.unlink()
            downloaded = transfer_with_retries(
                artifact, partial, completed, total_bytes, task_path, lock, task_id, started,
                max(args.max_retries, 0), max(args.retry_delay, 1),
            )
            emit_state(task_path, lock, task_id, "verifying", completed + downloaded,
                       total_bytes, started, 1, artifact["path"], downloaded, artifact["sizeBytes"])
            expected_sha256 = artifact.get("sha256")
            observed = sha256(partial) if expected_sha256 else None
            if expected_sha256 and observed.lower() != str(expected_sha256).lower():
                partial.unlink(missing_ok=True)
                raise ValueError(f"SHA-256 mismatch for {artifact['path']}")
            final_staged = staging / artifact["path"]
            os.replace(partial, final_staged)
            partial.with_suffix(partial.suffix + ".remote.json").unlink(missing_ok=True)
            completed += downloaded
            observed_artifacts.append({
                "path": artifact["path"],
                "bytes": downloaded,
                "sha256": observed,
                "sourceURL": artifact["url"],
            })

        manifest = {
            "schemaVersion": 1,
            "modelID": model_id,
            "repository": selected.get("repo_id") or plan.get("model"),
            "revision": selected.get("revision"),
            "totalBytes": total_bytes,
            "artifacts": observed_artifacts,
        }
        atomic_json(staging / "snapshot.json", manifest)
        snapshot_dir.parent.mkdir(parents=True, exist_ok=True)
        os.replace(staging, snapshot_dir)

        record = {
            "schemaVersion": 1,
            "taskID": task_id,
            "id": model_id,
            "name": selected.get("name") or selected.get("repo_id") or plan.get("model"),
            "repository": selected.get("repo_id") or plan.get("model"),
            "revision": selected.get("revision"),
            "runtime": selected.get("runtime"),
            "accelerator": selected.get("accelerator") or "cpu",
            "format": selected.get("format"),
            "quantization": selected.get("quantization"),
            "contextWindowTokens": selected.get("context_length"),
            "estimatedPeakMemoryBytes": selected.get("estimated_peak_memory_bytes"),
            "recommendedMemoryBytes": selected.get("recommended_memory_bytes") or selected.get("estimated_peak_memory_bytes"),
            "snapshotPath": str(snapshot_dir),
            "artifactPath": str(snapshot_dir / observed_artifacts[0]["path"]),
            "artifactBytes": total_bytes,
            "artifacts": observed_artifacts,
            "state": "configured",
            "enabled": True,
        }
        atomic_json(model_dir / "model.json", record)
        catalog_path = root / "catalog.json"
        catalog_lock = root / ".catalog.lock"
        acquire_lock(catalog_lock, task_id)
        try:
            catalog = load_object(catalog_path) if catalog_path.exists() else {"schemaVersion": 1, "models": []}
            models = [item for item in catalog.get("models", []) if isinstance(item, dict) and item.get("id") != model_id]
            models.append(record)
            catalog["schemaVersion"] = 2
            catalog["models"] = sorted(models, key=lambda item: str(item.get("name", "")).lower())
            atomic_json(catalog_path, catalog)
        finally:
            shutil.rmtree(catalog_lock, ignore_errors=True)
        for old_snapshot in snapshot_dir.parent.iterdir():
            if old_snapshot != snapshot_dir and old_snapshot.is_dir():
                shutil.rmtree(old_snapshot, ignore_errors=True)
        emit_state(task_path, lock, task_id, "configured", total_bytes, total_bytes, started, 1)
        print(json.dumps({"status": "configured", "model": record, "catalogPath": str(catalog_path)}, ensure_ascii=False))
    except SystemExit:
        raise
    except Exception as error:
        completed = 0
        if task_path.exists():
            completed = int(load_object(task_path).get("downloadedBytes") or 0)
        emit_state(task_path, lock, task_id, "failed_recoverable", completed, total_bytes,
                   started, 1, error=str(error))
        raise
    finally:
        shutil.rmtree(lock, ignore_errors=True)


if __name__ == "__main__":
    main()
