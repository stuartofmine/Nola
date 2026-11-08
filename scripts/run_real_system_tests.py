#!/usr/bin/env python3
"""Execute auditable, no-mock Nola test chains across product platforms."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
from pathlib import Path
import shlex
import signal
import subprocess
import sys
import time
import uuid

from real_e2e_evidence import attach_observation, record_process_failure, validate_receipt
from real_e2e_coverage import audit_coverage, validate_execution_claim


ROOT = Path(__file__).resolve().parent.parent
DEFAULT_MATRIX = ROOT / "scripts" / "real_e2e_matrix.json"
SKIP_MARKERS = (
    "xctskip",
    "pytest.skip",
    " skipped,",
    " skipped in ",
    "tests did not execute",
)


def load_matrix(path: Path) -> list[dict]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if value.get("schemaVersion") != 6:
        raise ValueError(f"unsupported matrix schema: {value.get('schemaVersion')!r}")
    contracts = value.get("contracts")
    if not isinstance(contracts, dict) or not contracts:
        raise ValueError("real E2E matrix has no shared scenario contracts")
    for contract_id, contract in contracts.items():
        if not isinstance(contract_id, str) or not contract_id:
            raise ValueError("every shared scenario contract requires a stable id")
        if not isinstance(contract, dict):
            raise ValueError(f"scenario contract {contract_id} is not an object")
        required = contract.get("requiredAssertions")
        if not isinstance(required, list) or not required or not all(
            isinstance(item, str) and item for item in required
        ):
            raise ValueError(f"scenario contract {contract_id} has no required assertions")
        validators = contract.get("validators")
        if not isinstance(validators, list) or not all(
            isinstance(item, str) and item for item in validators
        ):
            raise ValueError(f"scenario contract {contract_id} has invalid central validators")
        observation_kinds = contract.get("requiredObservationKinds")
        if not isinstance(observation_kinds, list) or not all(
            isinstance(item, str) and item for item in observation_kinds
        ):
            raise ValueError(
                f"scenario contract {contract_id} has invalid required observation kinds"
            )
    scenarios = value.get("scenarios")
    if not isinstance(scenarios, list) or not scenarios:
        raise ValueError("real E2E matrix has no scenarios")
    identifiers: set[str] = set()
    for scenario in scenarios:
        identifier = scenario.get("id")
        if not isinstance(identifier, str) or not identifier:
            raise ValueError("every scenario requires a stable id")
        if identifier in identifiers:
            raise ValueError(f"duplicate scenario id: {identifier}")
        identifiers.add(identifier)
        contract_id = scenario.get("contractId")
        if contract_id not in contracts:
            raise ValueError(f"scenario {identifier} references unknown contract {contract_id!r}")
        scenario["_contract"] = contracts[contract_id]
        execution_errors = validate_execution_claim(scenario)
        if execution_errors:
            raise ValueError("; ".join(execution_errors))
        if not scenario.get("platforms") or not scenario.get("layer"):
            raise ValueError(f"scenario {identifier} lacks platform or layer evidence")
        if not scenario.get("command") or int(scenario.get("timeoutSeconds", 0)) <= 0:
            raise ValueError(f"scenario {identifier} lacks an executable timeout contract")
        evidence = scenario.get("evidence")
        if scenario.get("realModel") and not isinstance(evidence, dict):
            raise ValueError(f"real-model scenario {identifier} lacks structured evidence")
        if isinstance(evidence, dict):
            for field in ("platforms", "modules", "boundaries"):
                if not evidence.get(field):
                    raise ValueError(f"scenario {identifier} evidence lacks {field}")
            boundaries = set(evidence.get("boundaries", []))
            if scenario.get("layer") == "product-ui" and not any(
                boundary.endswith("-ui") or boundary == "web-ui" for boundary in boundaries
            ):
                raise ValueError(f"product-ui scenario {identifier} lacks a real UI boundary")
    return scenarios


def select_scenarios(
    scenarios: list[dict], suite: str | None, platforms: set[str], identifiers: set[str]
) -> list[dict]:
    selected = []
    for scenario in scenarios:
        if suite is not None and suite not in scenario.get("suite", []):
            continue
        if platforms and not platforms.intersection(scenario["platforms"]):
            continue
        if identifiers and scenario["id"] not in identifiers:
            continue
        selected.append(scenario)
    return selected


def validate_real_configuration(config_path: Path) -> None:
    if not config_path.is_file():
        raise ValueError(f"missing real LLM configuration: {config_path}")
    value = json.loads(config_path.read_text(encoding="utf-8"))
    llm = value.get("llm", {})
    missing = [key for key in ("endpoint", "api_key", "model") if not llm.get(key)]
    if missing:
        raise ValueError(f"real LLM configuration is missing: {', '.join(missing)}")


def run_scenario(scenario: dict, artifact_dir: Path, environment: dict[str, str]) -> dict:
    identifier = scenario["id"]
    log_path = artifact_dir / f"{identifier}.log"
    started = time.monotonic()
    timed_out = False
    execution_id = uuid.uuid4().hex
    receipt_path = artifact_dir / f"{identifier}.evidence.json"
    child_environment = environment.copy()
    child_environment.update({
        "NOLA_E2E_SCENARIO_ID": identifier,
        "NOLA_E2E_EXECUTION_ID": execution_id,
        "NOLA_E2E_RECEIPT_PATH": str(receipt_path),
        "NOLA_E2E_ARTIFACT_ROOT": str(artifact_dir),
    })
    with log_path.open("w", encoding="utf-8") as log:
        log.write(f"scenario={identifier}\n")
        log.write(f"platforms={','.join(scenario['platforms'])}\n")
        log.write(f"layer={scenario['layer']}\n")
        log.write(f"command={scenario['command']}\n\n")
        log.flush()
        process = None
        try:
            process = subprocess.Popen(
                # Preserve the caller's audited toolchain. Login shells source
                # user profiles and can silently replace Node/Python/Cargo or
                # provider CLIs with incompatible versions.
                ["bash", "-c", scenario["command"]],
                cwd=ROOT,
                env=child_environment,
                stdout=log,
                stderr=subprocess.STDOUT,
                text=True,
                start_new_session=True,
            )
            timeout_seconds = int(scenario["timeoutSeconds"])
            heartbeat_seconds = max(
                1, int(environment.get("NOLA_E2E_HEARTBEAT_SECONDS", "30"))
            )
            next_heartbeat = time.monotonic() + heartbeat_seconds
            while process.poll() is None:
                elapsed = time.monotonic() - started
                if elapsed >= timeout_seconds:
                    timed_out = True
                    os.killpg(process.pid, signal.SIGTERM)
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait()
                    log.write(f"\nTIMEOUT after {timeout_seconds} seconds\n")
                    log.flush()
                    break
                if time.monotonic() >= next_heartbeat:
                    phase = ""
                    try:
                        with log_path.open("r", encoding="utf-8", errors="replace") as stream:
                            for line in stream:
                                stripped = line.strip()
                                if stripped.startswith("[") and "]" in stripped:
                                    phase = stripped
                    except OSError:
                        pass
                    phase_suffix = f"  phase={phase[:160]}" if phase else ""
                    print(
                        f"  RUNNING {elapsed:.0f}s{phase_suffix}  log={log_path}",
                        flush=True,
                    )
                    next_heartbeat += heartbeat_seconds
                time.sleep(min(0.25, max(0.01, timeout_seconds - elapsed)))
            return_code = 124 if timed_out else int(process.returncode or 0)
        except BaseException:
            if process is not None and process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
            raise
    duration = round(time.monotonic() - started, 3)
    output = log_path.read_text(encoding="utf-8", errors="replace").lower()
    skip_markers = [marker for marker in SKIP_MARKERS if marker in output]
    if not receipt_path.exists():
        contract = scenario.get("evidence", {})
        record_process_failure(
            receipt_path,
            scenario_id=identifier,
            execution_id=execution_id,
            platforms=scenario["platforms"],
            modules=contract.get("modules", ["scenario-runner"]),
            boundaries=contract.get("boundaries", ["process"]),
            process_log=log_path,
            return_code=return_code,
            timed_out=timed_out,
            skip_markers=skip_markers,
        )
    if scenario.get("execution", {}).get("semanticJudge") is True:
        try:
            preliminary = json.loads(receipt_path.read_text(encoding="utf-8"))
            observations = [
                observation
                for step in preliminary.get("steps", [])
                for observation in step.get("observations", [])
            ]
            if not any(item.get("kind") == "semantic_audit" for item in observations):
                transcripts = [item for item in observations if item.get("kind") == "transcript"]
                if not transcripts:
                    raise RuntimeError("semantic Judge requires a transcript observation")
                evidence_paths = [
                    str(item["path"])
                    for item in observations
                    if item.get("kind") in {"event_stream", "runtime_receipt", "persistence_audit"}
                    and item.get("path")
                ]
                semantic_path = artifact_dir / f"{identifier}.semantic-audit.json"
                command = [
                    sys.executable,
                    str(ROOT / "scripts" / "judge_real_e2e_response.py"),
                    "--config", environment["NOLA_CONFIG_PATH"],
                    "--transcript", str(transcripts[0]["path"]),
                    "--receipt", str(semantic_path),
                    "--current-date", dt.date.today().isoformat(),
                ]
                for evidence_path in evidence_paths:
                    command.extend(["--evidence-file", evidence_path])
                judged = subprocess.run(
                    command,
                    cwd=ROOT,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT,
                    text=True,
                    timeout=240,
                    check=False,
                )
                with log_path.open("a", encoding="utf-8") as log:
                    log.write("\n[real-e2e] independent semantic Judge\n")
                    log.write(judged.stdout)
                if semantic_path.is_file():
                    attach_observation(
                        receipt_path,
                        step_id=str(preliminary["steps"][0]["id"]),
                        observation={
                            "id": "runner-semantic-audit",
                            "kind": "semantic_audit",
                            "path": semantic_path,
                        },
                    )
        except (OSError, ValueError, KeyError, RuntimeError, subprocess.TimeoutExpired) as error:
            with log_path.open("a", encoding="utf-8") as log:
                log.write(f"\n[real-e2e] semantic Judge setup failed: {error}\n")
    receipt, evidence_errors = validate_receipt(receipt_path, scenario, execution_id)
    passed = return_code == 0 and not timed_out and not skip_markers and not evidence_errors
    return {
        "id": identifier,
        "platforms": scenario["platforms"],
        "layer": scenario["layer"],
        "realModel": bool(scenario.get("realModel")),
        "passed": passed,
        "returnCode": return_code,
        "timedOut": timed_out,
        "skipMarkers": skip_markers,
        "evidenceErrors": evidence_errors,
        "evidence": receipt,
        "receipt": str(receipt_path),
        "durationSeconds": duration,
        "log": str(log_path),
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--matrix", type=Path, default=DEFAULT_MATRIX)
    parser.add_argument("--suite", choices=("smoke", "full"))
    parser.add_argument("--platform", action="append", default=[])
    parser.add_argument("--scenario", action="append", default=[])
    parser.add_argument("--list", action="store_true", help="list selected scenarios without running")
    parser.add_argument("--artifacts", type=Path)
    parser.add_argument("--config", type=Path, default=Path.home() / ".nola" / "config.json")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    scenarios = load_matrix(args.matrix)
    effective_suite = args.suite or (None if args.scenario else "smoke")
    selected = select_scenarios(
        scenarios, effective_suite, set(args.platform), set(args.scenario)
    )
    if not selected:
        print("No real E2E scenario matched the requested matrix slice.", file=sys.stderr)
        return 2
    if args.list:
        for scenario in selected:
            platforms = ",".join(scenario["platforms"])
            print(f"{scenario['id']}\t{platforms}\t{scenario['layer']}\t{scenario['command']}")
        return 0

    if any(scenario.get("realModel") for scenario in selected):
        try:
            validate_real_configuration(args.config.expanduser())
        except (OSError, ValueError, json.JSONDecodeError) as error:
            print(str(error), file=sys.stderr)
            return 2

    stamp = dt.datetime.now().strftime("%Y%m%d-%H%M%S")
    artifact_dir = args.artifacts or ROOT / ".artifacts" / "real-e2e" / stamp
    artifact_dir.mkdir(parents=True, exist_ok=True)
    environment = os.environ.copy()
    environment["NOLA_CONFIG_PATH"] = str(args.config.expanduser())
    environment["NOLA_REAL_LLM_E2E"] = "1"
    environment["NOLA_RUN_REAL_MODEL_E2E"] = "1"

    results = []
    for index, scenario in enumerate(selected, 1):
        print(f"[{index}/{len(selected)}] {scenario['id']} ...", flush=True)
        result = run_scenario(scenario, artifact_dir, environment)
        results.append(result)
        state = "PASS" if result["passed"] else "FAIL"
        print(f"  {state} {result['durationSeconds']:.1f}s  {result['log']}", flush=True)

    covered_platforms = sorted({p for result in results for p in result["platforms"]})
    passed_ids = {result["id"] for result in results if result["passed"]}
    coverage_suite = effective_suite if effective_suite in {"smoke", "full"} else "smoke"
    coverage = audit_coverage(
        selected,
        suite=coverage_suite,
        passed_scenario_ids=passed_ids,
        evidence_by_scenario={
            result["id"]: result["evidence"]
            for result in results
            if result["passed"] and isinstance(result.get("evidence"), dict)
        },
    )
    execution_complete = len(results) == len(selected) and all(result["passed"] for result in results)
    explicit_slice = bool(args.scenario or args.platform)
    scope_requires_coverage = effective_suite in {"smoke", "full"} and not explicit_slice
    report = {
        "schemaVersion": 1,
        "suite": (
            f"{effective_suite}-filtered"
            if explicit_slice and effective_suite in {"smoke", "full"}
            else effective_suite or "explicit-scenarios"
        ),
        "selectedCount": len(selected),
        "executedCount": len(results),
        "passedCount": sum(1 for result in results if result["passed"]),
        "coveredPlatforms": covered_platforms,
        "executionComplete": execution_complete,
        "coverage": coverage,
        "scopeComplete": coverage["complete"] if scope_requires_coverage else None,
        "complete": execution_complete and (coverage["complete"] if scope_requires_coverage else True),
        "results": results,
    }
    report_path = artifact_dir / "report.json"
    report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"report={report_path}")
    return 0 if report["complete"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
