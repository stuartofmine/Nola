> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Core Task and Permission Adoption

## Purpose

This document compares the durable sub-agent and permission boundaries in the
reference implementations with Nola's current architecture. It is an
implementation contract, not model-routing guidance. User text never selects
or authorizes a capability; the model emits a typed call and Core evaluates the
concrete call, resource, effect, and permission scope.

## Reference systems

### Codex

- A spawned agent is a child thread, not a long-running shell command.
- The thread graph stores a directional parent/child edge with an explicit
  open or closed lifecycle state.
- The Host supplies agent spawning as an injected capability. Extensions do
  not create an unrelated process and then encode its identity in tool text.
- Approval and sandbox policy are evaluated from structured tool metadata and
  the active permission profile.

### Claude Code

- An in-process teammate receives an independent identity, context, task
  record, message list, model override, and abort controller.
- Task state is registered before the teammate loop starts, so the UI can show
  work immediately.
- Worker permission requests are durable messages routed to the leader. A
  request has an id, worker identity, tool call id, structured input, status,
  resolution, optional modified input, and permission updates.
- Cleanup and cancellation are explicit lifecycle operations rather than
  inference from missing output.

### OpenCode

- A subtask is persisted as an assistant message plus a running Tool Part
  before execution.
- The subtask gets its own abort controller and agent profile.
- Permission evaluation merges the sub-agent rules with session rules and
  suspends on a structured request.
- Completion, failure, and cancellation update the same Tool Part instead of
  replacing it with unrelated assistant prose.
- Session approvals are cached as structured rules, preventing repeated
  approval loops for the same approved capability.

## Nola before this adoption

Core `TaskHandle` persisted task state, progress, leases, retries, cancellation
intent, and result references. It did not persist the execution relationship
that makes a Task resumable as a child Agent.

The CLI separately stored `parent_task_id`, `child_session_id`, and tool-call
information in its runtime-task database. Cancellation handles and per-task
permission grants lived in process-global maps. Other clients did not share
that execution contract. After restart, Core could recover a task status but
could not identify the child session, permission scope, or cancellation scope
that owned the work.

This split causes several user-visible failures:

- a foreground Turn can appear hung while a client-private task is running;
- permission decisions can be requested repeatedly after recovery;
- cancellation can update the UI without reaching the child execution;
- task results can arrive as ordinary tool text rather than the original Part;
- platform behavior diverges even when every client uses the same Core state.

## Adopted Core contract

`TaskHandle` now owns a durable `TaskLineage` with:

- `parent_task_id` for the Task graph;
- `child_session_id` for the independently recoverable Agent session;
- `parent_call_id` for the Tool Part that presents progress and results;
- `permission_scope_id` for suspended and remembered decisions;
- `cancellation_scope_id` for propagation to the active execution.

Core enforces these invariants:

- a Task cannot be its own parent;
- a declared parent must already exist in the projection;
- one child session cannot back two live Tasks;
- state updates preserve lineage;
- the Host cannot attach a promoted Task to a different tool call;
- omitted `parent_call_id` is filled from the correlated Host effect.
- cancelling a parent produces a stable child-first cancellation sequence;
- every live descendant receives its own Task update and Host cancellation
  effect before the parent scope is torn down.

Core permission provenance now distinguishes structured delegation from
authority inferred from text:

- a delegated child may use explicit `Conversation` rules inherited from its
  parent configuration;
- a parent `Turn` grant never propagates to a child;
- scheduled, replayed, and recovered work cannot use conversation grants;
- copying the parent permission scope is a Host contract, not a prompt or
  keyword-routing behavior.

The fields are optional for checkpoint decoding, but optionality is migration
compatibility, not the target execution model. A Host that launches a child
Agent must provide the child session, permission scope, and cancellation scope.

Additional cross-platform presentation invariants:

- `TaskProjection` is the sole durable source for asynchronous task state. The originating Tool Part may complete with a typed task handle so the model can continue; clients must not keep a second synthetic Tool Part running.
- Clients bind task presentation to the originating assistant bubble by `owner_turn_id`. `parent_call_id` links the task back to the concrete tool call for audit and detail views.
- Task lineage is decoded without reinterpretation on iOS, macOS, Android, and Web. Clients do not infer parentage from text, task titles, or capability names.
- A client persists the active Core Turn identity with the assistant message. Restored task checkpoints are filtered by that identity, so switching conversations cannot move old tasks into the newest bubble.
- Asynchronous workers commit progress and terminal results through idempotent `UpdateTask` submissions after claiming the Task. They never manufacture a delayed response for the original tool effect.
- Core validates ownership tokens, monotonic progress, cancellation monotonicity, and exact completion watermarks before accepting worker output. A terminal worker update atomically closes every pending Host effect bound to that Task.
- CLI runtime tools return a typed `RuntimeTaskPromotion`; the Host driver emits `TaskPromoted` directly instead of parsing JSON output. The background worker then uses the same lease token and input watermark in both SQLite execution state and Core Task state.
- SQLite execution state and Core Task state commit the same completion watermark. If the Core claim fails after the SQLite lease is acquired, the Host releases that lease into `retry_waiting`; it must never leave a database task falsely running.
- Worker operation identifiers are generated from a process-local monotonic sequence rather than wall-clock milliseconds, so multiple tool activities in one millisecond cannot be collapsed by Core idempotency.

## Required next migration

### 1. Typed tool execution disposition

The Host tool executor must return one of these structured dispositions:

- `completed`: terminal Tool Result;
- `suspended_for_permission`: nonterminal permission request;
- `promoted_to_task`: nonterminal Task promotion with full lineage;
- `failed`: terminal typed failure.

Do not discover promotion by tool name, output strings, JSON embedded inside a
string, shell command prefixes, or a client-only registry.

### 2. Core permission broker

Permission requests must be keyed by the concrete tool call and permission
scope. A suspended request survives checkpointing. Resolution resumes that
exact effect and records an explicit scope such as once, session, workspace,
or persistent policy. A chat message is never treated as approval.

Child Agents use the same broker. The leader UI may present their requests,
but the decision is written to the child's structured permission scope rather
than copied into a process-local set.

During migration, CLI background-Agent conversation grants are persisted under
the concrete `conversation:<owner_session_id>` scope and matched against the
structured tool/resource rule. They are no longer cached in `AgentTaskHandle`.
This fixes repeated prompts after worker or process changes, but the pending
request remains legacy Host state until it is represented as a checkpointed
Core permission effect.

### 3. Complete Host cancellation propagation

Core now records cancellation intent for the full descendant tree and emits
child-first cancellation effects. Hosts must next route each effect through
`cancellation_scope_id`, acknowledge it, and commit a terminal Task update.
Cancellation must also close the parent/child graph edge and mark the original
Tool Part as cancelled.

### 4. Result reconciliation

The originating Tool Part completes exactly once with a typed `task_handle`.
The promoted Task then owns progress, permission waits, cancellation, and the
terminal `result_ref`; `parent_call_id` places that Task projection beside the
originating Part in the same assistant message. Clients must not reopen or
rewrite the terminal Tool Part, maintain a second task timeline, or reconstruct
state from localized labels.

### 5. Recovery

On startup, the Host claims live Tasks through the existing lease protocol,
restores the child session, reattaches permission and cancellation scopes, and
continues from the checkpoint. If the child cannot be restored, the Host emits
a typed terminal failure; it does not leave the Task running indefinitely.

### 6. Single-writer command mailbox

The durable parent Agent session must be owned by one serial event loop. Model
streams, foreground Host responses, background Task claims, heartbeats,
progress, permission resolutions, and terminal worker updates submit commands
to that loop through a session mailbox. A worker must not lock and mutate the
parent runtime directly, and a UI request must not hold the parent session lock
for an entire model Turn.

The mailbox acknowledges durable command acceptance separately from command
application. Core remains the only reducer. Every accepted event is appended
to the session journal before clients receive it. CLI, Web, and native clients
subscribe using a cursor and reconstruct the same Task projection after a
disconnect; request-scoped SSE and process-local callbacks are transports, not
state owners.

The CLI Host now has a SQLite-backed per-session FIFO Task inbox. Background
workers durably enqueue claim, heartbeat, progress, terminal, and cancellation
submissions without waiting on the parent Turn mutex or requiring the Host to
be resident. An idle Host drains immediately; an active foreground Turn drains
before releasing its emitter; an unloaded Host drains after restoration. Core
dispatch commits the checkpoint before the Host acknowledges and removes the
command. A crash in that interval replays the command, so operation IDs and
idempotent cancellation remain mandatory.

Completed-task delivery uses two independent durable cursors: `unread` for the
human notification surface and `model_notified` for model context. Reading a
task does not consume model delivery. Building Host identity or preparing a
Turn does not mutate either cursor; `model_notified` is committed only after a
successful Turn consumed the stable task-mailbox context component.

## Typed Tool output boundary

Known protocol tools are normalized by the Host before `ToolCompleted` enters
Core. `skill_invoke` returns a root `invoked_skill` object and
`request_user_input` returns a root pending-input object. Core does not scan
`text`, `output`, `result`, or `data` wrappers and never parses JSON embedded in
a string to discover protocol state. Ordinary tool text therefore cannot
accidentally activate a Skill or suspend a Turn.

## Verification matrix

The migration is complete only when the same contract passes on CLI, Web,
macOS, iOS, and Android:

| Scenario | Required evidence |
| --- | --- |
| Spawn | Running Part and Task exist before child model output |
| Parent graph | Child references an existing parent and open edge |
| Permission | One request suspends; once/session/workspace choices resume the exact call |
| Recovery | Restart restores child session and does not repeat an approved request |
| Cancellation | Parent request reaches child and closes both Task and Part |
| Result | Tool Part completes once with a handle; linked Task reaches one terminal result |
| Unknown tool | Typed recoverable error returns to the model without client prompt rewriting |
| Multilingual input | Identical tool schema and policy path; no language-specific routing |

Unit fixtures prove serialization and state-machine invariants. End-to-end
tests must additionally prove a real child model call, a real suspended
permission decision, restart recovery, cancellation, and UI projection. A
mock that returns a prewritten Task result is not sufficient evidence.

## Declared capability effects

Engineering-loop state is driven by a versioned effect declaration in the frozen Host capability catalog. Core no longer classifies a tool by its name, file extension, shell executable, or command text. Fixed capabilities declare `investigate`, `mutate`, `validate`, `mutate_and_validate`, or `execute`. Polymorphic capabilities declare a schema-constrained argument and a closed mapping; CLI `bash` uses the required `operation_kind` enum.

The effect declaration is bookkeeping evidence, not authorization. Core still evaluates the concrete capability, resource subject, permission rule, input origin, and result independently. A model cannot gain access by declaring a safer operation kind.

Permission domains are descriptive policy labels, not parsable authorization metadata. Core therefore never derives a path, URL, command, memory scope, or Skill scope from words contained in a domain label. Versioned built-ins currently retain explicit resource rules; an unknown Host, MCP, or Skill capability is bound to its complete canonical invocation until its catalog supplies a typed resource declaration.

The concrete permission resource is frozen into the Core-owned `ToolCallState` immediately after Host input validation. Permission suspension, checkpoint recovery, conversation grants, and sibling-call re-evaluation reuse that exact subject; they do not reinterpret the capability name or a newer catalog revision.
# Durable background permission suspension

Background-Agent permission requests are persisted per concrete call in
SQLite before a worker waits. Core remains the permission authority; the
condition variable is only a wake-up optimization. A conversation grant is a
structured rule and resolves every currently pending request matched by that
rule. Cancellation durably denies the task's pending calls. Custom input ends
the current invocation and returns to the normal user-input queue instead of
being converted into a synthetic tool failure or discarded.
# Durable command consumption

Task commands are not merely stored before dispatch. A Host atomically leases
the oldest available command and may acknowledge it only with the same lease
owner after Core dispatch and checkpoint persistence. Concurrent Host
processes therefore cannot execute the same live command; a crashed consumer's
lease expires and makes the idempotent command replayable.
# Tool provenance boundary

The Host catalog owns tool provenance in non-model-visible metadata. Core freezes that declaration into each Tool Part alongside structured input/output, permission state, artifacts, cancellation state, and replay identity. Skill, MCP, device, extension, built-in, and internal capabilities therefore remain distinguishable without coupling UI or permission policy to display names.
# Background Agent state authority

Background Agent permission waits and ordinary status changes now update Core's TaskProjection under the Task ownership token, not only the CLI execution database. SQLite remains the Host's durable work queue; Core remains the cross-client lifecycle authority. A client therefore sees `waiting` before rendering a permission prompt and returns to `running` only after the durable decision resolves.

## Skill lifecycle authority

The Task ownership rule also applies to Skills. Hosts only resolve and materialize Skill content. The structured materialization starts a Core-owned Skill run whose budget, capability dependencies, usage, and terminal state are checkpointed with the conversation.
