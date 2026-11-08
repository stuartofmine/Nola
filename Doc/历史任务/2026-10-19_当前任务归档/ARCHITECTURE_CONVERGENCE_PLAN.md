> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Nola Architecture Convergence Plan

Status: normative implementation plan

This document turns the principles in `../历史任务/实施与审计/ARCHITECTURE.md` into deletion targets,
module contracts, and acceptance gates. It is intentionally incompatible with
the current duplicate projection paths. Nola has not shipped, so convergence
must remove obsolete paths rather than preserve aliases, dual writes, or
fallback reconstruction.

## 1. Problem statement

The repository currently has the right architectural intent but does not encode
that intent in its interfaces. The same agent run can be interpreted by Core,
the application service, the Store, the platform Host, and a client-specific
trace reducer. This creates several independent state machines for one user
conversation.

Observed consequences include:

- streamed content being appended and later overwritten by a final snapshot;
- old messages being replaced instead of appended;
- tool identity degrading into generic titles such as `tool` or `running`;
- thought, tool, and Markdown presentation differing between live playback and
  restored history;
- context usage being recomputed differently by each client;
- diagnostic trace fields leaking into product UI;
- tests passing while screenshots show duplicate, missing, or unstable content.

These are architectural failures. They cannot be made reliable by adding more
client-side deduplication or string matching.

## 2. Non-negotiable invariants

1. The Store projection is the only durable product truth.
2. One user submission creates exactly one user message, one stable assistant
   message, and one run.
   `SubmitTurn`/`OpenRemoteTurn` allocates and returns one immutable
   `TurnIdentity` containing submission, run, user-message, assistant-message,
   and initial Part IDs. Every later command references that identity exactly;
   Store, Core, Host, and clients must never normalize or re-derive IDs.
3. Core is the only owner of agent state transitions and part-reduction rules.
4. The application layer is the only writer to product state.
5. Hosts execute typed effects and return correlated typed responses. They do
   not construct messages or UI state.
6. Clients render typed projections. They do not merge deltas, replay traces,
   infer tool identity, or rebuild canonical messages.
7. Diagnostic traces are disposable. Removing every diagnostic trace must not
   change the restored conversation or visible UI.
8. Protocol fields use one canonical spelling. Missing required identity fields
   fail decoding instead of falling back to a title or alias.
9. Permission decisions apply to normalized typed resources, not shell prefixes
   or localized text.
10. UI verification includes observed screenshots and temporal behavior, not
    only process exit codes or snapshot generation.

## 3. Target dependency graph

```text
Platform UI
    -> Platform Presenter
        -> nola-application query/command ports
            -> nola-agent-core
            -> nola-store
            -> nola-domain
        -> nola-protocol (wire only)

nola-runtime-driver
    -> nola-application effect port
    -> platform Host

nola-observability
    <- events copied after commits; never imported by product reducers
```

Forbidden dependency directions:

- Domain must not import Store, UI, Host, or provider types.
- Core must not import UI, platform APIs, database records, or localized text.
- Store must not call a provider or platform effect directly.
- Host must not mutate conversations, messages, runs, context, or permissions.
- UI must not consume provider deltas, raw Host events, or diagnostic traces.

## 4. Module responsibilities

### 4.1 `nola-domain`

Owns stable business identities and immutable product contracts:

- conversation, message, message-part, run, tool-call, attachment, and task IDs;
- `ConversationProjection`, `MessageView`, and `MessagePartView`;
- typed tool, reasoning, source, attachment, task, and error parts;
- typed context usage and run activity;
- typed permission subject, resource, scope, and decision.

Database row DTOs and application command payloads do not belong here. Existing
Store-oriented records must move into `nola-store`.

### 4.2 `nola-agent-core`

Owns deterministic transitions:

```text
(AgentState, AgentCommand) -> (AgentState, DomainEvents, Effects)
```

It owns part identity, append/replace semantics, tool lifecycle, reasoning
lifecycle, compaction decisions, cancellation, and terminalization. It never
persists, performs IO, emits localized labels, or decides presentation layout.

### 4.3 `nola-application`

Owns transactions and use cases through interface-segregated ports:

- `ConversationCommands`: submit, rename, pin, delete;
- `AgentRunCommands`: accept Host response, cancel, resume;
- `ConversationQueries`: timeline, run activity, context usage;
- `PermissionCommands` and `PermissionQueries`;
- `CapabilityCommands` and `CapabilityQueries`;
- `AutomationCommands` and `AutomationQueries`.

The public product API must not expose generic persistence operations such as
`AppendMessages`, `ApplyConversationProjection`, or diagnostic trace writes.
Those operations allow callers to bypass invariants and must be deleted.

### 4.4 `nola-store`

Owns repositories, transactions, the sole current schema, and materialized projections. It
must be split by bounded context instead of remaining one large implementation:

```text
store/
  conversation/
  run/
  permission/
  capability/
  context/
  automation/
  sync/
```

Internal projection/recovery journals and bounded diagnostics are not public
business-content replay sources. Trace journals belong to observability and
cannot restore business state. A separately defined typed committed assistant
journal may provide public replay only after Application/Store atomic publication
and owner-bound validation; that production rollout is not yet connected.

As of 2026-10-02, the Store seals one complete ordered publication set per actual
Admission/Transition: multiple owner groups and bounded frames share one atomic
writer and an immutable set receipt. Application frames use actual canonical
byte budgets, and canonical owner remapping borrows public Part fields rather
than cloning accumulated bodies. Shared lib and full Application/Store/Domain
tests passed on current source; the full CLI binary rerun passed 1,408 tests,
with two Worker tests ignored. A subsequent Skill ownership cutover binds an
immutable source selection to each Runtime, including workers and permission
recovery; its current-source acceptance is tracked separately in section 8.8 of
[the redesign record](../实施与审计/MODULAR_REDESIGN_2026-09-28.md). Selection is neither a grant
nor a sealed file-content snapshot. The
first set's producer coverage, Task/Skill/Attempt adapters, idle/recovery source
proofs, and production coordinator remain required before consumer cutover.
Self-declared frame counts prove replay immutability, not that no accepted Core
change was omitted. Preparations preceding set publication remain owned by the
outer command transaction and must be rolled back on failure.

### 4.5 `nola-protocol`

Owns versioned transport envelopes only. Each message carries:

- protocol version;
- request/run/effect correlation IDs;
- typed payload;
- sequence or projection revision where ordering matters.

The protocol uses canonical `snake_case`. There are no camelCase aliases,
localized enum values, or fallback decoding from generic titles.

### 4.6 Platform Hosts

Hosts expose capability adapters behind one effect interface. A Host receives a
typed effect, resolves platform permission, executes it, and returns a typed
result with the same correlation ID. macOS, iOS, Android, CLI, and Web may offer
different catalogs, but they share effect and result semantics.

### 4.7 Platform clients

Clients have only two product-facing responsibilities:

1. send typed application commands;
2. query and render typed Store projections.

A live update notification contains a conversation ID and projection revision.
The client fetches the new projection or changed typed parts. It must not merge
raw provider chunks or Host events into its own canonical message array.

## 5. Canonical turn flow

```text
User intent
  -> Application transaction creates user message + assistant shell + run
  -> Core reduces command and emits domain events + effects
  -> Store commits events, projection revision, and effect outbox atomically
  -> Runtime Driver leases an effect
  -> Host executes and returns a correlated response
  -> Application passes response through Core
  -> Store atomically commits changed parts and next effects
  -> UI observes revision and reads ConversationProjection
```

The assistant shell therefore exists immediately. Waiting, reasoning, tool use,
streaming text, completion, cancellation, and failure are states of that stable
message and run, not replacement messages.

## 6. Presentation contract

`MessagePartView` is the sole cross-platform presentation input. Important
rules:

- `capability_id` is required for every tool part;
- Reasoning and Tool `title` is a required independent display String (empty is
  allowed), never an identity fallback;
- Tool human summary, structured output and output preview remain independent;
  missing output, explicit JSON null and empty values must not collapse together;
- permission-display metadata and message/Part/call output references are
  descriptive evidence, not executable grants or private-path read authority;
- protocol state is an enum, never a raw string shown to users;
- reasoning summary and tool activity are distinct typed parts;
- streamed text updates an existing part by stable `part_id` and monotonically
  increasing revision;
- finalization changes state without re-sending or replacing the full message;
- context usage comes from the same conversation query used by the composer and
  inspector;
- localization maps stable display keys at the client boundary.

## 7. Migration sequence

### Phase 0: architecture gates

- Add dependency-direction tests using Cargo metadata.
- Add strict protocol fixtures and reject aliases or missing required fields.
- Record current duplicate write and reducer call sites as deletion targets.

### Phase 1: stable domain and protocol

- Introduce typed conversation projection contracts in `nola-domain`.
- Move Store DTOs out of Domain.
- Version Host and application protocol envelopes.
- Generate or share bindings rather than maintaining handwritten field mappings.

### Phase 2: one application write path

- Replace generic `ApplicationCommand` persistence variants with use-case ports.
- Make submit, Host response, cancel, permission decision, and compaction the
  only run mutation commands.
- Remove dual writes to message, projection, and trace journals.
- Replace Codex, Claude Code, and OpenCode transcript commits with Host adapters
  that translate executor events into standard correlated Host responses. The
  adapters must not construct Store Parts or terminal conversation projections.

### Phase 3: macOS chat vertical slice

- Make Mac query `ConversationProjection` for live and restored timelines.
- Delete Mac trace-to-message and session-history projection reducers.
- Keep only a stateless presenter from typed parts to SwiftUI view models.
- Use the same context query for composer footer and inspector.

### Phase 4: remaining clients

- Delete equivalent reducers from iOS, Android, CLI Web, and CLI presentation.
- Consume the same fixtures and typed projections on every platform.
- Keep platform-specific layout and animation, not platform-specific semantics.

### Phase 5: observability separation

- Move diagnostic trace persistence outside product commands and schema.
- Prove that deleting trace storage does not change restored UI.
- Retain logs only for diagnosis and bounded replay of external effects.

### Phase 6: bounded-context split

- Split Store and Application monoliths by the ports in section 4.
- Enforce module ownership and public API size in CI.

Current cutover note: capability catalog repositories and the operations family
(automation, workspace ownership, and replica synchronization) are now borrowed
as capability-specific SQLite objects. Workspace checkpoint deletion and session
lease release were removed from the Run-recovery maintenance port, so an
operations command cannot acquire recovery authority as a side effect of sharing
one physical connection. Conversation and Agent-runtime families remain separate
migration work and must not be inferred complete from these cuts.

## 8. Required verification gates

### Contract gates

- Missing `capability_id` for a tool part fails decoding.
- Unknown fields and alternate casing fail strict fixtures.
- Duplicate and out-of-order Host responses are idempotent by correlation ID.
- A terminal response cannot replace previously committed text.
- Live and restored projections are structurally identical.

### Cross-platform gates

- One canonical fixture produces equivalent typed timelines on Mac, iOS,
  Android, CLI, and Web.
- No client imports or invokes a trace reducer for product presentation.
- Context usage is identical in composer, inspector, restored session, and CLI
  footer.
- Permission behavior is based on typed normalized resources on every Host.

### Real-model E2E gates

- Multi-turn conversation with reasoning, search, file reads, file writes,
  permission suspension, resume, cancellation, and compaction.
- Kill and restart during streaming, then verify one assistant message with no
  duplicate or missing content.
- Switch conversations and restart the app, then compare Store projection and
  visible timeline.

### Visual gates

Screenshots or recordings are captured and actually reviewed at submission,
waiting, reasoning, tool start, tool completion, partial Markdown, finalization,
conversation switch, and restore. The review checks:

- duplicate, missing, or overwritten messages;
- layout jumps, flicker, unstable expansion, and scroll-follow behavior;
- incremental Markdown and open code-block rendering;
- tool identity, arguments, results, files, and localized labels;
- typography, spacing, alignment, clipping, contrast, and hierarchy;
- absence of protocol/debug strings in product UI.

Generating a screenshot is not a pass condition. A human or visual evaluator
must record observations against these criteria, and each failure must be linked
to the first divergent event, projection revision, or presenter output.

## 9. Completion definition

Architecture convergence is complete only when:

- there is one product write path and one durable conversation projection;
- Core owns all semantic reduction;
- Hosts only execute typed effects;
- all clients render the same typed projection without local semantic reducers;
- diagnostic traces can be removed without changing behavior;
- contract, replay, real-model E2E, restart, and observed visual gates pass;
- obsolete compatibility, aliases, dual writes, and fallback stores are deleted.
## Migration ledger

This ledger is deliberately deletion-oriented. A migrated consumer must stop
decoding or invoking the superseded primitive; adapters that preserve both
paths do not count as convergence.

The delegated Agent migration is specified by
`Doc/软件架构设计/模块/10-Goal任务与子代理/DELEGATED_AGENT_HOST_CONTRACT.md`. Core now has a versioned execution
request/event vocabulary, a frozen backend identity, and typed permission
continuations. Activation remains blocked until the CLI Host owns a resumable
worker registry: the current synchronous executor still depends on `Emitter`
behavior and cannot return to the committed runtime while waiting for a Core
permission decision. The extracted observer port is only the first adapter
boundary, not completion evidence.

| Boundary | Current contract | Remaining deletion |
|---|---|---|
| macOS conversation read | `ConversationProjectionV1` | Remove remaining diagnostic-only client reducers after their callers disappear. |
| macOS/CLI history restore | `ImportConversationTranscript` with explicit provenance | Remove any remaining `append_messages` product command callers. |
| iOS sync and backup restore | `ImportConversationTranscript` | Remove raw append after Web and Core-unavailable flows gain explicit use cases. |
| iOS Device Agent write | `BeginAssistantTurn` plus typed Host responses through Application | Move Web and Core-unavailable interactive paths onto an explicit remote/completion Run contract. |
| iOS conversation read | `ConversationProjectionV1` through `ConversationProjectionStore` | Render source, citation, attachment, task, Skill, compaction, and error Parts directly instead of legacy metadata adapters. |
| Remote terminal identity | `conversation_projection_committed_v1` owns exact `submission_id`; client commit ports accept only Envelope and timestamp | Delete remaining client active-turn identity caches used for transport correlation; identity may remain only as transient presentation state. |
| External agents | `DelegatedAgentRequestV1`, correlated delegated events, resumable CLI Host workers, and typed permission continuation are active; the legacy Application command and projection DTO are deleted | Finish Mac and remaining client migration, then remove residual adapter/Emitter execution entry points that are no longer reachable. |
| Mac live conversation | Core Part lifecycle now drives text, reasoning, tool, source, and citation projection through one monotonic reducer; trace is diagnostic-only | Separate reversible pre-acceptance submission overlays from authoritative conversation messages and delete remaining terminal reconciliation semantics. |
| Web live conversation | Trace product projection and host-owned terminal message pushes are disabled; Core Parts own the streaming shell and canonical refresh owns `currentMessages` | Delete unused trace-to-bubble recovery helpers and move the remaining template state into a typed presenter module. |
| iOS live conversation | Core Parts exclusively own text, reasoning, tools, and sources; Trace no longer repairs Tool Parts and the non-Core Web stream fallback is rejected | Remove now-dead trace presentation compatibility state and consume the shared projection fixture through the production presenter. |
| Android live conversation | The reducer accepts multiple model Message IDs per Turn and concatenates ordered Text/Reasoning Parts within the latest model message; the unused raw `append_messages` repository path is deleted | Separate pending submission overlay from canonical history and stop rebuilding whole Markdown presentation state for every Part snapshot. |
| iOS message mutation | Live Device and remote flows use Turn contracts; `ChatManager`, `ConversationStore`, and the iOS binding no longer expose raw `append_messages` | Observe real local/remote multi-turn runs and prove exactly one user message and one stable assistant shell per submission. |
| Conversation rewind | `RevertConversation` is a typed Application intent; Application derives visibility mutations from the canonical projection and atomically commits them with the Core checkpoint | Add conflict, replay, active-Run, and boundary contract cases; clients must never regain a raw projection input. |
| Skill and MCP ports | Skill and MCP commands are grouped behind independent nested Application protocols across CLI, macOS, iOS, and Android. Application now borrows separate `SqliteSkillRepository` and `SqliteMcpRepository` objects; the full `NolaStore` implements neither repository port, and compile-fail contracts prevent capability widening. | Replace remaining Store-shaped upsert DTOs with domain inputs, add shared protocol fixtures, then repeat the narrow repository-object cutover for data sources, memory, tasks, automation, sync, and diagnostics. |
| Data-source ports | Configuration/override/usage, document-index, context-checkpoint, and product-setting commands occupy separate nested Application protocols. Their Application branches now borrow four separate SQLite repository objects; the full `NolaStore` implements none of these ports, including the data-source read supertrait. | Replace Store records with domain requests, define capability-specific authorization, and connect scheduling/Skill orchestration without exposing index persistence primitives. |
| Memory port | Long-term-memory save, deduplication, query, batch load, and deletion now share one nested Application protocol across CLI, macOS, iOS, and Android; removed flat commands are rejected | Replace `StoredMemoryRecord` and encoded namespace/scope query fields with Core memory domain types, then prove autonomous cross-session write/recall through real-model sessions. |
| Model port | Profile selection/configuration and Host runtime reporting share one nested Application protocol across CLI, macOS, iOS, and Android. Application now borrows a narrow `SqliteModelRepository`; the full `NolaStore` no longer implements `ModelRepository`, and compile-fail contracts prevent widening the model repository into another capability. | Split profile intent from runtime lifecycle domain types, enforce explicit start/stop transitions, repeat the narrow repository-object cutover for the remaining capability families, and prove selected profile never implies process startup. |
| Plan and runtime-task ports | Conversation planning and durable background-worker lifecycle now use separate nested protocols; CLI, Mac plan storage, and real-service fixtures use those boundaries | Replace string states with typed transitions, move event construction into Application, and prove lease/retry/restart behavior without UI-owned state machines. |
| Remote Host authority | Terminal envelopes carry a source Host identity, but opening a remote Run does not yet bind an authenticated Host/lease and commit currently proves only a non-empty identity | Bind authority at Run creation and require the terminal envelope to match the stored Host session/lease before accepting its snapshot or projection. |
| Remote terminal event | The CLI terminal producer now emits the shared `conversation_projection_committed_v1` contract instead of mislabeling a complete projection as `core_session_projection_updated`, which Web dropped and Mac interpreted as snapshot-only | Add producer-to-mobile and producer-to-Mac fixture tests proving the exact event survives transport once and commits once. |
| Permission lifecycle | Core owns typed policy/user decisions, pending identity, suspension, conversation lifetime, and structured resource grants; Host envelopes carry resume handles | Delete remaining Boolean UI APIs and client caches, then prove restart/reconnect resume without Trace reconstruction. |
| Device-tool bridge | Request ID, nonce, Host/client/session identity, and tool ID correlate the typed request/result | Remove residual client DTO fields and any dead digest-keyed result caches; results must return only through the Host response port. |
| Visual acceptance | Schema-v2 real-window sequence plus per-checkpoint screenshot/recording evidence | Run the real lifecycle and require a human or image-capable reviewer to open every image and watch motion recordings. |

No row is complete until the old command is no longer decodable at the
Application boundary and a contract test proves that removal.

Architecture migrations must also use declaration-scoped edit anchors. A
patch matched only by a common statement such as `guard let store` is not
acceptable evidence that a change landed inside its intended port or use case.
Review and contract fixtures must bind changes to the owning function, type, or
protocol declaration.
