#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Aggregate repeated Harbor trials against a fixed benchmark manifest."""

from __future__ import annotations

import argparse
import collections
import datetime as dt
import gzip
import json
import math
from pathlib import Path
from typing import Any, Iterable

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_MANIFEST = ROOT / "evals/benchmarks/terminal-bench/benchmark.json"


def read_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"expected a JSON object: {path}")
    return value


def timestamp_key(value: object) -> tuple[int, str]:
    if not isinstance(value, str) or not value:
        return (0, "")
    candidate = value[:-1] + "+00:00" if value.endswith("Z") else value
    try:
        parsed = dt.datetime.fromisoformat(candidate)
        if parsed.tzinfo is None:
            parsed = parsed.replace(tzinfo=dt.timezone.utc)
        return (1, parsed.astimezone(dt.timezone.utc).isoformat())
    except ValueError:
        return (0, value)


def task_id_for(result: dict[str, Any]) -> str | None:
    task_name = result.get("task_name")
    if isinstance(task_name, str) and task_name:
        return task_name.rstrip("/").rsplit("/", 1)[-1]
    task = result.get("task_id")
    if isinstance(task, dict) and isinstance(task.get("path"), str):
        return Path(task["path"]).name
    return None


def official_reward(result: dict[str, Any]) -> float | None:
    """Return Harbor's canonical reward without guessing from diagnostic fields."""
    verifier_result = result.get("verifier_result")
    rewards = (
        verifier_result.get("rewards") if isinstance(verifier_result, dict) else None
    )
    reward = rewards.get("reward") if isinstance(rewards, dict) else None
    if (
        isinstance(reward, (int, float))
        and not isinstance(reward, bool)
        and math.isfinite(reward)
    ):
        return float(reward)
    return None


def ctrf_summary(result_path: Path) -> dict[str, Any] | None:
    """Read CTRF as verifier diagnostics, never as the benchmark reward."""
    path = result_path.parent / "verifier" / "ctrf.json"
    try:
        report = read_json(path)
    except (OSError, UnicodeError, json.JSONDecodeError, ValueError):
        return None
    results = report.get("results")
    summary = results.get("summary") if isinstance(results, dict) else None
    if not isinstance(summary, dict):
        return None
    counts: dict[str, Any] = {}
    for key in ("tests", "passed", "failed", "skipped", "pending", "other"):
        value = summary.get(key)
        if isinstance(value, int) and not isinstance(value, bool) and value >= 0:
            counts[key] = value
    tests = counts.get("tests")
    passed = counts.get("passed")
    if isinstance(tests, int) and tests > 0 and isinstance(passed, int):
        counts["pass_rate"] = passed / tests
    assertions = results.get("tests")
    if isinstance(assertions, list):
        counts["assertions"] = [
            {
                key: assertion[key]
                for key in ("name", "status", "raw_status", "file_path")
                if key in assertion
            }
            for assertion in assertions
            if isinstance(assertion, dict)
        ]
    counts["role"] = "diagnostic_only"
    return counts


def auxiliary_verifier_diagnostics(result_path: Path) -> list[dict[str, Any]]:
    """Summarize verifier-owned JSON evidence without promoting it to reward."""
    verifier_dir = result_path.parent / "verifier"
    reports: list[dict[str, Any]] = []
    for path in sorted(verifier_dir.glob("*.json")):
        if path.name in {"ctrf.json", "reward.json"}:
            continue
        try:
            value = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeError, json.JSONDecodeError):
            continue
        if not isinstance(value, dict):
            continue
        scalars = {
            key: item
            for key, item in value.items()
            if isinstance(item, (str, int, float, bool)) or item is None
        }
        collections: dict[str, Any] = {}
        for key, item in value.items():
            if not isinstance(item, list):
                continue
            summary: dict[str, Any] = {"count": len(item)}
            mappings = [entry for entry in item if isinstance(entry, dict)]
            passed = [entry.get("passed") for entry in mappings]
            if (
                mappings
                and len(mappings) == len(item)
                and all(isinstance(flag, bool) for flag in passed)
            ):
                summary["passed"] = sum(passed)
                summary["failed"] = len(passed) - sum(passed)
                weighted = [
                    entry
                    for entry in mappings
                    if isinstance(entry.get("weight"), (int, float))
                    and not isinstance(entry.get("weight"), bool)
                ]
                if weighted:
                    summary["weight_total"] = sum(entry["weight"] for entry in weighted)
                    summary["weight_passed"] = sum(
                        entry["weight"] for entry in weighted if entry["passed"]
                    )
            collections[key] = summary
        reports.append(
            {
                "path": str(path.resolve()),
                "role": "diagnostic_only",
                "scalars": scalars,
                "collections": collections,
            }
        )
    return reports


def lifecycle(result: dict[str, Any]) -> dict[str, bool]:
    execution = result.get("agent_execution")
    verifier = result.get("verifier")
    agent_result = result.get("agent_result")
    metadata = agent_result.get("metadata") if isinstance(agent_result, dict) else None
    return {
        "agent_started": isinstance(execution, dict)
        and bool(execution.get("started_at")),
        "agent_finished": isinstance(execution, dict)
        and bool(execution.get("finished_at")),
        "adapter_reported": isinstance(metadata, dict),
        "verifier_started": isinstance(verifier, dict)
        and bool(verifier.get("started_at")),
        "verifier_finished": isinstance(verifier, dict)
        and bool(verifier.get("finished_at")),
        "official_reward_reported": official_reward(result) is not None,
    }


def nola_trace_summary(result_path: Path) -> dict[str, Any] | None:
    """Summarize executed calls by stable call ID, never projection updates."""
    path = result_path.parent / "agent" / "nola.ndjson.gz"
    try:
        stream = gzip.open(path, "rt", encoding="utf-8", errors="replace")
    except OSError:
        return None
    started: dict[str, tuple[str, dict[str, Any]]] = {}
    completed: set[str] = set()
    final_engineering: dict[str, Any] | None = None
    code_memory_registered = False
    try:
        with stream:
            for line in stream:
                try:
                    event = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if not isinstance(event, dict):
                    continue
                if event.get("type") == "agent_trace":
                    trace = event.get("trace")
                    if not isinstance(trace, dict):
                        continue
                    payload = trace.get("payload")
                    if not isinstance(payload, dict):
                        continue
                    call_id = payload.get("call_id")
                    if not isinstance(call_id, str) or not call_id:
                        continue
                    if trace.get("kind") == "tool.started":
                        name = payload.get("tool_name")
                        arguments = payload.get("arguments")
                        if isinstance(name, str):
                            started.setdefault(
                                call_id,
                                (
                                    name,
                                    arguments if isinstance(arguments, dict) else {},
                                ),
                            )
                    elif trace.get("kind") == "tool.completed":
                        completed.add(call_id)
                elif event.get("type") == "core_session_projection_updated":
                    snapshot = event.get("snapshot")
                    if not isinstance(snapshot, dict):
                        continue
                    engineering = snapshot.get("engineering")
                    if isinstance(engineering, dict):
                        final_engineering = engineering
                    catalog = snapshot.get("skill_runtime")
                    if isinstance(catalog, dict):
                        catalog = catalog.get("catalog")
                    if isinstance(catalog, dict):
                        catalog = catalog.get("mcp")
                    if isinstance(catalog, dict):
                        servers = catalog.get("servers")
                        if isinstance(servers, dict):
                            code_memory_registered |= "nola.codebase-memory" in servers
    except OSError:
        return None

    capability_counts = collections.Counter(name for name, _ in started.values())
    bash_operation_kinds = collections.Counter(
        str(arguments.get("operation_kind") or "missing")
        for name, arguments in started.values()
        if name == "bash"
    )
    code_memory_calls = sum(
        count
        for name, count in capability_counts.items()
        if name.startswith("codebase_")
    )
    matched_completed = completed.intersection(started)
    return {
        "schema_version": 1,
        "path": str(path.resolve()),
        "started_call_count": len(started),
        "completed_call_count": len(matched_completed),
        "unmatched_completed_call_count": len(completed - started.keys()),
        "capability_counts": dict(sorted(capability_counts.items())),
        "bash_operation_kinds": dict(sorted(bash_operation_kinds.items())),
        "code_memory_registered": code_memory_registered,
        "code_memory_call_count": code_memory_calls,
        "final_engineering": final_engineering,
    }


def classify(result: dict[str, Any]) -> str:
    stages = lifecycle(result)
    if stages["official_reward_reported"] and stages["verifier_finished"]:
        return "scored"
    if stages["verifier_started"] or stages["official_reward_reported"]:
        return "verifier_invalid"
    if not stages["agent_started"]:
        return "agent_not_started" if result.get("exception_info") else "incomplete"
    agent_result = result.get("agent_result")
    metadata = agent_result.get("metadata") if isinstance(agent_result, dict) else None
    exit_code = metadata.get("nola_exit_code") if isinstance(metadata, dict) else None
    if result.get("exception_info") or (isinstance(exit_code, int) and exit_code != 0):
        return "agent_failed"
    return "agent_incomplete"


def validity(classification: str) -> str:
    if classification == "scored":
        return "valid_score"
    if classification == "agent_not_started":
        return "invalid_infrastructure"
    if classification == "verifier_invalid":
        return "invalid_verifier"
    if classification == "agent_failed":
        return "unscored_agent_failure"
    return "incomplete"


def attempt_timestamp(result: dict[str, Any]) -> object:
    verifier = result.get("verifier")
    if isinstance(verifier, dict) and verifier.get("finished_at"):
        return verifier["finished_at"]
    exception = result.get("exception_info")
    if isinstance(exception, dict) and exception.get("occurred_at"):
        return exception["occurred_at"]
    return result.get("finished_at") or result.get("started_at")


def compact_attempt(path: Path, result: dict[str, Any]) -> dict[str, Any]:
    verifier_result = result.get("verifier_result")
    rewards = (
        verifier_result.get("rewards") if isinstance(verifier_result, dict) else None
    )
    agent_result = result.get("agent_result")
    metadata = agent_result.get("metadata") if isinstance(agent_result, dict) else None
    classification = classify(result)
    diagnostics = ctrf_summary(path)
    auxiliary_diagnostics = auxiliary_verifier_diagnostics(path)
    reward = official_reward(result)
    diagnostic_rate = (
        diagnostics.get("pass_rate") if isinstance(diagnostics, dict) else None
    )
    diagnostic_interpretation = None
    if isinstance(diagnostics, dict):
        tests = diagnostics.get("tests")
        all_passed = (
            isinstance(tests, int) and tests > 0 and diagnostics.get("passed") == tests
        )
        diagnostic_interpretation = {
            "assertion_shape": "single_assertion" if tests == 1 else "assertion_set",
            "all_assertions_passed": all_passed,
            "agrees_with_binary_reward": (
                (reward == 1.0) == all_passed
                if reward in (0.0, 1.0) and isinstance(all_passed, bool)
                else None
            ),
        }
    return {
        "trial_id": result.get("id"),
        "trial_name": result.get("trial_name"),
        "result_path": str(path.resolve()),
        "started_at": result.get("started_at"),
        "finished_at": result.get("finished_at"),
        "classification": classification,
        "validity": validity(classification),
        "lifecycle": lifecycle(result),
        "verifier_completed": classification == "scored",
        "official_reward": reward,
        "rewards": rewards,
        "agent_metadata": metadata,
        "agent_trace": nola_trace_summary(path),
        "verifier_diagnostics": diagnostics,
        "verifier_auxiliary_diagnostics": auxiliary_diagnostics,
        "diagnostic_reward_delta": (
            diagnostic_rate - reward
            if isinstance(diagnostic_rate, (int, float)) and reward is not None
            else None
        ),
        "diagnostic_interpretation": diagnostic_interpretation,
        "exception_info": result.get("exception_info"),
    }


def discover_attempts(
    roots: Iterable[Path], case_ids: set[str]
) -> dict[str, list[dict[str, Any]]]:
    attempts = {case_id: [] for case_id in case_ids}
    seen: set[Path] = set()
    for root in roots:
        candidates = [root] if root.is_file() else root.rglob("result.json")
        for path in candidates:
            resolved = path.resolve()
            if resolved in seen:
                continue
            seen.add(resolved)
            try:
                result = read_json(path)
            except (OSError, UnicodeError, json.JSONDecodeError, ValueError):
                continue
            case_id = task_id_for(result)
            if case_id in attempts:
                attempt = compact_attempt(path, result)
                attempt["_sort_timestamp"] = attempt_timestamp(result)
                attempts[case_id].append(attempt)
    for values in attempts.values():
        values.sort(
            key=lambda attempt: (
                timestamp_key(attempt.get("_sort_timestamp")),
                attempt["result_path"],
            )
        )
        for attempt in values:
            attempt.pop("_sort_timestamp", None)
    return attempts


def selected_attempt(attempts: list[dict[str, Any]]) -> dict[str, Any] | None:
    completed = [
        attempt for attempt in attempts if attempt["validity"] == "valid_score"
    ]
    return completed[-1] if completed else None


def primary_reward(attempt: dict[str, Any] | None) -> float | None:
    reward = attempt.get("official_reward") if attempt else None
    return float(reward) if isinstance(reward, (int, float)) else None


def artifact_contract(
    manifest_path: Path, case: dict[str, Any]
) -> dict[str, Any] | None:
    """Expose declared artifact transport without inferring it from test names."""
    source_path = case.get("source_path")
    if not isinstance(source_path, str) or not source_path:
        return None
    descriptor = manifest_path.parent / source_path / "task.toml"
    try:
        import tomllib

        with descriptor.open("rb") as stream:
            task = tomllib.load(stream)
    except (OSError, tomllib.TOMLDecodeError):
        return None
    declared = task.get("artifacts")
    verifier = task.get("verifier")
    collect = verifier.get("collect") if isinstance(verifier, dict) else None
    return {
        "descriptor_path": str(descriptor.resolve()),
        "declared_artifacts": declared if isinstance(declared, list) else [],
        "verifier_collect_steps": len(collect) if isinstance(collect, list) else 0,
        "evidence": "declaration_only",
    }


def aggregate(manifest_path: Path, roots: list[Path]) -> dict[str, Any]:
    manifest = read_json(manifest_path)
    raw_cases = manifest.get("cases")
    if not isinstance(raw_cases, list) or not raw_cases:
        raise ValueError("manifest cases must be a non-empty array")
    case_ids = [case.get("id") for case in raw_cases if isinstance(case, dict)]
    if len(case_ids) != len(raw_cases) or any(
        not isinstance(case_id, str) or not case_id for case_id in case_ids
    ):
        raise ValueError("every manifest case must have a non-empty string id")
    if len(set(case_ids)) != len(case_ids):
        raise ValueError("manifest case ids must be unique")

    found = discover_attempts(roots, set(case_ids))
    cases: list[dict[str, Any]] = []
    for manifest_case in raw_cases:
        case_id = manifest_case["id"]
        attempts = found[case_id]
        selected = selected_attempt(attempts)
        latest = attempts[-1] if attempts else None
        cases.append(
            {
                "id": case_id,
                "category": manifest_case.get("category"),
                "difficulty": manifest_case.get("difficulty"),
                "artifact_contract": artifact_contract(manifest_path, manifest_case),
                "status": (
                    "scored"
                    if selected
                    else latest["classification"]
                    if latest
                    else "missing"
                ),
                "selected_trial_id": selected.get("trial_id") if selected else None,
                "selected_result_path": selected.get("result_path")
                if selected
                else None,
                "reward": primary_reward(selected),
                "rewards": selected.get("rewards") if selected else None,
                "agent_metadata": selected.get("agent_metadata") if selected else None,
                "verifier_diagnostics": (
                    selected.get("verifier_diagnostics") if selected else None
                ),
                "latest_attempt_trial_id": latest.get("trial_id") if latest else None,
                "latest_attempt_classification": (
                    latest.get("classification") if latest else None
                ),
                "attempts": attempts,
            }
        )

    denominator = len(cases)
    rewards = [case["reward"] for case in cases if case["reward"] is not None]
    selected_test_summaries = [
        case["verifier_diagnostics"]
        for case in cases
        if isinstance(case.get("verifier_diagnostics"), dict)
    ]
    total_tests = sum(int(item.get("tests", 0)) for item in selected_test_summaries)
    total_passed = sum(int(item.get("passed", 0)) for item in selected_test_summaries)
    diagnostic_reward_disagreements = sum(
        attempt.get("diagnostic_interpretation", {}).get("agrees_with_binary_reward")
        is False
        for case in cases
        for attempt in case["attempts"]
        if isinstance(attempt.get("diagnostic_interpretation"), dict)
    )
    return {
        "schema_version": 1,
        "benchmark": manifest.get("name", "terminal-bench"),
        "variant": manifest.get("variant"),
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "manifest_path": str(manifest_path.resolve()),
        "input_roots": [str(root.resolve()) for root in roots],
        "summary": {
            "denominator": denominator,
            "attempts": sum(len(case["attempts"]) for case in cases),
            "scored": len(rewards),
            "reward_coverage": len(rewards) / denominator,
            "missing": sum(case["status"] == "missing" for case in cases),
            "agent_not_started": sum(
                case["status"] == "agent_not_started" for case in cases
            ),
            "invalid_verifier": sum(
                case["status"] == "verifier_invalid" for case in cases
            ),
            "agent_failed": sum(case["status"] == "agent_failed" for case in cases),
            "invalid_attempts": sum(
                attempt["validity"].startswith("invalid_")
                for case in cases
                for attempt in case["attempts"]
            ),
            "agent_not_started_attempts": sum(
                attempt["classification"] == "agent_not_started"
                for case in cases
                for attempt in case["attempts"]
            ),
            "reward_sum": sum(rewards),
            "mean_reward_over_scored_cases": sum(rewards) / len(rewards)
            if rewards
            else None,
            # Manifest-wide outcome includes missing/invalid tasks as zero.
            # Distinct from observed reward and paired with explicit coverage.
            "fixed_denominator_mean_reward": sum(rewards) / denominator,
            "verifier_test_count": total_tests,
            "verifier_test_passed": total_passed,
            "verifier_test_pass_rate": total_passed / total_tests
            if total_tests
            else None,
            "diagnostic_reward_disagreements": diagnostic_reward_disagreements,
        },
        "cases": cases,
    }


def markdown_report(report: dict[str, Any]) -> str:
    summary = report["summary"]
    lines = [
        f"# {report['benchmark']} aggregate",
        "",
        f"Variant: `{report.get('variant')}`",
        "",
        "Official reward over scored cases: "
        + (
            f"**{summary['mean_reward_over_scored_cases']:.4f}** "
            f"({summary['reward_sum']:.4f} over {summary['scored']} scored tasks)"
            if summary.get("mean_reward_over_scored_cases") is not None
            else "unavailable"
        ),
        "",
        f"Reward coverage: {summary['scored']}/{summary['denominator']} "
        f"({summary['reward_coverage']:.1%}) · Zero-filled manifest mean: "
        f"{summary['fixed_denominator_mean_reward']:.4f}",
        "",
        f"Agent not started: {summary['agent_not_started']} · "
        f"Verifier invalid: {summary['invalid_verifier']} · "
        f"Agent failed before scoring: {summary['agent_failed']} · "
        f"Missing: {summary['missing']} · "
        f"Invalid attempts: {summary['invalid_attempts']} · "
        f"Attempts retained: {summary['attempts']}",
        "",
        "Verifier diagnostic assertion pass rate (not reward): "
        + (
            f"{summary['verifier_test_passed']}/{summary['verifier_test_count']} "
            f"({summary['verifier_test_pass_rate']:.1%})"
            if summary.get("verifier_test_pass_rate") is not None
            else "unavailable"
        ),
        f"Diagnostic/reward disagreements: {summary['diagnostic_reward_disagreements']}",
        "",
        "| Task | Status | Reward | Verifier tests | Attempts | Selected trial |",
        "|---|---:|---:|---:|---:|---|",
    ]
    for case in report["cases"]:
        reward = "—" if case["reward"] is None else f"{case['reward']:.4f}"
        selected = case["selected_trial_id"] or "—"
        tests = case.get("verifier_diagnostics")
        test_text = (
            f"{tests.get('passed', 0)}/{tests.get('tests', 0)}"
            if isinstance(tests, dict) and "tests" in tests
            else "—"
        )
        lines.append(
            f"| {case['id']} | {case['status']} | {reward} | {test_text} | "
            f"{len(case['attempts'])} | {selected} |"
        )
    return "\n".join(lines) + "\n"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "roots", nargs="+", type=Path, help="Harbor job or run directories"
    )
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--json-output", type=Path, required=True)
    parser.add_argument("--markdown-output", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    missing = [path for path in [args.manifest, *args.roots] if not path.exists()]
    if missing:
        raise SystemExit("missing input: " + ", ".join(map(str, missing)))
    report = aggregate(args.manifest, args.roots)
    args.json_output.parent.mkdir(parents=True, exist_ok=True)
    args.markdown_output.parent.mkdir(parents=True, exist_ok=True)
    args.json_output.write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    args.markdown_output.write_text(markdown_report(report), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
