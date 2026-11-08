---
name: assist-meetings
description: Prepare agendas and turn meeting notes, transcripts, recordings, or chat logs into summaries, explicit decisions, action items, owners, deadlines, and follow-ups. Use for meeting preparation or post-meeting execution; never imply audio transcription without an authorized transcription capability.
---

# Assist with Meetings

## Nola runtime contract

- Read `NOLA_MANIFEST.json` before execution. Confirm required tools, runtime dependencies, permissions, and validation gates before starting expensive work.
- All Skill files and scripts are bundled locally. Never download a replacement Skill, run a package installer, or silently substitute an unavailable tool.
- Remove temporary helper files before the final validation. After the last successful validation, do not mutate the workspace again; report the validated artifact directly.
- Keep tool results, user-visible progress, and final claims distinct. A draft, preview, local artifact, or advisory report is not evidence of an external write.

## Skill coordination

- Keep meeting synthesis here. Delegate approved events and tasks to scheduling, participant messages to email, and durable decisions to memory; never bundle those side effects into implicit follow-up.
- Load only the smallest set of Skills needed. Pass verified artifacts, identifiers, source anchors, and unresolved constraints across a handoff; do not replay the same discovery or computation.


## Operating modes

- **Prepare**: synthesize context into objectives, agenda, pre-reads, decisions needed, roles, and time boxes.
- **Synthesize**: turn notes or transcript into a concise factual record.
- **Execute**: propose follow-up messages, tasks, reminders, and calendar changes, then commit only approved actions.
- **Track**: compare new material with prior decisions and action-item status.

## Readiness

- Identify meeting title, date, timezone, participants, purpose, source completeness, confidentiality, and desired audience.
- Detect whether transcript or recording tools are available and authorized. If only notes are supplied, label the result as notes-based.
- Preserve speaker labels when reliable and mark ambiguous speaker attribution rather than guessing.

## Progress contract

Use stable stages: preparing agenda, parsing meeting source, resolving speakers, extracting decisions, structuring actions, and validating follow-up. Do not flash a new description for every transcript chunk or tool event.

## Workflow

1. Remove exact duplicate transcript segments while preserving timestamps and meaningful repetition.
2. Separate context, evidence, proposals, objections, explicit decisions, action items, risks, and parking-lot topics.
3. Record every decision with decision text, decision maker or group, timestamp or source anchor, rationale, and conditions.
4. Normalize every action item to outcome, owner, due date and timezone, dependency, status, and source anchor. Leave missing fields visibly unassigned.
5. Draft a participant summary with decisions and actions first; keep detailed discussion optional.
6. Preview any task, reminder, calendar, email, or memory writes before executing them.

## User control

- Confirm before creating tasks or events, sending minutes, notifying participants, or storing meeting content as long-term memory.
- Do not convert tentative language into an explicit decision or assign an owner who did not accept responsibility.
- Respect off-the-record, confidential, and restricted source sections.

## Validation

- Trace every decision and action item back to the source and verify names, dates, timezone, figures, commitments, and negation.
- Check for duplicate actions, conflicting deadlines, missing owners, and contradictions between summary and transcript.
- Distinguish transcription uncertainty from meeting uncertainty and flag incomplete source coverage.

## Failure handling

If the source is truncated, corrupted, or lacks reliable speakers, produce a bounded partial record and list what cannot be established. Do not fill missing decisions from context or memory.

## Completion

Provide meeting basis, executive summary, explicit decisions, action register, open questions, risks, source limitations, and a separate receipt for any external tasks, events, messages, or memories actually created.
