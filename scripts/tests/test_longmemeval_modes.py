# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest import mock


RUNNER_PATH = (
    Path(__file__).resolve().parents[2]
    / "evals"
    / "benchmarks"
    / "longmemeval"
    / "run.py"
)
spec = importlib.util.spec_from_file_location("longmemeval_run", RUNNER_PATH)
runner = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(runner)


def event_stream(
    answer: str, tool_name: str | None = None, tool_arguments: dict[str, object] | None = None
) -> str:
    events: list[dict[str, object]] = []
    if tool_name:
        events.append(
            {
                "type": "agent_trace",
                "trace": {
                    "kind": "tool.started",
                    "payload": {
                        "call_id": f"call-{tool_name}",
                        "tool_name": tool_name,
                        "arguments": tool_arguments or {},
                    },
                },
            }
        )
    events.append(
        {
            "type": "core_part_updated",
            "part_id": "answer",
            "part": {"kind": "text", "text": answer},
        }
    )
    return "".join(json.dumps(event, ensure_ascii=False) + "\n" for event in events)


class LongMemEvalModeTests(unittest.TestCase):
    @unittest.skipUnless(os.name == "posix", "process-group lifecycle is POSIX-only")
    def test_isolated_process_timeout_terminates_descendant_group(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            pid_path = Path(temporary) / "child.pid"
            program = (
                "import pathlib,subprocess,time; "
                f"p=subprocess.Popen(['sleep','30']); pathlib.Path({str(pid_path)!r}).write_text(str(p.pid)); "
                "time.sleep(30)"
            )
            with self.assertRaises(subprocess.TimeoutExpired):
                runner.run_isolated_process(
                    [sys.executable, "-c", program],
                    cwd=Path(temporary),
                    environment=os.environ.copy(),
                    timeout=0.5,
                )
            child_pid = int(pid_path.read_text())
            status = ""
            for _ in range(20):
                probe = subprocess.run(
                    ["ps", "-o", "stat=", "-p", str(child_pid)],
                    capture_output=True,
                    text=True,
                    check=False,
                )
                status = probe.stdout.strip()
                if not status or status.startswith("Z"):
                    break
                time.sleep(0.05)
            self.assertTrue(not status or status.startswith("Z"), status)

    def architecture_case(self) -> dict[str, object]:
        return {
            "question_id": "fixture-question",
            "question_type": "multi-session",
            "question_date": "2026/09/11",
            "question": "Which preference should be used?",
            "haystack_session_ids": ["source-a", "source-b"],
            "answer_session_ids": ["source-b"],
            "haystack_dates": ["2026/09/01", "2026/09/02"],
            "haystack_sessions": [
                [{"role": "user", "content": "FIRST_PRIVATE_MARKER", "has_answer": False}],
                [{"role": "user", "content": "SECOND_PRIVATE_MARKER", "has_answer": True}],
            ],
        }

    def test_memory_architecture_persists_each_session_and_queries_without_history(self) -> None:
        case = {
            "question_id": "fixture-question",
            "question_type": "multi-session",
            "question_date": "2026/09/11",
            "question": "Which preference should be used?",
            "haystack_session_ids": ["source-a", "source-b"],
            "answer_session_ids": ["source-b"],
            "haystack_dates": ["2026/09/01", "2026/09/02"],
            "haystack_sessions": [
                [{"role": "user", "content": "FIRST_PRIVATE_MARKER", "has_answer": False}],
                [{"role": "user", "content": "SECOND_PRIVATE_MARKER", "has_answer": True}],
            ],
        }
        calls: list[tuple[list[str], dict[str, object]]] = []

        def fake_run(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
            calls.append((command, kwargs))
            if len(command) > 1 and command[1] == "memory":
                return subprocess.CompletedProcess(
                    command,
                    0,
                    json.dumps(
                        {
                            "type": "memory_records",
                            "records": [
                                {"id": "memory-a", "kind": "preference"},
                                {"id": "memory-b", "kind": "reference"},
                            ],
                        }
                    ),
                    "",
                )
            prompt = str(kwargs["input_text"])
            if "Current Date:" in prompt:
                return subprocess.CompletedProcess(
                    command,
                    0,
                    event_stream(
                        "Remembered answer", "memory_search", {"query": "preference"}
                    ),
                    "",
                )
            return subprocess.CompletedProcess(
                command, 0, event_stream("Imported", "memory_save"), ""
            )

        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(
            runner, "run_isolated_process", side_effect=fake_run
        ):
            root = Path(temporary)
            result = runner.run_architecture_case(
                case,
                binary=root / "nola",
                config_home=root / "home",
                output=root / "output",
                timeout=10,
                max_iterations=4,
            )

        agent_calls = [call for call in calls if "--session" in call[0]]
        self.assertEqual(len(agent_calls), 3)
        source_calls = agent_calls[:2]
        query_call = agent_calls[2]
        source_session_ids = [
            command[command.index("--session") + 1] for command, _ in source_calls
        ]
        self.assertEqual(len(set(source_session_ids)), 2)
        self.assertTrue(all("-history-" in value for value in source_session_ids))
        self.assertIn("-query", query_call[0][query_call[0].index("--session") + 1])

        first_prompt = str(source_calls[0][1]["input_text"])
        second_prompt = str(source_calls[1][1]["input_text"])
        query_prompt = str(query_call[1]["input_text"])
        self.assertIn("FIRST_PRIVATE_MARKER", first_prompt)
        self.assertNotIn("SECOND_PRIVATE_MARKER", first_prompt)
        self.assertIn("SECOND_PRIVATE_MARKER", second_prompt)
        self.assertNotIn("FIRST_PRIVATE_MARKER", second_prompt)
        self.assertNotIn("has_answer", first_prompt)
        self.assertNotIn("FIRST_PRIVATE_MARKER", query_prompt)
        self.assertNotIn("SECOND_PRIVATE_MARKER", query_prompt)
        self.assertIn(case["question"], query_prompt)

        database_paths = {
            str(kwargs["environment"]["NOLA_DB_PATH"]) for _, kwargs in agent_calls
        }
        self.assertEqual(len(database_paths), 1)
        self.assertTrue(
            all(
                kwargs["environment"]["NOLA_AUTOMATIC_MEMORY_MAINTENANCE_ENABLED"]
                == "false"
                for _, kwargs in agent_calls
            )
        )
        self.assertTrue(
            all(
                kwargs["environment"]["NOLA_RESIDENT_MEMORY_RECALL_ENABLED"]
                == "false"
                for _, kwargs in agent_calls
            )
        )
        evidence = result["architecture_evidence"]
        self.assertEqual(evidence["automatic_memory_maintenance"], "disabled")
        self.assertEqual(evidence["resident_memory_recall"], "disabled")
        self.assertFalse(evidence["full_history_supplied_to_query"])
        self.assertEqual(evidence["ingestion_session_count"], 2)
        self.assertEqual(evidence["distinct_ingestion_session_count"], 2)
        self.assertEqual(evidence["ingestion_memory_save_calls"], 2)
        self.assertEqual(evidence["query_memory_search_calls"], 1)
        self.assertEqual(evidence["reference_session_count"], 1)
        self.assertEqual(evidence["reference_turn_count"], 1)
        self.assertEqual(evidence["reference_turns"][0]["session_id"], "source-b")
        self.assertEqual(evidence["reference_sessions_with_memory_save_call"], 1)
        self.assertEqual(
            evidence["query_memory_search_evidence"][0]["arguments"]["query"],
            "preference",
        )
        self.assertEqual(evidence["memory_inventory_before_query"]["count"], 2)
        self.assertEqual(result["case_id"], "fixture-question")
        self.assertEqual(result["case_name"], case["question"])

    def test_ndjson_decoder_preserves_unicode_line_separator_inside_text(self) -> None:
        answer = "first\u2028second"
        stdout = event_stream(answer, "memory_search", {"query": "first\u2028second"})
        self.assertEqual(runner.assistant_text(stdout), answer)
        self.assertEqual(runner.tool_calls(stdout), ["memory_search"])
        self.assertEqual(
            runner.tool_trace_evidence(stdout)[0]["arguments"]["query"], answer
        )
        self.assertEqual(runner.ndjson_diagnostics(stdout)["invalid_record_count"], 0)
        diagnostics = runner.ndjson_diagnostics(stdout + "not-json\n")
        self.assertEqual(diagnostics["invalid_record_count"], 1)
        self.assertIn("sha256", diagnostics["invalid_records"][0])

    def test_provider_response_evidence_preserves_alias_and_concrete_model(self) -> None:
        detail = json.dumps(
            {
                "configured_model": "routing-alias",
                "response_model": "publisher/resolved-model",
                "response_id": "response-42",
                "attempt": 2,
            }
        )
        stdout = json.dumps(
            {
                "type": "agent_trace",
                "trace": {
                    "kind": "tool.progress",
                    "run_id": "run-1",
                    "event_id": "event-1",
                    "payload": {
                        "summary": "model_response_provenance",
                        "detail": detail,
                    },
                },
            }
        ) + "\n"

        self.assertEqual(
            runner.provider_response_evidence(stdout),
            [
                {
                    "configured_model": "routing-alias",
                    "response_model": "publisher/resolved-model",
                    "response_id": "response-42",
                    "attempt": 2,
                    "run_id": "run-1",
                    "event_id": "event-1",
                }
            ],
        )

        unknown_stdout = stdout.replace("publisher/resolved-model", "")
        self.assertEqual(
            runner.provider_response_evidence(unknown_stdout)[0]["response_model"],
            "unknown",
        )

    def test_memory_save_metrics_separate_attempts_successes_and_distinct_records(self) -> None:
        turns = [
            {
                "tool_evidence": [
                    {
                        "tool_name": "memory_save",
                        "status": "failed",
                        "output_preview": '{"error":"invalid_tool_arguments"}',
                    },
                    {
                        "tool_name": "memory_save",
                        "status": "succeeded",
                        "output_preview": "memory_saved: mem-1\nartifact_uri: nola://memory/mem-1",
                    },
                    {
                        "tool_name": "memory_save",
                        "status": "succeeded",
                        "output_preview": "memory_saved: mem-1\nartifact_uri: nola://memory/mem-1",
                    },
                ]
            }
        ]
        self.assertEqual(
            runner.memory_save_metrics(turns),
            {"attempted": 3, "succeeded": 2, "distinct_persisted": 1},
        )

    def test_architecture_case_stops_after_first_failed_ingestion(self) -> None:
        case = self.architecture_case()

        def fail_then_inventory(
            command: list[str], **_kwargs: object
        ) -> subprocess.CompletedProcess[str]:
            if len(command) > 1 and command[1] == "memory":
                return subprocess.CompletedProcess(
                    command,
                    0,
                    json.dumps({"records": [{"id": "persisted-1", "kind": "reference"}]}),
                    "",
                )
            return subprocess.CompletedProcess(
                command, 1, "", "pending Host request payload mismatch"
            )

        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(
            runner, "run_isolated_process", side_effect=fail_then_inventory
        ) as run:
            root = Path(temporary)
            result = runner.run_architecture_case(
                case,
                binary=root / "nola",
                config_home=root / "home",
                output=root / "output",
                timeout=10,
                max_iterations=4,
            )

        self.assertEqual(run.call_count, 2)
        self.assertEqual(result["status"], "failed")
        self.assertEqual(len(result["ingestion"]), 1)
        self.assertIn("historical session 1 failed", result["error"])
        self.assertEqual(
            result["architecture_evidence"]["memory_inventory_before_query"]["count"],
            1,
        )

    def test_provider_quota_is_separate_from_agent_generation_failure(self) -> None:
        def failure_stream(metadata: dict[str, object]) -> str:
            detail = "transport failure: " + json.dumps(
                {"error": {"code": 429, "metadata": metadata}}
            )
            return json.dumps(
                {
                    "type": "agent_trace",
                    "trace": {
                        "kind": "run.failed",
                        "payload": {"detail": detail},
                    },
                }
            )

        self.assertEqual(
            runner.provider_failure_metadata(
                failure_stream({"limit_source": "provider_daily"})
            )["failure_class"],
            "provider_quota",
        )
        self.assertEqual(
            runner.provider_failure_metadata(failure_stream({}))["failure_class"],
            "provider_rate_limit",
        )
        self.assertEqual(
            runner.provider_failure_metadata("schema rejected"),
            {},
        )

    def test_pre_response_transport_failure_is_retryable_but_other_failures_are_not(self) -> None:
        stdout = json.dumps(
            {
                "type": "error",
                "detail": (
                    "LLM request failed: error sending request for url "
                    "(https://provider.invalid/chat): channel closed"
                ),
            }
        ) + "\n"
        metadata = runner.provider_failure_metadata(stdout)
        self.assertEqual(metadata["failure_class"], "provider_transport")
        self.assertEqual(metadata["transport_kind"], "connection_interrupted")
        self.assertTrue(
            runner.retryable_provider_transport_failure(
                {
                    "status": "failed",
                    "failure_metadata": metadata,
                    "provider_responses": [],
                    "tool_evidence": [],
                }
            )
        )
        for failure_class in (
            "provider_quota", "provider_rate_limit", "generation_timeout", None
        ):
            self.assertFalse(
                runner.retryable_provider_transport_failure(
                    {
                        "status": "failed",
                        "failure_metadata": {"failure_class": failure_class},
                        "provider_responses": [],
                        "tool_evidence": [],
                    }
                )
            )
        self.assertFalse(
            runner.retryable_provider_transport_failure(
                {
                    "status": "failed",
                    "failure_metadata": metadata,
                    "provider_responses": [{"response_model": "resolved"}],
                    "tool_evidence": [],
                }
            )
        )

    def test_structured_provider_failure_metadata_takes_priority(self) -> None:
        stdout = json.dumps(
            {
                "type": "error",
                "detail": "channel closed",
                "failure_metadata": {
                    "failure_class": "provider_quota",
                    "http_status": 429,
                    "retryable": False,
                },
            }
        ) + "\n"
        self.assertEqual(
            runner.provider_failure_metadata(stdout),
            {
                "failure_class": "provider_quota",
                "http_status": 429,
                "retryable": False,
            },
        )

    def test_architecture_retries_transport_failure_and_archives_ingestion(self) -> None:
        case = self.architecture_case()
        transport = json.dumps(
            {"type": "error", "detail": "LLM request failed: channel closed"}
        ) + "\n"
        agent_attempt = 0

        def transient_then_complete(
            command: list[str], **kwargs: object
        ) -> subprocess.CompletedProcess[str]:
            nonlocal agent_attempt
            if len(command) > 1 and command[1] == "memory":
                return subprocess.CompletedProcess(
                    command, 0, json.dumps({"records": []}), ""
                )
            agent_attempt += 1
            if agent_attempt == 1:
                return subprocess.CompletedProcess(command, 1, transport, "")
            answer = (
                "Remembered answer"
                if "Current Date:" in str(kwargs["input_text"])
                else "Imported"
            )
            return subprocess.CompletedProcess(command, 0, event_stream(answer), "")

        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(
            runner, "run_isolated_process", side_effect=transient_then_complete
        ), mock.patch.object(runner.time, "sleep") as sleep:
            root = Path(temporary)
            result = runner.run_architecture_case(
                case,
                binary=root / "nola",
                config_home=root / "home",
                output=root / "output",
                timeout=10,
                max_iterations=4,
                transport_max_attempts=2,
                transport_retry_backoff=0.25,
            )
            case_dir = root / "output" / "cases" / "fixture-question"
            archived = json.loads(
                (case_dir / "ingestion-001.attempt-001.json").read_text()
            )
            current = json.loads((case_dir / "ingestion-001.json").read_text())

        self.assertEqual(result["status"], "completed")
        self.assertEqual(archived["failure_class"], "provider_transport")
        self.assertEqual(current["attempt_number"], 2)
        self.assertTrue(current["session_id"].endswith("-attempt-002"))
        self.assertEqual(
            result["architecture_evidence"]["ingestion_attempt_count_cumulative"], 3
        )
        sleep.assert_called_once_with(0.25)

    def test_architecture_retries_transport_failure_and_archives_query(self) -> None:
        case = self.architecture_case()
        query_attempt = 0
        transport = json.dumps(
            {"type": "error", "detail": "HTTP/2 stream error while sending request"}
        ) + "\n"

        def query_transient_then_complete(
            command: list[str], **kwargs: object
        ) -> subprocess.CompletedProcess[str]:
            nonlocal query_attempt
            if len(command) > 1 and command[1] == "memory":
                return subprocess.CompletedProcess(
                    command, 0, json.dumps({"records": []}), ""
                )
            if "Current Date:" in str(kwargs["input_text"]):
                query_attempt += 1
                if query_attempt == 1:
                    return subprocess.CompletedProcess(command, 1, transport, "")
                return subprocess.CompletedProcess(
                    command, 0, event_stream("Remembered answer", "memory_search"), ""
                )
            return subprocess.CompletedProcess(command, 0, event_stream("Imported"), "")

        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(
            runner, "run_isolated_process", side_effect=query_transient_then_complete
        ), mock.patch.object(runner.time, "sleep"):
            root = Path(temporary)
            result = runner.run_architecture_case(
                case,
                binary=root / "nola",
                config_home=root / "home",
                output=root / "output",
                timeout=10,
                max_iterations=4,
                transport_max_attempts=2,
                transport_retry_backoff=0,
            )
            case_dir = root / "output" / "cases" / "fixture-question"
            archived = json.loads((case_dir / "query.attempt-001.json").read_text())
            current = json.loads((case_dir / "query.json").read_text())

        evidence = result["architecture_evidence"]
        self.assertEqual(result["status"], "completed")
        self.assertEqual(archived["failure_class"], "provider_transport")
        self.assertEqual(current["attempt_number"], 2)
        self.assertEqual(evidence["query_attempt_count_cumulative"], 2)
        self.assertGreaterEqual(
            result["campaign_elapsed_seconds"],
            evidence["query_elapsed_seconds_cumulative"],
        )

    def test_provider_transport_failure_is_redacted_and_classified(self) -> None:
        detail = (
            "error sending request for url (https://provider.invalid/api): "
            "http2 error: stream error received: unspecific protocol error detected"
        )
        stdout = json.dumps(
            {
                "type": "agent_trace",
                "trace": {"kind": "run.failed", "payload": {"detail": detail}},
            }
        )

        self.assertEqual(
            runner.provider_failure_metadata(stdout),
            {
                "failure_class": "provider_transport",
                "transport_kind": "http2_stream",
            },
        )
        self.assertNotIn("provider.invalid", json.dumps(runner.provider_failure_metadata(stdout)))
        self.assertEqual(
            runner.provider_failure_metadata(
                json.dumps(
                    {
                        "type": "agent_trace",
                        "trace": {
                            "kind": "run.failed",
                            "payload": {"detail": "memory schema rejected"},
                        },
                    }
                )
            ),
            {},
        )

    def test_stream_provider_unavailable_envelope_records_concrete_model(self) -> None:
        detail = json.dumps(
            {
                "choices": [],
                "error": {
                    "code": 502,
                    "message": "upstream internal error",
                    "metadata": {"error_type": "provider_unavailable"},
                },
                "model": "publisher/concrete-model",
                "provider": "ExampleProvider",
            }
        )
        stdout = json.dumps(
            {
                "type": "agent_trace",
                "trace": {"kind": "run.failed", "payload": {"detail": detail}},
            }
        )

        self.assertEqual(
            runner.provider_failure_metadata(stdout),
            {
                "failure_class": "provider_unavailable",
                "http_status": 502,
                "provider_error_type": "provider_unavailable",
                "response_model": "publisher/concrete-model",
                "response_provider": "ExampleProvider",
            },
        )

    def test_campaign_elapsed_includes_archived_attempts_once(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            case_dir = Path(temporary)
            (case_dir / "ingestion-001.attempt-001.json").write_text(
                json.dumps({"elapsed_seconds": 2.25})
            )
            (case_dir / "ingestion-001.attempt-002.stderr.txt").write_text("ignored")
            elapsed = runner.campaign_elapsed_seconds(
                case_dir,
                [{"elapsed_seconds": 3.5}],
                {"elapsed_seconds": 1.25},
            )
        self.assertEqual(elapsed, 7.0)

    def test_failed_ingestion_retries_in_a_new_session_and_preserves_evidence(self) -> None:
        case = self.architecture_case()
        failed = subprocess.CompletedProcess(["nola"], 1, "", "provider failed")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = root / "output"
            with mock.patch.object(runner, "run_isolated_process", return_value=failed):
                first = runner.run_architecture_case(
                    case,
                    binary=root / "nola",
                    config_home=root / "home",
                    output=output,
                    timeout=10,
                    max_iterations=4,
                )
            self.assertEqual(first["status"], "failed")
            case_dir = output / "cases" / "fixture-question"
            (case_dir / "nola.db").touch()

            calls: list[list[str]] = []

            def resumed_turn(
                command: list[str], **kwargs: object
            ) -> subprocess.CompletedProcess[str]:
                calls.append(command)
                if len(command) > 1 and command[1] == "memory":
                    return subprocess.CompletedProcess(
                        command, 0, json.dumps({"records": []}), ""
                    )
                prompt = str(kwargs["input_text"])
                answer = "Remembered answer" if "Current Date:" in prompt else "Imported"
                return subprocess.CompletedProcess(command, 0, event_stream(answer), "")

            with mock.patch.object(
                runner, "run_isolated_process", side_effect=resumed_turn
            ):
                resumed = runner.run_architecture_case(
                    case,
                    binary=root / "nola",
                    config_home=root / "home",
                    output=output,
                    timeout=10,
                    max_iterations=4,
                )

            agent_calls = [command for command in calls if "--session" in command]
            retried_session = agent_calls[0][agent_calls[0].index("--session") + 1]
            self.assertTrue(retried_session.endswith("-attempt-002"))
            self.assertEqual(resumed["status"], "completed")
            archived = json.loads(
                (case_dir / "ingestion-001.attempt-001.json").read_text()
            )
            self.assertEqual(archived["status"], "failed")
            current = json.loads((case_dir / "ingestion-001.json").read_text())
            self.assertEqual(current["logical_session_id"], "longmemeval-fixture-question-history-001")
            self.assertEqual(current["attempt_number"], 2)

    def test_boundary_stop_finishes_active_ingestion_without_starting_another(self) -> None:
        case = self.architecture_case()
        stop_requested = threading.Event()

        def complete_then_stop(
            command: list[str], **kwargs: object
        ) -> subprocess.CompletedProcess[str]:
            stop_requested.set()
            return subprocess.CompletedProcess(
                command, 0, event_stream("Imported", "memory_save"), ""
            )

        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(
            runner, "run_isolated_process", side_effect=complete_then_stop
        ) as run:
            root = Path(temporary)
            result = runner.run_architecture_case(
                case,
                binary=root / "nola",
                config_home=root / "home",
                output=root / "output",
                timeout=10,
                max_iterations=4,
                stop_requested=stop_requested,
            )
            saved = json.loads(
                (
                    root
                    / "output"
                    / "cases"
                    / "fixture-question"
                    / "ingestion-001.json"
                ).read_text()
            )

        agent_calls = [
            call for call in run.call_args_list if "--session" in call.args[0]
        ]
        self.assertEqual(len(agent_calls), 1)
        self.assertEqual(result["status"], "interrupted")
        self.assertEqual(result["architecture_evidence"]["ingestion_completed"], 1)
        self.assertEqual(saved["status"], "completed")

    def test_resume_reuses_ingestion_completed_before_boundary_stop(self) -> None:
        case = self.architecture_case()
        stop_requested = threading.Event()

        def first_turn(
            command: list[str], **_kwargs: object
        ) -> subprocess.CompletedProcess[str]:
            stop_requested.set()
            return subprocess.CompletedProcess(
                command, 0, event_stream("Imported", "memory_save"), ""
            )

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = root / "output"
            with mock.patch.object(runner, "run_isolated_process", side_effect=first_turn):
                first = runner.run_architecture_case(
                    case,
                    binary=root / "nola",
                    config_home=root / "home",
                    output=output,
                    timeout=10,
                    max_iterations=4,
                    stop_requested=stop_requested,
                )
            self.assertEqual(first["status"], "interrupted")
            (output / "cases" / "fixture-question" / "nola.db").touch()

            def resumed_turn(
                command: list[str], **kwargs: object
            ) -> subprocess.CompletedProcess[str]:
                if len(command) > 1 and command[1] == "memory":
                    return subprocess.CompletedProcess(
                        command, 0, json.dumps({"records": []}), ""
                    )
                prompt = str(kwargs["input_text"])
                answer = "Remembered answer" if "Current Date:" in prompt else "Imported"
                return subprocess.CompletedProcess(command, 0, event_stream(answer), "")

            with mock.patch.object(
                runner, "run_isolated_process", side_effect=resumed_turn
            ) as resumed:
                result = runner.run_architecture_case(
                    case,
                    binary=root / "nola",
                    config_home=root / "home",
                    output=output,
                    timeout=10,
                    max_iterations=4,
                    stop_requested=threading.Event(),
                )

        agent_calls = [
            call for call in resumed.call_args_list if "--session" in call.args[0]
        ]
        self.assertEqual(len(agent_calls), 2)
        self.assertNotIn("FIRST_PRIVATE_MARKER", str(agent_calls[0].kwargs["input_text"]))
        self.assertEqual(result["status"], "completed")
        self.assertEqual(result["architecture_evidence"]["ingestion_completed"], 2)

    def test_temporal_scope_parses_timestamps_instead_of_comparing_strings(self) -> None:
        case = {
            "question_id": "temporal-fixture",
            "question_type": "temporal-reasoning",
            "question": "What happened first?",
            "question_date": "2023/05/25 (Thu) 08:03",
            "haystack_session_ids": ["past", "future-answer", "future-distractor"],
            "answer_session_ids": ["future-answer"],
            "haystack_dates": [
                "2023/05/25 (Thu) 07:52",
                "2023/05/25 (Thu) 14:25",
                "2023/05/26 (Fri) 01:00",
            ],
        }
        scope = runner.temporal_scope(case)
        self.assertEqual(scope["future_session_count"], 2)
        self.assertEqual(scope["future_reference_session_count"], 1)
        self.assertEqual(scope["future_sessions"][0]["session_id"], "future-answer")
        self.assertEqual(scope["future_session_policy"], "retained_official_haystack")

    def test_completed_full_history_cache_is_enriched_without_model_rerun(self) -> None:
        case = {
            "question_id": "cached-fixture",
            "question_type": "single-session-user",
            "question": "What did I remember?",
            "question_date": "2023/05/25 (Thu) 08:03",
            "haystack_session_ids": ["source"],
            "answer_session_ids": ["source"],
            "haystack_dates": ["2023/05/25 (Thu) 14:25"],
            "haystack_sessions": [[{"role": "user", "content": "remembered"}]],
        }
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            result_path = root / "output" / "cases" / "cached-fixture" / "generation.json"
            result_path.parent.mkdir(parents=True)
            result_path.write_text(
                json.dumps(
                    {
                        "question_id": "cached-fixture",
                        "question_type": "single-session-user",
                        "status": "completed",
                        "hypothesis": "remembered",
                    }
                )
            )
            with mock.patch.object(runner, "run_isolated_process") as run:
                result = runner.run_case(
                    case,
                    binary=root / "nola",
                    config_home=root / "home",
                    output=root / "output",
                    timeout=10,
                )
            run.assert_not_called()
            self.assertEqual(result["evaluator_schema_version"], 4)
            self.assertEqual(result["case_name"], case["question"])
            self.assertEqual(result["temporal_scope"]["future_reference_session_count"], 1)

    def test_report_names_scope_for_each_mode(self) -> None:
        generation_config = {
            "endpoint": "https://generation.example/v1",
            "api_key": "secret",
            "model": "generation-model",
        }
        judge_config = {
            "endpoint": "https://judge.example/v1",
            "api_key": "secret",
            "model": "judge-model",
        }
        result = {
            "question_id": "fixture",
            "question_type": "multi-session",
            "is_abstention": False,
            "status": "completed",
            "elapsed_seconds": 1,
            "campaign_elapsed_seconds": 3,
            "hypothesis": "answer",
            "judge_status": "completed",
            "passed": True,
            "provider_responses": [
                {
                    "configured_model": "generation-model",
                    "response_model": "publisher/resolved-generation",
                }
            ],
            "judge_provider_response": {
                "configured_model": "judge-model",
                "response_model": "unknown",
            },
        }
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            runner.write_report(
                output,
                [result],
                generation_config,
                judge_config,
                runner.MEMORY_ARCHITECTURE_MODE,
            )
            report = json.loads((output / "report.json").read_text())
        self.assertEqual(report["evaluation_mode"], "memory-architecture")
        self.assertEqual(report["method"], "nola-cli-persisted-session-memory")
        self.assertEqual(
            report["architecture_summary"]["cases_with_full_history_supplied_to_query"],
            0,
        )
        self.assertIn(
            "background automatic memory extraction",
            report["evaluation_scope"]["does_not_evaluate"],
        )
        self.assertIn(
            "automatic resident-profile memory recall",
            report["evaluation_scope"]["does_not_evaluate"],
        )
        self.assertEqual(report["campaign_elapsed_seconds_sum"], 3)
        self.assertEqual(
            report["provider_response_provenance"]["generation"]["response_models"],
            {"publisher/resolved-generation": 1},
        )
        self.assertEqual(
            report["provider_response_provenance"]["judge"][
                "unknown_response_model_count"
            ],
            1,
        )

    def test_report_separates_provider_quota_from_quality_results(self) -> None:
        config = {
            "endpoint": "https://provider.example/v1",
            "api_key": "secret",
            "model": "route",
        }
        result = {
            "question_id": "fixture",
            "question_type": "multi-session",
            "is_abstention": False,
            "status": "failed",
            "failure_class": "provider_quota",
            "elapsed_seconds": 1,
            "campaign_elapsed_seconds": 5,
            "hypothesis": "",
            "judge_status": "skipped",
            "passed": False,
            "architecture_evidence": {
                "memory_inventory_at_terminal_boundary": {"count": 2},
            },
        }
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            runner.write_report(
                output,
                [result],
                config,
                config,
                runner.MEMORY_ARCHITECTURE_MODE,
            )
            report = json.loads((output / "report.json").read_text())
        self.assertEqual(report["failure_classes"], {"provider_quota": 1})
        self.assertEqual(report["provider_infrastructure_failures"], 1)
        self.assertEqual(report["quality_evaluable_cases"], 0)
        self.assertEqual(report["architecture_summary"]["cases_with_persisted_memory"], 1)


if __name__ == "__main__":
    unittest.main()
