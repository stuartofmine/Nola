#!/usr/bin/env python3
"""Attach an actual screenshot or recording observation to an E2E receipt.

Run this only after opening the image or watching the recording. Capture and
review are deliberately separate operations so UI automation cannot certify
its own visual output.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
from pathlib import Path

from real_e2e_evidence import attach_visual_review


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("receipt", type=Path)
    parser.add_argument("--step", required=True)
    parser.add_argument("--observation", required=True)
    parser.add_argument("--observer", required=True)
    parser.add_argument(
        "--method",
        required=True,
        choices=("opened_image", "watched_recording"),
    )
    parser.add_argument("--verdict", required=True, choices=("passed", "failed"))
    parser.add_argument("--finding", action="append", required=True)
    parser.add_argument("--observed-at")
    args = parser.parse_args()

    observed_at = args.observed_at or datetime.now(timezone.utc).isoformat()
    attach_visual_review(
        args.receipt,
        step_id=args.step,
        observation_id=args.observation,
        observer=args.observer,
        observed_at=observed_at,
        method=args.method,
        verdict=args.verdict,
        findings=args.finding,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
