#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
import copy
import unittest
from android_a2ui_real_model import selected_model

class CurrentModelFixtureTests(unittest.TestCase):
    def config(self):
        return {"llm":{"default_model":{"connection":"selected","model":"model-b"},"connections":[
            {"id":"other","endpoint":"https://other.invalid/v1","protocol":"responses","api_key_env":"OTHER_KEY","models":[{"id":"model-a"}]},
            {"id":"selected","endpoint":"https://provider.invalid/v1","protocol":"chat_completions","api_key_env":"SELECTED_KEY","models":[{"id":"model-b"}]}
        ]}}

    def test_resolves_exact_current_connection_without_legacy_llm_fields(self):
        value = selected_model(self.config(), {"SELECTED_KEY":"fixture-secret", "OTHER_KEY":"unused"})
        self.assertEqual(value["model"], "model-b")
        self.assertEqual(value["endpoint"], "https://provider.invalid/v1")
        self.assertEqual(value["apiKey"], "fixture-secret")
        self.assertEqual(value["wireProtocol"], "chat_completions")

    def test_missing_or_ambiguous_route_and_credential_never_fall_back(self):
        with self.assertRaises(ValueError): selected_model(self.config(), {})
        config = self.config(); config["llm"]["connections"].append(copy.deepcopy(config["llm"]["connections"][1]))
        with self.assertRaises(ValueError): selected_model(config, {"SELECTED_KEY":"fixture-secret"})
        config = self.config(); config["llm"]["default_model"]["model"] = "missing"
        with self.assertRaises(ValueError): selected_model(config, {"SELECTED_KEY":"fixture-secret"})
        config = self.config(); config["llm"]["connections"][1]["protocol"] = "unrecognized"
        with self.assertRaises(ValueError): selected_model(config, {"SELECTED_KEY":"fixture-secret"})

if __name__ == "__main__": unittest.main()
