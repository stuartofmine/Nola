#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Process-level tests for the real-provider CLI E2E entrypoint."""

from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
WRAPPER = ROOT / "scripts" / "test_real_cli_skill_permission.sh"


class RealCliWrapperTests(unittest.TestCase):
    def model_selection(self, stdout: str) -> dict:
        events = [json.loads(line) for line in stdout.splitlines()]
        self.assertTrue(all(isinstance(event, dict) for event in events), stdout)
        selections = [event for event in events if event.get("stage") == "model-selection"]
        self.assertEqual(len(selections), 1, stdout)
        return selections[0]

    def test_model_selection_requires_exactly_one_typed_stage_and_preserves_cleanup(self) -> None:
        selected = {"stage": "model-selection", "model": "fixture", "endpoint": "http://fixture.invalid/v1", "source": "explicit"}
        cleanup = {"preservedRuntime": "/owned/fixture", "exit": 0}
        stream = "\n".join(json.dumps(event) for event in (selected, cleanup))
        self.assertEqual(self.model_selection(stream), selected)
        with self.assertRaises(AssertionError):
            self.model_selection(json.dumps(cleanup))
        with self.assertRaises(AssertionError):
            self.model_selection("\n".join(json.dumps(selected) for _ in range(2)))

    def test_current_wrapper_keeps_home_and_evidence_and_builds_shared_oracle_service(self):
        source = WRAPPER.read_text(encoding="utf-8")
        self.assertNotIn('export HOME=', source)
        self.assertNotIn('rm -rf "$RUNTIME_ROOT"', source)
        self.assertIn('export NOLA_HOME=', source)
        self.assertIn('--example terminal_acceptance_oracle', source)
        self.assertIn('-p nola_service --bin nola-service', source)
        self.assertIn('test_real_cli_resilience.py" "$BINARY" "$ORACLE"', source)

    def run_preflight(self, profile: str) -> subprocess.CompletedProcess[str]:
        environment = os.environ.copy()
        environment.update(
            {
                "NOLA_REAL_E2E_PREFLIGHT_ONLY": "1",
                "NOLA_REAL_CLI_PROFILE": profile,
                "NOLA_REAL_CLI_BINARY": "/bin/true",
            }
        )
        return subprocess.run(
            ["/bin/bash", str(WRAPPER)],
            cwd=ROOT,
            env=environment,
            text=True,
            capture_output=True,
            timeout=10,
            check=False,
        )

    def test_debug_and_release_profiles_survive_nounset_shell_execution(self) -> None:
        for profile in ("debug", "release"):
            with self.subTest(profile=profile):
                result = self.run_preflight(profile)
                self.assertEqual(result.returncode, 0, result.stderr)
                payload = json.loads(result.stdout)
                self.assertEqual(payload["profile"], profile)
                self.assertEqual(payload["buildMode"], profile)
                self.assertEqual(payload["binary"], "/bin/true")

    def test_unknown_profile_fails_before_provider_or_build_work(self) -> None:
        result = self.run_preflight("optimized-ish")
        self.assertEqual(result.returncode, 2)
        self.assertIn("Unsupported NOLA_REAL_CLI_PROFILE", result.stderr)

    def test_environment_model_configuration_overrides_selected_profile(self) -> None:
        with tempfile.TemporaryDirectory(prefix="nola-model-selection-") as directory:
            database = Path(directory) / "nola.db"
            subprocess.run(
                [
                    "sqlite3",
                    str(database),
                    """
                    CREATE TABLE model_profiles(
                        model_name TEXT NOT NULL,
                        runtime TEXT NOT NULL,
                        source_json TEXT NOT NULL,
                        selected INTEGER NOT NULL,
                        enabled INTEGER NOT NULL,
                        updated_at_ms INTEGER NOT NULL,
                        deleted_at_ms INTEGER
                    );
                    INSERT INTO model_profiles VALUES(
                        'deepseek-v4-flash-vision-exp','remote',
                        '{"baseURL":"https://api.deepseek.com/chat/completions"}',
                        1,1,2,NULL
                    );
                    """,
                ],
                check=True,
                capture_output=True,
                text=True,
            )
            environment = os.environ.copy()
            environment.update(
                {
                    "MODEL_NAME": "qwen3.6-35b-a3b",
                    "MODEL_API_URL": "http://stale-qwen-endpoint.invalid/v1",
                    "NOLA_MODEL_PROFILE_DB": str(database),
                    "NOLA_SKIP_CLI_BUILD": "1",
                    "NOLA_REAL_CLI_BINARY": "/usr/bin/true",
                    "NOLA_REAL_E2E_API_KEY": "test-key",
                    "NOLA_REAL_E2E_MODEL_SELECTION_ONLY": "1",
                }
            )
            environment.pop("NOLA_REAL_E2E_MODEL", None)
            environment.pop("NOLA_REAL_E2E_ENDPOINT", None)
            result = subprocess.run(
                ["/bin/bash", str(WRAPPER)],
                cwd=ROOT,
                env=environment,
                text=True,
                capture_output=True,
                timeout=10,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            payload = self.model_selection(result.stdout)
            self.assertEqual(payload["model"], "qwen3.6-35b-a3b")
            self.assertEqual(payload["endpoint"], "http://stale-qwen-endpoint.invalid/v1")
            self.assertEqual(payload["source"], "environment")

    def test_selected_profile_is_used_when_no_environment_override_exists(self) -> None:
        with tempfile.TemporaryDirectory(prefix="nola-model-selection-") as directory:
            database = Path(directory) / "nola.db"
            subprocess.run(
                [
                    "sqlite3",
                    str(database),
                    """
                    CREATE TABLE model_profiles(
                        model_name TEXT NOT NULL,
                        runtime TEXT NOT NULL,
                        source_json TEXT NOT NULL,
                        selected INTEGER NOT NULL,
                        enabled INTEGER NOT NULL,
                        updated_at_ms INTEGER NOT NULL,
                        deleted_at_ms INTEGER
                    );
                    INSERT INTO model_profiles VALUES(
                        'stored-model','remote',
                        '{"baseURL":"https://stored-provider.test/v1/chat/completions"}',
                        1,1,2,NULL
                    );
                    """,
                ],
                check=True,
                capture_output=True,
                text=True,
            )
            environment = os.environ.copy()
            for name in (
                "MODEL_NAME",
                "MODEL_API_URL",
                "NOLA_REAL_E2E_MODEL",
                "NOLA_REAL_E2E_ENDPOINT",
            ):
                environment.pop(name, None)
            environment.update(
                {
                    "NOLA_MODEL_PROFILE_DB": str(database),
                    "NOLA_SKIP_CLI_BUILD": "1",
                    "NOLA_REAL_CLI_BINARY": "/usr/bin/true",
                    "NOLA_REAL_E2E_API_KEY": "test-key",
                    "NOLA_REAL_E2E_MODEL_SELECTION_ONLY": "1",
                }
            )
            result = subprocess.run(
                ["/bin/bash", str(WRAPPER)],
                cwd=ROOT,
                env=environment,
                text=True,
                capture_output=True,
                timeout=10,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            payload = self.model_selection(result.stdout)
            self.assertEqual(payload["model"], "stored-model")
            self.assertEqual(payload["endpoint"], "https://stored-provider.test/v1")
            self.assertEqual(payload["source"], "nola-store")

    def test_explicit_real_e2e_model_overrides_selected_profile(self) -> None:
        environment = os.environ.copy()
        environment.update(
            {
                "MODEL_NAME": "stale-generic-model",
                "MODEL_API_URL": "http://stale-generic-endpoint.invalid/v1",
                "NOLA_REAL_E2E_MODEL": "explicit-deepseek-model",
                "NOLA_REAL_E2E_ENDPOINT": "https://explicit.deepseek.test/v1",
                "NOLA_REAL_E2E_API_KEY": "test-key",
                "NOLA_MODEL_PROFILE_DB": "/does/not/exist",
                "NOLA_SKIP_CLI_BUILD": "1",
                "NOLA_REAL_CLI_BINARY": "/usr/bin/true",
                "NOLA_REAL_E2E_MODEL_SELECTION_ONLY": "1",
            }
        )
        result = subprocess.run(
            ["/bin/bash", str(WRAPPER)],
            cwd=ROOT,
            env=environment,
            text=True,
            capture_output=True,
            timeout=10,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        payload = self.model_selection(result.stdout)
        self.assertEqual(payload["model"], "explicit-deepseek-model")
        self.assertEqual(payload["endpoint"], "https://explicit.deepseek.test/v1")
        self.assertEqual(payload["source"], "explicit")

    def test_resilience_failure_is_structured_without_python_traceback(self) -> None:
        with tempfile.TemporaryDirectory(prefix="nola-real-wrapper-test-") as directory:
            provider_log = Path(directory) / "provider.ndjson"
            provider_log.write_text("")
            environment = os.environ.copy()
            environment.update(
                {
                    "NOLA_REAL_TEST_ENDPOINT": "http://127.0.0.1:1/v1",
                    "NOLA_REAL_TEST_MODEL": "unavailable-test-model",
                    "NOLA_REAL_TEST_API_KEY": "test-key",
                    "NOLA_REAL_TEST_PROVIDER_LOG": str(provider_log),
                    "NOLA_REAL_RESILIENCE_PROJECT": directory,
                }
            )
            result = subprocess.run(
                [
                    "python3",
                    str(ROOT / "scripts" / "test_real_cli_resilience.py"),
                    "/usr/bin/false",
                    "/usr/bin/false",
                ],
                cwd=ROOT,
                env=environment,
                text=True,
                capture_output=True,
                timeout=10,
                check=False,
            )
            self.assertEqual(result.returncode, 1)
            failure = next(
                (
                    payload
                    for line in result.stderr.splitlines()
                    if line.strip().startswith("{")
                    for payload in [json.loads(line)]
                    if payload.get("case") == "failure"
                ),
                None,
            )
            self.assertIsNotNone(failure, result.stderr)
            assert failure is not None
            self.assertEqual(failure["case"], "failure")
            self.assertFalse(failure["passed"])
            self.assertNotIn("Traceback", result.stderr)


if __name__ == "__main__":
    unittest.main()
