#!/usr/bin/env python3
"""Emit a compact, source-bounded evidence packet for peer review."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re


def packet(source: Path) -> dict[str, object]:
    text = source.read_text(encoding="utf-8")
    lines = [
        {"anchor": f"L{number}", "text": line.strip()}
        for number, line in enumerate(text.splitlines(), start=1)
        if line.strip() and not line.lstrip().startswith("#")
    ]
    if not lines:
        raise ValueError("manuscript contains no reviewable text")
    measurements = sorted(
        {
            match.group(0).strip()
            for match in re.finditer(
                r"(?<![\w.])\d+(?:\.\d+)?(?:\s*%|\s*ms)?", text
            )
        }
    )
    return {
        "source": str(source),
        "scope": "Only the anchored source lines below may support manuscript-specific claims.",
        "lines": lines,
        "measurements": measurements,
        "review_axes": [
            "originality",
            "scientific importance",
            "interdisciplinary readership",
            "technical soundness",
            "readability for nonspecialists",
        ],
        "output_contract": {
            "sections": [
                "Review setup",
                "Reviewer 1",
                "Reviewer 2",
                "Reviewer 3",
                "Cross-review synthesis",
                "Unsupported claims",
            ],
            "concerns": "Exactly two per reviewer; IDs R1-M1..R3-M2.",
            "fields": [
                "Axis",
                "Claim pointer",
                "Evidence pointer",
                "Concern",
                "Resolution test",
            ],
            "anchor_rule": "Every claim and evidence pointer must include a valid L<n> anchor.",
            "synthesis_rule": "Reference at least two concern IDs.",
        },
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps(packet(args.source), ensure_ascii=False, separators=(",", ":")))


if __name__ == "__main__":
    main()
