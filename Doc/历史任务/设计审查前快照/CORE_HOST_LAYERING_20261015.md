> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Core / Host Layering

> 2026-10-15 整体设计复核前的历史快照，仅保留当时叙述与证据。旧 ABI、Trace 权威、schema 数字、传输别名和其他旧规则均不作为现行规范。当前入口见[软件架构总纲](../../软件架构设计/总体设计/SOFTWARE_DESIGN.md)。

Nola's Agent architecture follows one rule: Core owns meaning; Host owns effects.
Clients render the Core event stream and never reconstruct Agent state from
transport-specific messages.

## Reference synthesis

The design intentionally combines a small set of proven ideas rather than
copying any one implementation.

### Codex CLI

- One long-lived session owns at most one active Turn.
- Every Turn captures an immutable model, tool, permission, environment, and
  context-window snapshot.
- User steering is accepted during a Turn but becomes model-visible only at a
  model request boundary.
- Context compaction is planned before sampling, not after a request fails.

### OpenCode

- Stable message and Part identifiers are the UI and persistence boundary.
- Text, reasoning, tool calls, artifacts, retries, and compaction have explicit
  lifecycle state.
- Tool input may stream while the Part identity remains stable.
- Tool output has a bounded preview and an optional full-result reference.

### Claude Code

- Tool calls and results are retained or removed as complete pairs.
- Compaction preserves recent complete turns and durable working state.
- Context-overflow recovery removes oldest complete API round groups rather
  than arbitrary individual records.
- Cancellation is explicit and propagated to every active effect.

### Open WebUI

- Remote Host connections have explicit session identity and lifecycle.
- Every request, response, progress event, and cancellation shares correlation
  identifiers.
- Heartbeats renew a bounded lease; disconnect and timeout are state changes,
  not UI-only notifications.

## Ownership

### AgentCore

`AgentCore` is the only public mutable owner of one conversation. Its private
`AgentRuntime` reducer may change without becoming a Host contract:

- append-only transcript;
- compactable model-visible context;
- at most one active Turn;
- session-owned boundary-aware input queue;
- permission grants;
- Host capability catalog and lease;
- pending Host effects;
- typed durable memory projection;
- promoted long-running task projection;
- MCP descriptors and Skill execution state;
- monotonic event cursor and checkpoint revision.

The internal vocabulary is intentionally split:

- `AgentCommand`: an accepted fact to reduce;
- `RuntimeEvent`: a transient result of reduction;
- `AgentEffect`: I/O intent planned by Core;
- `HostRequestEnvelope`: one correlated delivery attempt for an effect.

The recovery journal, not the effect planner, owns request identity, replay
safety, response sequence, cancellation acknowledgement, and Host lease
rebinding.

The durable Thread is split by ownership rather than by serialization:

| Thread subsystem | Owns | Must not own |
| --- | --- | --- |
| `conversation` | transcript, model context, context window, Part projection | Host lease or permission prompts |
| `execution` | active Turn, input queue, permission rules | transport requests or persistence |
| `host` | capability catalog, pending effect journal | transcript or Turn completion |
| `memory` | typed records, namespace, provenance, retention, ordered CRUD revision | prompt keyword routing or Host permission UI |
| `tasks` | explicitly promoted TaskHandles, owner Turn, progress, result reference, cancellation | ordinary short tool Parts or scheduler implementation |
| `capabilities` | MCP descriptors, Skill execution mode/budget, recoverable selection state | platform I/O or language-specific trigger phrases |
| `journal` | Trace context, event cursor, revision, idempotency IDs | model/tool policy |

These structs are flattened in checkpoints so runtime refactoring does not
silently redefine the durable wire schema.

## Reference ideas mapped to code

| Reference | Adopted mechanism | Nola module |
| --- | --- | --- |
| Codex CLI | one public thread conduit, immutable Turn context, boundary-aware steering | `kernel`, `session`, `turn`, `input_queue` |
| OpenCode | stable Item/Part identity and explicit text/reasoning/tool lifecycle | `parts`, `projection`, `tool_lifecycle` |
| Claude Code | preserve complete tool rounds, proactive compaction, explicit cancellation | `compaction_planner`, `context_window`, `effect_dispatch` |
| Open WebUI | remote executor identity, lease, correlated progress and reconnect | `host`, `HostEffectJournal`, Host envelopes |

Tool registration and model exposure are separate contracts. A Host may
register internal journal, permission, compaction, discovery, or diagnostic
executors without advertising them to the model. Core projects a tool schema
only when the current catalog marks the capability available, routes it as a
tool, and exposes it as visible. Deferred, internal, and hidden capabilities
remain outside `ModelRequest.tools`.

Capability selection is semantic and schema-driven. The model chooses a
concrete built-in tool, MCP tool/resource/prompt, or Skill from the catalog it
was given; Core then validates the typed call, owner, availability, permission,
and lifecycle. Clients must not rewrite the user query, force a tool, authorize
an operation, or select a Skill by matching words such as "search", "memory",
"current", or translated equivalents. Unknown or stale selections return a
recoverable typed result so the model can re-plan against the current catalog.

Memory mutation is explicit. `CreateMemory`, `UpdateMemory`, and
`DeleteMemory` submissions mutate typed `MemoryRecord` values and advance one
ordered `MemoryProjection` revision. A record carries namespace, provenance,
retention, and stable identity. This is an internal Core operation and does not
request Host permission; mentioning memory in ordinary user text performs no
write. The Host may provide persistence effects, but it may not reinterpret or
silently create records.

An ordinary tool lifecycle is not a background task. A `TaskHandle` exists only
after explicit promotion and carries stable task/session/Turn ownership, kind,
state, progress, current step, optional result reference, and cancellation
state. Long Skills, scheduled work, and data-source refreshes can share this
projection. Short tools remain message Parts and never appear in the task
center merely because they took noticeable wall-clock time.

Skills support typed `inline` and `fork` execution. Inline execution shares the
current Turn boundary; fork execution receives independent iteration/token/time
budgets and reports a bounded result back to its owner. Both modes use stable
operation identity, exactly one terminal transition, checkpoint/replay, and
late-update absorption. The Host executes effects; it does not invent the mode
or retry budget after dispatch.

Within the reducer, model planning, tool execution planning, compaction, and
Turn lifecycle are separate modules. Cross-module calls may pass state and
`AgentEffect`, but only `effect_dispatch` may create a `HostRequestEnvelope`.

Nola deliberately does not copy Codex's Tokio ownership model, OpenCode's
provider-specific event loop, Claude Code's opaque client state, or Open
WebUI's server-owned conversation semantics. The Core remains deterministic
and local-first; asynchronous I/O remains a Host concern.

## Architectural acceptance gate

An implementation is conforming only if a fake Host can drive an entire
Thread through typed submissions and envelopes without reading Core internals
or parsing Trace. The acceptance scenario must cover:

1. create or restore a Thread;
2. freeze one Turn context;
3. stream reasoning and assistant Parts;
4. request permission and execute a tool;
5. persist a checkpoint;
6. interrupt and acknowledge cancellation;
7. reconnect under a new Host lease;
8. compact an active Turn and an idle Thread;
9. create, update, restore, and delete a typed memory record;
10. promote and cancel a long-running task while a short tool remains unpromoted;
11. resolve an MCP descriptor and run inline/fork Skills with independent budgets;
12. replay ordered Core events into every client projection.

Platform smoke tests are supplemental. They do not replace this Core/Host
contract scenario.

The public mutation API is `AgentCore::submit(AgentSubmission)`. A submission
atomically produces an `AgentTransition` containing:

- sequenced Core events;
- correlated Host requests;
- the resulting runtime snapshot.

`UserInput.items` enters the append-only transcript.
`turn_context_items` and `turn_context_components` are prepared by Core and
frozen only into the active Turn. They must never be disguised as synthetic
system messages in the transcript. A model request is built from that frozen
Turn context plus the compacted model context.

### Turn

A Turn owns only Turn-scoped execution state:

- immutable `AgentTurnContext`;
- status and model request count;
- open streaming Part identifiers;
- tool lifecycles;
- loop detection and terminal failure.

Input waiting for a boundary is deliberately not stored inside the Turn.
`AgentInputQueue` belongs to the session and has two lanes:

- `current_turn`: drained immediately before the next model request;
- `next_turn`: promoted only after the active Turn terminates.

This prevents a Host, UI, or transport from injecting content into a stream
that is already being processed.

### Host

A Host may:

- stream a model response;
- execute a declared capability;
- ask the user for permission;
- compact context through a model;
- persist a checkpoint;
- report progress;
- cancel an effect.

A Host may not:

- append transcript or model-context items directly;
- create or complete a Turn;
- infer permission persistence;
- decide whether queued input steers or starts another Turn;
- synthesize Core events;
- compact or prune context without a Core request.
- infer memory writes from user text;
- turn an ordinary tool call into a TaskHandle;
- select MCP or Skill capabilities from client-side keywords.

The Host chooses the checkpoint storage location and implements
`journal.checkpoint`; Core owns checkpoint content, revision, cursor, and
reason. A remote Host transports the current user input separately from typed
`context_components`.

The macOS local child-process Host and remote Host use the same payload
semantics. `context_messages` is not a supported desktop Host field; the CLI
ingress deserializes `context_components` directly into Core types.

## Host effect ownership

Every external operation is one Core-owned effect. The effect remains in the
checkpoint until a correlated terminal Host response is accepted.

```text
dispatched -> accepted -> streaming -> terminal
      |           |           |
      +-----------+-----------+-> cancelling -> cancellation acknowledged
```

`terminal` is represented by removal from the pending-effect set; it is not a
second persisted state. Core accepts a response only when all ownership fields
match the pending effect:

- `effect_id` identifies the logical operation;
- `request_id` identifies the current delivery attempt;
- `host_id` and optional `lease_id` identify the concrete executor lifetime;
- `session_id` and optional `turn_id` identify the conversation owner;
- `catalog_revision` and `availability_epoch` identify the capability snapshot;
- `sequence` is strictly increasing for that delivery attempt.

The reducer passes the complete `HostResponseEnvelope` to
`HostEffectJournal::accept_response`. Platform code and response reducers do
not select or omit individual correlation fields.

Reusing an `effect_id` during recovery preserves logical identity, but recovery
creates a new `request_id` and explicitly rebinds the effect to the current Host
lease. Core never rewrites all pending effects merely because a newer catalog
arrived.

Cancellation is also an effect. Core emits a correlated `Cancel` request for
every active model, permission, tool, context, or storage effect owned by the
Turn. The Host returns `CancellationAcknowledged` for the named target effect.
There is no direct driver cancellation API and no transport-specific shortcut.

## Canonical flow

```text
AgentSubmission
  -> AgentCore
  -> AgentTransition
       -> CoreEventEnvelope[]   -> persistence + every client UI
       -> HostRequestEnvelope[] -> Host effect executor
  <- HostResponseEnvelope
  -> AgentCore
```

There is no platform-specific Agent loop beside this flow. CLI and Web may host
the runtime in-process; native clients may call it through FFI or connect to a
remote Agent Host. Both paths use the same submissions, events, requests,
responses, checkpoints, and correlation rules.

## Invariants

1. One session has at most one active Turn.
2. Only Core mutates transcript, model context, Turn, Part, and permission state.
3. Every externally visible mutation has one monotonic Core event.
4. Every Host response references one pending effect and matches its request,
   session, Turn, Host lease, epoch, catalog revision, and monotonic sequence.
5. A Part identifier never changes during streaming.
6. Tool call and tool result form one lifecycle and one model-context pair.
7. Queued input is consumed only at a named boundary.
8. Compaction preserves the append-only transcript.
9. Cancellation is terminal for the target Turn, propagates to every active
   Host effect, and remains recoverable until Host acknowledgement.
10. Transport payloads may carry authentication metadata but never alternate
    Agent semantics.
11. Context components never enter the transcript as synthetic system messages.
12. Checkpoint content belongs to Core; checkpoint location and writing belong
    to Host.
13. Idle manual compaction creates no Turn and triggers no normal model sample.
14. Active-Turn automatic compaction resumes that same Turn after completion.
15. Platform UI consumes Core Trace and never derives a second execution state.
16. Model-visible tool specifications are the intersection of configured
    schemas and Catalog capabilities marked available, tool-routed, and visible.
17. Trace and UI events are never parsed back into tool or permission requests.
18. Memory CRUD occurs only through typed Core submissions and advances one
    ordered projection revision; ordinary text cannot mutate memory.
19. A TaskHandle is created only by explicit promotion and retains stable
    session/Turn ownership through checkpoint recovery.
20. Short tool calls never appear in the task projection.
21. MCP and Skill selection is validated against typed descriptors, never
    inferred from language-specific client keyword lists.
22. Inline and fork Skill executions have explicit independent budgets,
    idempotent operation identity, and exactly one terminal state.
