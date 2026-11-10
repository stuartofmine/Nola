#!/usr/bin/env python3
"""Focused contracts for the paid live Skill E2E harness."""

import unittest
import os
import json
import sqlite3
import subprocess
import time
from unittest import mock
from pathlib import Path
import tempfile

import live_skill_e2e


class LiveSkillE2ETests(unittest.TestCase):
    def test_current_default_route_resolves_environment_credential_and_protocol(self):
        declaration = {"toolCalls": True, "imageInput": False, "reasoning": False, "samplingParameters": False}
        config = {"llm": {"default_model": {"connection": "chosen", "model": "selected"}, "connections": [
            {"id": "other", "models": [], "endpoint": "https://unused.invalid"},
            {"id": "chosen", "endpoint": "https://provider.invalid/v1", "protocol": "responses",
             "api_key_env": "FIXTURE_KEY", "models": [{"id": "selected", "executionCapabilities": declaration}]}]}}
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "config.json"
            path.write_text(json.dumps(config))
            original = path.read_bytes()
            with mock.patch.dict(os.environ, {"NOLA_CONFIG_PATH": str(path), "FIXTURE_KEY": "fixture-secret"}, clear=True):
                value = live_skill_e2e.live_llm_options()["llm"]
                self.assertEqual(value["model"], "selected")
                self.assertEqual(value["api_key"], "fixture-secret")
                self.assertEqual(value["provider"], "openai-responses")
                self.assertTrue(value["capabilities"]["native_tool_calls"])
                self.assertEqual(path.read_bytes(), original)
                config["llm"]["default_model"]["model"] = "missing"
                path.write_text(json.dumps(config))
                with self.assertRaises(live_skill_e2e.InfrastructureFailure):
                    live_skill_e2e.live_llm_options()
        live_skill_e2e.SOURCE_CONFIGURATION = None

    def test_success_retains_trace_for_evidence_audit(self) -> None:
        stdout = '{"type":"assistant_journal_committed_v1","through":{}}\n'
        process = mock.Mock(returncode=0)
        process.communicate.return_value = (stdout, "fixture diagnostics")
        oracle = mock.Mock()
        def audit(**request):
            command = live_skill_e2e.subprocess.Popen.call_args.args[0]
            return {"runs": [{"conversationId": command[command.index("--session") + 1],
                "runId": "run", "status": "sealed", "kind": "action_completed",
                "sealDigest": "a" * 64, "resultDigest": "b" * 64}]}
        oracle.query.side_effect = audit
        with tempfile.TemporaryDirectory() as directory:
            workspace = Path(directory)
            with mock.patch.object(live_skill_e2e.subprocess, "Popen", return_value=process) as launch, \
                 mock.patch.object(live_skill_e2e, "Oracle", return_value=oracle), \
                 mock.patch.object(live_skill_e2e, "parse_events", return_value=[json.loads(stdout)]), \
                 mock.patch.object(live_skill_e2e, "journal_tool_events", return_value=[]), \
                 mock.patch.object(live_skill_e2e, "infrastructure_failure_detail", return_value=None), \
                 mock.patch.object(live_skill_e2e, "validate_skill_tool_sequence", return_value=[]), \
                 mock.patch.object(live_skill_e2e, "validate_persisted_side_effects", return_value=[]), \
                 mock.patch.object(live_skill_e2e, "validate_artifact", return_value=[]):
                live_skill_e2e.run_case(Path("/fixture/nola"), workspace,
                    workspace / "runtime", "conduct-deep-research", "Research", (".md",),
                    {"llm": {"endpoint": "http://localhost/v1", "model": "fixture-model", "api_key": "fixture-key",
                        "provider": "openai-compatible", "capabilities": {"native_tool_calls": True}}}, Path("/fixture/oracle"))
                arguments = launch.call_args.args[0]
                environment = launch.call_args.kwargs["env"]
                self.assertNotIn("--yes", arguments)
                self.assertNotIn("--api-key", arguments)
                self.assertNotIn("fixture-key", arguments)
                self.assertNotIn("--no-save", arguments)
                self.assertEqual(environment.get("HOME"), os.environ.get("HOME"))
                self.assertEqual(environment["NOLA_PERMISSION_PROFILE"], "allow_all")
                self.assertFalse((Path(environment["NOLA_HOME"]) / ".nola/config.json").exists())
            trace = (workspace / "transcript-conduct-deep-research.jsonl").read_text()
            self.assertIn(stdout, trace)
            self.assertIn("STDERR\nfixture diagnostics", trace)

    def test_terminal_completion_requires_successful_task_outcome(self) -> None:
        def receipt(kind):
            return {"runs": [{"conversationId": "owner", "runId": "run", "status": "sealed",
                "kind": kind, "sealDigest": "a" * 64, "resultDigest": "b" * 64}]}
        for reason in ("incomplete", "blocked", "needs_input", "outcome_missing", None):
            self.assertTrue(live_skill_e2e.validate_terminal_outcome(receipt(reason), "owner"))
        for reason in ("answered", "action_completed"):
            self.assertEqual(live_skill_e2e.validate_terminal_outcome(receipt(reason), "owner"), [])
        self.assertTrue(live_skill_e2e.validate_terminal_outcome({"runs": []}, "owner"))
        self.assertTrue(live_skill_e2e.validate_terminal_outcome(receipt("answered"), "foreign"))
        incomplete = receipt("answered")
        incomplete["runs"][0].pop("sealDigest")
        self.assertTrue(live_skill_e2e.validate_terminal_outcome(incomplete, "owner"))

    def test_timeout_policy_gives_academic_skills_a_longer_bounded_window(self) -> None:
        self.assertEqual(live_skill_e2e.skill_timeout_seconds("nature-reviewer"), 600)
        self.assertEqual(live_skill_e2e.skill_timeout_seconds("nature-paper2ppt"), 600)
        self.assertEqual(live_skill_e2e.skill_timeout_seconds("conduct-deep-research"), 600)
        self.assertEqual(
            live_skill_e2e.skill_timeout_seconds("nature-reviewer", "5000"), 900
        )
        self.assertEqual(live_skill_e2e.skill_timeout_seconds("analyze-data"), 180)
        self.assertEqual(live_skill_e2e.skill_timeout_seconds("word-document-authoring"), 300)
        self.assertEqual(
            live_skill_e2e.skill_timeout_seconds("word-document-authoring", "5000"),
            600,
        )

    def test_timeout_cleanup_is_bounded_when_process_ignores_term(self) -> None:
        process = subprocess.Popen(
            ["/bin/sh", "-c", "trap '' TERM; while :; do sleep 1; done"],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )
        started = time.monotonic()
        live_skill_e2e.terminate_process_group(
            process,
            term_grace_seconds=0.05,
            kill_grace_seconds=0.2,
        )
        self.assertLess(time.monotonic() - started, 1.0)
        self.assertIsNotNone(process.poll())

    def test_find_binary_selects_newest_build_unless_overridden(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            repo = Path(temporary)
            target = repo / "target"
            release = target / "release/nola"
            debug = target / "debug/nola"
            release.parent.mkdir(parents=True)
            debug.parent.mkdir(parents=True)
            release.write_text("old", encoding="utf-8")
            debug.write_text("new", encoding="utf-8")
            release.chmod(0o755)
            debug.chmod(0o755)
            os.utime(release, ns=(1, 1))
            os.utime(debug, ns=(2, 2))
            with mock.patch.object(live_skill_e2e, "cargo_target_directory", return_value=target), mock.patch.dict(os.environ, {}, clear=False):
                os.environ.pop("NOLA_BIN", None)
                self.assertEqual(live_skill_e2e.find_binary(repo), debug)

    def test_find_binary_rejects_release_older_than_runtime_source(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            repo = Path(temporary)
            target = repo / "target"
            release = target / "release/nola"
            source = repo / "apps/cli/src/main.rs"
            release.parent.mkdir(parents=True)
            source.parent.mkdir(parents=True)
            release.write_text("stale", encoding="utf-8")
            source.write_text("current", encoding="utf-8")
            release.chmod(0o755)
            os.utime(release, ns=(1, 1))
            os.utime(source, ns=(2, 2))
            with mock.patch.object(live_skill_e2e, "cargo_target_directory", return_value=target), mock.patch.dict(os.environ, {}, clear=False):
                os.environ.pop("NOLA_BIN", None)
                with self.assertRaisesRegex(RuntimeError, "stale"):
                    live_skill_e2e.find_binary(repo)

    def test_provider_balance_failure_is_infrastructure(self) -> None:
        events = [{
            "type": "error",
            "detail": "LLM request failed (402 Payment Required): Insufficient Balance",
        }]
        self.assertEqual(
            "LLM request failed (402 Payment Required): Insufficient Balance",
            live_skill_e2e.infrastructure_failure_detail(events, "") or "",
        )

    def test_model_protocol_failure_remains_a_product_failure(self) -> None:
        events = [{
            "type": "error",
            "detail": "reasoning_content must be passed back to the API",
        }]
        self.assertIsNone(live_skill_e2e.infrastructure_failure_detail(events, ""))

    def test_connection_failure_is_infrastructure(self) -> None:
        self.assertEqual(
            live_skill_e2e.infrastructure_failure_detail([], "Connection refused"),
            "Connection refused",
        )

    def test_infrastructure_markers_need_current_explicit_error_detail(self) -> None:
        for event in (
            {"type": "assistant_delta", "detail": "402 Payment Required: Insufficient Balance"},
            {"type": "error", "payload": {"detail": "402 Payment Required"}},
            {"type": "error", "detail": None, "message": "Insufficient Balance"},
            {"type": "tool_completed", "output": "connection refused"},
        ):
            with self.subTest(event=event):
                self.assertIsNone(live_skill_e2e.infrastructure_failure_detail([event], ""))

    def test_skill_may_follow_read_only_discovery_but_precedes_execution(self) -> None:
        events = [
            {"kind": "tool.started", "payload": {"capability_id": "Skill", "arguments": {"skill": "collaborate-in-figma"}}},
            {"kind": "tool.completed", "payload": {"capability_id": "Skill", "output": "collaborate-in-figma"}},
            {"kind": "tool.started", "payload": {"capability_id": "read_file"}},
            {"kind": "tool.started", "payload": {"capability_id": "write_file"}},
        ]
        self.assertEqual(
            live_skill_e2e.validate_skill_tool_sequence(events, "collaborate-in-figma"),
            [],
        )

        read_only_preflight = [events[2], *events[:2], events[3]]
        self.assertEqual(
            live_skill_e2e.validate_skill_tool_sequence(
                read_only_preflight, "collaborate-in-figma"
            ),
            [],
        )

        unsafe_preflight = [events[3], *events[:2], events[2]]
        self.assertTrue(
            any(
                "executing, mutating, or external" in error
                for error in live_skill_e2e.validate_skill_tool_sequence(
                    unsafe_preflight, "collaborate-in-figma"
                )
            )
        )

    def test_strict_skill_allows_read_only_checks_after_validation(self) -> None:
        valid = [
            {"kind": "tool.started", "payload": {"capability_id": "Skill", "arguments": {"skill": "extract-image-text"}}},
            {"kind": "tool.started", "payload": {"capability_id": "bash", "arguments": {"command": "swift extract_text.swift --validate-only"}}},
            {"kind": "tool.started", "payload": {"capability_id": "file_tree", "arguments": {"root": "."}}},
            {"kind": "tool.started", "payload": {"capability_id": "requirements_update", "arguments": {"answer": "Done"}}},
        ]
        self.assertEqual(
            live_skill_e2e.validate_skill_tool_sequence(valid, "extract-image-text"),
            [],
        )
        invalid = [
            *valid,
            {"kind": "tool.started", "payload": {"capability_id": "write_file", "arguments": {"path": "receipt.txt"}}},
        ]
        self.assertTrue(
            any(
                "mutated state after" in error
                for error in live_skill_e2e.validate_skill_tool_sequence(
                    invalid, "extract-image-text"
                )
            )
        )

    def test_nature_reviewer_requires_its_bundled_final_validator(self) -> None:
        missing = [
            {"kind": "tool.started", "payload": {"capability_id": "Skill", "arguments": {"skill": "nature-reviewer"}}},
            {"kind": "tool.started", "payload": {"capability_id": "append_file", "arguments": {"path": "peer-review.md", "content": "review", "create": True}}},
            {"kind": "tool.started", "payload": {"capability_id": "read_file", "arguments": {"path": "peer-review.md"}}},
        ]
        self.assertIn(
            "strict Skill did not run its required final validation",
            live_skill_e2e.validate_skill_tool_sequence(missing, "nature-reviewer"),
        )

        valid = missing + [
            {"kind": "tool.started", "payload": {"capability_id": "bash", "arguments": {"command": "python3 scripts/validate_review.py --source manuscript.md --review peer-review.md"}}},
        ]
        self.assertEqual(
            live_skill_e2e.validate_skill_tool_sequence(valid, "nature-reviewer"),
            [],
        )

    def test_analyze_data_accepts_manifest_declared_deterministic_validator(self) -> None:
        events = [
            {"kind": "tool.started", "payload": {"capability_id": "Skill", "arguments": {"skill": "analyze-data"}}},
            {"kind": "tool.started", "payload": {"capability_id": "bash", "arguments": {"command": "python3 /runtime/analyze-data/scripts/analyze_table.py --input sales.csv --output analysis.md"}}},
            {"kind": "tool.started", "payload": {"capability_id": "bash", "arguments": {"command": "python3 /runtime/analyze-data/scripts/analyze_table.py --validate-only --input sales.csv --output analysis.md"}}},
        ]
        self.assertEqual(
            live_skill_e2e.validate_skill_tool_sequence(events, "analyze-data"),
            [],
        )

    def test_analyze_data_rejects_legacy_or_text_only_validator_mentions(self) -> None:
        base = [
            {"kind": "tool.started", "payload": {"capability_id": "Skill", "arguments": {"skill": "analyze-data"}}},
        ]
        for command in (
            "python3 analysis_pipeline.py --validate-only --input sales.csv --output analysis.md",
            "echo analyze_table.py --validate-only",
        ):
            errors = live_skill_e2e.validate_skill_tool_sequence(
                base + [{"kind": "tool.started", "payload": {"capability_id": "bash", "arguments": {"command": command}}}],
                "analyze-data",
            )
            self.assertIn(
                "strict Skill did not run its required final validation", errors
            )

    def test_missing_skill_invocation_is_rejected(self) -> None:
        events = [
            {"kind": "tool.started", "payload": {"capability_id": "read_file", "arguments": {"path": "draft.md"}}},
            {"kind": "tool.started", "payload": {"capability_id": "write_file", "arguments": {"path": "review.md"}}},
        ]
        self.assertTrue(
            any(
                "never invoked" in error
                for error in live_skill_e2e.validate_skill_tool_sequence(
                    events, "review-documents"
                )
            )
        )

    def test_dependency_discovery_is_rejected_after_host_preload(self) -> None:
        events = [
            {"kind": "tool.started", "payload": {"capability_id": "bash", "arguments": {"command": "which tesseract; pip install pytesseract"}}},
        ]
        self.assertTrue(
            any(
                "dependency discovery" in error
                for error in live_skill_e2e.validate_skill_tool_sequence(
                    events, "extract-image-text"
                )
            )
        )
    def test_short_ocr_artifact_uses_markers_not_generic_length(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "receipt.txt"
            path.write_text("TOTAL 42.00\n", encoding="utf-8")
            self.assertEqual(
                live_skill_e2e.validate_artifact(path, "extract-image-text"),
                [],
            )
            path.write_text("TUTHL 42.00\n", encoding="utf-8")
            self.assertTrue(
                any(
                    "source-grounded" in error
                    for error in live_skill_e2e.validate_artifact(
                        path, "extract-image-text"
                    )
                )
            )

    def test_text_artifact_requires_source_grounded_markers(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "analysis.md"
            path.write_text(
                "# Revenue analysis\n\n"
                + "Revenue reconciles to the source rows. " * 8
                + "\n\n## Limitations\nThe fixture does not establish causality.\n",
                encoding="utf-8",
            )
            self.assertEqual(
                live_skill_e2e.validate_artifact(path, "analyze-data"),
                [],
            )

            path.write_text("# Result\n\nA generic answer without evidence.\n", encoding="utf-8")
            errors = live_skill_e2e.validate_artifact(path, "analyze-data")
            self.assertTrue(any("too small" in error for error in errors))
            self.assertTrue(any("source-grounded" in error for error in errors))

    def test_automation_validation_uses_runtime_database_and_semantic_fields(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            runtime_home = Path(directory)
            database = runtime_home / "nola-e2e.db"
            spec = {"kind": "agent", "spec": {
                "prompt": "Analyze sales.csv", "execution_scope": {"kind": "local_workspace", "root": directory},
                "permission_profile": "cautious", "model_selection": "use_current_default_at_admission",
                "context_refs": [], "skill_refs": ["analyze-data"],
            }}
            arguments = {"frequency": "daily", "hour": 9, "minute": 15, "enabled": True,
                "skill_id": "analyze-data", "prompt": "Analyze sales.csv", "execution_spec": spec}
            events = [{"kind": "tool.started", "payload": {"capability_id": "nola_orchestrate",
                "call_id": "automation-call", "arguments": arguments}}]
            metadata = {"name": "Daily Sales Analysis", "enabled": True, "time": "09:15",
                "skillId": "analyze-data", "userPrompt": "Analyze sales.csv", "executionSpec": spec}
            with sqlite3.connect(database) as connection:
                connection.execute(
                    "CREATE TABLE automation_definitions "
                    "(enabled INTEGER NOT NULL, metadata_json TEXT NOT NULL, execution_spec_json TEXT NOT NULL, "
                    "updated_at_ms INTEGER NOT NULL, deleted_at_ms INTEGER)"
                )
                connection.execute(
                    "INSERT INTO automation_definitions VALUES (1, ?, ?, 1, NULL)",
                    (json.dumps(metadata), json.dumps(spec)),
                )
            self.assertEqual(
                live_skill_e2e.validate_persisted_side_effects(
                    runtime_home, "design-automations", events
                ),
                [],
            )
            with sqlite3.connect(database) as connection:
                foreign_spec = {"kind": "agent", "spec": {**spec["spec"], "prompt": "Different task"}}
                connection.execute("UPDATE automation_definitions SET execution_spec_json=?", (json.dumps(foreign_spec),))
            self.assertTrue(live_skill_e2e.validate_persisted_side_effects(runtime_home, "design-automations", events))
            with sqlite3.connect(database) as connection:
                metadata["userPrompt"] = "Different task"
                connection.execute("UPDATE automation_definitions SET execution_spec_json=?,metadata_json=?",
                    (json.dumps(spec), json.dumps(metadata)))
            self.assertTrue(live_skill_e2e.validate_persisted_side_effects(runtime_home, "design-automations", events))

    def test_only_current_transport_and_owned_completed_journal_tools_are_evidence(self) -> None:
        with self.assertRaises(ValueError):
            live_skill_e2e.parse_events('{"kind":"tool.started"}\n')
        transport = live_skill_e2e.parse_events('{"type":"assistant_text","body":{"kind":"tool.started","payload":{"capability_id":"Skill"}}}\n')
        with mock.patch.object(live_skill_e2e, "committed_journal_parts", return_value=[]):
            self.assertEqual(live_skill_e2e.journal_tool_events(transport, Path("/fixture/db"), "owner"), [])
        def part(state, owner="owner"):
            return {"head": {"identity": {"conversation_id": owner}, "metadata": {"kind": "tool", "data": {
                "capability_id": "skill", "tool_call_id": "call", "state": state}}},
                "bodies": {"tool_input": json.dumps({"protocol_version": 1, "value": {"skill": "analyze-data"}})}}
        with mock.patch.object(live_skill_e2e, "committed_journal_parts", return_value=[part("completed")]):
            events = live_skill_e2e.journal_tool_events(transport, Path("/fixture/db"), "owner")
            self.assertEqual(live_skill_e2e.started_tool_calls(events), [("Skill", {"skill": "analyze-data"})])
        for state in ("pending", "failed", "cancelled"):
            with mock.patch.object(live_skill_e2e, "committed_journal_parts", return_value=[part(state)]):
                self.assertEqual(live_skill_e2e.journal_tool_events(transport, Path("/fixture/db"), "owner"), [])
        with mock.patch.object(live_skill_e2e, "committed_journal_parts", return_value=[part("completed", "foreign")]):
            with self.assertRaises(ValueError):
                live_skill_e2e.journal_tool_events(transport, Path("/fixture/db"), "owner")
        self.assertIsNone(live_skill_e2e.event_tool_name({"kind": "tool.started", "payload": {"tool_name": "Skill"}}))


if __name__ == "__main__":
    unittest.main()
