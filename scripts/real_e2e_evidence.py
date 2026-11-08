#!/usr/bin/env python3
"""Structured evidence receipts for real Nola end-to-end scenarios."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import tempfile
from typing import Any, Iterable
from provider_trace_metrics import provider_trace_metrics


SCHEMA_VERSION = 1


def _strings(values: Iterable[str]) -> list[str]:
    return sorted({str(value).strip() for value in values if str(value).strip()})


def _file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _observation(value: dict[str, Any]) -> dict[str, Any]:
    identifier = str(value.get("id") or "").strip()
    kind = str(value.get("kind") or "").strip()
    raw_path = str(value.get("path") or "").strip()
    if not identifier or not kind or not raw_path:
        raise ValueError("every E2E observation requires id, kind, and path")
    path = Path(raw_path).expanduser().resolve()
    row: dict[str, Any] = {
        "id": identifier,
        "kind": kind,
        "path": str(path),
        "exists": path.is_file(),
        "bytes": path.stat().st_size if path.is_file() else None,
        "sha256": _file_sha256(path) if path.is_file() else None,
        "metrics": {},
    }
    visual_review = value.get("visualReview")
    if isinstance(visual_review, dict):
        row["visualReview"] = dict(visual_review)
    if kind == "provider_trace":
        row["metrics"] = provider_trace_metrics([path])
    return row


def _materialize_observation(
    value: dict[str, Any],
    *,
    receipt_path: Path,
    step_id: str,
) -> dict[str, Any]:
    source = Path(str(value.get("path") or "")).expanduser().resolve()
    if not source.is_file():
        return _observation(value)
    safe_step = re.sub(r"[^A-Za-z0-9._-]+", "-", step_id).strip("-") or "step"
    safe_id = re.sub(r"[^A-Za-z0-9._-]+", "-", str(value.get("id") or "")).strip("-")
    evidence_dir = receipt_path.parent / f"{receipt_path.stem}.observations"
    evidence_dir.mkdir(parents=True, exist_ok=True)
    suffix = "".join(source.suffixes[-2:]) or ".evidence"
    destination = evidence_dir / f"{safe_step}--{safe_id}{suffix}"
    if source != destination.resolve():
        descriptor, temporary = tempfile.mkstemp(
            prefix=f".{destination.name}.",
            dir=evidence_dir,
        )
        os.close(descriptor)
        try:
            shutil.copy2(source, temporary)
            os.replace(temporary, destination)
        finally:
            try:
                os.unlink(temporary)
            except FileNotFoundError:
                pass
    materialized = dict(value)
    materialized["path"] = str(destination)
    row = _observation(materialized)
    row["sourcePath"] = str(source)
    return row


def record_step(
    step_id: str,
    *,
    platforms: Iterable[str],
    modules: Iterable[str],
    boundaries: Iterable[str],
    assertions: dict[str, bool],
    real_provider_requests: int = 0,
    assertion_evidence: dict[str, Iterable[str]] | None = None,
    observations: Iterable[dict[str, Any]] = (),
    metrics: dict[str, Any] | None = None,
    artifacts: Iterable[str | Path] = (),
) -> Path | None:
    """Atomically add one independently verified step to the active receipt."""
    step_platforms = _strings(platforms)
    step_modules = _strings(modules)
    step_boundaries = _strings(boundaries)
    configured = os.environ.get("NOLA_E2E_RECEIPT_PATH", "").strip()
    if not configured:
        return None
    path = Path(configured)
    scenario_id = os.environ.get("NOLA_E2E_SCENARIO_ID", "").strip()
    execution_id = os.environ.get("NOLA_E2E_EXECUTION_ID", "").strip()
    if not scenario_id or not execution_id:
        raise RuntimeError("structured E2E evidence requires scenario and execution IDs")

    if path.exists():
        receipt = json.loads(path.read_text(encoding="utf-8"))
        if receipt.get("schemaVersion") != SCHEMA_VERSION:
            raise RuntimeError("refusing to append to an unsupported evidence receipt schema")
        if receipt.get("scenarioId") != scenario_id or receipt.get("executionId") != execution_id:
            raise RuntimeError("refusing to append to a stale or foreign E2E receipt")
    else:
        receipt = {
            "schemaVersion": SCHEMA_VERSION,
            "scenarioId": scenario_id,
            "executionId": execution_id,
            "platforms": [],
            "modules": [],
            "boundaries": [],
            "realProviderRequests": 0,
            "claimedRealProviderRequests": 0,
            "steps": [],
        }

    observation_rows = [
        _materialize_observation(
            dict(item),
            receipt_path=path,
            step_id=step_id,
        )
        for item in observations
    ]
    evidence_by_assertion = assertion_evidence or {}
    assertion_rows = [
        {
            "id": key,
            "passed": bool(value),
            "evidence": _strings(evidence_by_assertion.get(key, [])),
        }
        for key, value in sorted(assertions.items())
    ]
    artifact_rows = []
    for artifact in artifacts:
        artifact_path = Path(artifact).expanduser().resolve()
        artifact_rows.append({
            "path": str(artifact_path),
            "exists": artifact_path.exists(),
            "bytes": artifact_path.stat().st_size if artifact_path.is_file() else None,
        })
    step = {
        "id": step_id,
        "platforms": step_platforms,
        "modules": step_modules,
        "boundaries": step_boundaries,
        "status": "passed" if assertion_rows and all(row["passed"] for row in assertion_rows) else "failed",
        "assertions": assertion_rows,
        "metrics": metrics or {},
        "artifacts": artifact_rows,
        "observations": observation_rows,
    }
    receipt["steps"] = [item for item in receipt.get("steps", []) if item.get("id") != step_id]
    receipt["steps"].append(step)
    receipt["platforms"] = _strings([*receipt.get("platforms", []), *step_platforms])
    receipt["modules"] = _strings([*receipt.get("modules", []), *step_modules])
    receipt["boundaries"] = _strings([*receipt.get("boundaries", []), *step_boundaries])
    receipt["claimedRealProviderRequests"] = int(
        receipt.get("claimedRealProviderRequests", 0)
    ) + max(0, int(real_provider_requests))
    receipt["realProviderRequests"] = sum(
        int(observation.get("metrics", {}).get("successfulModelRequests", 0))
        for recorded_step in receipt["steps"]
        for observation in recorded_step.get("observations", [])
        if observation.get("kind") == "provider_trace"
    )

    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            json.dump(receipt, handle, ensure_ascii=False, indent=2)
            handle.write("\n")
        os.replace(temporary, path)
    finally:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass
    return path


def record_process_failure(
    receipt_path: Path,
    *,
    scenario_id: str,
    execution_id: str,
    platforms: Iterable[str],
    modules: Iterable[str],
    boundaries: Iterable[str],
    process_log: Path,
    return_code: int,
    timed_out: bool,
    skip_markers: Iterable[str] = (),
) -> Path:
    """Create a durable failed receipt when an adapter dies before reporting evidence."""
    if receipt_path.exists():
        return receipt_path
    observation = _materialize_observation(
        {"id": "process-log", "kind": "process_log", "path": str(process_log)},
        receipt_path=receipt_path,
        step_id="scenario-process",
    )
    platform_rows = _strings(platforms)
    module_rows = _strings(modules)
    boundary_rows = _strings(boundaries)
    receipt = {
        "schemaVersion": SCHEMA_VERSION,
        "scenarioId": scenario_id,
        "executionId": execution_id,
        "platforms": platform_rows,
        "modules": module_rows,
        "boundaries": boundary_rows,
        "realProviderRequests": 0,
        "claimedRealProviderRequests": 0,
        "steps": [{
            "id": "scenario-process",
            "platforms": platform_rows,
            "modules": module_rows,
            "boundaries": boundary_rows,
            "status": "failed",
            "assertions": [{
                "id": "process_exited_successfully",
                "passed": False,
                "evidence": ["process-log"],
            }],
            "metrics": {
                "returnCode": int(return_code),
                "timedOut": bool(timed_out),
                "skipMarkers": _strings(skip_markers),
            },
            "artifacts": [],
            "observations": [observation],
        }],
    }
    receipt_path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix=f".{receipt_path.name}.", dir=receipt_path.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            json.dump(receipt, handle, ensure_ascii=False, indent=2)
            handle.write("\n")
        os.replace(temporary, receipt_path)
    finally:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass
    return receipt_path


def attach_observation(
    receipt_path: Path,
    *,
    step_id: str,
    observation: dict[str, Any],
) -> None:
    """Attach runner-produced evidence to an existing current execution receipt."""
    receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
    if receipt.get("schemaVersion") != SCHEMA_VERSION:
        raise ValueError("unsupported evidence receipt schema")
    steps = receipt.get("steps") if isinstance(receipt.get("steps"), list) else []
    step = next((item for item in steps if item.get("id") == step_id), None)
    if step is None:
        raise ValueError(f"receipt step does not exist: {step_id}")
    row = _materialize_observation(
        observation,
        receipt_path=receipt_path,
        step_id=step_id,
    )
    observations = step.get("observations") if isinstance(step.get("observations"), list) else []
    step["observations"] = [
        item for item in observations if item.get("id") != row.get("id")
    ] + [row]
    descriptor, temporary = tempfile.mkstemp(prefix=f".{receipt_path.name}.", dir=receipt_path.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            json.dump(receipt, handle, ensure_ascii=False, indent=2)
            handle.write("\n")
        os.replace(temporary, receipt_path)
    finally:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass


def attach_visual_review(
    receipt_path: Path,
    *,
    step_id: str,
    observation_id: str,
    observer: str,
    observed_at: str,
    method: str,
    verdict: str,
    findings: Iterable[str],
) -> None:
    """Record a review only after a person or visual agent opened the artifact."""
    receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
    if receipt.get("schemaVersion") != SCHEMA_VERSION:
        raise ValueError("unsupported evidence receipt schema")
    steps = receipt.get("steps") if isinstance(receipt.get("steps"), list) else []
    step = next((item for item in steps if item.get("id") == step_id), None)
    if step is None:
        raise ValueError(f"receipt step does not exist: {step_id}")
    observations = step.get("observations") if isinstance(step.get("observations"), list) else []
    observation = next(
        (item for item in observations if item.get("id") == observation_id),
        None,
    )
    if observation is None:
        raise ValueError(f"receipt observation does not exist: {observation_id}")
    if observation.get("kind") not in {"ui_screenshot", "ui_recording"}:
        raise ValueError("visual reviews may only annotate screenshots or recordings")
    normalized_findings = _strings(findings)
    normalized_verdict = verdict.strip().lower()
    if (
        not observer.strip()
        or not observed_at.strip()
        or method.strip() not in {"opened_image", "watched_recording"}
        or normalized_verdict not in {"passed", "failed"}
        or not normalized_findings
    ):
        raise ValueError("visual review requires observer, time, method, verdict, and findings")
    observation["visualReview"] = {
        "observer": observer.strip(),
        "observedAt": observed_at.strip(),
        "method": method.strip(),
        "verdict": normalized_verdict,
        "findings": normalized_findings,
    }
    descriptor, temporary = tempfile.mkstemp(prefix=f".{receipt_path.name}.", dir=receipt_path.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            json.dump(receipt, handle, ensure_ascii=False, indent=2)
            handle.write("\n")
        os.replace(temporary, receipt_path)
    finally:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass


def validate_receipt(path: Path, scenario: dict, execution_id: str) -> tuple[dict | None, list[str]]:
    errors: list[str] = []
    try:
        receipt = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return None, ["structured evidence receipt was not produced"]
    except (OSError, json.JSONDecodeError) as error:
        return None, [f"structured evidence receipt is unreadable: {error}"]

    if receipt.get("schemaVersion") != SCHEMA_VERSION:
        errors.append("unsupported evidence receipt schema")
    if receipt.get("scenarioId") != scenario.get("id"):
        errors.append("receipt scenario ID does not match the running scenario")
    if receipt.get("executionId") != execution_id:
        errors.append("receipt execution ID is stale or foreign")

    contract = scenario.get("evidence", {})
    scenario_contract = scenario.get("_contract", {})
    for key, receipt_key in (("platforms", "platforms"), ("modules", "modules"), ("boundaries", "boundaries")):
        missing = set(contract.get(key, [])) - set(receipt.get(receipt_key, []))
        if missing:
            errors.append(f"missing {key}: {', '.join(sorted(missing))}")
    steps = receipt.get("steps") if isinstance(receipt.get("steps"), list) else []
    required_observation_kinds = set(contract.get("requiredObservationKinds", [])) | set(
        scenario_contract.get("requiredObservationKinds", [])
    )
    if required_observation_kinds and receipt.get("schemaVersion") != SCHEMA_VERSION:
        errors.append("scenario requires structured evidence observations")
    if scenario.get("execution", {}).get("realUI") and receipt.get("schemaVersion") != SCHEMA_VERSION:
        errors.append("real UI scenarios require current evidence schema visual reviews")
    minimum_steps = max(
        int(contract.get("minimumSteps", 1)),
        int(scenario_contract.get("minimumSteps", 1)),
    )
    if len(steps) < minimum_steps:
        errors.append("receipt has fewer independently verified steps than required")
    if not steps:
        errors.append("receipt has no verified steps")
    for step in steps:
        if step.get("status") != "passed":
            errors.append(f"step did not pass: {step.get('id', '<unknown>')}")
        assertions = step.get("assertions") if isinstance(step.get("assertions"), list) else []
        if not assertions or any(item.get("passed") is not True for item in assertions):
            errors.append(f"step has missing or failed assertions: {step.get('id', '<unknown>')}")
        for artifact in step.get("artifacts", []):
            if artifact.get("exists") is not True:
                errors.append(f"recorded artifact did not exist: {artifact.get('path', '<unknown>')}")
        observations = step.get("observations") if isinstance(step.get("observations"), list) else []
        observation_ids = {item.get("id") for item in observations}
        for observation in observations:
            path_value = observation.get("path")
            path = Path(path_value) if isinstance(path_value, str) else None
            if path is None or not path.is_file():
                errors.append(f"observation is missing: {observation.get('id', '<unknown>')}")
                continue
            if observation.get("sha256") != _file_sha256(path):
                errors.append(f"observation changed after recording: {observation.get('id', '<unknown>')}")
            if observation.get("kind") == "provider_trace":
                current = provider_trace_metrics([path])
                if current.get("malformedRows"):
                    errors.append("provider trace contains malformed rows")
                hosts = set(current.get("upstreamHosts", []))
                if hosts and hosts.issubset({"127.0.0.1", "localhost", "::1"}):
                    errors.append("provider trace reached only loopback upstream hosts")
            if observation.get("kind") == "ui_screenshot" and path.stat().st_size < 1024:
                errors.append(f"UI screenshot is empty or implausibly small: {observation.get('id', '<unknown>')}")
            if observation.get("kind") in {"ui_screenshot", "ui_recording"}:
                review = observation.get("visualReview")
                expected_method = (
                    "opened_image"
                    if observation.get("kind") == "ui_screenshot"
                    else "watched_recording"
                )
                if (
                    not isinstance(review, dict)
                    or not str(review.get("observer") or "").strip()
                    or not str(review.get("observedAt") or "").strip()
                    or review.get("method") != expected_method
                    or review.get("verdict") not in {"passed", "failed"}
                    or not isinstance(review.get("findings"), list)
                    or not any(str(item).strip() for item in review.get("findings", []))
                ):
                    errors.append(
                        f"visual artifact was captured but not actually observed: "
                        f"{observation.get('id', '<unknown>')}"
                    )
                elif review.get("verdict") != "passed":
                    errors.append(
                        f"visual observation failed: {observation.get('id', '<unknown>')}"
                    )
            if observation.get("kind") == "accessibility_tree" and path.stat().st_size < 64:
                errors.append(f"accessibility evidence is empty: {observation.get('id', '<unknown>')}")
            if observation.get("kind") == "persistence_audit":
                try:
                    audit = json.loads(path.read_text(encoding="utf-8"))
                except (OSError, UnicodeDecodeError, json.JSONDecodeError):
                    errors.append(f"persistence audit is unreadable: {observation.get('id', '<unknown>')}")
                else:
                    if not isinstance(audit, dict) or audit.get("passed") is not True:
                        errors.append(f"persistence audit did not pass: {observation.get('id', '<unknown>')}")
            if observation.get("kind") == "semantic_audit":
                try:
                    audit = json.loads(path.read_text(encoding="utf-8"))
                except (OSError, UnicodeDecodeError, json.JSONDecodeError):
                    errors.append(f"semantic audit is unreadable: {observation.get('id', '<unknown>')}")
                else:
                    if (
                        not isinstance(audit, dict)
                        or audit.get("kind") != "real_model_response_judgement"
                        or audit.get("passed") is not True
                        or not str(audit.get("judge_model") or "").strip()
                    ):
                        errors.append(f"semantic audit did not pass: {observation.get('id', '<unknown>')}")
        if contract.get("requireAssertionEvidence"):
            for assertion in assertions:
                references = set(assertion.get("evidence", []))
                if not references:
                    errors.append(f"assertion has no evidence reference: {assertion.get('id', '<unknown>')}")
                elif not references.issubset(observation_ids):
                    errors.append(f"assertion references unknown evidence: {assertion.get('id', '<unknown>')}")
    observed_kinds = {
        observation.get("kind")
        for step in steps
        for observation in step.get("observations", [])
    }
    missing_observation_kinds = required_observation_kinds - observed_kinds
    if missing_observation_kinds:
        errors.append(
            "missing observation kinds: " + ", ".join(sorted(missing_observation_kinds))
        )
    passed_assertions = {
        assertion.get("id")
        for step in steps
        for assertion in step.get("assertions", [])
        if assertion.get("passed") is True
    }
    missing_assertions = set(scenario_contract.get("requiredAssertions", [])) - passed_assertions
    if missing_assertions:
        errors.append(
            "missing contract assertions: " + ", ".join(sorted(missing_assertions))
        )
    if scenario_contract.get("perPlatformSteps") is True:
        for platform in scenario.get("platforms", []):
            if not any(step.get("platforms") == [platform] for step in steps):
                errors.append(f"missing independent platform step: {platform}")
    required_chains = contract.get("requiredChains")
    if required_chains is None:
        required_chains = [
            {
                "id": f"{platform}-declared-product-chain",
                "platform": platform,
                "modules": contract.get("modules", []),
                "boundaries": contract.get("boundaries", []),
            }
            for platform in scenario.get("platforms", [])
        ]
    if not isinstance(required_chains, list):
        errors.append("scenario requiredChains must be a list")
        required_chains = []
    for index, chain in enumerate(required_chains):
        if not isinstance(chain, dict):
            errors.append(f"invalid required chain at index {index}")
            continue
        chain_id = str(chain.get("id") or f"chain-{index + 1}")
        platform = str(chain.get("platform") or "").strip()
        modules = set(chain.get("modules", []))
        boundaries = set(chain.get("boundaries", []))
        assertions = set(chain.get("assertions", []))
        matching = []
        for step in steps:
            if platform and step.get("platforms") != [platform]:
                continue
            step_modules = set(step.get("modules", []))
            step_boundaries = set(step.get("boundaries", []))
            step_assertions = {
                item.get("id")
                for item in step.get("assertions", [])
                if item.get("passed") is True
            }
            if (
                modules.issubset(step_modules)
                and boundaries.issubset(step_boundaries)
                and assertions.issubset(step_assertions)
            ):
                matching.append(step)
        if not matching:
            errors.append(f"required product chain was not observed in one step: {chain_id}")
    if scenario.get("realModel") and int(receipt.get("realProviderRequests", 0)) < 1:
        errors.append("real-model scenario recorded no provider request")
    from real_e2e_judge import evaluate_receipt

    judge = evaluate_receipt(receipt, scenario)
    receipt["judge"] = judge
    errors.extend(judge["errors"])
    return receipt, errors
