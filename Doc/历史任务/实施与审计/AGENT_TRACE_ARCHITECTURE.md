> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Agent Trace Architecture

> Status correction (2026-10-11): the trace-as-chat-authority design below is a
> historical record, not a current normative contract. Trace is diagnostic.
> Business recovery and UI consume committed canonical Application/Store
> projections. See [architecture contracts](../../软件架构设计/README.md) and
> [the audit](../2026-10-19_当前任务归档/AUDIT.md). Retaining this document does not authorize
> reconstructing messages or terminal state from diagnostic events.

## Purpose

Chat UI is a projection of one append-only execution trace. It is not an
independent state machine. The same trace must drive:

- the assistant bubble;
- current run status;
- thinking disclosure;
- tool parameters, results, files, and artifacts;
- permission lifecycle;
- context usage;
- validation and terminal state;
- replay after reopening a conversation.

The final assistant message is a projection of the trace. It is not the source
of truth for the execution process.

## Layers

## Implementation Map

The contract is shared, while transport and rendering remain platform-native:

| Responsibility | Production implementation |
| --- | --- |
| Event envelope, event kinds, reducer, redaction, and projection | `core/src/agent_trace.rs` |
| Canonical long-task contract fixture | `core/tests/fixtures/long_task_trace_v1.json` |
| Journal schema and storage | `core/nola-store/src/schema.sql`, `cli/src/store/event.rs` |
| CLI event normalization and emission | `cli/src/agent.rs` |
| Local Web SSE transport and journal replay API | `cli/src/web.rs` |
| Web reducer and presenter | `web/app/static/js/agent-trace-runtime.js`, `web/app/templates/chat.html` |
| macOS projection and presentation | `NolaMac/Models/MacAssistantPresentation.swift`, `NolaMac/Services/MacLocalAgentRuntime.swift`, `NolaMac/UI/MacChatWorkspaceView.swift` |
| iOS trace adapter and chat presentation | `Nola/Services/AgentTrace.swift`, `Nola/UI/ChatView.swift` |
| Android trace adapter, reducer, and presenter | `android/app/src/main/java/com/nola/agent/trace/`, `ChatViewModel.kt` |

The implementation boundary is intentional:

- adapters translate native callbacks into contract events;
- the reducer owns semantic state and ordering;
- journals own durable event history;
- presenters own localization, layout, animation, and expansion state;
- presenters must not reconstruct execution state from raw provider callbacks.

### 1. Core Trace Contract

`core/src/agent_trace.rs` owns the versioned event envelope, event kinds,
sequence rules, resource semantics, validation semantics, and pure reducer.
The contract contains semantic codes and structured values only. Platforms
localize presentation strings.

### 2. Host Adapter

Every provider or runtime converts native callbacks into `AgentTraceEvent`
before UI state is updated:

- CLI and Web normalize `Emitter` events;
- macOS normalizes Agent Host events;
- iOS normalizes Core, provider, and device-tool callbacks;
- Android normalizes Core, provider, and Web Host callbacks.

An adapter assigns one `trace_id`, `run_id`, `turn_id`, `message_id`, monotonic
`sequence`, and stable tool `span_id` for the whole run.

### 3. Trace Journal

Trace events are appended immediately. `agent_run` stores run-level state and
`trace_event` stores the full event envelope. Tool completion, app termination,
or final model text must not be prerequisites for persistence.

Clients replay by `trace_id` and `sequence`. Duplicate `event_id` values are
idempotent. A sequence gap pauses projection and requests replay.

### 4. Pure Reducer

The reducer is the only component allowed to turn events into run state:

```text
TraceEvent[] -> AgentRunProjection
```

It owns assistant text, thinking text, activities, validation, usage, and
terminal state. Reducers never render UI, localize strings, execute tools, or
write storage.

### 5. Platform Presenter

Each platform maps `AgentRunProjection` into native view models:

- one stable assistant bubble per `message_id`;
- one stable activity row per tool `span_id`;
- a compact current-step summary;
- expandable parameters, result excerpts, resources, and artifacts;
- validation-incomplete and error states that cannot look successful.

Transient transport callbacks must never directly mutate views.

### 6. Incremental Markdown Projection

The assistant text stream owns one renderer from the empty pending state to the
completed state. A platform may buffer incomplete Markdown constructs, but it
must:

- commit stable semantic blocks with stable IDs;
- derive committed block identity from Core Part identity, never array offset,
  parser ordinal, current text length, or render pass;
- animate only newly committed phrases or blocks;
- never replace the renderer when the first delta arrives;
- never replace the renderer when the run completes;
- keep already revealed content stationary;
- hydrate Mermaid or other rich blocks without replacing unrelated blocks.
- parse only the mutable tail with incomplete-syntax repair; completed blocks
  are memoized and never reparsed when later text arrives;
- render an opened code fence immediately as a plain code surface, then add
  syntax highlighting without replacing that surface or waiting for the
  closing fence.

## Presentation Contract

### Stable Assistant Bubble

The assistant bubble is inserted with the user message, before the first host
response. Its view identity is the trace `message_id` for the whole run:

```text
pending -> active -> streaming -> completed | failed | cancelled
```

These are presentation states of one bubble, not different views. Receiving
the first assistant delta or a terminal event must not replace the bubble,
move the activity area to another container, or create a second assistant
message.

Completion is a geometry no-op. Metadata, copy actions, terminal status, and
the final Markdown flush occupy geometry reserved while streaming. The
terminal event may change opacity, labels, and enabled state, but it must not
insert a new row, change bubble padding, switch renderer implementations, or
move already visible text.

### Activity Header

The compact activity header answers one question: what is Nola doing now?

- The title is a localized semantic phase such as processing, thinking,
  searching, reading, editing, or validating.
- Tool names, bounded arguments, progress, and results are detail rows.
- Repeated events with the same semantic title and detail do not publish a new
  view state.
- Rapid tool progress may be coalesced, but start, permission, failure, and
  terminal events are never dropped.
- A completed run collapses to one localized completion title. It does not
  retain an internal event name such as `tool_output_read`.

### Tool Detail

Every tool span has one stable row keyed by `trace_id + span_id`. The collapsed
row shows icon, localized action, primary resource, status, and duration. The
expanded row may show redacted structured arguments, bounded result excerpts,
files or URLs, validation, and failure details. Raw protocol names and
unbounded tool output are not user-facing labels.

### Reveal Motion

Only newly committed semantic text may animate. The intended motion is a
single restrained reveal, approximately:

```text
opacity 0.18 -> 1
vertical offset 3px -> 0
duration 420-560ms
ease-out
```

Already visible text must never animate again. Do not blur glyphs, animate
baselines, mask a whole document, or run simultaneous per-token effects; they
reduce legibility and cause shimmer during reflow. A terminal event must not
trigger a second reveal. Reduce Motion disables translation while preserving
immediate readable output.

### Typography Rhythm

Markdown uses one vertical rhythm per platform. Renderer nesting must not add
the same paragraph gap at the message, semantic-block, Markdown-document, and
leaf levels. Body line height targets 1.45-1.58; adjacent paragraphs use
0.55-0.75 line of separation; list items use 0.25-0.4 line; headings own their
leading separation and do not inherit another block gap. Blank source lines
separate blocks but repeated blank lines do not multiply layout spacing.

### Scrolling and Layering

Automatic following is enabled only while the user remains near the latest
content. It targets a comfortable reading anchor above the composer instead
of the absolute bottom. The opaque composer is above message content in the
z-axis; messages scroll behind it and receive bottom content inset equal to
the composer height. User scrolling suspends follow until they explicitly
return to the latest message.

## Renderer State

Incremental Markdown maintains two regions:

1. committed blocks with stable IDs and immutable rendered identity;
2. one mutable tail containing the currently incomplete Markdown construct.

The tail may be reparsed as chunks arrive. Committed blocks are not reparsed
or replaced. Completion flushes the same tail through the same parser; it does
not swap raw text for a separately rendered document. Renderer tests must
observe block identities and visible text, not merely compare the final source
string.

## Recovery Protocol

Recovery uses the journal as the authority and the live stream only as a
low-latency delivery mechanism:

1. The client creates the assistant bubble with its final `message_id` when the
   user message is accepted, before waiting for a host or model byte.
2. The host assigns stable run identifiers and appends every normalized event
   before treating it as durable or recoverable UI state.
3. The reducer accepts the next sequence, ignores an already-seen `event_id`,
   and buffers later sequences while an earlier sequence is missing.
4. A sequence gap pauses projection of later events. The client requests
   journal events after its last contiguous sequence; it must not guess,
   renumber, or render around the gap.
5. Replay is restricted by `conversation_id` and `trace_id`, sorted by
   `sequence`, and fed through the same reducer used for live delivery.
6. Replayed events advance the reducer and cursor without replaying reveal
   animation, stealing focus, forcing scroll, or creating another bubble.
7. After replay reaches a contiguous cursor, live SSE delivery resumes. Events
   received both from replay and SSE remain idempotent by `event_id`.
8. Refresh, process restart, or transport reconnection restores tool
   parameters, resource operations, bounded real results, assistant text,
   usage, validation, and terminal state from the journal.
9. A missing terminal event remains interrupted or recoverable. Clients must
   not synthesize success from the presence of assistant text or tool output.

Tool `call_id` values are not isolation keys. A replay lookup and every
presenter key must include the trace/run scope, so identical provider call IDs
in separate runs or conversations cannot merge.

## Semantic No-op Rules

An accepted event can advance journal/replay state without changing visible
semantics. Examples include:

- a duplicate `event_id`;
- an empty assistant or thinking delta;
- a repeated phase with the same semantic code and detail;
- repeated tool progress with unchanged status, arguments, resources, result,
  validation, and duration bucket;
- an unchanged usage snapshot;
- a replayed event whose projection is already hydrated.

For these events the reducer may update its contiguous sequence, seen-event
set, or replay cursor, but the presenter must publish **zero** view changes.
That means:

- no SwiftUI/Compose observable-state publication;
- no DOM mutation;
- no Markdown node replacement;
- no reveal animation;
- no automatic scroll;
- no focus or accessibility announcement.

Presenters compare semantic projections, not event arrival count. Stable keys
are `message_id` for the assistant bubble, committed Markdown block ID for
rendered blocks, and `trace_id + span_id` for a tool row. Tool start,
permission transitions, validation failures, terminal states, and newly
visible output are semantic changes and must never be discarded as no-ops.

## Required Event Order

A normal long task follows this shape:

```text
run.started
phase.changed
thinking.delta*
tool.started
tool.progress*
tool.permission.requested?
tool.permission.resolved?
tool.completed
validation.updated?
assistant.delta*
usage.snapshot*
run.completed | run.failed | run.cancelled
```

Thinking and assistant deltas may interleave with tools. `sequence`, not arrival
time or DOM position, defines order.

## Execution Narrative and SOP

One run presents one ordered execution narrative. The compact phase title,
thinking detail, tool rows, Skill steps, permission challenge, validation, and
answer are projections of that narrative; clients must not create a second
status-bar workflow beside the bubble.

- The model chooses concrete capabilities from the visible catalog. Clients
  never infer routing from prompt keywords.
- `tool_search` is discovery for capabilities absent from the active model
  projection, not a mandatory prelude to every task.
- A Skill run is opened once, owns stable step IDs, and closes once. Repeated
  catalog lookup, Skill loading, or unchanged progress is a semantic no-op.
- Independent safe reads may execute concurrently; mutations remain ordered
  behind their resource and permission barriers.
- Internal orchestration events may update diagnostics but do not become user
  rows. User-visible rows state the semantic action, primary resource, current
  outcome, and bounded evidence.
- Terminal success requires the requested artifact/result and required
  validation. A generic tool completion is not task completion.

## Adaptive Navigation

Desktop and tablet sidebars use the platform split-view primitive where
available. Their divider is directly draggable, keyboard/accessibility
reachable, and the chosen width persists per device. Desktop width is bounded
by a readable minimum and by the conversation pane's minimum width; compact
windows convert the same navigation model to an overlay instead of shrinking
labels or maintaining a second sidebar implementation.

## Invariants

1. A run has exactly one `trace_id`, `run_id`, `turn_id`, and assistant
   `message_id`.
2. Sequence begins at one and is strictly monotonic.
3. `event_id` is globally unique and replay-safe.
4. Tool start, progress, permission, and completion share one `span_id`.
5. UI components read one reducer projection, never parallel state stores.
6. Final completion is impossible while validation is incomplete or failed.
7. Sensitive values are redacted before journal insertion.
8. Reopening a conversation reproduces the same process order from the journal.

## E2E Acceptance Scenarios

All clients must run the same trace fixtures plus one real host execution:

The canonical cross-platform fixture is
`core/tests/fixtures/long_task_trace_v1.json`. Platform tests may copy it into
their test bundle at build time, but must not maintain a semantically different
fixture.

1. waiting -> thinking -> multi-tool -> Markdown answer -> completed;
2. permission requested after partial text, then resumed without loss;
3. duplicate tool progress and terminal events remain idempotent;
4. sequence gap pauses projection and replay resumes it;
5. cancellation and failure preserve partial output and terminal state;
6. validation-incomplete cannot produce a success summary;
7. refresh or process restart replays tools, files, and final text in order;
8. headings, lists, tables, links, code fences, and Mermaid cross chunk
   boundaries without renderer replacement;
9. already visible text does not animate again;
10. two concurrent runs with identical tool call IDs remain isolated.

UI E2E additionally asserts:

11. the assistant bubble exists before the first model byte;
12. bubble and committed Markdown block identities survive first delta and
    completion;
13. duplicate progress events cause no extra view publication;
14. an expanded tool shows redacted parameters, resource operations, and the
    real bounded/truncated tool result rather than a generic completion label;
15. a long stream stays above the composer without forced scrolling after the
    user scrolls away;
16. semantic no-op events produce zero UI/DOM mutations;
17. gap recovery and refresh replay preserve one bubble and stable committed
    Markdown identities;
18. equal tool call IDs across runs and conversations remain isolated;
19. screenshot geometry and contrast remain within platform-specific baselines.
20. completion changes no message-frame origin and no committed block frame;
21. paragraph/list/heading gaps match platform typography tokens without
    additive nesting;
22. desktop sidebar drag persists width and never reduces the conversation
    below its readable minimum;
23. a long Skill/tool task has one ordered narrative, no duplicate status
    surface, and no repeated unchanged discovery/progress rows.
