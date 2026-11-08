#!/usr/bin/env python3
"""Audit whether a real-E2E selection proves each claimed product boundary."""

from __future__ import annotations

from collections import defaultdict
import argparse
import json
from pathlib import Path
import sys
from typing import Any, Iterable


EXECUTION_FIELDS = {
    "runtime",
    "productionCode",
    "productEntrypoint",
    "realProvider",
    "realPersistence",
    "realUI",
    "semanticJudge",
    "isolation",
}

CLIENT_PLATFORMS = ("cli", "web", "mac", "ios", "android")

RUNTIME_PLATFORMS = {
    "native-process": {"core", "cli", "mac", "backend"},
    "native-app": {"mac"},
    "terminal-pty": {"cli"},
    "http-service": {"web", "backend"},
    "local-service-stack": {"cli", "web", "backend"},
    "browser": {"web"},
    "simulator": {"ios"},
    "emulator": {"android"},
    "simulator-and-emulator": {"ios", "android"},
}

SMOKE_REQUIREMENTS = {
    "core": {"contract"},
    "cli": {"product-entrypoint", "real-provider", "real-persistence", "terminal-ui", "semantic-judge"},
    "web": {"product-entrypoint", "real-provider", "real-persistence", "product-ui", "semantic-judge"},
    "mac": {"product-entrypoint", "real-provider", "real-persistence", "product-ui", "semantic-judge"},
    "ios": {"product-entrypoint", "real-provider", "real-persistence", "product-ui", "semantic-judge"},
    "android": {"product-entrypoint", "real-provider", "real-persistence", "product-ui", "semantic-judge"},
    "backend": {"product-entrypoint", "real-persistence", "http-api"},
}

FULL_REQUIREMENTS = {
    **SMOKE_REQUIREMENTS,
}

SMOKE_MODULE_REQUIREMENTS = {
    "core": {"core"},
    "cli": {"agent", "core", "provider", "tools", "persistence", "terminal"},
    "web": {"agent", "core", "provider", "tools", "skills", "persistence", "ui"},
    "mac": {
        "agent", "core", "provider", "tools", "skills", "permissions",
        "persistence", "rendering", "ui",
    },
    "ios": {"agent", "core", "provider", "tools", "persistence", "rendering", "ui"},
    "android": {"agent", "core", "provider", "data-source", "skills", "persistence", "ui"},
    "backend": {"auth", "chat", "database", "http"},
}

FULL_MODULE_REQUIREMENTS = {
    **SMOKE_MODULE_REQUIREMENTS,
    "cli": SMOKE_MODULE_REQUIREMENTS["cli"] | {
        "audit", "cancellation", "data-source", "memory", "permissions", "skills",
    },
    "web": SMOKE_MODULE_REQUIREMENTS["web"] | {
        "audit", "cancellation", "data-source", "memory", "permissions",
    },
    "mac": SMOKE_MODULE_REQUIREMENTS["mac"] | {
        "audit", "cancellation", "compaction", "data-source", "memory",
    },
    "ios": SMOKE_MODULE_REQUIREMENTS["ios"] | {
        "audit", "data-source", "permissions", "skills", "streaming",
    },
    "android": SMOKE_MODULE_REQUIREMENTS["android"] | {
        "audit", "permissions", "streaming",
    },
}


def validate_execution_claim(scenario: dict[str, Any]) -> list[str]:
    identifier = scenario.get("id", "<unknown>")
    execution = scenario.get("execution")
    if not isinstance(execution, dict):
        return [f"scenario {identifier} lacks execution fidelity metadata"]
    missing = EXECUTION_FIELDS - set(execution)
    errors = [
        f"scenario {identifier} execution metadata lacks: {', '.join(sorted(missing))}"
    ] if missing else []
    for key in (
        "productionCode", "productEntrypoint", "realProvider",
        "realPersistence", "realUI", "semanticJudge",
    ):
        if key in execution and not isinstance(execution[key], bool):
            errors.append(f"scenario {identifier} execution.{key} must be boolean")
    if execution.get("realUI") and scenario.get("layer") != "product-ui":
        errors.append(f"scenario {identifier} claims real UI outside product-ui layer")
    if execution.get("realProvider") != bool(scenario.get("realModel")):
        errors.append(f"scenario {identifier} real provider claim disagrees with realModel")
    runtime = execution.get("runtime")
    platforms = {str(item) for item in scenario.get("platforms", [])}
    allowed_platforms = RUNTIME_PLATFORMS.get(str(runtime))
    if allowed_platforms is None:
        errors.append(f"scenario {identifier} has unsupported runtime: {runtime}")
    elif not platforms or not platforms.issubset(allowed_platforms):
        errors.append(
            f"scenario {identifier} runtime {runtime} is incompatible with platforms: "
            + ", ".join(sorted(platforms))
        )
    observed = set(scenario.get("_contract", {}).get("requiredObservationKinds", []))
    observed.update(scenario.get("evidence", {}).get("requiredObservationKinds", []))
    if execution.get("realUI"):
        required = (
            {"terminal_capture", "terminal_metrics"}
            if execution.get("runtime") == "terminal-pty"
            else {"ui_screenshot", "accessibility_tree", "ui_timeline"}
        )
        missing_ui = required - observed
        if missing_ui:
            errors.append(
                f"scenario {identifier} real UI claim lacks observations: "
                + ", ".join(sorted(missing_ui))
            )
    if execution.get("realPersistence") and not {
        "persistence_store", "persistence_audit"
    }.intersection(observed):
        errors.append(
            f"scenario {identifier} real persistence claim lacks a persistence_store or persistence_audit observation"
        )
    if execution.get("semanticJudge") and "semantic_audit" not in observed:
        errors.append(
            f"scenario {identifier} semantic judge claim lacks semantic_audit observation"
        )
    chains = scenario.get("evidence", {}).get("requiredChains", [])
    if not isinstance(chains, list):
        errors.append(f"scenario {identifier} requiredChains must be a list")
    else:
        for index, chain in enumerate(chains):
            if not isinstance(chain, dict):
                errors.append(f"scenario {identifier} has invalid required chain {index + 1}")
                continue
            if "platforms" in chain:
                errors.append(
                    f"scenario {identifier} required chain {index + 1} uses invalid platforms field"
                )
            platform = chain.get("platform")
            if not isinstance(platform, str) or platform not in platforms:
                errors.append(
                    f"scenario {identifier} required chain {index + 1} lacks a valid platform"
                )
    return errors


def observed_scenario_lanes(scenario: dict[str, Any], receipt: dict[str, Any]) -> set[str]:
    """Derive execution lanes from immutable observations, not matrix claims."""
    lanes = scenario_lanes(scenario)
    kinds = {
        str(observation.get("kind") or "")
        for step in receipt.get("steps", [])
        for observation in step.get("observations", [])
    }
    if int(receipt.get("realProviderRequests", 0) or 0) < 1:
        lanes.discard("real-provider")
    if not {"persistence_store", "persistence_audit"}.intersection(kinds):
        lanes.discard("real-persistence")
    if scenario.get("execution", {}).get("realUI"):
        required_ui = (
            {"terminal_capture", "terminal_metrics"}
            if scenario.get("execution", {}).get("runtime") == "terminal-pty"
            else {"ui_screenshot", "accessibility_tree", "ui_timeline"}
        )
        visual_artifacts = [
            observation
            for step in receipt.get("steps", [])
            for observation in step.get("observations", [])
            if observation.get("kind") in {"ui_screenshot", "ui_recording"}
        ]
        visual_reviews_passed = bool(visual_artifacts) and all(
            isinstance(observation.get("visualReview"), dict)
            and observation["visualReview"].get("verdict") == "passed"
            and bool(observation["visualReview"].get("findings"))
            for observation in visual_artifacts
        )
        if not required_ui.issubset(kinds) or not visual_reviews_passed:
            lanes.discard("product-ui")
            lanes.discard("terminal-ui")
    if "semantic_audit" not in kinds:
        lanes.discard("semantic-judge")
    return lanes


def scenario_lanes(scenario: dict[str, Any]) -> set[str]:
    execution = scenario.get("execution", {})
    lanes: set[str] = set()
    if scenario.get("layer") == "contract":
        lanes.add("contract")
    if execution.get("productionCode"):
        lanes.add("production-code")
    if execution.get("productEntrypoint"):
        lanes.add("product-entrypoint")
    if execution.get("realProvider"):
        lanes.add("real-provider")
    if execution.get("realPersistence"):
        lanes.add("real-persistence")
    if execution.get("realUI"):
        lanes.add("product-ui")
    if execution.get("semanticJudge"):
        lanes.add("semantic-judge")
    runtime = execution.get("runtime")
    if runtime == "terminal-pty" and execution.get("realUI"):
        lanes.add("terminal-ui")
    if runtime == "http-service":
        lanes.add("http-api")
    return lanes


def audit_coverage(
    scenarios: Iterable[dict[str, Any]],
    *,
    suite: str,
    passed_scenario_ids: set[str] | None = None,
    evidence_by_scenario: dict[str, dict[str, Any]] | None = None,
) -> dict[str, Any]:
    requirements = FULL_REQUIREMENTS if suite == "full" else SMOKE_REQUIREMENTS
    module_requirements = (
        FULL_MODULE_REQUIREMENTS if suite == "full" else SMOKE_MODULE_REQUIREMENTS
    )
    available: dict[str, set[str]] = defaultdict(set)
    available_modules: dict[str, set[str]] = defaultdict(set)
    considered = []
    for scenario in scenarios:
        identifier = str(scenario.get("id") or "")
        if passed_scenario_ids is not None and identifier not in passed_scenario_ids:
            continue
        considered.append(identifier)
        receipt = (evidence_by_scenario or {}).get(identifier)
        lanes = observed_scenario_lanes(scenario, receipt) if receipt is not None else scenario_lanes(scenario)
        receipt_platforms = receipt.get("platforms", []) if receipt is not None else scenario.get("platforms", [])
        receipt_modules = receipt.get("modules", []) if receipt is not None else scenario.get("evidence", {}).get("modules", [])
        for platform in receipt_platforms:
            available[str(platform)].update(lanes)
            available_modules[str(platform)].update(
                str(module) for module in receipt_modules
            )
    platforms = {}
    for platform, required in requirements.items():
        present = available.get(platform, set())
        missing = required - present
        required_modules = module_requirements.get(platform, set())
        present_modules = available_modules.get(platform, set())
        missing_modules = required_modules - present_modules
        platforms[platform] = {
            "required": sorted(required),
            "verified": sorted(present),
            "missing": sorted(missing),
            "requiredModules": sorted(required_modules),
            "verifiedModules": sorted(present_modules),
            "missingModules": sorted(missing_modules),
            "complete": not missing and not missing_modules,
        }
    return {
        "profile": suite,
        "evidenceMode": "executed-receipts" if evidence_by_scenario is not None else "matrix-declarations",
        "complete": all(row["complete"] for row in platforms.values()),
        "consideredScenarios": sorted(considered),
        "platforms": platforms,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--matrix", type=Path, default=Path(__file__).with_name("real_e2e_matrix.json"))
    parser.add_argument("--suite", choices=("smoke", "full"), default="smoke")
    args = parser.parse_args()
    try:
        value = json.loads(args.matrix.read_text(encoding="utf-8"))
        contracts = value.get("contracts", {})
        scenarios = []
        errors = []
        for source in value.get("scenarios", []):
            scenario = dict(source)
            scenario["_contract"] = contracts.get(scenario.get("contractId"), {})
            if args.suite not in scenario.get("suite", []):
                continue
            errors.extend(validate_execution_claim(scenario))
            scenarios.append(scenario)
        report = audit_coverage(scenarios, suite=args.suite)
        report["matrixErrors"] = errors
        report["complete"] = report["complete"] and not errors
        print(json.dumps(report, ensure_ascii=False, indent=2))
        return 0 if report["complete"] else 1
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(str(error), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
