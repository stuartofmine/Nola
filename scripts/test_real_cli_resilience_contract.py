"""Offline contracts for the real-provider acceptance reader."""

import json
import os
import pathlib
import stat
import sqlite3
import tempfile
import unittest
import copy
from types import SimpleNamespace
from unittest.mock import Mock, patch

from test_real_cli_resilience import (InteractiveCLI, canonical_part_text, canonical_text_parts,
    audit_denied_file_invocation, configured_environment, isolated_environment, provider_arguments,
    retained_project, run_outcomes, authentication_fault, sanitized_failure, DEBUG_SECRET,
    audit_wait_receipts, audit_queued_command, observe_empty_wait, command_receipt_snapshot,
    audit_authentication_failure, audit_debug_log_segment, audit_debug_human_answer,
    capture_debug_human_answer, audit_local_context_block)


class PermissionHarnessTests(unittest.TestCase):
    @staticmethod
    def debug_records(terminal="answered"):
        records = [
            ("prompt_composition", 1, {"type":"prompt_composition"}),
            ("request_start", 19, {}),
        ]
        if terminal == "answered":
            records.append(("provider_response", 19, {"type":"provider_response",
                "return_kind":"tool_calls", "content_chars":0, "tool_call_count":1}))
        else:
            records.extend([("request_http_error", 19, {}), ("model_provider_failure", 1,
                {"type":"model_provider_failure", "code":"authentication_failed", "status":401,
                 "retryable":False})])
        records.append(("agent_terminal", 1, {"type":"agent_terminal", "reason":terminal}))
        return [{"phase":phase, "payload":{"run_id":"owned-invocation", "execution_id":execution, **payload}}
                for phase, execution, payload in records]

    @staticmethod
    def debug_log(records):
        return "".join("nola_agent_debug ts=synthetic phase=" + record["phase"] +
            " payload=" + json.dumps(record["payload"]) + "\n" for record in records)

    def test_debug_oracle_binds_real_identity_layers_without_copying_body(self):
        for terminal in ("failed", "answered"):
            result = audit_debug_log_segment(self.debug_log(self.debug_records(terminal)),
                terminal=terminal, forbidden=(DEBUG_SECRET, "private-answer"))
            self.assertEqual(result["terminal"], terminal)
            self.assertTrue(result["providerIdentitiesMatched"])
            self.assertTrue(result["controllerIdentityMatched"])
            self.assertEqual(result["modelRequests"], 1)

    def test_debug_oracle_rejects_mixed_missing_duplicate_or_out_of_order_evidence(self):
        for fault in ("foreign_run", "foreign_controller", "foreign_model", "missing_response",
                      "duplicate_response", "duplicate_terminal", "wrong_terminal", "out_of_order", "missing_execution"):
            with self.subTest(fault=fault):
                records = self.debug_records()
                if fault == "foreign_run":
                    records[2]["payload"]["run_id"] = "foreign"
                elif fault == "foreign_controller":
                    records[-1]["payload"]["execution_id"] = 2
                elif fault == "foreign_model":
                    records[2]["payload"]["execution_id"] = 20
                elif fault == "missing_response":
                    records.pop(2)
                elif fault == "duplicate_response":
                    records.insert(2, copy.deepcopy(records[2]))
                elif fault == "duplicate_terminal":
                    records.append(copy.deepcopy(records[-1]))
                elif fault == "wrong_terminal":
                    records[-1]["payload"]["reason"] = "incomplete"
                elif fault == "out_of_order":
                    records[1], records[2] = records[2], records[1]
                else:
                    records[2]["payload"].pop("execution_id")
                with self.assertRaises(AssertionError):
                    audit_debug_log_segment(self.debug_log(records), terminal="answered", forbidden=(DEBUG_SECRET,))

    def test_debug_oracle_rejects_body_and_secret_leaks_even_with_valid_stages(self):
        for forbidden in (DEBUG_SECRET, "private-answer", "private-request"):
            records = self.debug_records()
            records[2]["payload"]["text"] = forbidden
            with self.assertRaisesRegex(AssertionError, "disclosed"):
                audit_debug_log_segment(self.debug_log(records), terminal="answered", forbidden=(forbidden,))
        records = self.debug_records("failed")
        records[3]["payload"]["status"] = 200
        with self.assertRaises(AssertionError):
            audit_debug_log_segment(self.debug_log(records), terminal="failed", forbidden=(DEBUG_SECRET,))

    def test_no_save_human_answer_accepts_only_exact_current_renderer_wrapping(self):
        receipt = "    • Tool · requirements_update\n\n"
        for output in ("answer", "answer\n", "\r\x1b[2K    answer\n\n", "\n\x1b[2K    answer\n\n",
                       receipt + "\x1b[2K    answer\n"):
            audit_debug_human_answer(output, "answer")
        for output in ("Reply with answer", "answer\nextra", "answer\nanswer", "    answer trailing",
                       "answer\r\x1b[2Kerased", "\x1b[31manswer\x1b[0m", "", "answer\t",
                       receipt + receipt + "\x1b[2K    answer\n", receipt + "answer\n",
                       receipt + "inserted\n\x1b[2K    answer\n",
                       "    • Tool · write_file\n\n\x1b[2K    answer\n",
                       receipt + "\x1b[31m\x1b[2K    answer\n"):
            with self.subTest(output=repr(output)), self.assertRaises(AssertionError):
                audit_debug_human_answer(output, "answer")

    def test_debug_capture_keeps_exact_synthetic_stdout_and_redacts_credentials_before_write(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            stdout = "\n\x1b[2K    answer\n"
            capture_debug_human_answer(root, stdout, (DEBUG_SECRET, "owned-provider-key"))
            path = root / "real-debug-answer.stdout.json"
            self.assertEqual(json.loads(path.read_text())["stdout"], stdout)
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
            self.assertFalse(json.loads(path.read_text())["durableSealProven"])
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            with self.assertRaisesRegex(AssertionError, "disclosed"):
                capture_debug_human_answer(root, DEBUG_SECRET + " owned-provider-key", (DEBUG_SECRET, "owned-provider-key"))
            raw = (root / "real-debug-answer.stdout.json").read_text()
            self.assertNotIn(DEBUG_SECRET, raw)
            self.assertNotIn("owned-provider-key", raw)
            self.assertTrue(json.loads(raw)["credentialDisclosure"])

    def test_provider_failure_is_owned_by_controller_not_http_execution(self):
        for wrong_execution in (19, 2):
            records = self.debug_records("failed")
            records[3]["payload"]["execution_id"] = wrong_execution
            with self.subTest(execution=wrong_execution), self.assertRaisesRegex(AssertionError, "foreign failure"):
                audit_debug_log_segment(self.debug_log(records), terminal="failed", forbidden=(DEBUG_SECRET,))

    def test_empty_assertion_records_stage_and_real_traceback(self):
        with patch("test_real_cli_resilience.CURRENT_STAGE","failed-turn-injected-401"):
            try:
                assert False
            except AssertionError as failure:
                report = sanitized_failure(failure)
        self.assertEqual(report["stage"],"failed-turn-injected-401")
        self.assertEqual(report["primary"]["type"],"AssertionError")
        self.assertIn("test_empty_assertion_records_stage",report["traceback"])
        self.assertIn("AssertionError",report["traceback"])

    def test_failure_keeps_primary_and_cleanup_separate_and_redacts_both_credentials(self):
        failure = RuntimeError("relay cleanup failed")
        failure.primary_failure = AssertionError("real-fixture-key " + DEBUG_SECRET)
        failure.cleanup_failures = [OSError("real-fixture-key " + DEBUG_SECRET)]
        with patch.dict(os.environ,{"NOLA_REAL_TEST_API_KEY":"real-fixture-key"}):
            report = sanitized_failure(failure)
        self.assertEqual(report["primary"]["type"],"AssertionError")
        self.assertEqual(report["cleanup"][0]["type"],"OSError")
        self.assertNotIn("real-fixture-key",json.dumps(report))
        self.assertNotIn(DEBUG_SECRET,json.dumps(report))

    def test_fault_case_keeps_one_endpoint_and_restores_outer_route_after_failure(self):
        environment = {"NOLA_REAL_TEST_ENDPOINT":"http://127.0.0.1:1234/v1",
            "NOLA_REAL_TEST_MODEL":"owned-model","NOLA_REAL_TEST_PROVIDER_LOG":"outer.ndjson",
            "NOLA_REAL_E2E_EXECUTION_CAPABILITIES_JSON":json.dumps({"toolCalls":True,
                "reasoning":True,"imageInput":False,"samplingParameters":False})}
        relay = Mock(endpoint="http://127.0.0.1:4321/v1")
        context = Mock()
        context.__enter__ = Mock(return_value=relay)
        context.__exit__ = Mock(return_value=False)
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ,environment,clear=True), \
                patch("test_real_cli_resilience.AuthenticationFaultRelay",return_value=context) as factory:
            with self.assertRaisesRegex(AssertionError,"primary preserved"):
                with authentication_fault(pathlib.Path(directory),"owned-case"):
                    self.assertEqual(os.environ["NOLA_REAL_TEST_ENDPOINT"],relay.endpoint)
                    self.assertNotEqual(os.environ["NOLA_REAL_TEST_PROVIDER_LOG"],"outer.ndjson")
                    raise AssertionError("primary preserved")
            self.assertEqual(os.environ["NOLA_REAL_TEST_ENDPOINT"],environment["NOLA_REAL_TEST_ENDPOINT"])
            self.assertEqual(os.environ["NOLA_REAL_TEST_PROVIDER_LOG"],"outer.ndjson")
            self.assertEqual(factory.call_args.args[0],environment["NOLA_REAL_TEST_ENDPOINT"])

    def test_retained_evidence_and_only_exact_owned_vault_is_scrubbed(self):
        import hashlib
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory).resolve()
            with retained_project(directory) as project:
                database = project / "nola-e2e.db"
                database.write_bytes(b"retained isolated evidence")
                vault = project / ".nola-host-secrets"
                vault.mkdir()
                name = hashlib.sha256(os.fsencode(database.resolve())).hexdigest() + ".sqlite"
                for suffix in ("", "-wal", "-shm", "-journal"):
                    (vault / (name + suffix)).write_text("isolated credential")
                unrelated = vault / "another-database.sqlite"
                unrelated.write_text("not owned by this runner")
            self.assertEqual(database.read_bytes(), b"retained isolated evidence")
            self.assertEqual(unrelated.read_text(), "not owned by this runner")
            self.assertFalse(any((vault / (name + suffix)).exists() for suffix in ("", "-wal", "-shm", "-journal")))

    def test_only_current_permission_modes_and_no_boolean_translation(self):
        with tempfile.TemporaryDirectory() as directory:
            project = pathlib.Path(directory)
            for profile in ("cautious", "allow_all"):
                environment = isolated_environment(project, profile)
                self.assertEqual(environment["NOLA_PERMISSION_PROFILE"], profile)
                self.assertEqual(environment.get("HOME"), os.environ.get("HOME"))
                for key in ("NOLA_HOME", "NOLA_MAC_APP_SUPPORT", "NOLA_EXTERNAL_EXECUTION_HOME"):
                    self.assertTrue(pathlib.Path(environment[key]).is_relative_to(project))
            for obsolete in ("recommended", "safe", "full_access", "", True):
                with self.assertRaises(ValueError):
                    isolated_environment(project, obsolete)
            with self.assertRaises(TypeError):
                InteractiveCLI(pathlib.Path("unused"), project, "unused", auto_approve=True)

    def test_credentials_are_private_not_argv_and_scrubbed_on_failure(self):
        declaration = json.dumps({"toolCalls": True, "imageInput": False, "reasoning": False,
                                  "samplingParameters": False})
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {
            "HOME": "/unmodified-host-home", "NOLA_HOME": "/must-not-be-used",
            "MODEL_API_KEY": "must-not-propagate", "DATABASE_PATH": "/must-not-be-used",
            "NOLA_CONFIG_PATH": "/must-not-be-read", "NOLA_REAL_TEST_MODEL": "fixture-model",
            "NOLA_REAL_TEST_ENDPOINT": "http://127.0.0.1:1/v1",
            "NOLA_REAL_TEST_API_KEY": "isolated-test-secret",
            "NOLA_REAL_E2E_EXECUTION_CAPABILITIES_JSON": declaration,
        }, clear=True):
            arguments = provider_arguments()
            self.assertNotIn("--api-key", arguments)
            self.assertNotIn("isolated-test-secret", arguments)
            with self.assertRaisesRegex(RuntimeError, "injected"):
                with configured_environment(pathlib.Path(directory)) as environment:
                    configuration = pathlib.Path(environment["NOLA_HOME"]) / ".nola/config.json"
                    self.assertEqual(stat.S_IMODE(configuration.stat().st_mode), 0o600)
                    self.assertEqual(environment["HOME"], "/unmodified-host-home")
                    self.assertNotIn("NOLA_CONFIG_PATH", environment)
                    self.assertNotIn("NOLA_REAL_TEST_API_KEY", environment)
                    self.assertNotIn("MODEL_API_KEY", environment)
                    self.assertNotIn("DATABASE_PATH", environment)
                    self.assertEqual(set(json.loads(configuration.read_text())), {"llm"})
                    raise RuntimeError("injected")
            self.assertFalse(configuration.exists())

    def test_current_oracle_filters_exact_owner_and_requires_formal_read(self):
        with tempfile.TemporaryDirectory() as directory:
            database = pathlib.Path(directory) / "owned.sqlite"
            database.touch()
            oracle = Mock()
            oracle.query.return_value = {"runs": [
                {"conversationId": "foreign", "kind": "answered"},
                {"conversationId": "session", "kind": None},
            ]}
            self.assertEqual(run_outcomes(database, "session", oracle, live=True), [None])
            oracle.query.assert_called_once_with(operation="audit", database=str(database), live=True)
            oracle.query.side_effect = RuntimeError("invalid seal")
            with self.assertRaisesRegex(RuntimeError, "invalid seal"):
                run_outcomes(database, "session", oracle, live=True)
            oracle.query.side_effect = None
            oracle.query.reset_mock()
            run_outcomes(database, "session", oracle, live=False)
            oracle.query.assert_called_once_with(operation="audit", database=str(database), live=False)

    def test_raw_bytes_and_erased_text_are_not_visible_frame_witnesses(self):
        cli = InteractiveCLI.__new__(InteractiveCLI)
        cli.output = bytearray(b"FAKE-SUCCESS\r\x1b[2Kactual")
        cli.terminal = SimpleNamespace(frames=[{"rawOffset": len(cli.output), "screen": "actual"}],
            screen=lambda: "actual", pty_closed=True, pump=lambda: None)
        self.assertEqual(cli.wait_for("actual", 0.1), "actual")
        with self.assertRaises(AssertionError):
            cli.wait_for("FAKE-SUCCESS", 0.1)

    def test_composer_or_cancelled_receipt_cannot_be_successful_answer(self):
        with tempfile.TemporaryDirectory() as directory:
            cli = InteractiveCLI.__new__(InteractiveCLI)
            cli.database = pathlib.Path(directory) / "owned.sqlite"
            cli.database.touch()
            cli.session, cli.oracle = "owner", Mock()
            cli.drain_until_quiet = Mock(return_value="Type a message")
            with patch("test_real_cli_resilience.run_outcomes", return_value=[None]):
                with self.assertRaises(AssertionError):
                    cli.wait_for_answer(1, timeout=0.001)
            for kind in ("cancelled", "incomplete", "blocked"):
                with patch("test_real_cli_resilience.run_outcomes", return_value=[kind]):
                    with self.assertRaises(AssertionError):
                        cli.wait_for_answer(1, timeout=0.1)

    def test_numeric_choice_does_not_match_predecision_composer_or_send_enter(self):
        cli = InteractiveCLI.__new__(InteractiveCLI)
        cli.fd = 42
        cli.output = bytearray(b"Type a message\nReturn to composer")
        with patch("test_real_cli_resilience.os.write") as write, \
             patch.object(cli, "wait_for", return_value="new frame") as wait:
            self.assertEqual(cli.choose_and_wait("3", "Type a message", 20), "new frame")
        write.assert_called_once_with(42, b"3")
        wait.assert_called_once_with("Type a message", 20, start=len(cli.output))

    def test_failure_preserves_raw_capture_and_closes_child(self):
        with tempfile.TemporaryDirectory() as directory:
            cli = InteractiveCLI.__new__(InteractiveCLI)
            cli.database = pathlib.Path(directory) / "test.sqlite"
            cli.session = "fixture"
            cli.output = bytearray(b"\x1b[2Jpermission frame")
            with patch.object(cli, "close") as close:
                cli.__exit__(AssertionError, AssertionError("provider failed"), None)
            close.assert_called_once_with()
            self.assertEqual((pathlib.Path(directory) / "fixture-failed-terminal.raw").read_bytes(), cli.output)

    def test_primary_assertion_and_cleanup_error_are_both_persisted_without_rewriting_expectation(self):
        with tempfile.TemporaryDirectory() as directory:
            cli = InteractiveCLI.__new__(InteractiveCLI)
            cli.database = pathlib.Path(directory) / "test.sqlite"
            cli.session, cli.output = "fixture", bytearray(b"actual terminal")
            original = AssertionError("expected FIRST but canonical answer was SECOND")
            with patch.object(cli, "close", side_effect=PermissionError(1, "group signal denied")):
                with self.assertRaisesRegex(RuntimeError, "Primary AssertionError: expected FIRST") as raised:
                    cli.__exit__(AssertionError, original, None)
            self.assertIs(raised.exception.__cause__, original)
            report = json.loads((pathlib.Path(directory) / "fixture-failure.json").read_text())
            self.assertEqual(report["primary"]["type"], "AssertionError")
            self.assertIn("canonical answer was SECOND", report["primary"]["message"])
            self.assertEqual(report["cleanup"][0]["failure"]["errno"], 1)
            self.assertEqual((pathlib.Path(directory) / "fixture-failed-terminal.raw").read_bytes(), cli.output)

    def test_cleanup_attempts_oracle_and_configuration_even_after_terminal_failure(self):
        cli = InteractiveCLI.__new__(InteractiveCLI)
        cli.terminal = SimpleNamespace(close=Mock(side_effect=PermissionError(1, "denied")))
        cli.oracle = SimpleNamespace(close=Mock(side_effect=RuntimeError("oracle failed")))
        cli.configuration = Mock()
        cli.configuration.__exit__ = Mock()
        with self.assertRaisesRegex(RuntimeError, "cleanup failed"):
            cli.close()
        cli.configuration.__exit__.assert_called_once_with(None, None, None)
        self.assertEqual([failure["stage"] for failure in cli.cleanup_failures], ["terminal", "oracle"])

    def test_cleanup_only_failure_is_not_success_and_error_evidence_redacts_fixture_secret(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ,
                {"NOLA_REAL_TEST_API_KEY":"private-fixture-key"}):
            cli = InteractiveCLI.__new__(InteractiveCLI)
            cli.database = pathlib.Path(directory) / "test.sqlite"
            cli.session, cli.output = "fixture", bytearray()
            with patch.object(cli, "close", side_effect=PermissionError(1, "private-fixture-key")):
                with self.assertRaisesRegex(RuntimeError, "cleanup failed"):
                    cli.__exit__(None, None, None)
            raw = (pathlib.Path(directory) / "fixture-failure.json").read_text()
            self.assertNotIn("private-fixture-key", raw)
            report = json.loads(raw)
            self.assertIsNone(report["primary"])
            self.assertEqual(report["cleanup"][0]["failure"]["errno"], 1)


class EmptyWaitAndQueueReceiptTests(unittest.TestCase):
    def snapshot(self, records=None):
        records = records or {}
        counts = {}
        for record in records.values():
            key = (record["type"],record["status"])
            counts[key] = counts.get(key,0)+1
        return {"records":records,"groups":[{"type":kind,"status":status,"count":count}
            for (kind,status),count in sorted(counts.items())]}

    def record(self, kind, command, result, status="committed"):
        return {"type":kind,"status":status,"command":command,"result":result}

    def queue_fixture(self):
        submission = {"type":"user_input","delivery":"next_turn","submission_id":"original-command",
            "submitted_at_epoch_ms":10,"turn_context_items":[],"turn_context_platform":"cli",
            "turn_context_components":[],"items":[{"type":"message","role":"user","content":"follow up"}]}
        source = {"session_id":"session","command_id":"original-command","created_at_ms":10,
            "payload_json":json.dumps(submission)}
        lease = {**source,"payload_json":json.dumps({"type":"submission","submission":submission}),
            "lease_owner":"original-owner","lease_until_ms":100,"delivery_attempt":1}
        records = {("cli","enqueue"):self.record("enqueue_agent_command",{"input":source},True),
            ("cli","claim"):self.record("claim_next_agent_command",{"session_id":"session","lease_owner":"original-owner"},lease),
            ("cli","ack"):self.record("acknowledge_agent_command",{"session_id":"session",
                "command_id":"original-command","lease_owner":"original-owner"},True)}
        return self.snapshot(), self.snapshot(records)

    def test_wait_uses_primary_keys_and_reports_legitimate_renewal_separately(self):
        before = self.snapshot({("client","old"):self.record("old-completed-work",{},True)})
        renewal = {"run_id":"run","attempt_id":"attempt","worker_generation":"worker",
            "lease_owner":"owner","lease_token":"owned-token"}
        records = {**before["records"],("client","heartbeat"):self.record("heartbeat_effect_delivery",
            {"input":renewal},{key:value for key,value in renewal.items() if key != "run_id"}),
            ("client","trace"):self.record("record_agent_trace",{"event":{"conversation_id":"session"}},True)}
        report = audit_wait_receipts(before,self.snapshot(records),"session","run")
        self.assertEqual(report["leaseRenewalCount"],1)
        self.assertEqual(report["emptyAgentClaimReceipts"],0)
        self.assertEqual(len(report["newReceiptIDs"]),2)
        self.assertNotIn("owned-token",json.dumps(report))

    def test_empty_claim_unknown_write_and_foreign_renewal_cannot_pass_wait(self):
        cases = [("claim_next_agent_command",{},None,"committed"),
            ("claim_next_agent_command",{}, {"command_id":"unexpected-work"},"committed"),
            ("claim_committed_host_requests",{},[],"committed"),
            ("claim_committed_host_requests",{},None,"committed"),
            ("unexpected_write",{},True,"committed"),
            ("record_agent_trace",{"event":{"conversation_id":"foreign"}},True,"committed"),
            ("heartbeat_effect_delivery",{"input":{"run_id":"foreign"}},None,"committed"),
            ("record_agent_trace",{"event":{"conversation_id":"session"}},None,"accepted")]
        for kind,command,result,status in cases:
            with self.subTest(kind=kind,result=result,status=status), self.assertRaises(AssertionError):
                audit_wait_receipts(self.snapshot(),self.snapshot({("cli","new"):self.record(kind,command,result,status)}),"session","run")

    def test_changed_existing_claim_and_removed_receipts_are_not_hidden_by_new_id_count(self):
        before = self.snapshot({("cli","existing"):self.record("claim_next_agent_command",{},None,"accepted")})
        for after in (self.snapshot({("cli","existing"):self.record("claim_next_agent_command",{},None)}),self.snapshot()):
            with self.subTest(after=after), self.assertRaises(AssertionError):
                audit_wait_receipts(before,after,"session",None)

    def test_queue_requires_original_nonempty_claim_and_exact_ack(self):
        before, after = self.queue_fixture()
        report = audit_queued_command(before,after,"session","follow up")
        self.assertEqual(report["commandID"],"original-command")
        self.assertTrue(report["originalCommandAcknowledged"])
        changes = [lambda records:records.pop(("cli","claim")),
            lambda records:records[("cli","claim")].update(result=None),
            lambda records:records.pop(("cli","ack")),
            lambda records:records[("cli","ack")].update(type="permission_response"),
            lambda records:records[("cli","claim")]["result"].update(session_id="foreign"),
            lambda records:records[("cli","claim")]["result"].update(delivery_attempt=2),
            lambda records:records[("cli","claim")]["result"].update(payload_json="{}"),
            lambda records:records[("cli","ack")]["command"].update(lease_owner="foreign"),
            lambda records:records[("cli","ack")]["command"].update(command_id="another-command"),
            lambda records:records[("cli","ack")].update(result=False),
            lambda records:records.update({("cli","duplicate"):copy.deepcopy(records[("cli","ack")])})]
        for index, change in enumerate(changes):
            with self.subTest(case=index):
                records = copy.deepcopy(after["records"])
                change(records)
                with self.assertRaises(AssertionError):
                    audit_queued_command(before,self.snapshot(records),"session","follow up")

    def test_queue_claim_accepts_only_one_exact_current_submission_wrapper(self):
        before, after = self.queue_fixture()
        original = json.loads(after["records"][("cli","enqueue")]["command"]["input"]["payload_json"])
        wrong_body = copy.deepcopy(original)
        wrong_body["items"][0]["content"] = "another answer"
        wrong_context = copy.deepcopy(original)
        wrong_context["turn_context_items"] = [{"type":"message","role":"user","content":"extra context"}]
        wrong_time_type = copy.deepcopy(original)
        wrong_time_type["submitted_at_epoch_ms"] = 10.0
        wrong_nested_extra = copy.deepcopy(original)
        wrong_nested_extra["items"][0]["unexpected"] = True
        candidates = [original, {"type":"other","submission":original},
            {"type":"submission","submission":original,"unexpected":True},
            {"type":"submission","submission":{"type":"submission","submission":original}},
            {"type":"submission","submission":wrong_body},
            {"type":"submission","submission":wrong_context},
            {"type":"submission","submission":wrong_time_type},
            {"type":"submission","submission":wrong_nested_extra}]
        for index, candidate in enumerate(candidates):
            with self.subTest(case=index):
                records = copy.deepcopy(after["records"])
                records[("cli","claim")]["result"]["payload_json"] = json.dumps(candidate)
                with self.assertRaises(AssertionError):
                    audit_queued_command(before,self.snapshot(records),"session","follow up")
        # Serialization key order is not a different admitted submission.
        after["records"][("cli","claim")]["result"]["payload_json"] = json.dumps(
            {"submission":original,"type":"submission"},sort_keys=True)
        self.assertTrue(audit_queued_command(before,after,"session","follow up")["originalCommandAcknowledged"])

    def test_queue_claim_rejects_changed_original_timestamp_or_identity(self):
        before, after = self.queue_fixture()
        for field, value in (("created_at_ms",11),("created_at_ms",10.0),("command_id","foreign")):
            with self.subTest(field=field,value=value):
                records = copy.deepcopy(after["records"])
                records[("cli","claim")]["result"][field] = value
                with self.assertRaises(AssertionError):
                    audit_queued_command(before,self.snapshot(records),"session","follow up")

    def test_receipt_snapshot_preserves_typed_groups_and_null_claim(self):
        with tempfile.TemporaryDirectory() as directory:
            database = pathlib.Path(directory)/"owned.sqlite"
            with sqlite3.connect(database) as connection:
                connection.executescript("CREATE TABLE command_receipts(client_id TEXT,request_id TEXT,command_type TEXT,status TEXT,command_json TEXT,result_json TEXT)")
                connection.execute("INSERT INTO command_receipts VALUES ('cli','request','claim_next_agent_command','committed','{}','null')")
            snapshot = command_receipt_snapshot(database)
            self.assertIsNone(snapshot["records"][("cli","request")]["result"])
            with self.assertRaises(AssertionError):
                audit_wait_receipts(self.snapshot(),snapshot,"session",None)

    def test_observation_pumps_thirty_seconds_without_input_and_rejects_changed_permission(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            target = root/"existing.txt"
            target.write_bytes(b"original")
            permission = {"request_id":"permission","call_id":"call","turn_id":"run","capability_id":"write_file",
                "arguments":{"path":str(target)},"resource":json.dumps({"resources":[{"kind":"file","access":"write",
                    "components":list(target.resolve().parts[1:])}]})}
            permissions = {"revision":2,"pending":[permission],"receipts":[]}
            for changed in ("stable","request","owner","bytes"):
                target.write_bytes(b"original")
                terminal = SimpleNamespace(reaped=False,pty_closed=False,pump=Mock(),
                    _owned_group=Mock(return_value=[{"pid":123,"state":"Z" if changed == "owner" else "S"}]))
                if changed == "bytes":
                    terminal.pump.side_effect = lambda:target.write_bytes(b"changed")
                cli = SimpleNamespace(database=root/"db",session="session",pid=123,terminal=terminal)
                later = copy.deepcopy(permissions)
                if changed == "request":
                    later["pending"][0]["request_id"] = "replacement"
                reads = [SimpleNamespace(returncode=0,stdout=json.dumps({"id":"session","permission_projection":value}))
                    for value in (permissions,later)]
                with patch("test_real_cli_resilience.configured_run",side_effect=reads), \
                        patch("test_real_cli_resilience.command_receipt_snapshot",return_value=self.snapshot()), \
                        patch("test_real_cli_resilience.time.monotonic",side_effect=[0,0,31,31]):
                    if changed != "stable":
                        with self.assertRaises(AssertionError):
                            observe_empty_wait(cli,root/"nola",root,target,pending=True)
                    else:
                        report = observe_empty_wait(cli,root/"nola",root,target,pending=True)
                        self.assertGreaterEqual(report["durationSeconds"],30)
                        terminal.pump.assert_called_once_with()


class DenyEvidenceTests(unittest.TestCase):
    def fixture(self, directory):
        root = pathlib.Path(directory)
        database, target = root / "owned.sqlite", (root / "existing.txt").resolve()
        original = b"unchanged owned fixture"
        target.write_bytes(original)
        resource = {"kind":"file","filesystem_id":"owned-filesystem","access":"write",
            "components":list(target.parts[1:])}
        subject = {"execution_target":"fixture-host","capability":"write_file",
            "operation":"filesystem","resources":[resource]}
        receipt = {"request_id":"permission","turn_id":"run","call_id":"write-call",
            "capability_id":"write_file","resource":json.dumps(subject),
            "outcome":{"type":"authorization","decision":"deny"}}
        core = {"session_id":"conversation","turn_id":"run","revision":2,"cursor":{"sequence":22}}
        part = {"kind":"tool","id":"tool-part","turn_id":"run","message_id":"assistant",
            "call_id":"write-call","capability_id":"write_file","state":"failed",
            "permission":{"input_preview":"{\"request_id\":\"per", "truncated":True}}
        value = {"kind":"blocked","reason":{"code":"permission.denied","detail":None}}
        result = {"result_id":"result","run_id":"run","conversation_id":"conversation","value":value}
        seal = {"identity":{"run_id":"run","conversation_id":"conversation","assistant_message_id":"assistant"},
            "completed_at_ms":100,"result":result}
        events = [(2,"permission_settled",{"type":"permission_settled","receipt":receipt,"core_envelope":core}),
            (3,"part_updated",{"type":"part_updated","part":part}),
            (4,"run_result_decided",{"decision":{"result_id":"result","value":value}}),
            (5,"turn_completed",{"turn_id":"run"})]
        host_base = {"session_id":"conversation","turn_id":"run","host_id":"fixture-host"}
        resolution = {**host_base,"effect_id":"permission-effect","request_id":"permission-rpc",
            "payload":{"type":"permission_resolved","request_id":"permission",
                "resolution":{"type":"authorization","decision":"deny"}}}
        resources = {**host_base,"effect_id":"resources-effect","request_id":"resources-rpc",
            "payload":{"type":"permission_resources_resolved","call_id":"write-call","facts":{
                "execution_target":"fixture-host","resolution_id":"resolution","invocation_digest":"synthetic",
                "bindings":[{"resource":resource,"observation":"overwrite","evidence_id":"resource-evidence",
                    "demand":{"locator_kind":"file_path","access":"write","locator":str(target)}}]}}}
        model = {**host_base,"effect_id":"model-effect","request_id":"model-rpc",
            "payload":{"type":"model_completed","items":[],"tool_calls":[]}}
        hosts = (resolution, resources, model)
        deliveries = []
        for host in hosts:
            identity = host["effect_id"] + "-attempt"
            host["delivery"] = {"attempt_id":identity,"worker_generation":"worker",
                "lease_owner":"owner","lease_token":"fixture-token"}
            record = {"id":identity,"run_id":"run","effect_id":host["effect_id"],
                "request_id":host["request_id"],"state":"settled","execution_started_at_ms":1,
                "worker_generation":"worker","lease_owner":"owner","lease_token":"fixture-token"}
            deliveries.append((identity,host["effect_id"],host["request_id"],json.dumps(record)))
        connection = sqlite3.connect(database)
        try:
            connection.executescript("""
                CREATE TABLE runs(id TEXT,conversation_id TEXT,kind TEXT,status TEXT,
                    assistant_message_id TEXT,completed_at_ms INTEGER,created_at_ms INTEGER);
                CREATE TABLE run_events(run_id TEXT,sequence INTEGER,event_kind TEXT,payload_json TEXT);
                CREATE TABLE run_results(run_id TEXT,result_json TEXT);
                CREATE TABLE assistant_run_terminal_seals(run_id TEXT,conversation_id TEXT,seal_json TEXT);
                CREATE TABLE host_event_inbox(run_id TEXT,effect_id TEXT,request_id TEXT,payload_json TEXT);
                CREATE TABLE effect_outbox(run_id TEXT,effect_id TEXT,effect_kind TEXT);
                CREATE TABLE effect_delivery_attempts(id TEXT,run_id TEXT,effect_id TEXT,
                    request_id TEXT,state TEXT,record_json TEXT);
            """)
            connection.execute("INSERT INTO runs VALUES ('run','conversation','conversation','sealed','assistant',100,1)")
            connection.execute("INSERT INTO run_results VALUES ('run',?)", (json.dumps(result),))
            connection.execute("INSERT INTO assistant_run_terminal_seals VALUES ('run','conversation',?)", (json.dumps(seal),))
            connection.executemany("INSERT INTO run_events VALUES ('run',?,?,?)",
                [(sequence,kind,json.dumps(event)) for sequence,kind,event in events])
            connection.executemany("INSERT INTO host_event_inbox VALUES ('run',?,?,?)",
                [(event["effect_id"],event["request_id"],json.dumps(event)) for event in hosts])
            connection.executemany("INSERT INTO effect_delivery_attempts VALUES (?,'run',?,?,'settled',?)",deliveries)
            connection.executemany("INSERT INTO effect_outbox VALUES ('run',?,?)",
                [("permission-effect","request_permission"),("resources-effect","resolve_permission_resources"),
                    ("model-effect","model_stream")])
            connection.commit()
        finally:
            connection.close()
        return database, target, original

    def mutate(self, database, table, where, change):
        column = "payload_json"
        with sqlite3.connect(database) as connection:
            rowid, raw = connection.execute(f"SELECT rowid,{column} FROM {table} WHERE {where}").fetchone()
            value = json.loads(raw)
            change(value)
            connection.execute(f"UPDATE {table} SET {column}=? WHERE rowid=?", (json.dumps(value),rowid))

    def test_truncated_permission_preview_does_not_replace_exact_formal_denial(self):
        with tempfile.TemporaryDirectory() as directory:
            database, target, original = self.fixture(directory)
            result = audit_denied_file_invocation(database,"conversation",target,original)
            self.assertEqual(result["decision"],"deny")
            self.assertEqual(result["tool_state"],"failed")
            self.assertEqual(result["reason"],"permission.denied")
            self.assertEqual(result["physical_executions"],0)
            # Even a misleading bounded preview cannot override formal facts.
            self.mutate(database,"run_events","event_kind='part_updated'",
                lambda event:event["part"].update(permission={"decision":"allow_once","truncated":True}))
            self.assertEqual(audit_denied_file_invocation(database,"conversation",target,original),result)

    def test_pruned_outbox_still_uses_exact_durable_non_execution_deliveries(self):
        with tempfile.TemporaryDirectory() as directory:
            database, target, original = self.fixture(directory)
            with sqlite3.connect(database) as connection:
                connection.execute("DELETE FROM effect_outbox")
            self.assertEqual(audit_denied_file_invocation(database,"conversation",target,original)["physical_executions"],0)

    def test_foreign_settlement_identity_or_non_deny_decision_is_rejected(self):
        changes = [lambda event:event["core_envelope"].update(session_id="foreign"),
            lambda event:event["core_envelope"].update(turn_id="foreign"),
            lambda event:event["receipt"].update(turn_id="foreign"),
            lambda event:event["receipt"].update(call_id="foreign"),
            lambda event:event["receipt"].update(request_id="foreign"),
            lambda event:event["receipt"].update(capability_id="bash"),
            lambda event:event["receipt"].update(outcome={"type":"return_to_composer"}),
            lambda event:event["receipt"].update(outcome={"type":"authorization","decision":"allow_once"})]
        for index, change in enumerate(changes):
            with self.subTest(case=index), tempfile.TemporaryDirectory() as directory:
                database, target, original = self.fixture(directory)
                self.mutate(database,"run_events","event_kind='permission_settled'",change)
                with self.assertRaises(AssertionError):
                    audit_denied_file_invocation(database,"conversation",target,original)

    def test_host_owner_request_and_resource_binding_are_exact(self):
        cases = [("permission-effect",lambda event:event.update(session_id="foreign")),
            ("permission-effect",lambda event:event.update(turn_id="foreign")),
            ("permission-effect",lambda event:event.update(effect_id="foreign")),
            ("permission-effect",lambda event:event["payload"].update(request_id="foreign")),
            ("permission-effect",lambda event:event.update(host_id="foreign")),
            ("permission-effect",lambda event:event["payload"].update(resolution={"type":"return_to_composer"})),
            ("resources-effect",lambda event:event["payload"].update(call_id="foreign")),
            ("resources-effect",lambda event:event["payload"]["facts"]["bindings"][0]["resource"].update(components=["foreign"])),
            ("resources-effect",lambda event:event["payload"]["facts"]["bindings"][0]["demand"].update(locator="/foreign"))]
        for index, (effect,change) in enumerate(cases):
            with self.subTest(case=index), tempfile.TemporaryDirectory() as directory:
                database, target, original = self.fixture(directory)
                self.mutate(database,"host_event_inbox",f"effect_id='{effect}'",change)
                with self.assertRaises(AssertionError):
                    audit_denied_file_invocation(database,"conversation",target,original)

    def test_settled_resource_cannot_be_another_fixture(self):
        with tempfile.TemporaryDirectory() as directory:
            database, target, original = self.fixture(directory)
            def replace(event):
                subject = json.loads(event["receipt"]["resource"])
                subject["resources"][0]["components"] = ["other-file"]
                event["receipt"]["resource"] = json.dumps(subject)
            self.mutate(database,"run_events","event_kind='permission_settled'",replace)
            with self.assertRaisesRegex(AssertionError,"resource differs"):
                audit_denied_file_invocation(database,"conversation",target,original)

    def test_missing_or_duplicate_formal_and_host_evidence_is_rejected(self):
        for table, where in [("run_events","event_kind='permission_settled'"),
                ("host_event_inbox","effect_id='permission-effect'"),
                ("host_event_inbox","effect_id='resources-effect'")]:
            for operation in ("missing","duplicate"):
                with self.subTest(table=table,operation=operation), tempfile.TemporaryDirectory() as directory:
                    database, target, original = self.fixture(directory)
                    with sqlite3.connect(database) as connection:
                        if operation == "missing":
                            connection.execute(f"DELETE FROM {table} WHERE {where}")
                        else:
                            connection.execute(f"INSERT INTO {table} SELECT * FROM {table} WHERE {where}")
                    with self.assertRaises(AssertionError):
                        audit_denied_file_invocation(database,"conversation",target,original)

    def test_cancelled_tool_or_substitute_call_is_not_a_denied_failed_invocation(self):
        for changed in ("cancelled","substitute"):
            with self.subTest(case=changed), tempfile.TemporaryDirectory() as directory:
                database, target, original = self.fixture(directory)
                with sqlite3.connect(database) as connection:
                    raw = connection.execute("SELECT payload_json FROM run_events WHERE event_kind='part_updated'").fetchone()[0]
                    event = json.loads(raw)
                    if changed == "cancelled":
                        event["part"]["state"] = "cancelled"
                        connection.execute("UPDATE run_events SET payload_json=? WHERE event_kind='part_updated'",(json.dumps(event),))
                    else:
                        event["part"].update(id="other-part",call_id="other-call",capability_id="bash")
                        connection.execute("INSERT INTO run_events VALUES ('run',6,'part_updated',?)",(json.dumps(event),))
                with self.assertRaises(AssertionError):
                    audit_denied_file_invocation(database,"conversation",target,original)

    def test_denial_cannot_have_execution_or_changed_original_bytes(self):
        for changed in ("dispatch","evidence","bytes","pruned_attempt"):
            with self.subTest(case=changed), tempfile.TemporaryDirectory() as directory:
                database, target, original = self.fixture(directory)
                with sqlite3.connect(database) as connection:
                    if changed == "dispatch":
                        connection.execute("INSERT INTO effect_outbox VALUES ('run','physical','execute_tool')")
                    elif changed == "evidence":
                        connection.execute("INSERT INTO run_events VALUES ('run',6,'execution_evidence_observed','{}')")
                    elif changed == "pruned_attempt":
                        connection.execute("DELETE FROM effect_outbox")
                        record = {"id":"physical","run_id":"run","effect_id":"unknown-effect",
                            "request_id":"unknown-rpc","state":"settled","execution_started_at_ms":9,
                            "worker_generation":"worker","lease_owner":"owner","lease_token":"token"}
                        connection.execute("INSERT INTO effect_delivery_attempts VALUES ('physical','run','unknown-effect','unknown-rpc','settled',?)",
                            (json.dumps(record),))
                        source = {"effect_id":"unknown-effect","request_id":"unknown-rpc",
                            "session_id":"conversation","turn_id":"run","host_id":"fixture-host",
                            "delivery":{"attempt_id":"physical","worker_generation":"worker","lease_owner":"owner","lease_token":"token"},
                            "payload":{"type":"execution_started"}}
                        connection.execute("INSERT INTO host_event_inbox VALUES ('run','unknown-effect','unknown-rpc',?)",
                            (json.dumps(source),))
                    else:
                        target.write_bytes(b"changed")
                with self.assertRaises(AssertionError):
                    audit_denied_file_invocation(database,"conversation",target,original)

    def test_missing_or_foreign_durable_delivery_fence_cannot_prove_zero_execution(self):
        for changed in ("missing","request","owner","fence"):
            with self.subTest(case=changed), tempfile.TemporaryDirectory() as directory:
                database, target, original = self.fixture(directory)
                if changed == "missing":
                    with sqlite3.connect(database) as connection:
                        connection.execute("DELETE FROM effect_delivery_attempts WHERE effect_id='permission-effect'")
                elif changed == "request":
                    self.mutate(database,"host_event_inbox","effect_id='model-effect'",
                        lambda event:event.update(request_id="foreign"))
                elif changed == "owner":
                    self.mutate(database,"host_event_inbox","effect_id='model-effect'",
                        lambda event:event.update(session_id="foreign"))
                else:
                    self.mutate(database,"host_event_inbox","effect_id='model-effect'",
                        lambda event:event["delivery"].update(lease_token="foreign"))
                with self.assertRaises(AssertionError):
                    audit_denied_file_invocation(database,"conversation",target,original)

    def test_blocked_must_be_permission_denied_not_an_unrelated_blocker(self):
        with tempfile.TemporaryDirectory() as directory:
            database, target, original = self.fixture(directory)
            with sqlite3.connect(database) as connection:
                result = json.loads(connection.execute("SELECT result_json FROM run_results").fetchone()[0])
                result["value"]["reason"]["code"] = "budget.exhausted"
                seal = json.loads(connection.execute("SELECT seal_json FROM assistant_run_terminal_seals").fetchone()[0])
                seal["result"] = result
                connection.execute("UPDATE run_results SET result_json=?",(json.dumps(result),))
                connection.execute("UPDATE assistant_run_terminal_seals SET seal_json=?",(json.dumps(seal),))
            self.mutate(database,"run_events","event_kind='run_result_decided'",
                lambda event:event["decision"].update(value=result["value"]))
            with self.assertRaisesRegex(AssertionError,"not a permission denial"):
                audit_denied_file_invocation(database,"conversation",target,original)


class LocalContextBudgetEvidenceTests(unittest.TestCase):
    def fixture(self, directory):
        database, _target, _original = DenyEvidenceTests().fixture(directory)
        prompt = "The complete, unmodified current input."
        value = {"kind":"blocked", "reason":{"code":"context.input_budget_exceeded", "detail":None},
            "blocking_fact_refs":["budget-fact"], "resolution_condition":"Reduce mandatory input."}
        fact = {"turn_id":"run", "fact":{"fact_id":"budget-fact",
            "kind":{"kind":"blocker", "code":"context.input_budget_exceeded"}}}
        with sqlite3.connect(database) as connection:
            result = json.loads(connection.execute("SELECT result_json FROM run_results").fetchone()[0])
            result["value"] = value
            seal = json.loads(connection.execute("SELECT seal_json FROM assistant_run_terminal_seals").fetchone()[0])
            seal["result"] = result
            connection.execute("UPDATE run_results SET result_json=?", (json.dumps(result),))
            connection.execute("UPDATE assistant_run_terminal_seals SET seal_json=?", (json.dumps(seal),))
            for table in ("run_events", "host_event_inbox", "effect_delivery_attempts", "effect_outbox"):
                connection.execute(f"DELETE FROM {table}")
            connection.executemany("INSERT INTO run_events VALUES ('run',?,?,?)", [
                (1,"runtime_decision_fact_observed",json.dumps(fact)),
                (2,"run_result_decided",json.dumps({"decision":{"result_id":result["result_id"],"value":value}})),
                (3,"turn_completed",json.dumps({"type":"turn_completed"}))])
            connection.executescript("""
                CREATE TABLE run_attempts(id TEXT,run_id TEXT);
                CREATE TABLE messages(id TEXT,role TEXT,conversation_id TEXT,ordinal INTEGER);
                CREATE TABLE message_parts(id TEXT,message_id TEXT,kind TEXT,ordinal INTEGER,
                    content_text TEXT,content_json TEXT);
                CREATE TABLE message_part_body_references(part_id TEXT,field TEXT,reference_json TEXT);
                CREATE TABLE canonical_text_chunks(content_id TEXT,offset_bytes INTEGER,text TEXT);
                INSERT INTO messages VALUES ('input','user','conversation',0);
            """)
            connection.execute("INSERT INTO message_parts VALUES ('input','input','text',0,?,'')", (prompt,))
        return database, prompt

    def test_known_impossible_input_has_a_formal_blocker_no_attempt_and_exact_body(self):
        with tempfile.TemporaryDirectory() as directory:
            database, prompt = self.fixture(directory)
            result = audit_local_context_block(database, "conversation", prompt)
            self.assertEqual(result["runKind"], "blocked")
            self.assertTrue(result["canonicalExactInput"])
            self.assertEqual(result["physicalAttempts"], 0)

    def test_local_block_cannot_hide_pruned_attempt_foreign_fact_or_changed_input(self):
        for fault in ("attempt", "delivery", "host", "foreign_fact", "missing_fact", "changed_input", "assistant"):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as directory:
                database, prompt = self.fixture(directory)
                with sqlite3.connect(database) as connection:
                    if fault == "attempt":
                        connection.execute("INSERT INTO run_attempts VALUES ('physical','run')")
                    elif fault == "delivery":
                        connection.execute("INSERT INTO effect_delivery_attempts VALUES ('physical','run','pruned-effect','request','settled','{}')")
                    elif fault == "host":
                        connection.execute("INSERT INTO host_event_inbox VALUES ('run','effect','request','{}')")
                    elif fault == "foreign_fact":
                        raw = json.loads(connection.execute("SELECT payload_json FROM run_events WHERE event_kind='runtime_decision_fact_observed'").fetchone()[0])
                        raw["turn_id"] = "foreign-run"
                        connection.execute("UPDATE run_events SET payload_json=? WHERE event_kind='runtime_decision_fact_observed'", (json.dumps(raw),))
                    elif fault == "missing_fact":
                        connection.execute("DELETE FROM run_events WHERE event_kind='runtime_decision_fact_observed'")
                    elif fault == "changed_input":
                        connection.execute("UPDATE message_parts SET content_text='truncated'")
                    else:
                        connection.execute("INSERT INTO messages VALUES ('answer','assistant','conversation',1)")
                        connection.execute("INSERT INTO message_parts VALUES ('answer','answer','text',0,'fabricated answer','')")
                with self.assertRaises(AssertionError):
                    audit_local_context_block(database, "conversation", prompt)


class AuthenticationFailureEvidenceTests(unittest.TestCase):
    def fixture(self, directory):
        database, _target, _original = DenyEvidenceTests().fixture(directory)
        failure = {"code":"authentication_failed","http_status":401,"retryable":False,
            "message":"The provider rejected its credentials."}
        value = {"kind":"failed","error_ref":"error-fact","observed_effect_refs":[],"unknown_effect_refs":[]}
        core = {"session_id":"conversation","turn_id":"run","revision":2,"cursor":{"sequence":3}}
        facts = {"type":"runtime_decision_fact_observed","turn_id":"run","core_envelope":core,
            "fact":{"fact_id":"error-fact","kind":{"kind":"error","code":"runtime_failure","detail":failure["message"]}}}
        terminal = {"type":"turn_failed","session_id":"conversation","turn_id":"run",
            "core_envelope":core,"error":failure["message"]}
        attempt = {"id":"model-attempt","run_id":"run","effect_id":"model-effect",
            "invocation_id":"model-effect","state":"failed","failure":failure}
        delivery = {"attempt_id":"physical-attempt","worker_generation":"worker",
            "lease_owner":"owner","lease_token":"owned-token"}
        host = {"effect_id":"model-effect","request_id":"model-rpc","session_id":"conversation",
            "turn_id":"run","host_id":"host","attempt_id":"model-attempt","delivery":delivery,
            "payload":{"type":"failed","error":failure}}
        physical = {"id":"physical-attempt","run_id":"run","effect_id":"model-effect","request_id":"model-rpc",
            "state":"settled",**{key:val for key,val in delivery.items() if key != "attempt_id"}}
        with sqlite3.connect(database) as connection:
            result = json.loads(connection.execute("SELECT result_json FROM run_results").fetchone()[0])
            result["value"] = value
            seal = json.loads(connection.execute("SELECT seal_json FROM assistant_run_terminal_seals").fetchone()[0])
            seal["result"] = result
            connection.execute("UPDATE run_results SET result_json=?",(json.dumps(result),))
            connection.execute("UPDATE assistant_run_terminal_seals SET seal_json=?",(json.dumps(seal),))
            connection.execute("DELETE FROM run_events")
            connection.executemany("INSERT INTO run_events VALUES ('run',?,?,?)",[
                (12,"turn_failed",json.dumps(terminal)),
                (13,"runtime_decision_fact_observed",json.dumps(facts)),
                (14,"run_result_decided",json.dumps({"decision":{"result_id":result["result_id"],"value":value}}))])
            connection.execute("DELETE FROM host_event_inbox")
            connection.execute("INSERT INTO host_event_inbox VALUES ('run','model-effect','model-rpc',?)",(json.dumps(host),))
            connection.execute("DELETE FROM effect_delivery_attempts")
            connection.execute("INSERT INTO effect_delivery_attempts VALUES ('physical-attempt','run','model-effect','model-rpc','settled',?)",(json.dumps(physical),))
            connection.executescript("CREATE TABLE run_attempts(id TEXT,run_id TEXT,effect_id TEXT,record_json TEXT); CREATE TABLE run_attempt_deliveries(attempt_id TEXT,request_id TEXT); CREATE TABLE run_invocation_payloads(run_id TEXT,config_json TEXT)")
            connection.execute("INSERT INTO run_invocation_payloads VALUES ('run',?)",
                (json.dumps({"session_id":"conversation","host_catalog":{"host_id":"host"}}),))
            connection.execute("INSERT INTO run_attempts VALUES ('model-attempt','run','model-effect',?)",(json.dumps(attempt),))
            connection.execute("INSERT INTO run_attempt_deliveries VALUES ('model-attempt','model-rpc')")
        return database

    def mutate(self, database, table, column, change, where="1=1"):
        with sqlite3.connect(database) as connection:
            row, raw = connection.execute(f"SELECT rowid,{column} FROM {table} WHERE {where}").fetchone()
            value = json.loads(raw)
            change(value)
            connection.execute(f"UPDATE {table} SET {column}=? WHERE rowid=?",(json.dumps(value),row))

    def test_normalized_human_message_is_not_the_authentication_oracle(self):
        with tempfile.TemporaryDirectory() as directory:
            database = self.fixture(directory)
            report = audit_authentication_failure(database,"conversation")
            self.assertEqual(report["failureCode"],"authentication_failed")
            self.assertEqual(report["httpStatus"],401)
            self.assertEqual(report["runKind"],"failed")
            self.assertEqual(report["attemptID"],"model-attempt")

    def test_wrong_host_owner_request_attempt_or_typed_failure_is_rejected(self):
        changes = [lambda host:host.update(session_id="foreign"),lambda host:host.update(turn_id="foreign"),
            lambda host:host.update(effect_id="foreign"),lambda host:host.update(request_id="foreign"),
            lambda host:host.update(attempt_id="foreign"),lambda host:host.update(host_id=""),
            lambda host:host.update(host_id="foreign"),
            lambda host:host["payload"]["error"].update(code="provider_overloaded"),
            lambda host:host["payload"]["error"].update(http_status=403),
            lambda host:host["payload"]["error"].update(http_status=True),
            lambda host:host["payload"]["error"].update(retryable=True),
            lambda host:host["delivery"].update(lease_owner="foreign")]
        for index, change in enumerate(changes):
            with self.subTest(case=index), tempfile.TemporaryDirectory() as directory:
                database = self.fixture(directory)
                self.mutate(database,"host_event_inbox","payload_json",change)
                with self.assertRaises(AssertionError):
                    audit_authentication_failure(database,"conversation")

    def test_attempt_or_error_fact_cannot_be_borrowed_from_another_owner(self):
        cases = [("run_attempts","record_json",lambda attempt:attempt.update(run_id="foreign"),"1=1"),
            ("run_attempts","record_json",lambda attempt:attempt.update(state="succeeded"),"1=1"),
            ("run_attempts","record_json",lambda attempt:attempt.update(invocation_id="foreign"),"1=1"),
            ("run_attempts","record_json",lambda attempt:attempt["failure"].update(http_status=403),"1=1"),
            ("run_events","payload_json",lambda event:event["core_envelope"].update(session_id="foreign"),"event_kind='runtime_decision_fact_observed'"),
            ("run_events","payload_json",lambda event:event["fact"].update(fact_id="foreign"),"event_kind='runtime_decision_fact_observed'"),
            ("run_events","payload_json",lambda event:event.update(error="another failure"),"event_kind='turn_failed'")]
        for index, (table,column,change,where) in enumerate(cases):
            with self.subTest(case=index), tempfile.TemporaryDirectory() as directory:
                database = self.fixture(directory)
                self.mutate(database,table,column,change,where)
                with self.assertRaises(AssertionError):
                    audit_authentication_failure(database,"conversation")

    def test_missing_duplicate_or_mismatched_durable_sources_fail_closed(self):
        statements = ["DELETE FROM assistant_run_terminal_seals", "DELETE FROM host_event_inbox",
            "INSERT INTO host_event_inbox SELECT * FROM host_event_inbox",
            "DELETE FROM run_attempts", "INSERT INTO run_attempts SELECT * FROM run_attempts",
            "DELETE FROM run_attempt_deliveries", "UPDATE run_attempt_deliveries SET request_id='foreign'",
            "DELETE FROM effect_delivery_attempts", "UPDATE effect_delivery_attempts SET state='executing'",
            "DELETE FROM run_invocation_payloads", "INSERT INTO run_invocation_payloads SELECT * FROM run_invocation_payloads",
            "DELETE FROM run_events WHERE event_kind='runtime_decision_fact_observed'",
            "INSERT INTO run_events SELECT * FROM run_events WHERE event_kind='runtime_decision_fact_observed'"]
        for statement in statements:
            with self.subTest(statement=statement), tempfile.TemporaryDirectory() as directory:
                database = self.fixture(directory)
                with sqlite3.connect(database) as connection:
                    connection.execute(statement)
                with self.assertRaises((AssertionError,ValueError)):
                    audit_authentication_failure(database,"conversation")


class CanonicalTextReaderTests(unittest.TestCase):
    def test_reads_typed_chunked_bodies_in_order_without_trace_echoes(self):
        with tempfile.TemporaryDirectory() as directory:
            database = pathlib.Path(directory) / "test.sqlite"
            with sqlite3.connect(database) as connection:
                connection.executescript("""
                    CREATE TABLE messages(id TEXT, role TEXT, conversation_id TEXT, ordinal INTEGER);
                    CREATE TABLE message_parts(id TEXT, message_id TEXT, kind TEXT, ordinal INTEGER,
                        content_text TEXT, content_json TEXT);
                    CREATE TABLE message_part_body_references(part_id TEXT, field TEXT, reference_json TEXT);
                    CREATE TABLE canonical_text_chunks(content_id TEXT, offset_bytes INTEGER, text TEXT);
                    INSERT INTO messages VALUES ('u','user','session',0), ('a','assistant','session',1),
                        ('other','assistant','other-session',0);
                    INSERT INTO message_parts VALUES ('u','u','text',0,'question',''),
                        ('a','a','text',0,'',''), ('b','a','text',1,'',''),
                        ('reasoning','a','reasoning',2,'not an answer',''),
                        ('other','other','text',0,'not this session','');
                """)
                encoded = json.dumps({"kind": "text", "text": "答案一"}, ensure_ascii=False)
                connection.execute("INSERT INTO message_part_body_references VALUES (?,?,?)",
                    ('a', 'content_json', json.dumps({'content_id': 'body'})))
                # Deliberately insert chunks out of order, using UTF-8 offsets.
                split = 15
                connection.executemany("INSERT INTO canonical_text_chunks VALUES (?,?,?)", [
                    ('body', len(encoded[:split].encode()), encoded[split:]),
                    ('body', 0, encoded[:split]),
                ])
                connection.execute("UPDATE message_parts SET content_json=? WHERE id='b'",
                    (json.dumps({'kind': 'text', 'text': 'second'}),))
                self.assertEqual(canonical_part_text(connection, 'empty', '', ''), '')
                self.assertEqual(canonical_part_text(connection, 'reasoning', '',
                    json.dumps({'kind': 'reasoning', 'text': 'not an answer'})), '')
            self.assertEqual(canonical_text_parts(database, 'session'),
                [('user', 'question'), ('assistant', '答案一'), ('assistant', 'second')])


if __name__ == '__main__':
    unittest.main()
