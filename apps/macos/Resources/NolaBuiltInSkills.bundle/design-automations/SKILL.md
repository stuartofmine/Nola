---
name: design-automations
description: Design, configure, test, enable, pause, and explain scheduled or event-driven automations with explicit triggers, inputs, actions, permissions, retries, and observability. Use for recurring reports, monitoring, synchronization, reminders, or multi-step workflows.
---

# Design Automations

## Nola runtime contract

- Read `NOLA_MANIFEST.json` before execution. Confirm required tools, runtime dependencies, permissions, and validation gates before starting expensive work.
- All Skill files and scripts are bundled locally. Never download a replacement Skill, run a package installer, or silently substitute an unavailable tool.
- Remove temporary helper files before the final validation. After the last successful validation, do not mutate the workspace again; report the validated artifact directly.
- Keep tool results, user-visible progress, and final claims distinct. A draft, preview, local artifact, or advisory report is not evidence of an external write.

## Skill coordination

- Compose enabled data sources and enabled Skills as named steps rather than copying their instructions. Use scheduling for human calendar plans; use automation only for durable recurring or event-driven execution.
- Load only the smallest set of Skills needed. Pass verified artifacts, identifiers, source anchors, and unresolved constraints across a handoff; do not replay the same discovery or computation.


## Operating modes

- **Draft**: define a workflow without creating or enabling it.
- **Test**: execute a safe dry-run or one-time representative run.
- **Enable**: persist an approved schedule or event trigger.
- **Operate**: inspect history, diagnose failure, pause, resume, edit, or retire an automation.

## Readiness

- Define owner, trigger, timezone, schedule or event, input sources, workspace, steps, output, success criteria, retention, and notification channel.
- Check data-source connections, host availability, permissions, secrets, cost, rate limits, side effects, and behavior while the Mac is asleep or offline.
- Identify duplicate risk and whether each action can be made idempotent.

## Progress contract

Use stable stages: validating dependencies, designing workflow, running dry-run, checking outputs, waiting for approval, enabling schedule, and verifying next run. Never expose secrets or repeat unchanged timer status.

## Workflow

1. Model steps with explicit inputs, outputs, preconditions, timeout, and side-effect class.
2. Define idempotency keys, checkpoints, deduplication window, concurrency policy, timeout, retry conditions, exponential backoff, and retry cap.
3. Separate transient tool failure from permanent validation or authorization failure. Add a dead-letter or human-review path for unrecoverable work.
4. Add structured run history: trigger time, step, tool, bounded result, duration, status, error, and correlation identifier.
5. Run a dry-run with safe representative data, then a one-time controlled execution when side effects permit.
6. Show trigger, steps, permissions, outputs, retry policy, failure notification, and next-run calculation before enabling.

## Native persistence contract

- On macOS, a durable automation must be created or updated through `nola_orchestrate` after the user approves the preview. A JSON or Markdown automation draft is only a proposal and is never evidence that an automation exists.
- An imperative user request that explicitly says to create, enable, schedule, pause, resume, update, or delete a specific automation is lifecycle approval for that requested action. Do not ask the same confirmation again. Call `nola_orchestrate`; the Host permission boundary remains authoritative and will suspend the call if an additional runtime approval is required.
- Ask a follow-up only when a material field such as schedule, timezone, target data, output, or destructive scope is genuinely missing or ambiguous. In a non-interactive run, do not replace an already explicit command with `request_user_input`.
- Keep a new automation disabled until its dependencies and representative dry-run pass. Re-read the returned identifier and persisted fields before reporting success.
- Use the stable workspace-and-name identity for idempotent updates. Repeating the same approved request must update one automation rather than create duplicates.
- A disabled automation must not create a runner, receipt, execution log, notification, or side effect. A controlled run must produce a bounded log, structured receipt, and visible history entry before the automation may be enabled.

## User control

- Confirm before enabling, changing, pausing, resuming, or deleting an automation and before actions that send, publish, purchase, delete, overwrite, or write memory.
- Use least privilege and never embed credentials in prompts, logs, artifacts, or visible configuration.
- Pause after repeated failures, ambiguous output, authorization changes, or unexpected side effects instead of looping.

## Validation

- Verify trigger timing and timezone, next-run calculation, idempotency, duplicate prevention, concurrency, output path, timeout, error path, retry cap, and notification delivery.
- Confirm the automation appears in the automation center with identifier, enabled state, next run, last run, and readable history.
- Re-run the same test input and verify that idempotent actions do not duplicate side effects.

## Failure handling

On partial creation or execution, re-read persisted automation and run history before retrying. Report successful steps and identifiers, leave the automation disabled when validation is incomplete, and never hide a failed step behind overall success.

## Completion

Report lifecycle state (`Draft`, `Tested`, `Enabled`, `Paused`, or `Failed`), identifier, schedule, dependencies, permissions, validation evidence, last run, and next run when available.
