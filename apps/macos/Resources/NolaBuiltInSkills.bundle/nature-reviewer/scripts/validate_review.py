#!/usr/bin/env python3
"""Deterministically validate a source-grounded three-reviewer report."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path


REQUIRED_SECTIONS = (
    "review setup",
    "reviewer 1",
    "reviewer 2",
    "reviewer 3",
    "cross-review synthesis",
    "unsupported claims",
)
REQUIRED_LABELS = ("claim pointer", "evidence pointer", "resolution test")


def section_heading_count(text: str, section: str) -> int:
    section_pattern = (
        r"(?:risk\s*/\s*)?unsupported claims"
        if section == "unsupported claims"
        else re.escape(section)
    )
    pattern = re.compile(
        rf"^\s*(?:#{{1,6}}\s*)?(?:\*\*)?{section_pattern}(?:\*\*)?"
        r"(?:\s*[—:|-].*)?\s*$",
        re.IGNORECASE | re.MULTILINE,
    )
    return len(pattern.findall(text))


def source_markers(text: str) -> set[str]:
    return {
        marker.casefold()
        for marker in re.findall(r"(?<![\w.])\d+(?:\.\d+)?(?:\s*%|\s*ms)?", text)
        if len(marker.strip()) >= 2
    }


def field_value(body: str, label: str) -> str:
    match = re.search(
        rf"(?:^|\n)\s*(?:[-*]\s*)?(?:\*\*)?{re.escape(label)}(?:\*\*)?\s*:\s*(.+)",
        body,
        re.IGNORECASE,
    )
    return match.group(1).strip() if match else ""


def valid_line_anchors(pointer: str, source_lines: set[int]) -> set[int]:
    return {
        int(value)
        for value in re.findall(r"\bL(\d+)\b", pointer, re.IGNORECASE)
        if int(value) in source_lines
    }


def reviewer_concern_blocks(text: str) -> list[tuple[str, str]]:
    """Parse concern declarations only inside their Reviewer section.

    IDs cited later in the synthesis are references, not new concern blocks.
    """
    blocks = []
    for reviewer in (1, 2, 3):
        section = re.search(
            rf"^\s*#{{1,6}}\s*reviewer {reviewer}\s*$([\s\S]*?)"
            rf"(?=^\s*#{{1,6}}\s*(?:reviewer {reviewer + 1}|cross-review synthesis)\s*$)",
            text,
            re.IGNORECASE | re.MULTILINE,
        )
        if not section:
            continue
        section_text = section.group(1)
        declaration = re.compile(
            rf"^\s*(?:#{{1,6}}\s*)?(?:[-*]\s*)?(?:\*\*)?"
            rf"(?:concern id\s*:\s*)?(R{reviewer}-(?:M|m)\d+)\b[^\n]*$",
            re.IGNORECASE | re.MULTILINE,
        )
        matches = list(declaration.finditer(section_text))
        for index, match in enumerate(matches):
            end = matches[index + 1].start() if index + 1 < len(matches) else len(section_text)
            blocks.append((match.group(1), section_text[match.end() : end]))
    return blocks


def validate(source: Path, review: Path) -> dict[str, object]:
    errors: list[str] = []
    if not source.is_file():
        errors.append(f"source does not exist: {source}")
        source_text = ""
    else:
        source_text = source.read_text(encoding="utf-8")
    if not review.is_file():
        errors.append(f"review does not exist: {review}")
        review_text = ""
    else:
        review_text = review.read_text(encoding="utf-8")

    folded = review_text.casefold()
    if len(review_text.strip()) < 800:
        errors.append("review is too short for three auditable reviewer reports")
    for section in REQUIRED_SECTIONS:
        count = section_heading_count(review_text, section)
        if count != 1:
            errors.append(f"required section must appear exactly once: {section} (found {count})")
    for label in REQUIRED_LABELS:
        if folded.count(label) < 3:
            errors.append(f"required evidence label must appear at least three times: {label}")

    concern_ids = set(re.findall(r"\bR[123]-(?:M|m)\d+\b", review_text))
    represented_reviewers = {
        int(match.group(1))
        for match in re.finditer(r"\bR([123])-(?:M|m)\d+\b", review_text)
    }
    if represented_reviewers != {1, 2, 3}:
        errors.append("stable concern IDs must cover Reviewer 1, Reviewer 2, and Reviewer 3")
    if len(concern_ids) < 6:
        errors.append("review must contain at least six distinct stable concern IDs")

    nonempty_source_lines = {
        number
        for number, line in enumerate(source_text.splitlines(), start=1)
        if line.strip() and not line.lstrip().startswith("#")
    }
    concern_blocks = reviewer_concern_blocks(review_text)
    if len(concern_blocks) != 6:
        errors.append(f"review must contain exactly six concern blocks (found {len(concern_blocks)})")
    for concern_id, body in concern_blocks:
        claim_pointer = field_value(body, "claim pointer")
        evidence_pointer = field_value(body, "evidence pointer")
        resolution = field_value(body, "resolution test")
        if not claim_pointer or not evidence_pointer or not resolution:
            errors.append(f"{concern_id} has an empty required evidence field")
            continue
        if not valid_line_anchors(claim_pointer, nonempty_source_lines):
            errors.append(f"{concern_id} claim pointer lacks a valid source-line anchor")
        if not valid_line_anchors(evidence_pointer, nonempty_source_lines):
            errors.append(f"{concern_id} evidence pointer lacks a valid source-line anchor")

    synthesis_match = re.search(
        r"^\s*#{1,6}\s*cross-review synthesis\s*$([\s\S]*?)(?=^\s*#{1,6}\s|\Z)",
        review_text,
        re.IGNORECASE | re.MULTILINE,
    )
    synthesis_ids = (
        set(re.findall(r"\bR[123]-(?:M|m)\d+\b", synthesis_match.group(1)))
        if synthesis_match
        else set()
    )
    if len(synthesis_ids) < 2:
        errors.append("cross-review synthesis must reference at least two concern IDs")

    markers = source_markers(source_text)
    grounded = sorted(marker for marker in markers if marker in folded)
    if markers and len(grounded) < min(2, len(markers)):
        errors.append("review does not preserve enough measurable source-data anchors")
    if "universally superior" in source_text.casefold() and "universally superior" not in folded:
        errors.append("review does not anchor the manuscript's universal-superiority claim")

    return {
        "valid": not errors,
        "source": str(source),
        "review": str(review),
        "review_bytes": len(review_text.encode("utf-8")),
        "concern_ids": len(concern_ids),
        "source_markers_found": grounded,
        "errors": errors,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--review", required=True, type=Path)
    args = parser.parse_args()
    result = validate(args.source, args.review)
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0 if result["valid"] else 1


if __name__ == "__main__":
    sys.exit(main())
