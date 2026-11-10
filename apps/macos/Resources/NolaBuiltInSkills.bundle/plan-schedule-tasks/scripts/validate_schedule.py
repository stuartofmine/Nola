#!/usr/bin/env python3
"""Build and validate a deterministic, plan-only schedule conflict report."""

from __future__ import annotations

import argparse
import hashlib
import json
from datetime import datetime
from pathlib import Path
from typing import Any


def parse_time(value: str) -> datetime:
    parsed = datetime.fromisoformat(value)
    if parsed.tzinfo is None or parsed.utcoffset() is None:
        raise ValueError(f"timestamp must include a UTC offset: {value}")
    return parsed


def overlaps(left_start: datetime, left_end: datetime, right_start: datetime, right_end: datetime) -> bool:
    return left_start < right_end and left_end > right_start


def outside_working_hours(start: datetime, end: datetime, start_minute: int, end_minute: int) -> bool:
    if start.date() != end.date():
        return True
    actual_start = start.hour * 60 + start.minute
    actual_end = end.hour * 60 + end.minute
    return actual_start < start_minute or actual_end > end_minute


def build_plan(source: dict[str, Any], raw_bytes: bytes) -> dict[str, Any]:
    timezone = str(source.get("timezone", "")).strip()
    if not timezone:
        raise ValueError("timezone is required")
    working_hours = source.get("working_hours", {})
    work_start = int(working_hours.get("start_minute", 9 * 60))
    work_end = int(working_hours.get("end_minute", 18 * 60))
    commitments = source.get("commitments", [])
    proposals = source.get("proposals", [])
    normalized_commitments: list[dict[str, Any]] = []
    for item in commitments:
        start = parse_time(str(item["start"]))
        end = parse_time(str(item["end"]))
        if end <= start:
            raise ValueError(f"commitment {item.get('persistent_id')} has a non-positive duration")
        persistent_id = str(item.get("persistent_id", "")).strip()
        if not persistent_id:
            raise ValueError("existing commitment is missing persistent_id")
        normalized_commitments.append({
            "persistent_id": persistent_id,
            "title": str(item.get("title", "Untitled commitment")),
            "start": start,
            "end": end,
        })

    results: list[dict[str, Any]] = []
    for proposal in proposals:
        start = parse_time(str(proposal["start"]))
        end = parse_time(str(proposal["end"]))
        if end <= start:
            raise ValueError(f"proposal {proposal.get('proposal_id')} has a non-positive duration")
        conflict_ids = [
            item["persistent_id"]
            for item in normalized_commitments
            if overlaps(start, end, item["start"], item["end"])
        ]
        results.append({
            "proposal_id": str(proposal["proposal_id"]),
            "persistent_id": None,
            "title": str(proposal.get("title", "Untitled proposal")),
            "start": start.isoformat(),
            "end": end.isoformat(),
            "timezone": timezone,
            "crosses_midnight": start.date() != end.date(),
            "outside_working_hours": outside_working_hours(start, end, work_start, work_end),
            "conflict_status": "conflict" if conflict_ids else "clear",
            "conflicts_with": conflict_ids,
            "commit_action": "none",
        })

    conflict_count = sum(item["conflict_status"] == "conflict" for item in results)
    return {
        "schema_version": 1,
        "mode": "plan_only",
        "timezone": timezone,
        "writes_performed": False,
        "input_sha256": hashlib.sha256(raw_bytes).hexdigest(),
        "proposals": results,
        "reconciliation": {
            "proposal_count": len(results),
            "conflict_count": conflict_count,
            "clear_count": len(results) - conflict_count,
        },
    }


def validate_plan(plan: dict[str, Any]) -> dict[str, Any]:
    errors: list[str] = []
    proposals = plan.get("proposals", [])
    reconciliation = plan.get("reconciliation", {})
    if plan.get("mode") != "plan_only":
        errors.append("mode must be plan_only")
    if plan.get("writes_performed") is not False:
        errors.append("plan-only validation must not perform external writes")
    if not plan.get("timezone"):
        errors.append("timezone is missing")
    for proposal in proposals:
        if proposal.get("persistent_id") is not None:
            errors.append(f"proposal {proposal.get('proposal_id')} unexpectedly has a persistent_id")
        if proposal.get("commit_action") != "none":
            errors.append(f"proposal {proposal.get('proposal_id')} unexpectedly has a commit action")
        conflicts = proposal.get("conflicts_with", [])
        expected = "conflict" if conflicts else "clear"
        if proposal.get("conflict_status") != expected:
            errors.append(f"proposal {proposal.get('proposal_id')} conflict status is inconsistent")
        parse_time(str(proposal.get("start", "")))
        parse_time(str(proposal.get("end", "")))
    actual_conflicts = sum(item.get("conflict_status") == "conflict" for item in proposals)
    if reconciliation.get("proposal_count") != len(proposals):
        errors.append("proposal_count does not reconcile")
    if reconciliation.get("conflict_count") != actual_conflicts:
        errors.append("conflict_count does not reconcile")
    if reconciliation.get("clear_count") != len(proposals) - actual_conflicts:
        errors.append("clear_count does not reconcile")
    return {"validated": not errors, "errors": errors, "proposal_count": len(proposals)}


def render_markdown(plan: dict[str, Any]) -> str:
    rows = []
    for proposal in plan.get("proposals", []):
        start = parse_time(str(proposal["start"]))
        end = parse_time(str(proposal["end"]))
        flags = []
        if proposal.get("conflict_status") == "conflict":
            flags.append("conflict")
        if proposal.get("outside_working_hours"):
            flags.append("outside working hours")
        if proposal.get("crosses_midnight"):
            flags.append("crosses midnight")
        status = ", ".join(flags) if flags else "clear"
        rows.append(
            f"| {start:%a %Y-%m-%d} | {start:%H:%M}-{end:%H:%M} | "
            f"{proposal['title']} | {status} |"
        )
    reconciliation = plan.get("reconciliation", {})
    return "\n".join([
        "# Weekly schedule proposal",
        "",
        f"Timezone: `{plan.get('timezone', '')}`",
        "",
        "This is a plan-only proposal. No calendar or reminder writes were performed.",
        "",
        "| Day | Time | Task | Conflict status |",
        "| --- | --- | --- | --- |",
        *rows,
        "",
        "## Reconciliation",
        "",
        f"- Proposed blocks: {reconciliation.get('proposal_count', 0)}",
        f"- Clear blocks: {reconciliation.get('clear_count', 0)}",
        f"- Conflicting blocks: {reconciliation.get('conflict_count', 0)}",
        "- Commit action: none",
        "",
        "## Review before committing",
        "",
        "Confirm deadlines, dependencies, buffers, timezone, and any existing commitments before creating provider-backed events.",
        "",
    ])


def validate_markdown(plan: dict[str, Any], markdown_path: Path) -> list[str]:
    if not markdown_path.is_file():
        return [f"markdown output is missing: {markdown_path}"]
    actual = markdown_path.read_text(encoding="utf-8")
    expected = render_markdown(plan)
    return [] if actual == expected else ["markdown output does not reconcile with the validated plan"]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input")
    parser.add_argument("--output", required=True)
    parser.add_argument("--markdown-output")
    parser.add_argument("--validate-only", action="store_true")
    args = parser.parse_args()
    output = Path(args.output)
    if args.validate_only:
        plan = json.loads(output.read_text(encoding="utf-8"))
        receipt = validate_plan(plan)
        if args.markdown_output:
            receipt["errors"].extend(validate_markdown(plan, Path(args.markdown_output)))
            receipt["validated"] = not receipt["errors"]
        print(json.dumps(receipt, ensure_ascii=False, sort_keys=True))
        return 0 if receipt["validated"] else 2
    if not args.input:
        parser.error("--input is required unless --validate-only is used")
    raw = Path(args.input).read_bytes()
    source = json.loads(raw.decode("utf-8"))
    plan = build_plan(source, raw)
    receipt = validate_plan(plan)
    if not receipt["validated"]:
        raise ValueError("generated schedule failed validation: " + "; ".join(receipt["errors"]))
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(plan, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if args.markdown_output:
        markdown_output = Path(args.markdown_output)
        markdown_output.parent.mkdir(parents=True, exist_ok=True)
        markdown_output.write_text(render_markdown(plan), encoding="utf-8")
        receipt["markdown_output"] = str(markdown_output)
    print(json.dumps(receipt, ensure_ascii=False, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
