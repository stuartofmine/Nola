#!/usr/bin/env python3
"""Focused contracts for the source-bounded reviewer workflow."""

import json
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
SKILL = ROOT / "apps/macos/Resources/NolaBuiltInSkills.bundle/nature-reviewer"
PREPARE = SKILL / "scripts/prepare_review.py"
VALIDATE = SKILL / "scripts/validate_review.py"


def valid_review() -> str:
    sections = [
        "# Review setup\n\nThe bounded source contains one short results paragraph. "
        "The review does not infer absent methods, uncertainty, controls, or citations.",
    ]
    axes = ["technical soundness", "significance", "readability"]
    for reviewer in range(1, 4):
        concerns = []
        for index in range(1, 3):
            concern_id = f"R{reviewer}-M{index}"
            concerns.append(
                f"## {concern_id}\n"
                f"- Axis: {axes[reviewer - 1]}\n"
                "- Claim pointer: L3, the reported comparison.\n"
                "- Evidence pointer: L3, which reports 86% versus 81% and 47 ms versus 42 ms.\n"
                "- Concern: The source gives a measured comparison but does not report uncertainty or establish broad generality; this limits how strongly the result can be interpreted by readers.\n"
                "- Resolution test: Scope the conclusion to the observed setting, report uncertainty, and explain the accuracy-latency trade-off with enough methodological detail to reproduce it."
            )
        sections.append(f"# Reviewer {reviewer}\n\n" + "\n\n".join(concerns))
    sections.extend(
        [
            "# Cross-review synthesis\n\nR1-M1 and R2-M1 converge on missing uncertainty, while R3-M2 emphasizes accessibility and the unsupported scope of the universally superior claim.",
            "# Unsupported claims\n\nThe phrase universally superior at L3 is not established by the single bounded comparison. Generalization beyond the supplied 120 samples is not assessable.",
        ]
    )
    return "\n\n".join(sections) + "\n"


class NatureReviewerPipelineTests(unittest.TestCase):
    def test_packet_is_bounded_to_real_source_lines(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "manuscript.md"
            source.write_text(
                "# Adaptive inference\n\nWe tested 120 samples: accuracy was 86% versus 81%, latency was 47 ms versus 42 ms, and the method is universally superior.\n",
                encoding="utf-8",
            )
            completed = subprocess.run(
                ["python3", str(PREPARE), "--source", str(source)],
                check=True,
                text=True,
                capture_output=True,
            )
            packet = json.loads(completed.stdout)
            self.assertEqual(packet["lines"], [{"anchor": "L3", "text": source.read_text(encoding="utf-8").splitlines()[2]}])
            self.assertIn("86%", packet["measurements"])

    def test_validator_requires_source_anchors_not_only_labels(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "manuscript.md"
            review = root / "peer-review.md"
            source.write_text(
                "# Adaptive inference\n\nWe tested 120 samples: accuracy was 86% versus 81%, latency was 47 ms versus 42 ms, and the method is universally superior.\n",
                encoding="utf-8",
            )
            review.write_text(valid_review(), encoding="utf-8")
            command = [
                "python3", str(VALIDATE), "--source", str(source), "--review", str(review)
            ]
            passed = subprocess.run(command, text=True, capture_output=True)
            self.assertEqual(passed.returncode, 0, passed.stdout + passed.stderr)
            payload = json.loads(passed.stdout)
            self.assertTrue(payload["valid"])

            review.write_text(valid_review().replace("L3", "the manuscript"), encoding="utf-8")
            failed = subprocess.run(command, text=True, capture_output=True)
            self.assertNotEqual(failed.returncode, 0)
            self.assertIn("source-line anchor", failed.stdout)


if __name__ == "__main__":
    unittest.main()
