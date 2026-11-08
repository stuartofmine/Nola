# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import importlib.util
import gzip
import json
import tempfile
import unittest
from pathlib import Path

path = Path(__file__).resolve().parents[1] / "aggregate_terminal_bench.py"
spec = importlib.util.spec_from_file_location("aggregate_terminal_bench", path)
aggregator = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(aggregator)


class AggregateTerminalBenchTests(unittest.TestCase):
    def write_json(self, path: Path, value: object) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value), encoding="utf-8")

    def trial(
        self,
        case_id: str,
        trial_id: str,
        finished_at: str,
        *,
        reward: float | None = None,
        exception: bool = False,
        agent_started: bool = True,
    ) -> dict[str, object]:
        return {
            "id": trial_id,
            "task_name": f"suite/{case_id}",
            "trial_name": f"{case_id}__{trial_id}",
            "started_at": "2026-01-01T00:00:00Z",
            "finished_at": finished_at,
            "agent_execution": {"started_at": "2026-01-01T00:00:01Z"}
            if agent_started
            else None,
            "agent_result": {
                "metadata": {"nola_exit_code": 0 if reward is not None else 1}
            },
            "verifier": {"started_at": finished_at, "finished_at": finished_at}
            if reward is not None
            else None,
            "verifier_result": {"rewards": {"reward": reward}}
            if reward is not None
            else None,
            "exception_info": {
                "exception_type": "SyntheticError",
                "exception_message": "fixture failure",
                "occurred_at": finished_at,
            }
            if exception
            else None,
        }

    def write_ctrf(
        self, result_path: Path, *, tests: int, passed: int, failed: int
    ) -> None:
        self.write_json(
            result_path.parent / "verifier/ctrf.json",
            {
                "reportFormat": "CTRF",
                "results": {
                    "summary": {
                        "tests": tests,
                        "passed": passed,
                        "failed": failed,
                        "skipped": 0,
                    }
                },
            },
        )

    def write_agent_trace(self, result_path: Path, events: list[object]) -> None:
        path = result_path.parent / "agent/nola.ndjson.gz"
        path.parent.mkdir(parents=True, exist_ok=True)
        with gzip.open(path, "wt", encoding="utf-8") as stream:
            for event in events:
                stream.write(json.dumps(event) + "\n")

    def test_uses_manifest_denominator_and_latest_completed_verifier(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = root / "manifest.json"
            self.write_json(
                manifest,
                {
                    "name": "synthetic-benchmark",
                    "variant": "fixture",
                    "cases": [
                        {"id": "case-a", "category": "one", "difficulty": "low"},
                        {"id": "case-b", "category": "two", "difficulty": "high"},
                        {"id": "case-c", "category": "two", "difficulty": "medium"},
                    ],
                },
            )
            first = root / "first"
            second = root / "second"
            self.write_json(
                first / "case-a__old/result.json",
                self.trial("case-a", "old", "2026-01-01T01:00:00Z", reward=0.25),
            )
            selected_result = second / "case-a__new/result.json"
            self.write_json(
                selected_result,
                self.trial("case-a", "new", "2026-01-01T02:00:00Z", reward=0.75),
            )
            self.write_ctrf(selected_result, tests=8, passed=6, failed=2)
            self.write_json(
                second / "case-a__latest-error/result.json",
                self.trial(
                    "case-a", "latest-error", "2026-01-01T03:00:00Z", exception=True
                ),
            )
            self.write_json(
                first / "case-b__infra/result.json",
                self.trial(
                    "case-b",
                    "infra",
                    "2026-01-01T04:00:00Z",
                    exception=True,
                    agent_started=False,
                ),
            )
            self.write_json(first / "result.json", {"n_total_trials": 2})

            report = aggregator.aggregate(manifest, [first, second])

            self.assertEqual(report["summary"]["denominator"], 3)
            self.assertEqual(report["summary"]["attempts"], 4)
            self.assertEqual(report["summary"]["scored"], 1)
            self.assertEqual(report["summary"]["agent_not_started"], 1)
            self.assertEqual(report["summary"]["invalid_attempts"], 1)
            self.assertEqual(report["summary"]["missing"], 1)
            self.assertEqual(report["summary"]["fixed_denominator_mean_reward"], 0.25)
            self.assertEqual(report["summary"]["mean_reward_over_scored_cases"], 0.75)
            self.assertEqual(report["summary"]["verifier_test_count"], 8)
            self.assertEqual(report["summary"]["verifier_test_passed"], 6)
            self.assertEqual(report["summary"]["verifier_test_pass_rate"], 0.75)
            by_id = {case["id"]: case for case in report["cases"]}
            self.assertEqual(by_id["case-a"]["selected_trial_id"], "new")
            self.assertEqual(by_id["case-a"]["verifier_diagnostics"]["passed"], 6)
            self.assertNotIn("verifier_tests", by_id["case-a"])
            self.assertNotIn("verifier_tests", by_id["case-a"]["attempts"][0])
            self.assertNotIn("invalid_infrastructure", report["summary"])
            self.assertEqual(len(by_id["case-a"]["attempts"]), 3)
            self.assertEqual(by_id["case-b"]["status"], "agent_not_started")
            self.assertIsNone(by_id["case-b"]["selected_trial_id"])
            self.assertEqual(by_id["case-c"]["status"], "missing")
            markdown = aggregator.markdown_report(report)
            self.assertIn("over 1 scored tasks", markdown)
            self.assertIn("Zero-filled manifest mean: 0.2500", markdown)
            self.assertIn(
                "Verifier diagnostic assertion pass rate (not reward): 6/8 (75.0%)",
                markdown,
            )

    def test_does_not_promote_internal_diagnostic_score_to_reward(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = root / "manifest.json"
            self.write_json(manifest, {"cases": [{"id": "case-a"}]})
            result_path = root / "case-a/result.json"
            trial = self.trial("case-a", "trial", "2026-01-01T01:00:00Z")
            trial["verifier"] = {
                "started_at": "2026-01-01T01:00:00Z",
                "finished_at": "2026-01-01T01:00:01Z",
            }
            trial["verifier_result"] = {"rewards": {"diagnostic_score": 0.8}}
            self.write_json(result_path, trial)
            self.write_ctrf(result_path, tests=1, passed=1, failed=0)

            report = aggregator.aggregate(manifest, [root])

            case = report["cases"][0]
            self.assertEqual(case["status"], "verifier_invalid")
            self.assertIsNone(case["reward"])
            self.assertEqual(report["summary"]["scored"], 0)
            attempt = case["attempts"][0]
            self.assertEqual(attempt["validity"], "invalid_verifier")
            self.assertEqual(attempt["verifier_diagnostics"]["role"], "diagnostic_only")

    def test_retains_assertion_identity_but_keeps_it_diagnostic(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = root / "manifest.json"
            self.write_json(manifest, {"cases": [{"id": "case-a"}]})
            result_path = root / "case-a/result.json"
            self.write_json(
                result_path,
                self.trial("case-a", "trial", "2026-01-01T01:00:00Z", reward=0.0),
            )
            self.write_json(
                result_path.parent / "verifier/ctrf.json",
                {
                    "results": {
                        "summary": {"tests": 1, "passed": 1, "failed": 0},
                        "tests": [
                            {
                                "name": "suite.py::test_entrypoint",
                                "status": "passed",
                                "raw_status": "passed",
                                "file_path": "suite.py",
                                "trace": "large implementation detail",
                            }
                        ],
                    }
                },
            )

            attempt = aggregator.aggregate(manifest, [root])["cases"][0]["attempts"][0]

            diagnostics = attempt["verifier_diagnostics"]
            self.assertEqual(
                diagnostics["assertions"][0]["name"], "suite.py::test_entrypoint"
            )
            self.assertNotIn("trace", diagnostics["assertions"][0])
            self.assertEqual(attempt["official_reward"], 0.0)
            self.assertEqual(attempt["diagnostic_reward_delta"], 1.0)

    def test_summarizes_internal_verifier_scores_without_using_them_as_reward(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = root / "manifest.json"
            self.write_json(manifest, {"cases": [{"id": "case-a"}]})
            result_path = root / "case-a/result.json"
            self.write_json(
                result_path,
                self.trial("case-a", "trial", "2026-01-01T01:00:00Z", reward=0.0),
            )
            self.write_json(
                result_path.parent / "verifier/details.json",
                {
                    "diagnostic_score": 0.6,
                    "points": 3,
                    "total": 5,
                    "checks": [
                        {"passed": True, "weight": 2},
                        {"passed": False, "weight": 3},
                    ],
                },
            )

            attempt = aggregator.aggregate(manifest, [root])["cases"][0]["attempts"][0]

            diagnostics = attempt["verifier_auxiliary_diagnostics"][0]
            self.assertEqual(diagnostics["role"], "diagnostic_only")
            self.assertEqual(diagnostics["scalars"]["diagnostic_score"], 0.6)
            self.assertEqual(diagnostics["collections"]["checks"]["weight_passed"], 2)
            self.assertEqual(attempt["official_reward"], 0.0)

    def test_trace_counts_stable_calls_instead_of_repeated_part_projections(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = root / "manifest.json"
            self.write_json(manifest, {"cases": [{"id": "case-a"}]})
            result_path = root / "case-a/result.json"
            self.write_json(
                result_path,
                self.trial("case-a", "trial", "2026-01-01T01:00:00Z", reward=0.0),
            )
            started = {
                "type": "agent_trace",
                "trace": {
                    "kind": "tool.started",
                    "payload": {
                        "call_id": "call-1",
                        "tool_name": "bash",
                        "arguments": {"operation_kind": "inspect"},
                    },
                },
            }
            self.write_agent_trace(
                result_path,
                [
                    started,
                    {
                        "type": "core_part_updated",
                        "part": {"kind": "tool", "call_id": "call-1"},
                    },
                    started,
                    {
                        "type": "agent_trace",
                        "trace": {
                            "kind": "tool.completed",
                            "payload": {"call_id": "call-1", "tool_name": "bash"},
                        },
                    },
                    {
                        "type": "agent_trace",
                        "trace": {
                            "kind": "tool.started",
                            "payload": {
                                "call_id": "call-2",
                                "tool_name": "codebase_search",
                                "arguments": {},
                            },
                        },
                    },
                    {
                        "type": "core_session_projection_updated",
                        "snapshot": {
                            "engineering": {"phase": "diagnosing"},
                            "skill_runtime": {
                                "catalog": {
                                    "mcp": {"servers": {"nola.codebase-memory": {}}}
                                }
                            },
                        },
                    },
                ],
            )

            trace = aggregator.aggregate(manifest, [root])["cases"][0]["attempts"][0][
                "agent_trace"
            ]

            self.assertEqual(trace["started_call_count"], 2)
            self.assertEqual(trace["completed_call_count"], 1)
            self.assertEqual(trace["unmatched_completed_call_count"], 0)
            self.assertEqual(
                trace["capability_counts"], {"bash": 1, "codebase_search": 1}
            )
            self.assertEqual(trace["bash_operation_kinds"], {"inspect": 1})
            self.assertTrue(trace["code_memory_registered"])
            self.assertEqual(trace["code_memory_call_count"], 1)


if __name__ == "__main__":
    unittest.main()
