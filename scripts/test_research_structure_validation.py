# SPDX-License-Identifier: Apache-2.0
"""A formatting receipt must never masquerade as evidence verification."""
import importlib.util
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "apps/macos/Resources/NolaBuiltInSkills.bundle/conduct-deep-research/scripts/validate_research.py"
SPEC = importlib.util.spec_from_file_location("research_validator", SCRIPT)
VALIDATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VALIDATOR)


class ResearchStructureTests(unittest.TestCase):
    def validate(self, extra=""):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "report.md"
            path.write_text(
                "# Scope\n" + "Research notes. " * 70
                + "\n## Sources\nhttps://archive.example/a\nhttps://archive.example/b\nhttps://archive.example/c\n"
                + "## Limitations\nAccessed: 2026-10-15\n" + extra,
                encoding="utf-8",
            )
            return VALIDATOR.validate(path)

    def test_one_host_can_hold_separate_original_papers(self):
        result = self.validate()
        self.assertTrue(result["structure_validated"])
        self.assertEqual(result["distinct_hosts"], 1)
        self.assertFalse(result["evidence_verified"])

    def test_unrelated_domain_does_not_certify_independence(self):
        result = self.validate("https://unrelated.example/news\n")
        self.assertEqual(result["distinct_hosts"], 2)
        self.assertFalse(result["evidence_verified"])
        self.assertNotIn("validated", result)
        self.assertNotIn("independent_hosts", result)
        self.assertIn("scope_coverage", result["remaining_checks"])

    def test_malformed_report_still_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "empty.md"
            path.write_text("# Empty", encoding="utf-8")
            with self.assertRaises(ValueError):
                VALIDATOR.validate(path)


if __name__ == "__main__":
    unittest.main()
