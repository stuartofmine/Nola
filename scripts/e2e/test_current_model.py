# SPDX-License-Identifier: Apache-2.0
import unittest
from current_model import selected_model


class SelectedModelTests(unittest.TestCase):
    def config(self):
        return {"llm": {"default_model": {"connection": "chosen", "model": "model"}, "connections": [
            {"id": "chosen", "protocol": "responses", "endpoint": "https://provider.invalid/v1",
             "api_key_env": "FIXTURE_KEY", "models": [{"id": "model"}]}]}}

    def test_resolves_declared_route_without_inventing_capabilities(self):
        result = selected_model(self.config(), {"FIXTURE_KEY": "fixture-secret"})
        self.assertEqual(result["wireProtocol"], "responses")
        self.assertNotIn("capabilities", result)
        self.assertNotIn("executionCapabilities", result)

    def test_invalid_routes_never_select_another_model_or_protocol(self):
        cases = [{}, self.config(), self.config(), self.config(), self.config(), self.config()]
        cases[1]["llm"]["connections"] *= 2
        cases[2]["llm"]["connections"][0]["models"] *= 2
        cases[3]["llm"]["default_model"]["model"] = "missing"
        cases[4]["llm"]["connections"][0]["protocol"] = "unknown"
        cases[5]["llm"]["connections"][0]["endpoint"] = "https://user:secret@provider.invalid"
        for config in cases:
            with self.subTest(config=config), self.assertRaises(ValueError):
                selected_model(config, {"FIXTURE_KEY": "fixture-secret"})

    def test_missing_credential_does_not_use_unrelated_provider_keys(self):
        with self.assertRaises(ValueError):
            selected_model(self.config(), {"OPENAI_API_KEY": "unrelated"})


if __name__ == "__main__":
    unittest.main()
