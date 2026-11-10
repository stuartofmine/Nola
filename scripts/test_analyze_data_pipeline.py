#!/usr/bin/env python3
"""Focused contracts for the bundled deterministic table-analysis pipeline."""

from pathlib import Path
import json
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
PIPELINE = (
    ROOT
    / "apps/macos/Resources/NolaBuiltInSkills.bundle/analyze-data/scripts/analyze_table.py"
)


class AnalyzeDataPipelineTests(unittest.TestCase):
    def test_generates_and_revalidates_reconciled_markdown(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "sales.csv"
            report = root / "analysis.md"
            source.write_text(
                "month,region,revenue,cost\n"
                "Jan,North,120,80\nJan,South,150,95\n"
                "Feb,North,140,88\nFeb,South,165,102\n",
                encoding="utf-8",
            )
            base = [
                "python3",
                str(PIPELINE),
                "--input",
                str(source),
                "--output",
                str(report),
                "--group-column",
                "month",
                "--numeric-columns",
                "revenue,cost",
            ]
            generated = subprocess.run(base, check=True, text=True, capture_output=True)
            receipt = json.loads(generated.stdout)
            self.assertTrue(receipt["validated"])
            self.assertEqual(receipt["rows"], 4)
            self.assertEqual(receipt["totals"], {"cost": "365.00", "revenue": "575.00"})
            text = report.read_text(encoding="utf-8")
            self.assertIn("## Limitations", text)
            self.assertIn("| Jan | 2 | 270.00 | 175.00 |", text)

            validated = subprocess.run(
                [*base, "--validate-only"], check=True, text=True, capture_output=True
            )
            self.assertTrue(json.loads(validated.stdout)["validated"])

    def test_validation_rejects_source_drift(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "values.tsv"
            report = root / "analysis.md"
            source.write_text("team\tvalue\nA\t1\nB\t2\n", encoding="utf-8")
            base = [
                "python3",
                str(PIPELINE),
                "--input",
                str(source),
                "--output",
                str(report),
            ]
            subprocess.run(base, check=True, capture_output=True)
            source.write_text("team\tvalue\nA\t1\nB\t9\n", encoding="utf-8")
            failed = subprocess.run(
                [*base, "--validate-only"], text=True, capture_output=True
            )
            self.assertNotEqual(failed.returncode, 0)
            self.assertIn("does not reconcile", failed.stderr)


if __name__ == "__main__":
    unittest.main()
