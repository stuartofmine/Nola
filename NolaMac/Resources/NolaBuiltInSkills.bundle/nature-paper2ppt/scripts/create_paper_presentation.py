#!/usr/bin/env python3
"""Create an editable paper presentation through Nola's native PPTX generator."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--spec", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    bundle_root = Path(__file__).resolve().parents[2]
    generator = bundle_root / "powerpoint/scripts/create_presentation.py"
    if not generator.is_file():
        raise SystemExit(f"PowerPoint generator is missing: {generator}")
    if not args.spec.is_file():
        raise SystemExit(f"Presentation spec is missing: {args.spec}")

    completed = subprocess.run(
        [
            sys.executable,
            str(generator),
            "--spec",
            str(args.spec),
            "--output",
            str(args.output),
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        if completed.stdout:
            print(completed.stdout, end="", file=sys.stderr)
        if completed.stderr:
            print(completed.stderr, end="", file=sys.stderr)
        return completed.returncode
    if not args.output.is_file() or args.output.stat().st_size == 0:
        raise SystemExit("PowerPoint generator completed without a non-empty PPTX")

    print(
        json.dumps(
            {
                "output": str(args.output.resolve()),
                "bytes": args.output.stat().st_size,
                "generator": str(generator),
                "next_validation": "audit_pptx_quality.py followed by rendered visual review",
            },
            ensure_ascii=False,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
