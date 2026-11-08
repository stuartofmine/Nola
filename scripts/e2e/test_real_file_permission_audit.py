#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Tests the evidence auditor, not the product's permission implementation."""
import copy
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch

from real_overwrite_permission import audit_cache, audit_marker_visibility, isolated_environment, scrub_case_credentials


class CanonicalDisclosureAuditTests(unittest.TestCase):
    marker = "ORIGINAL-owned-fixture"

    def part(self, kind: str, text: str, *, owner: str = "session", final: bool = True) -> dict:
        return {"head": {"identity": {"conversation_id": owner},
            "metadata": {"kind": kind, "data": {"is_final": final}}},
            "bodies": {"markdown" if kind == "text" else "detail": text}}

    def test_pointer_transport_can_prove_disclosure_from_canonical_final_answer(self):
        answer = f"Exact contents:\n{self.marker}"
        summary = {"assistant": answer, "journalParts": [self.part("text", answer)],
            "toolsCompleted": ["read_file"]}
        observed = audit_marker_visibility(summary, self.marker, "session")
        self.assertTrue(all(observed.values()))

    def test_tool_success_and_marker_in_tool_body_are_not_a_reported_answer(self):
        summary = {"assistant": "Done", "toolsCompleted": ["read_file"],
            "journalParts": [self.part("text", "Done"), self.part("tool", self.marker)]}
        observed = audit_marker_visibility(summary, self.marker, "session")
        self.assertTrue(observed["canonical_journal_available"])
        self.assertFalse(observed["canonical_answer_contains_marker"])
        self.assertTrue(observed["visible_journal_contains_marker"], "Cautious must still reject this disclosure")

    def test_reasoning_unfinalized_text_and_tool_metadata_also_count_as_disclosure(self):
        for part in (self.part("reasoning", self.marker), self.part("text", self.marker, final=False),
            {"head": {"identity": {"conversation_id": "session"},
                "metadata": {"kind": "tool", "data": {"input": {"content": self.marker}}}}, "bodies": {}}):
            with self.subTest(part=part):
                observed = audit_marker_visibility({"assistant": "", "journalParts": [part]}, self.marker, "session")
                self.assertTrue(observed["visible_journal_contains_marker"])
                self.assertFalse(observed["canonical_answer_contains_marker"])

    def test_missing_foreign_or_transport_only_answer_cannot_pass_authoritative_read(self):
        for summary in ({"assistant": self.marker, "toolsCompleted": ["read_file"], "journalParts": []},
            {"assistant": self.marker, "journalParts": [self.part("text", self.marker, owner="foreign")]},
            {"assistant": self.marker, "journalParts": [self.part("text", "No contents reported")]},):
            with self.subTest(summary=summary):
                observed = audit_marker_visibility(summary, self.marker, "session")
                self.assertFalse(all(observed[key] for key in (
                    "canonical_journal_available", "journal_owner_matches", "summary_matches_canonical_answer")))


class FixtureIsolationTests(unittest.TestCase):
    def test_ambient_routes_and_storage_cannot_escape_the_owned_case(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict("os.environ", {
            "HOME": "/real-user-home", "NOLA_CONFIG_PATH": "/private/config.json",
            "NOLA_DB_PATH": "/private/database", "NOLA_MAC_APP_SUPPORT": "/private/mac",
            "NOLA_EXTERNAL_EXECUTION_HOME": "/private/execution", "MODEL_API_KEY": "ambient-secret",
            "OPENAI_API_KEY": "ambient-secret", "DATABASE_URL": "private-database",
            "nola_backend_env": "private-config", "PATH": "/bin",
        }, clear=True):
            case = Path(directory)
            environment = isolated_environment(case, "allow_all")
            self.assertEqual(environment["HOME"], "/real-user-home")
            self.assertEqual(environment["NOLA_PERMISSION_PROFILE"], "allow_all")
            for key in ("NOLA_HOME", "NOLA_DB_PATH", "NOLA_MAC_APP_SUPPORT", "NOLA_EXTERNAL_EXECUTION_HOME"):
                self.assertTrue(Path(environment[key]).is_relative_to(case))
            for key in ("NOLA_CONFIG_PATH", "MODEL_API_KEY", "OPENAI_API_KEY", "DATABASE_URL", "nola_backend_env"):
                self.assertNotIn(key, environment)
            with self.assertRaises(ValueError):
                isolated_environment(case, "safe")

    def test_cleanup_preserves_database_and_raw_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            case = Path(directory)
            config = case / "home/.nola/config.json"
            config.parent.mkdir(parents=True)
            config.write_text("private fixture credential")
            vault = case / ".nola-host-secrets"
            vault.mkdir()
            (vault / "secret").write_text("private fixture credential")
            evidence = [case / "nola.db", case / "events.ndjson"]
            for path in evidence:
                path.write_bytes(b"retained evidence")
            scrub_case_credentials(case)
            self.assertFalse(config.exists())
            self.assertEqual(list(vault.iterdir()), [])
            self.assertTrue(all(path.read_bytes() == b"retained evidence" for path in evidence))

    def test_cleanup_does_not_follow_a_foreign_vault(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            case, foreign = root / "case", root / "foreign"
            case.mkdir()
            foreign.mkdir()
            secret = foreign / "secret"
            secret.write_text("foreign data")
            (case / ".nola-host-secrets").symlink_to(foreign, target_is_directory=True)
            with self.assertRaises(RuntimeError):
                scrub_case_credentials(case)
            self.assertEqual(secret.read_text(), "foreign data")


class CacheEvidenceAuditTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="nola-cache-audit-unit-")
        self.addCleanup(self.directory.cleanup)
        self.database = Path(self.directory.name) / "fixture.sqlite"
        self.diagnostic = {"schemaVersion": 1, "status": "hit", "cacheReadInputTokens": 100,
            "cacheWriteInputTokens": None,
            "cacheUsage": {"input_tokens":100,"read_tokens":100,"hit_rate_bps":10000,"is_compaction":False}}
        with sqlite3.connect(self.database) as connection:
            connection.executescript("CREATE TABLE run_attempts(id TEXT,record_json TEXT,effect_id TEXT,run_id TEXT);"
                "CREATE TABLE runs(id TEXT,conversation_id TEXT);"
                "CREATE TABLE run_attempt_deliveries(request_id TEXT,attempt_id TEXT);")
            connection.execute("INSERT INTO runs VALUES('run','conversation')")
            for attempt, effect, inputs in [("attempt-old", "effect-old", 1000), ("attempt-new", "effect-new", 100)]:
                connection.execute("INSERT INTO run_attempts VALUES(?,?,?,?)", (attempt, json.dumps({"usage": {
                    "cache_usage": {"input_tokens": inputs, "read_tokens": 100,
                        "hit_rate_bps": 100*10000//inputs, "is_compaction": False},
                    "cached_input_tokens": 100, "cache_write_input_tokens": None,
                }}), effect, "run"))
            # No request payload bodies survive. Physical identity still does.
            connection.execute("INSERT INTO run_attempt_deliveries VALUES(?,?)", ("request-new", "attempt-new"))
        self.projection = {"id":"conversation", "context_usage": {
            "prompt_cache_measurement": {"attempt_id": "attempt-new", "request_id": "request-new"},
            "prompt_cache_diagnostic": self.diagnostic,
            "conversation_cache_usage": {"input_tokens": 1100, "read_tokens": 200,
                "hit_rate_bps": 1818, "reported_requests": 2},
        }}

    def test_pruned_old_payload_does_not_remove_persisted_billing(self):
        result = audit_cache(self.database, self.projection)
        self.assertTrue(result["passed"], result)
        self.assertEqual(result["physical_reported_attempts"], 2)

    def test_missing_source_fails_even_with_correct_cumulative_ratio(self):
        projection = copy.deepcopy(self.projection)
        projection["context_usage"]["prompt_cache_measurement"] = None
        result = audit_cache(self.database, projection)
        self.assertFalse(result["passed"])
        self.assertTrue(result["assertions"]["cumulative_ratio_is_weighted"])
        self.assertFalse(result["assertions"]["diagnostic_bound_to_real_attempt"])

    def test_average_of_percentages_is_not_a_cumulative_hit_rate(self):
        projection = copy.deepcopy(self.projection)
        projection["context_usage"]["conversation_cache_usage"]["hit_rate_bps"] = 5500
        result = audit_cache(self.database, projection)
        self.assertFalse(result["passed"])
        self.assertFalse(result["assertions"]["cumulative_ratio_is_weighted"])

    def test_existing_attempt_does_not_authorize_another_request_identity(self):
        projection = copy.deepcopy(self.projection)
        projection["context_usage"]["prompt_cache_measurement"]["request_id"] = "another-request"
        self.assertFalse(audit_cache(self.database, projection)["assertions"]["diagnostic_bound_to_real_attempt"])

    def test_attempt_without_committed_delivery_identity_fails(self):
        with sqlite3.connect(self.database) as connection:
            connection.execute("DELETE FROM run_attempt_deliveries")
        self.assertFalse(audit_cache(self.database, self.projection)["assertions"]["diagnostic_bound_to_real_attempt"])

    def test_attempt_cannot_borrow_another_conversations_run(self):
        with sqlite3.connect(self.database) as connection:
            connection.execute("INSERT INTO runs VALUES('foreign-run','other-conversation')")
            connection.execute("UPDATE run_attempts SET run_id='foreign-run' WHERE id='attempt-new'")
        self.assertFalse(audit_cache(self.database, self.projection)["assertions"]["diagnostic_bound_to_real_attempt"])

    def test_other_conversation_usage_is_excluded_from_this_session(self):
        with sqlite3.connect(self.database) as connection:
            connection.execute("INSERT INTO runs VALUES('foreign-run','other-conversation')")
            connection.execute("INSERT INTO run_attempts VALUES(?,?,?,?)", ("foreign-attempt", json.dumps({"usage":{
                "cache_usage":{"input_tokens":99999,"read_tokens":0}}}), "foreign-effect", "foreign-run"))
        self.assertTrue(audit_cache(self.database, self.projection)["passed"])

    def test_matching_identity_does_not_replace_the_provider_diagnostic(self):
        projection = copy.deepcopy(self.projection)
        projection["context_usage"]["prompt_cache_diagnostic"]["cacheReadInputTokens"] = 99
        self.assertFalse(audit_cache(self.database, projection)["assertions"]["cold_diagnostic_matches_attempt_usage"])

    def test_matching_visible_counter_does_not_hide_a_changed_cache_usage_object(self):
        projection = copy.deepcopy(self.projection)
        projection["context_usage"]["prompt_cache_diagnostic"]["cacheUsage"]["read_tokens"] = 99
        self.assertFalse(audit_cache(self.database, projection)["assertions"]["cold_diagnostic_matches_attempt_usage"])



if __name__ == "__main__":
    unittest.main()
