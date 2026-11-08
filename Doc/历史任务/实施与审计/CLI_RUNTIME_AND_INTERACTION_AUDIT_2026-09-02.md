> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# CLI Runtime and Interaction Audit

Date: 2026-09-02

## CLI presentation boundary update (2026-10-09)

The historical gate below describes the September audit, not acceptance of the
October UI changes. The Codex TUI comparison found presentation ownership gaps
even though the canonical Core/Application execution path remains valid.

- `terminal_input` owns the terminal event source. Focused synchronous adapters
  request reads without prefetch; permission focus remains serialized. Runtime
  listeners no longer drain input for a timed grace period when a turn ends.
- `composer` contains the phase-independent UTF-8 editor and submit-key contract.
  Idle projections update the session composer; switching back from execution
  preserves its cursor instead of clearing the shared draft. Running input uses
  the same multiline layout routine as idle input.
- `terminal_frames` coalesces ordinary invalidations through a bounded mailbox.
  Lifecycle/permission/command handoffs are synchronous paint barriers. All
  managed rendering is serialized by the surface lock; frame writes are batched
  with synchronized-update markers. Stable cells are drained in frame-sized
  batches; the mutable suffix cannot overtake queued history. Model output
  handlers do not sleep for animation.
- Handoff barriers drain all already-committed cells in one synchronized update,
  rather than consuming only one animation cell. This prevents pending output
  from crossing permission/line-mode ownership boundaries or being stranded at
  turn completion. Ordinary frames retain their one-cell animation budget.
- Modal terminal mode is borrowed: approval, denial, cancellation, and freeform
  return restore the caller's mode instead of unconditionally disabling it.
  Queued prompts pause the visible renderer only after acquiring modal ownership;
  the execution listener is released only after the surface has resumed. User
  question cards share the managed permission surface and its paging support.
- `terminal_document` retains Markdown source in transcript cells and keeps an
  unfinished paragraph/table/fence mutable. Accepted paragraphs and unambiguous
  complete plain lines may enter history (retaining delimiter lookahead);
  resize re-renders retained source cells at the new width. The viewport replay
  cache is bounded separately from the canonical journal.
- Public journal text uses the source-backed path; a tool call no longer discards
  that text based on its tool name. Reviewed/private visibility remains controlled
  by Core journal metadata. Non-TTY output retains its separate text formatter.

This does not move permission decisions into the UI. It also does not claim that
every slash-command screen is an asynchronous widget: command/modal adapters
remain synchronous behind the shared event source and paint barriers. Acceptance
must cover source-backed streaming, completion anchors, Unicode editing, resize,
permission return, and process-level PTY behavior before declaring the gate green.

October acceptance: 86 selected unit tests and 21 real CLI-process PTY tests
passed, with no compiler warnings. Coverage includes a provider paused inside an
unfinished Markdown span, ragged tables, large single deltas, long scrollback,
completion anchors, interrupted-stream continuation, Chinese resize, queued
follow-ups, permission return, and composer/footer restoration. Initial regression
runs exposed a premature part-completion line break, a missing live placeholder,
and a long-stream suffix overtaking its prefix; all three were corrected and the
combined gate rerun successfully. This is not a claim that the full CLI suite,
all terminals, or real paid providers were exercised in this presentation change.

Follow-up handoff-barrier acceptance: 41 terminal UI unit tests (including two
new queue-drain regressions) and 7 CLI-process PTY tests passed. The PTY gate
covers streaming completion, large-delta animation, interrupted continuation,
cancellation, queued follow-ups, permission return, and thinking/tool ordering.
This run used direct Cargo because mbx's Git configuration probe blocked in
`access()`; no repository or global cache defaults were changed.

Modal ownership follow-up: 61 interaction/UI unit tests and 4 CLI-process PTY
tests passed, including a new denial-plus-draft packet regression. An additional
recommended-profile test did not reach that boundary: it expected `cat
ordinary-output.txt` to run without confirmation, but the runtime requested
approval. That permission-contract discrepancy remains open; the original test
and permission policy were not relaxed to make this UI gate pass.

## Executive verdict

The CLI now has a coherent canonical path:

```text
Terminal composer
  -> CLI session adapter
  -> Application command boundary
  -> Rust Agent Core
  -> committed effect outbox
  -> Agent Host
  -> correlated Host responses
  -> Application projection and terminal rendering
```

The audit found that most severe failures were not terminal rendering defects. They were ownership defects between Core checkpoints, the Application hot-Core cache, the committed effect outbox, durable boundary input, and background workers. These boundaries have been tightened and are now covered by process-level tests.

The current gate is green: the complete Core test command passes, both CLI binaries pass their unit suites, and all 70 real PTY interaction tests pass serially.

## Scope

This audit covers:

- `cli/` interactive input, commands, terminal rendering, permissions, task supervision, background workers, context presentation, localization, and restart behavior.
- `core/` Agent state transitions, queued input, effect recovery, context and goal ownership.
- `core/nola-application/` canonical persistence, hot-Core caching, projections, and effect outbox transactions.
- `core/nola-runtime-driver/` ordering between Host responses and boundary submissions.
- Real PTY sessions, process termination/restart, SQLite recovery state, and large-conversation performance coverage.

This audit does not authorize keyword-based routing. The model selects concrete tool calls from schemas; Core evaluates the concrete capability, arguments, resource, effect, replay safety, and permission policy.

## Authority model

| Concern | Canonical owner | Required invariant |
|---|---|---|
| Conversation transcript | Application Store | User, assistant, tool, and reasoning projections are committed exactly once and in causal order. |
| Agent decisions | Rust Agent Core | Clients do not infer tools or permissions from user-language keywords. |
| Executable Host delivery | Application effect outbox | Every dispatched effect has one committed request identity and replay policy. |
| Crash recovery state | Core checkpoint plus Application outbox | Checkpoints may redact materialized requests; the outbox must rebind them before execution or response acceptance. |
| Active interactive input | Terminal composer and durable command queue | Input typed during a run is queued as `NextTurn`, persisted, promoted at a safe boundary, then acknowledged once. |
| Background execution | Durable task records and supervisor | Worker PID, database identity, resource budget, result, and terminal state remain task-scoped. |
| Presentation | CLI emitter | Rendering never mutates canonical ordering or decides whether a tool is allowed. |

## Incident: the second input appeared to do nothing

### Observable symptom

After a process restart or an interrupted stream, the first response could recover, but a second user input occasionally produced no answer. The terminal eventually reported:

```text
Agent Core failed: application_rejected: reduce_and_accept_host_response failed:
pending Host effect has no request
```

### Durable evidence

The failed PTY database showed all of the following at the same time:

- The second input was already present in `accepted_submission_ids`.
- Its durable `agent_commands` row was still leased because the enclosing Host cycle had not drained.
- The model effect remained pending in `effect_outbox` with a complete `HostRequestEnvelope`.
- The Core checkpoint intentionally contained the same pending effect with `request: null`.
- The active run was interrupted rather than completed.

This proved that the input was not lost by the terminal. It had crossed the input boundary and failed later at the Host-response boundary.

### Root cause

Core checkpoints intentionally redact materialized Host requests because the Application outbox is the delivery authority. During cold recovery, `reduce_and_commit_active_submission` restored that redacted checkpoint, accepted the queued second input, and returned the Core to `ApplicationRuntime` as hot state without first rebinding committed outbox requests.

The next Host response preferred the hot Core cache. Because the cache contained a pending effect with `request: null`, it could not correlate the response even though the durable outbox still had the correct request.

This was timing-sensitive because it required this sequence:

1. A model stream is interrupted or the CLI process exits.
2. A queued `NextTurn` input remains durable.
3. A fresh Application Runtime first processes that boundary input.
4. The same runtime then receives the recovered model response.

Direct reducer tests did not reproduce the hot-cache handoff, which is why the defect initially escaped narrower tests.

### Fix

On the cold active-submission path, Application now loads committed Host requests once, restores Core, and rebinds every executable outbox request before submitting boundary input or retaining the Core as hot state.

The fix preserves the existing safety model:

- Checkpoints continue to redact request payloads.
- The outbox remains the sole executable-delivery authority.
- Durable input remains at-least-once until the complete Host cycle drains.
- Core submission IDs keep replay idempotent.
- The UI does not swallow or retry protocol errors.

### Regression coverage

- A deterministic Application E2E reproduces cold restore, queued second input, hot-Core retention, terminal response acceptance, and successor-turn creation.
- The real interrupted-process PTY scenario passed five consecutive runs.
- The complete 70-test PTY suite passed after the fix.

## Resolved audit findings

| Severity | Finding | Resolution |
|---|---|---|
| P0 | A cold boundary submission could cache a request-less pending effect and reject the next response. | Rebind committed outbox requests before retaining cold-restored Core. |
| P0 | An active Goal caused ordinary typed follow-ups to be treated as hidden automated continuation. | Only empty scheduler/resume prompts are hidden; nonempty composer input is durable visible user input. |
| P0 | A terminal Host response could close a run before late boundary input was promoted. | Drain boundary input before non-delta terminal responses; apply model deltas first to preserve attribution. |
| P1 | A repeated terminal reason could prevent a newly promoted response from being flushed. | Always flush promoted terminal turns, independent of repeated reason text. |
| P1 | Foreground long-running shell work could not transfer into durable background supervision. | Added explicit background-transfer support and Ctrl+B supervision. |
| P1 | A background worker could open a different database or race before its PID metadata was durable. | Pass the database path explicitly, persist worker PID before release, and retain the real child PID. |
| P1 | Background permission status used internal capability identifiers and English-only presentation. | Resolve localized tool display names and localized waiting-permission text. |
| P1 | MCP recovery tests intermittently read a newly accepted nonblocking socket as `WouldBlock`. | Switch the accepted test socket to blocking mode before protocol reads. |
| P2 | PTY expectations encoded obsolete English/internal UI labels. | Assert current localized user-facing labels and whitespace-tolerant terminal output. |

## Permission model

The CLI exposes three explicit profiles:

| Profile | Behavior |
|---|---|
| `safe` | Conservative mode; asks before broader mutations or execution. |
| `recommended` | Default; permits reversible workspace work and asks for destructive, irreversible, or materially external effects. |
| `allow_all` | Broad approval within the configured Host and workspace boundaries. |

Permission evaluation is based on the normalized concrete call. A shell wrapper is not the permission identity shown to the user when the underlying command can be represented. Conversation and task grants are scoped to their canonical resource and are reused without repeated prompts.

The real recommended-profile PTY test verifies that a normal `write_file` proceeds without a prompt, a destructive `rm -rf` request is presented in Chinese with the concrete operation, denial prevents deletion, and no keyword list selects the tool.

## Input and goal behavior

- Input submitted while a turn is running uses `AgentInputDelivery::NextTurn`.
- The input becomes canonical visible conversation content when Core promotes it at the terminal boundary.
- Durable command acknowledgement occurs only after the enclosing Host-effect loop drains.
- Replay after process loss is safe because Core records accepted submission IDs.
- `/goal resume` resumes state without inventing a visible user message.
- A real nonempty follow-up during a Goal is user steering, not hidden scheduler input.
- Returning to the composer from a permission panel is an interruption and does not silently continue the agent.

## Context and compaction

Core owns context-window accounting and compaction decisions. The CLI footer is a projection of the same canonical usage data in idle, streaming, normal, and debug modes. Manual `/compact` and automatic compaction use the same Application/Core path, while the full transcript remains preserved for replay.

Coverage includes repeated automatic compaction with an active Goal and permission request, manual compaction presentation, restart behavior, and a synthetic ten-million-character conversation under a controlled memory limit.

## Background execution

Background shell and delegated-agent tasks share durable task records and the same global concurrency supervisor. A worker receives its database path explicitly rather than rediscovering configuration from ambient state. Startup metadata is committed before execution begins, and terminal mutation uses the same Store identity.

Coverage includes process loss before spawn, process loss after command start, exact-once recovery policy, CPU/memory/process/output/wall-clock budgets, global fairness, cross-project contention, task-scoped permissions, and parent/child agent continuation.

## Terminal and localization

- Composer cursor placement and editing are PTY-tested.
- Context footer is hidden while a turn is active and restored consistently afterward.
- Tool receipts remain compact in scrollback and can retain canonical detail without flooding the terminal.
- Permission and background status use localized user-facing tool names rather than internal capability IDs.
- Long streamed output remains in normal terminal scrollback without enabling a private scroll region.

## Validation evidence

| Gate | Result |
|---|---|
| `cd core && cargo test --quiet` | Passed. |
| `cd cli && cargo check --all-targets` | Passed. |
| CLI `nola` unit suite | 876 passed. |
| CLI `nola-agent-host` unit suite | 876 passed. |
| Deterministic cold-boundary Application regression | Passed. |
| Interrupted-process second-input PTY, five consecutive runs | 5/5 passed. |
| Complete serialized CLI PTY suite | 70/70 passed in 240.74 seconds. |
| Ten-million-character conversation restore | Passed as part of the PTY suite. |

## Remaining risks and follow-up work

| Priority | Risk | Recommended work |
|---|---|---|
| P1 | Debug logs record generic Application command timing but not boundary submission IDs, outbox effect IDs, checkpoint revision, or hot/cold Core choice in one correlated record. | Add structured correlation fields without logging private prompt or tool output content. |
| P1 | Large tool results can lead the model to issue several sequential `tool_output_read` calls over adjacent ranges. | Expose bounded range metadata and encourage concurrent or larger safe reads through the tool contract and model guidance. |
| P2 | Project scope is anchored to the launch working directory, which can surprise users who start the CLI from a nested directory. | Make the active workspace root more prominent and add an explicit workspace-switch flow. |
| P2 | Real-provider E2E remains dependent on locally configured credentials and endpoint availability. | Keep deterministic protocol servers mandatory and run an optional credentialed provider gate in release environments. |

## Release gate

A CLI change that affects input, Host responses, permissions, context, goals, tasks, persistence, or rendering should not be considered complete with unit tests alone. The minimum release gate is:

1. Core tests pass.
2. Both CLI binaries compile and pass unit tests.
3. The complete PTY suite passes serially.
4. Process-restart scenarios leave no unacknowledged input that Core has not recorded idempotently.
5. SQLite inspection shows every pending executable effect has a corresponding committed outbox request.
6. User-visible permission and status text passes localization coverage.
