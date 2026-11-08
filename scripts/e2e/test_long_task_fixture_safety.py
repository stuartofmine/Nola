# SPDX-License-Identifier: Apache-2.0
"""Fixture safety checks; these do not count as real-provider acceptance."""
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from test_real_cli_long_tasks import fixture, selected_route


class LongTaskFixtureTests(unittest.TestCase):
    def test_child_has_no_provider_secret_or_ambient_route(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {
            "HOME": "/user-home", "PATH": "/bin", "OPENAI_API_KEY": "secret-a",
            "FREE_MODEL_KEY": "secret-b", "UNRELATED_SECRET": "secret-c",
            "NOLA_CONFIG_PATH": "/user-config", "NOLA_DB_PATH": "/user-db",
        }, clear=True):
            root = Path(directory)
            _, env, path = fixture(root, "fixture-model", "http://127.0.0.1:1234/v1")
            self.assertEqual(env["HOME"], "/user-home")
            for key in ("OPENAI_API_KEY", "FREE_MODEL_KEY", "UNRELATED_SECRET", "NOLA_CONFIG_PATH"):
                self.assertNotIn(key, env)
            self.assertTrue(Path(env["NOLA_DB_PATH"]).is_relative_to(root))
            config = json.loads(path.read_text())
            self.assertEqual(config["llm"]["default_model"], {"connection": "acceptance", "model": "fixture-model"})
            self.assertNotIn("api_key_env", config["llm"]["connections"][0])
            self.assertNotIn("secret-", path.read_text())

    def test_only_current_declared_https_responses_selection_is_accepted(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {"TEST_API_KEY": "fixture-secret"}, clear=True):
            path = Path(directory) / "config.json"
            config = {"llm": {"default_model": {"connection": "selected", "model": "model"},
                "connections": [{"id": "selected", "endpoint": "https://provider.invalid/v1",
                    "protocol": "responses", "api_key_env": "TEST_API_KEY", "models": [{"id": "model"}]}]}}
            path.write_text(json.dumps(config))
            self.assertEqual(selected_route(path), ("https://provider.invalid/v1", "model", "fixture-secret"))
            for endpoint in ("http://provider.invalid/v1", "https://key@provider.invalid/v1", "https://provider.invalid/v1?key=secret"):
                config["llm"]["connections"][0]["endpoint"] = endpoint
                path.write_text(json.dumps(config))
                with self.assertRaises(ValueError):
                    selected_route(path)
            config["llm"]["connections"][0]["endpoint"] = "https://provider.invalid/v1"
            config["llm"]["connections"][0]["protocol"] = "chat_completions"
            path.write_text(json.dumps(config))
            with self.assertRaises(ValueError):
                selected_route(path)
