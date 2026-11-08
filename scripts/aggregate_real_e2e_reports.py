#!/usr/bin/env python3
"""Aggregate independently executed real-E2E receipts into one verified coverage report."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

from real_e2e_coverage import audit_coverage
from real_e2e_evidence import validate_receipt
from run_real_system_tests import DEFAULT_MATRIX, load_matrix


def aggregate_reports(
    report_paths: list[Path],
    *,
    matrix_path: Path,
    suite: str,
) -> dict[str, Any]:
    scenarios = load_matrix(matrix_path)
    scenario_by_id = {scenario["id"]: scenario for scenario in scenarios}
    accepted: dict[str, dict[str, Any]] = {}
    sources: dict[str, str] = {}
    errors: list[str] = []
    rejected_results: list[dict[str, str]] = []
    for report_path in report_paths:
        try:
            report = json.loads(report_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            errors.append(f"unreadable report {report_path}: {error}")
            continue
        for result in report.get("results", []):
            identifier = str(result.get("id") or "")
            scenario = scenario_by_id.get(identifier)
            if scenario is None:
                errors.append(f"unknown scenario in {report_path}: {identifier}")
                continue
            if suite not in scenario.get("suite", []):
                continue
            if result.get("passed") is not True:
                rejected_results.append({
                    "scenarioId": identifier,
                    "report": str(report_path),
                    "reason": "scenario did not pass",
                })
                continue
            receipt_path = Path(str(result.get("receipt") or ""))
            execution_id = str(result.get("evidence", {}).get("executionId") or "")
            receipt, receipt_errors = validate_receipt(receipt_path, scenario, execution_id)
            if receipt_errors or receipt is None:
                errors.extend(
                    f"{identifier}: {error}" for error in receipt_errors or ["missing receipt"]
                )
                continue
            previous = sources.get(identifier)
            if previous is not None:
                errors.append(
                    f"duplicate passing scenario evidence: {identifier} ({previous}, {report_path})"
                )
                continue
            accepted[identifier] = receipt
            sources[identifier] = str(report_path)

    selected = [scenario_by_id[identifier] for identifier in accepted]
    coverage = audit_coverage(
        selected,
        suite=suite,
        passed_scenario_ids=set(accepted),
        evidence_by_scenario=accepted,
    )
    return {
        "schemaVersion": 1,
        "suite": suite,
        "evidenceMode": "independently-revalidated-receipts",
        "acceptedScenarioCount": len(accepted),
        "acceptedScenarios": sorted(accepted),
        "sources": sources,
        "rejectedResults": rejected_results,
        "errors": errors,
        "coverage": coverage,
        "complete": not errors and coverage["complete"],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", action="append", required=True, type=Path)
    parser.add_argument("--matrix", type=Path, default=DEFAULT_MATRIX)
    parser.add_argument("--suite", choices=("smoke", "full"), default="full")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    result = aggregate_reports(
        args.report,
        matrix_path=args.matrix,
        suite=args.suite,
    )
    payload = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if args.output is not None:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(payload, encoding="utf-8")
    print(payload, end="")
    return 0 if result["complete"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
