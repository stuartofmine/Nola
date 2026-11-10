---
name: plan-schedule-tasks
description: Plan calendars, tasks, reminders, focus blocks, deadlines, and routines using authorized schedule data and user constraints. Use for prioritization, conflict resolution, time blocking, rescheduling, or calendar and reminder changes; distinguish proposals from committed writes.
---

# Plan Schedule and Tasks

## Nola runtime contract

- Read `NOLA_MANIFEST.json` before execution. Confirm required tools, runtime dependencies, permissions, and validation gates before starting expensive work.
- All Skill files and scripts are bundled locally. Never download a replacement Skill, run a package installer, or silently substitute an unavailable tool.
- Remove temporary helper files before the final validation. After the last successful validation, do not mutate the workspace again; report the validated artifact directly.
- Keep tool results, user-visible progress, and final claims distinct. A draft, preview, local artifact, or advisory report is not evidence of an external write.

## Skill coordination

- Use automation only when the user requests durable recurring execution beyond native recurrence. Keep plan-only proposals separate from provider-backed calendar or reminder writes.
- Load only the smallest set of Skills needed. Pass verified artifacts, identifiers, source anchors, and unresolved constraints across a handoff; do not replay the same discovery or computation.


## Operating modes

- **Plan only**: produce a prioritized, conflict-aware proposal without changing external data.
- **Commit**: create or edit approved calendar, task, or reminder items and verify persistent identifiers.
- **Rebalance**: move flexible work around new constraints while preserving hard commitments.
- **Routine**: design recurring blocks with an explicit review and exception policy.

## Readiness

- Check connection and permission state before reading schedule data.
- Establish timezone, working hours, sleep and personal boundaries, calendars in scope, deadlines, priorities, duration estimates, travel time, buffers, dependencies, and flexibility.
- Read only the date range and fields needed. Ask about missing duration, attendees, recurrence, or ownership when it changes the result.

## Progress contract

Use stable stages: reading commitments, resolving constraints, building the plan, checking conflicts, applying approved changes, and verifying the calendar. Do not publish per-event read noise.

## Workflow

1. Normalize tasks into outcome, priority, effort, duration, deadline, dependency, energy, location, and flexibility.
2. Place hard commitments first, then deadlines, dependencies, focus blocks, buffers, breaks, and optional work within realistic capacity.
3. Detect overlap, travel infeasibility, timezone ambiguity, overload, fragmented focus time, and recurrence conflicts.
4. Show a preview table with create/edit/delete action, calendar or list, title, time, timezone, recurrence, reminders, and reason.
5. Apply only approved rows using stable identifiers. Preserve attendees, conferencing, notes, and unrelated metadata.
6. Re-read the affected range and reconcile committed results with the preview.

## User control

- Confirm before inviting people, deleting events, editing shared calendars, changing externally owned events, creating recurrence, or moving a hard commitment.
- Keep private event details out of unrelated tools and attendee-visible fields.
- Never claim a write succeeded without a persistent identifier or explicit provider success.

## Validation

- Verify timezone, all-day semantics, start/end, recurrence, attendees, reminders, calendar/list, ownership, and persistent identifiers.
- Re-run conflict detection on the final schedule and check duplicates, deadlines, dependency order, buffers, and daily capacity.
- Clearly mark plan-only items as proposals rather than confirmed events.
- For deterministic plan-only validation, use `scripts/validate_schedule.py --input <fixture.json> --output <plan.json>`. It normalizes offset-aware intervals, detects ordinary and cross-midnight overlaps, records working-hours violations, and guarantees that a proposal does not acquire a persistent provider identifier.
- For a file deliverable, use a temporary input ending in `.tmp` and have the bundled script generate both representations: `scripts/validate_schedule.py --input .schedule-input.tmp --output .schedule-validation.json --markdown-output schedule.md`. Do not hand-write `schedule.md` after this step.
- Remove `.schedule-input.tmp` before validation, then perform the final gate with `scripts/validate_schedule.py --validate-only --output .schedule-validation.json --markdown-output schedule.md`. The gate requires the Markdown to reconcile exactly with every validated interval and conflict result. Retain `.schedule-validation.json` as the hidden validation receipt; do not mutate or delete either validated file afterward.

## Failure handling

On partial write failure, stop the batch, re-read the affected range, report successful identifiers and failed rows, and propose a safe reconciliation. Do not blindly retry creates that may have succeeded.

## Completion

Present the final plan and separate proposed, confirmed, skipped, and failed changes, including identifiers, timezone, unresolved conflicts, and next review point.
