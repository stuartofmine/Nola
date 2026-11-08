> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Nola Architecture Redesign

Status: normative implementation blueprint
Updated: 2026-08-24
Parent contract: `ARCHITECTURE.md`

Model identity invariant: a stable model ID names exactly one canonical `ModelProfile`. Selection,
runtime source, and generation parameters are fields or projections of that entity. Clients must
not derive storage keys from display names or create parameter-only compatibility profiles.

iOS has no model catalog, last-selection, provider, or URL-history UserDefaults fallback. Web Host
credentials and identity fingerprints are Keychain secrets; model facts and URL history are
canonical Store records.

## 1. Decision

Nola is not five clients that happen to share an Agent library. It is one local-first product
runtime with several presentation and Host adapters.

The system has exactly four architectural roles:

```text
Presentation -> Application Service -> Domain/Core/Store
                                      -> Runtime Driver -> Agent Host
Sync Gateway <------------------------- committed operations
```

- **Application Service** is the only product writer on a device.
- **Nola Store** is the only durable source of product facts on that device.
- **Agent Core** is a deterministic reducer and planner. It performs no I/O.
- **Agent Host** executes effects. It never owns conversations, grants, tasks, or UI state.
- **Presentation** sends commands and renders projections. It never repairs runtime events.
- **Sync Gateway** replicates committed domain operations and blobs. It is not a second chat
  backend and cannot overwrite a device transcript.

Every feature must fit this model. A feature implemented only in a ViewModel, `UserDefaults`,
Room table, CLI table, JSON archive, Host cache, or HTTP handler is incomplete.

Codex, Claude Code, and OpenCode are Agent Host adapters. Their proprietary
stream events are translated into the standard Host response protocol and
reduced by Core. They do not receive a transcript persistence exception and do
not materialize product Messages or Parts directly.

### 1.1 Object and protocol ownership rule

Modularity is defined by ownership, not by directory layout. Each durable or
behavioral object has one semantic owner:

| Layer | Owns | Must not own |
| --- | --- | --- |
| Domain | identities, entities, invariants, lifecycle vocabulary | I/O, transport serialization, localization |
| Application | use-case objects, commands, queries, transactions, projections | SQL rows, UI state, Host implementation |
| Agent Core | deterministic transitions, plans, effect intents | time, network, filesystem, process, durable writes |
| Protocol | versioned Host/effect/sync envelopes and correlation rules | product policy, persistence records, UI labels |
| Store | `Stored*` records, indexes, revisions, inbox/outbox, blob references | public client commands, routing, effect execution |
| Host | effect execution and platform/provider adaptation | Message/Run construction, durable product state |
| Presentation | drafts, selection, expansion, animation, scroll intent | transcript folding, permission policy, Store DTOs |

Application ports are object protocols. A command targets a capability object
and expresses an intent, for example `automation.definition.upsert` or
`automation.run.claim`; it does not expose a table verb such as "write row" or
"set status column". `Stored*` types may occur only behind the Application
implementation boundary. Native bindings are projections of Application
contracts, never projections of Store schemas.

Identifiers are opaque, case-sensitive values. Presentation, transport, and
Host adapters preserve them byte-for-byte. They cannot lowercase submission
IDs, derive assistant IDs from display text, or replace an explicit identity
with a locally convenient one.

Object lifecycles are separate and typed: Session, Turn/Run, effect request,
permission request, automation definition, automation Run, and background task
cannot share an untyped status string. Transitions that cross a process use a
versioned Protocol request/response with stable request ID, sequence, and
idempotency key. Transport may correlate and deduplicate; it cannot interpret
or repair product state.

No compatibility alias is permitted before launch. Removing an old contract
means deleting its decoder case, writer, reader, migration fallback, and test
fixture. Unknown and removed operations fail closed before mutation.

## 2. Why the current architecture fails

The recurring product bugs are different symptoms of the same ownership errors.

| Symptom | Structural cause |
| --- | --- |
| New assistant text overwrites or duplicates old text | clients fold Provider/Host events and persist whole messages independently |
| Markdown appears only at completion or flickers | raw chunks and final rich text use different UI/persistence paths |
| Permission is requested repeatedly | grants exist in platform caches and transient Host loops rather than one ledger |
| Context usage differs between composer and inspector | each surface calculates or stores its own projection |
| Skill shown in UI is unavailable to the model | catalog, enabled state, files, and Host materialization have different owners |
| Memory, data sources, and schedules disappear or go stale | platform repositories are treated as product facts |
| Mac behavior differs from CLI/Web/mobile | every client contains a partial private Application layer |
| Cold restart loses or repeats work | process memory and checkpoints compete with committed outbox/inbox facts |
| Long-running tasks become slower after every event | an internal persistence effect is serialized inside its own checkpoint payload |
| Tests pass while the product remains broken | helper and fixture tests are labeled end-to-end without crossing product boundaries |
| Xcode changes rebuild the complete CLI | application, runtime service, Agent Host, and CLI packaging are one build unit |

### 2.1 Current code evidence

The following paths are incomplete boundaries that must be removed rather than supported:

- `cli/src/store/` owns sessions, permissions, memory, skills, schedules, tasks, goals, documents,
  background runs, and Web projections beside shared `nola-store`.
- `cli/src/web.rs` retains `builtin_skill_overrides` and uploaded Skill state outside the canonical
  Skill ledger.
- `NolaMac/Services/` contains separate archive, metadata, activity, automation, data-source,
  notification, health, Skill, bookmark, and model configuration stores.
- `NolaMac/UI/MacChatWorkspaceView.swift` remains a large composition root that knows product
  mutations and runtime details.
- `Nola/Services/ChatManager.swift`, `ConversationStore.swift`, `ContextManager.swift`, and the iOS
  Host executor jointly own conversation and execution facts.
- Android Room repositories, `DeviceAgentCoordinator`, `AndroidAgentTurnService`, permission broker,
  workers, and ViewModels form another private Application layer.
- `core/nola-mobile-bindings` contains model download, filesystem persistence, time-based
  intelligence policy, and ABI code. A binding crate must not own product behavior.
- root `nola_core` still reads real time and contains platform-oriented policy/dispatch code. It is
  a temporary facade, not yet a pure Agent Core.
- the Mac Xcode phase always builds and embeds both `nola-agent-host` and the full `nola` CLI,
  coupling unrelated build and deployment units.

## 3. Non-negotiable invariants

1. A device has one canonical Store and one logical Application writer.
2. Full transcripts are append-only Message/Part facts. Compaction never rewrites them.
3. A user submission atomically creates the User Message, queued Assistant Message, Run,
   checkpoint, and initial outbox effects.
4. Normal execution and crash recovery both consume the same committed outbox.
5. Every Host event is idempotent by stable event/effect/request ID and monotonic sequence.
6. Provider final text verifies or repairs a gap; it never replaces an already committed stream.
7. UI state can include drafts, selection, expansion, hover, animation, and scroll intent only.
8. The model chooses tools and Skills from typed schemas. Clients never route by text keywords.
9. Core receives time, IDs, randomness, capabilities, authorization, and token usage explicitly.
10. OS authorization, Nola grants, and per-Run decisions are separate typed concepts.
11. Credentials remain in platform secure storage. Store contains opaque credential references.
12. Background work, interactive work, and remote work share one Run/Effect/Task model.
13. Localization occurs only in presentation. Protocols and Store persist stable identifiers.
14. Adding a canonical writer is incomplete until every competing reader and writer is deleted.
15. Public Application ports contain no Store-owned `Stored*` records or mutation inputs.
16. Every cross-process effect has one request identity and one typed response/continuation path.
17. Clients preserve prompts, identities, ordering, and Core Parts; they do not normalize or repair them.
18. Session-, Run-, request-, and automation-scoped state use separate typed lifecycles.
19. Persistence is never a Host effect. Core emits a payload-free durability intent; Application
    creates and commits the checkpoint inside the same transaction as events and outbox changes.

## 4. Target packages

```text
core/
  nola-domain/            typed IDs, entities, commands, events, revisions, errors
  nola-protocol/          versioned Host/service/sync wire contracts
  nola-agent-core/        pure reducer, planners, budgets, permission decisions
  nola-store/             current SQLite ledger, projections, blob references
  nola-application/       commands, queries, transactions, subscriptions
  nola-runtime-driver/    outbox claim, inbox acceptance, retry, lease, recovery
  nola-sync/              optional encrypted operation/blob replication
  nola-bindings/          generated thin C/JNI/Swift/Kotlin ABI
  nola-testkit/           deterministic clocks, IDs, failure injection, evidence tools

hosts/
  common/                 Provider/tool/MCP/executor adapter traits
  apple/                  TCC, Keychain, HealthKit, Chrome, MLX, launchd
  android/                SAF, Keystore, Health Connect, WorkManager
  desktop/                filesystem, process, shell, git, browser transport
  remote/                 authenticated Host transport

clients/
  macos/ ios/ android/ cli/ web/
```

During implementation, root `nola_core` may re-export new packages, but no new behavior may be added
to modules scheduled for deletion. `nola-mobile-bindings` must be split: ABI remains in bindings, model download
moves to a Host adapter, and persistence/intelligence moves to Application services.

### 4.1 Dependency rules

```text
Domain          -> serde-compatible value dependencies only
Protocol        -> Domain
Agent Core      -> Domain + Protocol
Store           -> Domain
Application     -> Domain + Agent Core + Store + Protocol
Runtime Driver  -> Application port + Protocol + Host adapter trait
Host adapters   -> Protocol + platform/provider SDKs
Bindings        -> Application command/query facade only
Presentation    -> generated Application client
```

Forbidden dependencies are enforced in CI, not by convention:

- Core cannot import filesystem, HTTP, SQLite, process, platform, real-clock, or random APIs.
- Store cannot call Core or a Host.
- Host cannot import Store repositories or construct Message/Run domain facts.
- clients cannot execute SQL or invoke Core reducers directly.
- bindings cannot download models, choose paths, or implement business policy.

## 5. Runtime and process topology

### 5.1 Desktop

Desktop uses a local **Nola Application Service** as the single writer:

```text
Mac UI -----------\
CLI TUI -----------+-> local versioned service API -> Application + Store + Runtime Driver
CLI Web UI --------/                                      |
                                                           +-> nola-agent-host
                                                           +-> remote Agent Host
```

- Mac does not invoke CLI commands or parse CLI stdout.
- CLI and Web are presentation adapters, not databases or alternate runtimes.
- `nola-agent-host` is the sole path for model/tool/Skill/MCP/platform effects, whether the Host is
  local or remote.
- The Application Service owns the database writer lock. A foreground app and background daemon
  cannot independently open write coordinators.
- If the service is absent, a client may launch it and reconnect; it must not fall back to a private
  Store writer.

### 5.2 Mobile

iOS and Android embed the same Application service API in process. OS background workers acquire the
same device writer lease and submit commands; they do not run a second state machine. HealthKit,
Health Connect, SAF, notifications, and local inference are Host adapters.

### 5.3 Backend

The backend owns account identity, subscription, explicit encrypted sync, push delivery, optional
remote scheduling, and remote model relay. It does not own the authoritative local transcript.
Cloudflare Worker is the production contract. FastAPI must implement the same generated API or be
removed; it cannot evolve as a second backend product.

## 6. Bounded contexts and ownership

| Domain | Aggregate roots | Durable facts | Host responsibility |
| --- | --- | --- | --- |
| Identity & Settings | Account, Device, ProductSettings, ConversationSettings | revisions and credential refs | login transport and secure credential vault |
| Conversation | Conversation, Message, Part, Attachment, Artifact | append-only transcript and blob refs | attachment bytes and artifact creation |
| Execution | Submission, Turn, Run, Effect, HostEvent, Lease | run ledger, inbox, outbox, checkpoint | execute effects and report ordered events |
| Capability | Tool, Skill, SkillVersion, MCPServer, Binding | versioned catalog and activation | enumerate/load/execute capabilities |
| Authorization | PermissionRequest, Grant, OSAuthorizationSnapshot | scoped grant ledger and observed OS state | query/request OS authorization |
| Knowledge | ContextSnapshot, MemoryRecord, DataSource, SourceSnapshot, Index | immutable revisions and provenance | source read and index computation |
| Automation | Automation, Schedule, TaskRun, TaskEvent | definitions, watermarks, task graph | wakeup, heartbeat, notification |
| Model Runtime | ModelProfile, ExecutorProfile, HostProfile, RuntimeInstance | versioned config and lifecycle projection | API/local/external executor start and inference |
| Diagnostics & Sync | DiagnosticSpan, SyncOperation, Tombstone | redacted diagnostics and sync outbox | transport and log export |

Domains communicate through committed commands/events, never by editing another domain's tables.
For example, an Automation trigger submits `StartRun(origin: automation)`; a Skill contributes typed
capabilities; a Memory worker consumes committed Message events.

## 7. Canonical transaction protocols

### 7.1 Submit a Turn

`SubmitTurn(command_id, conversation_id, expected_revision, content, attachments)` commits:

1. deduplicated command receipt;
2. stable User Message and attachment Parts;
3. stable queued Assistant Message and open status Part;
4. Turn and Run with a captured ContextSnapshot reference;
5. Core transition, checkpoint, events, and outbox effects;
6. new conversation/projection revision.

Only after commit may Runtime Driver dispatch. The UI therefore shows the Assistant bubble and a
localized processing state immediately, before first token.

### 7.2 Accept a Host event

`AcceptHostEvent(event_id, effect_id, request_id, sequence, payload)` runs one transaction:

1. inbox dedupe and sequence validation;
2. Core reduction using the explicit environment snapshot;
3. append/update only the addressed stable Part;
4. write Run/Task projections, checkpoint, and successor effects;
5. acknowledge the triggering event/effect;
6. publish one projection revision.

Unknown, duplicate, delayed, and out-of-order events have explicit recoverable outcomes. A Host loop
cannot recursively persist the next tool call outside this transaction.

### 7.2.1 Durability boundary

Core transitions contain three distinct outputs:

1. public domain events;
2. true external Host effects;
3. an optional `AgentDurabilityIntent` carrying only a `CheckpointReason`.

The intent contains no snapshot, serialized state, request identity, or Host routing metadata.
Application decides the commit boundary, calls `core.checkpoint(reason)`, and atomically stores that
snapshot with the transition's events, inbox acknowledgement, and successor outbox effects. A
streaming byte budget may request an additional manual checkpoint, but it uses the same Application
transaction and never enters the Host catalog or pending-effect journal.

The in-process Application runtime may retain one hot, non-terminal Core per conversation. Deferred
stream events are moved between batches rather than cloned on every delta. A terminal transition,
checkpoint revision mismatch, or replacement Run evicts the hot Core immediately. Allocator pressure
relief is not a correctness or memory-bounding mechanism.

A reconnecting Host loads executable work through the Application runtime, not directly from a hot
cache or an independent transcript. The durable outbox decides which effects may execute, but it does
not duplicate stream progress. Application composes each committed delivery identity with the replay
cursor from the canonical Core checkpoint. If the Application process survived the Host restart, the
matching hot Core then overlays only a newer in-memory cursor for those same committed effects. If
Application also restarted, checkpoint and outbox fall back together to the last atomic commit. This
makes Host restarts lossless without writing every stream delta to SQLite, maintaining two cursor
authorities, or exposing an uncommitted side effect.

### 7.3 Streaming and complete messages

An Assistant Message is an ordered collection of stable Parts:

```text
status | reasoning | tool-call | tool-result | text | artifact | permission
```

- Each stream has a stable `message_id`, `part_id`, `stream_id`, and sequence.
- Text deltas append idempotently to one Part; sparse gaps are retained until repaired.
- Reasoning and tool output never leak into the final answer body.
- Final text is reconstructed from text Parts, not a Provider completion cache.
- Closing a Part records final digest/usage/status but does not replace content.
- Tool full output and large text use content-addressed blobs; projections contain safe previews.

This is the only canonical message. Host traces, provider transcripts, and UI animation buffers are
diagnostic or presentation evidence, not alternate answers.

Session queries return these ordered Parts under their owning Message. A top-level
`ConversationProjection` remains available for incremental UI reconciliation, but it is not an
alternate transcript. Provider-specific `tool_calls`, tool-result messages, and reasoning replay are
derived model-context projections only and must never be exposed as the durable product contract.

### 7.4 Incremental Markdown presentation

Application publishes parsed block projections with stable `block_id` and a mutable tail marker.
Presentation keeps mounted views stable:

- an opening fence creates a code block immediately;
- only the mutable tail is reparsed as text arrives;
- committed blocks never animate again;
- semantic phrase batches receive a subtle opacity/blur transition without layout translation;
- completion changes metadata only and never swaps raw text for a second rendered tree;
- scroll following is derived from user intent and keeps the latest content above the opaque composer.

## 8. Capability, Skill, MCP, and authorization

### 8.1 One capability snapshot

The model and every gallery/inspector read the same `CapabilitySnapshot`:

```text
configured definition
+ enabled binding and version
+ live Host support
+ OS authorization snapshot
+ Nola grant/risk decision
+ health/freshness
= model-visible typed capability
```

No client rewrites a prompt or selects a tool from language-specific words. Query rewriting, site
restriction, and search strategy happen only after the model explicitly calls the Web Search tool.

### 8.2 Skill

- Definition identity and immutable versions live in Store.
- Bundled and user Skills use the same lifecycle; bundled content may be reset/disabled but not
  physically deleted.
- Enabled state, deletion tombstone, activation scope, required tools, content digest, and provenance
  are product facts.
- Host materializes exactly the version named by an effect and verifies its digest.
- Turn activation clears at the next Turn unless a user explicitly pins the Skill to the conversation.

### 8.3 MCP

- Store contains server descriptor, transport type, enabled binding, capability snapshot, health, and
  opaque secret reference.
- Host owns the connection, process, credentials, and protocol transport.
- UI gallery and model catalog derive from the same Store projection.
- Tool identity includes server/version/schema digest; reconnect cannot silently change semantics.

### 8.4 Authorization

Authorization has three independent layers:

```text
OS authorization       can this process access the platform resource?
Nola grant             did the user permit this subject/operation/resource/scope?
Run decision           does this concrete typed call require allow/ask/deny now?
```

The recommended mode allows internal writes, reversible changes, and ordinary reads. Destructive,
irreversible, external publication, privilege escalation, or newly broadened resource access asks.
Matching uses typed operation/resource normalization, never rendered shell prefixes. Conversation
grants survive restart and are revoked by revision. An OS permission request suspends the same effect;
it does not fail and create a replacement tool call.

## 9. Knowledge: context, memory, and data sources

### 9.1 Context

- Full transcript remains unchanged.
- ContextSnapshot records source ranges, provenance, tokenizer/model profile, budget, and policy.
- Compaction is code-planned and LLM-summarized, then Core validates coverage and references.
- A summary is a new snapshot, never an edited transcript.
- Composer footer and inspector query the same persisted `ContextUsageProjection`.
- Conversation switches load the selected conversation's latest committed usage immediately.

### 9.2 Long-term memory

- Extraction is an internal model effect and does not request filesystem permission.
- Memory candidates, accepted records, merges, supersession, deletion, source citations, and recall
  receipts live in Store.
- Cross-conversation recall is an explicit conversation/product setting.
- Recalled memory is visible in the ContextSnapshot provenance and can be inspected or removed.
- A worker consumes committed messages and uses idempotent jobs; it never reads a UI message array.

### 9.3 Data sources

Adding a source is a validation transaction, not merely saving a URL or permission toggle:

1. save a draft connector and opaque credential reference;
2. Host performs a real read;
3. Application validates and stores read recipe, detected schema, redacted sample, latency, freshness,
   authorization, and evidence;
4. only a successful revision becomes selectable/default-enabled;
5. every use records snapshot revision and provenance.

Product defaults and conversation overrides are separate typed bindings. Platform health sources live
inside the data center, not as page-header special cases.

## 10. Automation, long tasks, and model runtime

### 10.1 Automation and TaskRun

Automation is a trigger definition. TaskRun is one execution fact. A data source read, Skill workflow,
tool effects, artifact, and notification form one inspectable task graph.

TaskRun supports lease, heartbeat, progress, checkpoint, cancel, resume, retry, artifact, and terminal
status. Work is promoted to background only when its expected duration exceeds the interactive
lifecycle or the user requests it. The UI shows concise current progress by default and the complete
ordered process/parameters on expansion.

### 10.2 Model and executor are orthogonal

Store represents four independent selections:

- model profile: model ID, context window, sampling/tool/reasoning parameters;
- executor profile: Nola native, Codex, Claude Code, or another compatible executor;
- Host profile: this device or a connected Host;
- permission profile: recommended, safe, or unrestricted/custom.

Selecting a local model does not start it. Runtime lifecycle is explicit:
`stopped -> starting(progress) -> ready -> stopping/failed`. Startup is reused across conversations,
and the model's real context window/estimated memory requirement is visible before launch.

## 11. Presentation architecture

All clients expose the same feature boundaries without sharing platform UI widgets:

```text
AppShell
  ConversationList
  Chat
    MessageTimeline
    RunActivity
    Composer
    ConversationInspector
  DataSources
  Capabilities (Tools / Skills / MCP)
  Automations and Tasks
  Models and Hosts
  Memory
  Settings and Diagnostics
```

Local UI state is limited to drafts, focus, navigation selection, expansion, hover, scroll intent, and
animation progress. Every business value is a projection. Stable IDs prevent hover, streaming,
thinking expansion, or localization changes from remounting a message bubble.

Tools and reasoning are visible ordered Parts, not a hidden status bar. The collapsed state summarizes
current activity; expansion shows tool name, typed parameters, affected files/resources, progress,
result preview, artifact links, and permission decisions. Internal protocol labels such as
`agent_running_turns`, `tool_output_read`, and `request_agent_decision` are never user-facing.

Localized resources map stable presentation keys at render time. Stored content, protocol status, tool
IDs, Skill IDs, and diagnostic events are not localized strings. Placeholder parity is enforced across
all supported locales and secondary pages.

## 12. Store, blobs, diagnostics, and sync

Store has three storage classes:

1. **Ledger**: commands, domain events, Host inbox, effect outbox, sync operations.
2. **Projection**: rebuildable conversation, message, run, capability, source, memory, task, and model
   read models.
3. **Blob**: content-addressed attachments, large text, full tool output, indices, and artifacts.

Every mutation carries `operation_id`, aggregate ID, and expected revision. Duplicate operations return
the original receipt; revision conflicts are explicit. Generic SQL/JSON execute is not a product API.

Diagnostics correlate command, Turn, Run, effect, Host event, tool call, model request, and projection
revision. Logs are structured, redacted, bounded, and exportable from the product. Debug mode changes
retention/detail, not product behavior or footer semantics.

Sync copies encrypted committed operations and referenced blobs. It never copies Core checkpoints,
leases, Provider continuation, UI state, or credentials. Conflict handling uses domain revisions and
tombstones; a server transcript cannot replace local message history.

## 13. Build and deployment units

Build products are separated:

| Product | Contains | Must not build implicitly |
| --- | --- | --- |
| Core libraries | Domain/Core/Store/Application/Protocol/Bindings | CLI UI or Agent Host executables |
| `nola-service` | Application writer + Store + Runtime Driver | Mac UI or CLI TUI |
| `nola-agent-host` | effect adapters | product Store/repositories |
| `nola` | CLI presentation/service client | Mac application resources |
| Nola Mac | SwiftUI client + packaged service/Host artifacts | full CLI unless explicitly shipped |
| Mobile apps | generated binding + platform Host adapter | desktop Host/CLI |

Xcode, Gradle, and CI phases declare real file inputs/outputs and use shared prebuilt artifacts where
appropriate. They do not use unconditional `alwaysOutOfDate` release builds for unrelated binaries.

## 14. Implementation strategy

Nola has not shipped. There is no supported historical product state. Each vertical slice has one
current owner and one current contract. Old stores, files, request shapes, enum aliases, importers,
read-only recovery paths, dual writes, and fallback writers are forbidden and must be deleted.

### Phase 0: freeze boundaries

- Add dependency and direct-persistence architecture checks.
- Freeze new client tables, JSON stores, `UserDefaults` business facts, and private Host loops.
- Split bindings from model download and generic persistence.
- Freeze protocol fixtures and stable IDs.

### Phase 1: Conversation and Execution service

- Complete Application commands/queries/subscriptions for submit, Host event, cancel, retry, branch,
  rename, archive, and recovery.
- Make Store outbox/inbox the only Runtime Driver input.
- Introduce desktop `nola-service` and writer lease.
- Prove crash recovery at submit, delta, permission, tool progress, and terminal boundaries.

Deletion gate: no client whole-transcript writer or private execution queue.

### Phase 2: cut over Mac, then CLI/Web

- Mac consumes only Application projections and uses `nola-agent-host` for all effects.
- Remove Mac CLI invocation, archive writer, fake checkpoint, and business state from the workspace
  view.
- Convert CLI TUI and Web HTTP/SSE into clients of the same service.
- Remove duplicate CLI session/Web message writers.

Deletion gate: cold restart reconstructs every visible message/Part from canonical Store only.

### Phase 3: Capability and Authorization

- Finish Tool/Skill/MCP definitions, versions, bindings, Host availability, and grants in Store.
- Replace CLI/Web/Mac/mobile state with canonical records and delete old tables/caches.
- Generate one catalog projection for both model and UI.

Deletion gate: one session grant survives restart; one enabled Skill/MCP version is identical in UI,
model context, and Host execution.

### Phase 4: Knowledge

- Move ContextSnapshot, compaction, memory, data-source validation/snapshots, and indices into
  Domain/Application/Store.
- Delete platform memory profiles, summary files, data-source registries, and independent context
  counters.

Deletion gate: transcript bytes remain unchanged by compaction; recall and source provenance survive
restart and match every UI surface.

### Phase 5: Automation and Model Runtime

- Unify schedules, TaskRun, background workers, artifacts, and notifications.
- Unify model/executor/Host/permission profiles and runtime lifecycle.
- Separate Mac build products and background service installation.

Deletion gate: a data-source -> Skill -> scheduled long task survives process death and resumes once.

### Phase 6: mobile, sync, and backend convergence

- Generate Swift/Kotlin bindings and remove iOS/Android business repositories.
- Make background workers submit Application commands.
- Implement operation/blob sync and align Worker/FastAPI to one generated contract.

Deletion gate: cross-device merge preserves stable IDs, ordering, tombstones, and local-only runtime
state.

## 15. Current implementation status

Completed foundations:

- `nola-domain`, `nola-store`, `nola-host-protocol`, `nola-application`, and
  `nola-runtime-driver` dependency skeletons exist.
- canonical Store persists conversations, Messages, Parts, Runs, events, inbox/outbox, checkpoints,
  context snapshots, remote mirrors, permission grants, and versioned Skills.
- native local and remote Mac paths replicate stable Parts rather than writing a final Assistant bubble.
- Store/Application permission and Skill ledgers have real SQLite reopen coverage.
- `nola-protocol`, `nola-service`, and `nola-service-client` provide the versioned desktop service
  protocol, real single-writer process, and shared Rust client/lifecycle boundary.
- Nola Mac canonical conversation commands now cross the packaged `nola-service`; Mac no longer
  opens the Core Store ABI as its conversation writer and no longer embeds the complete CLI.
- Real-process coverage now includes service launch, socket request/response, durable write/read,
  process restart, stable Message/Part identity, remote Part idempotency, and Mac repository reopen.
- Persistent CLI Session transactions and the CLI Permission, Skill, MCP, Memory, DataSource,
  Automation, and Runtime Task adapters now submit typed commands through `nola-service`; explicit
  `--no-save` sessions are the only supported in-process ephemeral Store.
- The CLI Web canonical conversation/settings reader and writer now use the same service client;
  canonical Markdown message projection and settings cold-reopen tests cross the real service.
- Web conversation profile changes are one typed `UpdateConversationProfile` Application command.
  Title and settings advance one revision and enqueue one sync mutation in the same immediate
  transaction; the Web adapter no longer composes independent rename and settings commits.
- `BeginAssistantTurn` now carries the raw Core configuration and submission. Application restores
  or initializes Core, reduces the submission, and atomically commits the User/Assistant messages,
  Run, checkpoint and external outbox before Runtime sees any effect. Clients can no longer submit
  a pre-reduced checkpoint/request bundle.
- Active-Run internal submissions use `ReduceAndCommitActiveSubmission`. Application restores the
  committed checkpoint and atomically commits events, Part mutations, terminal status, checkpoint,
  and successor outbox under an idempotent transition identity. Terminal submissions close all
  prior pending effects and do not persist a best-effort cancellation effect for a closed Run.
- Runtime Task Application commands cover save/load/list, claim and heartbeat leases, retry,
  completion, expired-task recovery, append-only events, and session event cursors. Real-process
  tests prove ownership rejection, completion, event detail, and cold reopen.
- The shared Rust service client checks and recovers the service before every new command, but never
  retries after a command has been sent. Legal JSON `null` command results remain distinguishable
  from failed response envelopes.
- Mac runtime packaging builds `nola-service` and `nola-agent-host` in one Cargo invocation and
  reuses artifacts only when a content digest proves all Rust inputs unchanged.

Incomplete and blocking:

- root Core is not pure and Application still exposes temporary package facades.
- Runtime Driver's obsolete transactional port, direct committed-response injection APIs, reducer
  loop, staging queue, and deferred in-memory execution APIs are deleted. Production effect
  execution has only the committed Application port.
- The CLI Core event presenter no longer reconstructs provider-shaped messages or owns a mutable
  shadow transcript. It projects canonical Core Part events for display only; persisted
  Message/Part history remains Store-owned.
- CLI `SessionStore` no longer caches a mutable `SavedSession` transcript. Every durable transcript
  read crosses the Application Service, and monotonic append validates against the current
  canonical message count before committing.
- macOS no longer stores conversation title/pin metadata in UserDefaults. Those values are part of
  the canonical Conversation/settings projection and are restored through Application Service.
- macOS assistant defaults no longer use `@AppStorage`; user guidance, retrieval, memory, and
  context-window defaults are versioned ProductSettings read and written through Application
  Service. Removed `begin_turn`/`commit_transition` Swift protocol facades are not accepted.
- iOS and Android memory profiles no longer maintain UserDefaults/DataStore business-fact shadows.
  Structured memory, Skills and MCP servers use their canonical entities; uploaded context,
  response style, model endpoint/model choice, resource principal, and assistant defaults use
  versioned ProductSettings. API secrets remain only in the platform credential vault.
- iOS no longer initializes the unused template CoreData `Item` database. Android no longer ships
  a second embedded prompt body as a resource fallback; prompt assets are required current inputs.
- Per-model generation parameters on iOS, macOS, and Android are canonical
  `ModelProfile.parameters_json` facts. The iOS UserDefaults writer and Android
  `model_parameters` DataStore are deleted, mobile Store adapters expose the current ModelProfile
  commands, and Swift parameter decoding requires the complete current schema.
- Application no longer exposes Store primitives for beginning arbitrary Runs, committing raw
  transitions, injecting pre-reduced Host events, writing/deleting checkpoints, or directly
  acknowledging effect-ledger rows. Those operations are private implementation details behind
  semantic assistant-turn, external-execution, and committed-response transactions.
- Application still has partial Conversation/Execution and product-domain use cases.
- Codex, Claude Code, and OpenCode still use the temporary
  `CommitExternalExecutionProjection` Application command. That path lets an
  adapter author Store-shaped Parts and terminate a Run without a correlated
  Core ModelStream response. It remains a blocking duplicate execution owner
  and must be replaced, not generalized.
- desktop `nola-service` owns the Mac canonical conversation path and the current CLI/Web slices.
  Mac non-conversation repositories still retain product writers.
- Model Runtime, Diagnostics, Sync, background-process metadata, and several Web projections are not
  canonical. MCP, Memory, DataSource, Automation, and Runtime Task have canonical CLI paths but are
  not yet converged across every client.
- Web still serializes a Web-shaped conversation document inside canonical settings. This temporary
  projection envelope must be replaced by shared typed conversation preference objects. The generic
  JSON `upsert_web_conversation` entry and its redundant post-message revision commits are deleted;
  only explicit create/profile and immutable message-projection operations remain.
- Mac/iOS/Android/CLI/Web have not deleted their old writers.
- bindings are handwritten/fat and platform DTO drift remains possible.
- CLI/Web memory-worker lifecycle submissions are committed as idle Application transactions.
  Manual compaction is a durable Application-owned Run with a committed Outbox and the same
  response transaction path as ordinary turns.

## 16. Verification gates

Tests are evidence-based and name the boundary they exercise.

| Layer | Required evidence |
| --- | --- |
| Reducer | deterministic clock/IDs, duplicate/out-of-order/cancel/property cases |
| Transaction | real SQLite, failure injection, exact ledger/projection/outbox invariants |
| Protocol | cross-language fixtures, version negotiation, cursor replay, unknown event recovery |
| Host adapter | real file/Chrome/MCP/model/OS authorization behavior with redaction |
| Product runtime | real service process, Store reopen, Host transport, crash and resume |
| Product UI | shipped UI, accessibility tree, screenshots, stable identity/timeline evidence |
| Real model | configured external model, natural multilingual follow-ups, autonomous schema-based tools |

Every completed slice must prove:

1. stable Submission/Turn/Run/Message/Part/tool/effect IDs;
2. exactly one durable terminal answer after the latest User Message;
3. replay does not duplicate or overwrite content;
4. older messages remain byte-for-byte unchanged;
5. permission wait and process restart resume the same effect;
6. UI, inspector, background worker, and diagnostics read one projection;
7. old tables/files receive zero writes;
8. tests use the production entry and retain raw evidence.

Real-model E2E validates semantic tool choice but never replaces deterministic transaction and recovery
tests. No test may be made to pass by keyword routing, prompt rewriting in a client, fixture-only tool
results, cached native test output, or a helper that bypasses the product entry.

## 17. Immediate implementation order

Do not expand secondary UI features until the ownership path is closed. The next work order is:

1. split concrete protocol payloads and thin bindings from root Core;
2. make Runtime Driver consume only committed Application outbox/inbox transactions;
3. complete Conversation/Execution Application service and desktop single-writer process;
4. cut Mac Chat to that service and delete its old writers/CLI invocation;
5. delete every remaining CLI/Web duplicate reader and obsolete persistence type;
6. finish Capability/Authorization, then Knowledge, Automation, and Model Runtime vertical slices;
7. replace the remaining iOS/Android domain stores and finish operation-sync convergence for every aggregate;
8. run the complete real E2E matrix and remove every temporary facade.

The measure of progress is deleted duplicate ownership, not the number of adapters added.

## 2026-08-20 desktop persistence convergence

- macOS title, pin, delete, knowledge and context settings have one durable writer: `nola-service` through the Application Service. A production service failure is surfaced as an error and never falls back to another store. Explicit archive injection is test-only ephemeral persistence.
- CLI background shell processes are `RuntimeTask(kind = "shell_process")` records. Stop is a durable state transition rather than history deletion; status survives process-handle loss and service cold restart.

## Immutable conversation projection contract

Web and desktop clients must not implement edit, delete, revert or compaction by updating or replacing transcript rows.

- `Message` and `Part` records are immutable identity-bearing ledger facts. Streaming may only advance an open Part through the validated Part transition protocol.
- A normal user submission is an atomic `BeginTurn`: completed User Message, queued Assistant Message, Run and checkpoint/outbox ownership are committed together.
- Edit creates a projection operation and a replacement branch; it never changes the source Message.
- Delete is a projection tombstone. The source Message remains available to audit/replay.
- Revert advances the selected projection head to an ancestor and records lineage; it does not truncate the ledger.
- Branch creates a new conversation projection with explicit parent conversation/message lineage.
- Compaction appends a `ContextSnapshot`; it never flips per-message context flags or rewrites visible history.
- Product UI reads the selected projection. Diagnostics and recovery may read the complete immutable ledger.
- Projection changes and any replacement turn must be committed by one Application Service transaction with an idempotency key and expected revision.

## Runtime identity and truthful persistence

Production failures exposed three identity boundaries that are now architectural invariants:

- A runtime session identifier provides in-memory turn continuity only. It does not authorize a
  Store read and must not be passed to durable footer, checkpoint, permission, task, or notification
  queries when persistence is disabled.
- UI state such as an empty Assistant placeholder, streamed text, or edited metadata is not durable
  evidence. Presentation may report a conversation as saved only after the Application Service
  acknowledges the corresponding command. Failed metadata commits remain pending for retry.
- Host capability catalogs are owner-filtered projections over the shared Store. A CLI/Agent Host
  may execute `http` and `localFile` connectors without decoding or mutating native `platform` or
  `web` records. Native clients resolve their own platform connectors and submit only validated
  context components to the runtime.

The shipped Mac bundle must build `nola-agent-host` and `nola-service` from the same Rust input
digest. Product E2E must seed foreign data-source kinds, run the bundled Host, complete a turn,
reopen the canonical Store, and prove that the foreign records were neither decoded as Host-owned
connectors nor modified.

## Idle Core transitions and Goal ownership

Goal, task guidance, conversation revert, and explicit session close are Core submissions outside a
normal model Turn. They must not dispatch only against a process-local Host. The Application Service
loads or bootstraps the canonical checkpoint, reduces the submission, rejects unexpected external
Host effects, and advances checkpoint revision in one transaction. Only after that acknowledgment
may a loaded Host restore the committed checkpoint and project events to UI.

- `GoalProjection` is the sole durable Goal state; UI snapshots are DTOs derived from the projection.
- `/goal resume` commits `Resume` before claiming that the workflow was restored. The following
  execution Turn may claim a lease, but it does not retroactively make the resume durable.
- A new conversation bootstraps its first canonical checkpoint through an idle submission.
- Required E2E starts and pauses a Goal through a real `nola-service` process, kills that process,
  resumes through a second process against the same SQLite database, and verifies the unified
  session projection and monotonic checkpoint revision.

## Model configuration and runtime boundary

Model selection and generation parameters are durable product facts; a running inference process is
a Host-owned effect. A model slice must therefore keep two typed records instead of persisting a
single ambiguous `running` flag:

- `ModelProfile`: provider/runtime kind, immutable model/artifact identity, validated generation and
  context parameters, enabled/selected state, revision, and provenance.
- `ModelRuntime`: host identity, profile identity, starting/ready/failed/stopped phase, endpoint
  metadata, process identity, timestamps, and last error. On cold start, a recorded ready process is
  untrusted until the owning Host probes it.

Download bytes and external runtime processes remain Host effects. Their committed catalog and
configuration changes flow through Application Service commands. CLI, Web, and native clients read
the same profile projection and must not maintain competing `settings.json` or UserDefaults writers.

## Conversation plan ownership

The current execution plan is a durable conversation fact, not a TUI cache. Message history can be
compacted and a client process can terminate while a multi-hour task remains active.

- `StoredConversationPlan` owns the validated plan-item JSON, monotonic revision, conversation ID,
  and update time in the current Nola Store schema.
- Plan tools save and load it only through `SaveConversationPlan` and `GetConversationPlan`
  Application commands. CLI, Web, and native clients render the same projection.
- Required E2E saves two revisions through one real `nola-service` process, kills it, starts a second
  process on the same database, and verifies the final plan and revision.

## Reusable document source ownership

A reusable document selected in Data Center or uploaded through Web is not a message attachment and
is not an execution artifact. It remains available across conversations, can be indexed before a
Turn exists, and needs independent provenance and retention. Nola Store therefore owns a
typed `DocumentSource` aggregate.

- `DocumentSource` stores scope, kind, source URI, MIME type, digest, byte count, title, extracted
  text, metadata, and canonical millisecond timestamps.
- Search returns `DocumentSourceSummary` with a bounded preview instead of transferring full
  multi-megabyte content through Application Service IPC.
- Reconciliation replaces one `scope + kind` collection atomically. CLI and Web are adapters and
  never issue SQL against the canonical table.
- `Attachment` remains message-owned input and `Artifact` remains run-owned output. Neither may
  be used as a shortcut for reusable Data Center material.
- Required E2E verifies literal query escaping, a real service cold restart, and post-restart search.

## Permission request and grant boundary

A permission request is recoverable execution state; a permission grant is an authorization fact.
They must not share a table or lifecycle.

- `PermissionRequest` belongs to a durable task and records the concrete capability, input,
  scope rule, pending/resolved status, decision, optional custom response, and timestamps.
- `allow_once`, `custom`, and `deny` resolve only the request. `allow_session` resolves the
  request and separately commits a conversation-scoped `PermissionGrant`.
- Custom response keeps the task suspended until the user submits text; opening a client, switching
  UI surfaces, or restarting a Host must not synthesize a decision.
- CLI, Web, and Mac query the Application Service projection rather than maintaining pending-state
  mirrors.
- Required E2E writes a pending request through a real service, kills and restarts the process,
  resolves it with a custom response, restarts again, and verifies the exact decision and response.
## Durable Agent command ownership

Cross-process Agent submissions are canonical Core execution state. They are
enqueued, leased, replayed after lease expiry, and acknowledged through the
Application Service. CLI and native Hosts are adapters only.

## Trace journal ownership

User-visible tool, reasoning, permission, usage, and terminal state is persisted
as canonical Parts, projections, and `run_events` by the Application Service.
The Trace Journal is a bounded diagnostic and transport-audit log only: it may
be sampled or deleted and cannot reconstruct or mutate product UI. Protocol
replay uses inbox/outbox cursor and Part revision, not rendered Trace strings.
CLI/Web never maintain parallel trace or transcript tables.

## Platform state ownership

Workspace checkpoints and session-run leases are platform capability records
owned by the Application Service. Hosts still create Git references and inspect
process liveness, but never write checkpoint or lease rows directly.

## Product settings ownership

Cross-client configuration is a versioned `ProductSetting` fact owned by the
Application Service. Feature schemas live inside validated JSON values while
the Store owns the key, revision, and canonical update time. CLI/Web adapters
must not write native preference files or parallel settings rows for shared product behavior.

The current schema contains the durable `sync_outbox` and monotonic `sync_cursors` facts.
`ProductSetting` mutation and its replica operation are committed in one SQLite
transaction, including deletion tombstones. Restarting any Host cannot lose an
accepted local setting change: the Application Service exposes pending changes,
acknowledgement, and cursor commands through the same Rust ABI used by macOS,
iOS, Android, CLI, and Web.

It also contains `sync_entity_revisions`, a revision/tombstone ledger independent
from live entity rows. Deleting an entity therefore cannot erase its causal
revision; recreating a deleted setting continues at the next revision instead
of restarting at one. Remote ProductSetting pages are applied by Core together
with their cursor in one immediate SQLite transaction. Same-revision divergent
operations roll back the full page, while stale or already-applied operations
are skipped without creating a local echo outbox entry.

The backend is an append-only remote replica, not a second product authority.
`POST /v1/sync/replicas` accepts idempotent operations and rejects divergent
same/older revisions; `GET /v1/sync/replicas` returns cursor-ordered committed
changes scoped to the authenticated client. Cloudflare Worker/D1 and FastAPI/
SQLite implement the same contract. The Rust CLI/Web Host now has an authenticated
HTTP transport that maps the Core outbox to this API and hands downloaded pages
back to Core for atomic entity-plus-cursor application. A live Uvicorn process
test covers real JWT verification, TCP transport, and file-backed SQLite; a
separate Host test covers Bearer transport, acknowledgement, cursor persistence,
and no-echo remote application.

ProductSetting, Conversation, MCPServer, DataSource, Automation, ModelProfile,
MemoryRecord, and Skill are now supported replicated aggregates. Common Conversation transactions (`begin_turn`, transition commits,
rename, settings, and deletion) enqueue a complete, revisioned canonical
`ConversationProjection`, not a Host execution input. Remote application replaces
Conversation, Message, Part, and ConversationSettings in the same transaction as
the revision marker and cursor. It deliberately does not copy checkpoints, Runs,
or Host effects. Two-device cold-reopen tests cover full message/Part content,
rename, tombstone, cursor persistence, and absence of echo outbox writes; the
Rust HTTP Host test additionally covers a mixed ProductSetting push and
Conversation pull. MCP configuration upsert/delete now owns its replica operation
in the same transaction and has two-device configuration/tombstone coverage.

DataSource synchronization preserves the validated read recipe, schema contract,
capabilities, privacy metadata, conversation-override cleanup, and deletion tombstone.
Automation synchronization covers both individual upserts and catalog reconciliation;
definitions removed by reconciliation emit tombstones, while Run leases and execution
state remain local runtime facts. ModelProfile synchronization owns configuration
revision allocation inside Store, versions every profile displaced by a selection
change, and keeps ModelRuntime Host-local. Client-supplied model revisions are ignored.

Schema v24 adds internal causal revisions to MemoryRecord and Skill without exposing
revision allocation to platform DTOs. Deduplicated Memory identity lookup, merge,
revision allocation, projection replacement, and outbox append are one immediate
transaction. Skill replication transfers the current executable immutable-version
projection and its enabled/tombstone state; historical versions remain local audit
facts. Installing a new Skill version preserves the user's enabled state, and a
same-version no-op cannot manufacture another operation.

Two-device cold-reopen tests now cover every supported aggregate, including no-op
suppression, stable identity, full projection content, selection displacement,
reconciliation removal, tombstones, cursor persistence, and absence of remote-apply
echo writes. PermissionGrant and permission-request replication remain intentionally
unclosed pending a domain decision about which authorization scopes may leave a device.
The authenticated transports also still need to be attached to account/session and
foreground/background Host lifecycles before synchronization is a shipped automatic feature.

`assistant.memory.enabled` is a native cross-client setting in this contract.
Its value is versioned JSON (`schemaVersion` plus `enabled`), not a platform boolean.
macOS, iOS, and Android use the same typed command adapter;
Android's memory repository reads and writes the canonical fact. UI-only state
such as selected panes, disclosure state, and transient notification feedback
remains platform-local.

iOS system-data-source defaults use versioned keys such as
`data-source.system-policy.ios.health`. Reading or changing a policy uses only the
Application Service. Per-conversation source overrides remain Conversation
settings and do not overwrite these global defaults.

macOS bundled Skill activation is owned by canonical `Skill.enabled`. Hiding an immutable bundled Skill from the
gallery uses the versioned `skill.builtin.hidden` ProductSetting; restoring the
gallery clears that canonical hidden set and enables the bundled Skill records.

Native backup/export serializes canonical ProductSettings. Restore and application-data reset send
setting commands through the Application Service; platform preference values are not accepted as
product settings.

# Canonical State and Protocol Rules

Nola is not yet released. Product-owned storage and protocols therefore support only the current schema. When a schema or contract changes, update fixtures and recreate development data instead of adding migrations, compatibility readers, dual writes, aliases, or output-marker parsers.

## Ownership

- Rust Core owns conversation, message, part, turn, task, permission, capability activation, context usage, model profile, data-source, Skill, MCP, memory, and product-setting records.
- A client may keep transient presentation state for animation, selection, and in-progress input. It must not treat `UserDefaults`, preferences, JSON archives, environment variables, or rendered Trace text as a second durable product store.
- Keychain or the platform credential vault owns secrets and trusted-host credentials only. Stable product identity and model parameters remain canonical Core records.
- Agent Host is the protocol boundary for non-host clients. Clients preserve the user prompt and submit typed commands; they do not infer tool routing from words in the prompt.
- Typed Core Part events own assistant content. Trace events describe live progress. A client must not synthesize a replacement assistant answer when a run fails or ends without text.
- Capability activation is accepted only from the typed Host activation field. Tool output text cannot activate capabilities.

## Current client contracts

- iOS model selection, model parameters, URL history, and paired-host capability settings use the canonical SQLite store. Paired-host tokens and identity fingerprints use Keychain.
- macOS creates a canonical conversation before writing conversation metadata. Context usage, execution settings, messages, and projections must survive reopening through the canonical repository; the removed JSON conversation archive is not a recovery path.
- macOS remote API endpoints, model identities, selection, and generation parameters are canonical `ModelProfile` state. The removed `nola.mac.chat.baseURL` and `nola.mac.chat.model` preference keys are neither read nor migrated; only the API secret remains in Keychain.
- macOS local model lookup accepts only the exact canonical model ID. Conversation selection uses the current source-qualified envelope (`configured:<model-id>` or `local:<model-id>`); display names, repository basenames, raw IDs, whitespace-normalized values, and unknown prefixes are not aliases.
- macOS local-model selection and parameter writes are fail-closed Application Service operations. UI selection changes only after the canonical `ModelProfile` commit succeeds; installed and bundled models share this path, and write failures must be shown to the user.
- macOS connected-host URL, expected Host ID, and stable client ID are one versioned canonical ProductSetting. The access token remains in Keychain. Removed `nola.mac.connectedHost.url`, `.id`, and `.clientID` preference values are not read or migrated.
- macOS execution location is conversation state. There is no global `nola.mac.defaultExecution` preference or inert settings control. Backup/reset excludes removed model, Host, local-model selection, and data-context preference keys.
- Shared prompt resources are required build artifacts. Missing resources are packaging errors, not a reason to switch to embedded prompt copies.
- LLM configuration is loaded from the configured credential store or explicit product defaults. Process environment variables are not an implicit configuration source.

## Required regression coverage

- Cold close/reopen against a real temporary SQLite database.
- Poisoned removed stores or keys must not affect the loaded value.
- Natural multilingual and ambiguous prompts must reach the model unchanged.
- Tool activation, permission, and result handling must use typed protocol fields.
- Streaming tests must verify stable message order, incremental Markdown, and no client-generated fallback answer.
- Unsupported schema versions must be rejected without mutation. Tests recreate their own stores and never require migration fixtures.

## Canonical resource locator contract (2026-08-24)

Resource identity crossing a Host boundary is owned by the Host Protocol, not by a client UI or a platform path helper.

- `HostArtifact.uri` MUST be a non-empty, normalized absolute URI. A local filesystem artifact MUST use a percent-encoded `file://` URI.
- A raw absolute path such as `/tmp/report.docx`, an unencoded Unicode/space-bearing file URI, or an empty display name is a protocol error.
- The platform Host resolves native paths and emits the URI. Host Protocol validates its wire shape. Core validates it before accepting a response, advancing response sequence, consuming a pending effect, writing a Part, Trace, transcript item, or checkpoint.
- Application Store preserves the canonical URI verbatim. Presentation may resolve a supported URI for display/open actions, but MUST NOT guess that an arbitrary string is a filesystem path.
- `name`, `mime_type`, and `operation` are resource metadata. `name` is never a locator and MUST NOT be used to reconstruct one.
- Direct tools, delegated Agents, background effects, replay, and restored projections use the same contract. There is no client-specific compatibility fallback before launch.

Part lifecycle and execution outcome remain separate facts. A terminal Part may be durably `completed` while its structured tool result records a recoverable execution failure; projection code must not collapse these fields or manufacture contradictory terminal states.

Document Skill prompt constraints are an execution guide, not a substitute for typed tool schemas. Domain payloads such as Word table columns and rows should continue moving toward explicit capability schemas validated before Host execution.

## 2026-08-25 historical projection and native UI convergence

Historical conversation operations use the same canonical projection contract
as a live Turn. `branch`, `rebase`, and `revert` rebuild completed Parts from the
validated immutable `AgentItem` ledger plus explicit `AgentItemOwnership`:

- assistant messages become completed Text Parts;
- reasoning items become completed Reasoning Parts;
- tool calls and results share one Tool Part identity and preserve structured
  input, output, preview, artifacts, and terminal outcome;
- compaction items become Compaction Parts;
- Message, Turn, and Part ownership is copied from the ledger rather than
  inferred from ordering or presentation state.

Application must reject or avoid a checkpoint whose transcript contains owned
items while `conversation_projection.parts` is empty. A branch or rebase is not
committed until its rebuilt projection, lineage, selected head, checkpoint, and
Store transaction agree. Cold reopen is the authoritative acceptance boundary.

Native clients render context accounting from the same Core/Application
snapshot. The compact control shows percentage plus used and total model-window
tokens; message count and character count remain diagnostics only. Presentation
may abbreviate token numbers, but it cannot recompute or substitute the source
metric.

Product UI evidence is state evidence, not a screenshot side effect. iOS and
Android scenarios wait for a terminal, stable canonical assistant bubble, then
retain screenshot, accessibility tree, timeline, provider trace, logs, and a
transcript cold-read from canonical Store. The test must not terminate the app
as soon as an expected marker first appears, because that can truncate the
stream it claims to validate.

External test orchestration is also protocol infrastructure. Each ADB,
simulator, browser, Host, and network probe has an individual deadline inside
the scenario deadline. A stale Android emulator is discarded and ADB recovered
before a clean AVD is started; no blocking subprocess may make an outer timeout
ineffective.

## Run-scoped invocation configuration and hot execution (2026-08-25)

Core owns the normalized LLM interaction protocol, context/data processing,
Message/Part state transitions, checkpoint semantics, and canonical data
updates. A platform owns the concrete provider selection, model selection,
generation parameters, conversation policy, credential references, and UI
defaults. Core MUST NOT discover those values from platform files, global
settings, a previous conversation turn, or a conversation checkpoint.

Each new locally-owned assistant or maintenance Run begins with one immutable,
platform-supplied `InvocationConfigSnapshot`. A remote mirror Run stores only
the Host-owned projection and never fabricates local provider configuration. The snapshot contains non-secret model
identity and capabilities, generation parameters, context policy, exposed Host
catalog revision, permission mode, and opaque credential references. The
Application layer validates and stores a redacted snapshot plus a stable digest
on the Run. A new Run always receives a new snapshot. Changing platform
settings never mutates an active Run.

Subsequent Host events carry `run_id`, `effect_id`, ordered event identity, and
the configuration digest. They do not resend the complete configuration. The
Application service resolves the immutable Run snapshot, rejects a digest
mismatch before mutation, and restores Core with that snapshot only when crash
recovery is necessary. Configuration evidence belongs to the Run; it is not a
conversation preference and is not part of the Core checkpoint.

The production Application service maintains an `ActiveRunRegistry`. Each
active Run has one serial actor containing the hot Core runtime, immutable
configuration snapshot, event cursor, semantic stream buffer, and transaction
scheduler. Store remains authoritative; the actor is a disposable acceleration
layer rebuilt from Run + checkpoint + event ledger after a crash. There is no
per-delta database reopen or full checkpoint decode on the healthy hot path.

Provider deltas are coalesced into stable `PartDeltaBatch` transactions at a
semantic Markdown boundary, a tool/state transition, a bounded byte threshold,
stream termination, or a short 50-100 ms latency deadline. One batch atomically
advances the Part, event cursor, checkpoint, acknowledged effect, and successor
outbox. Backpressure is explicit. A token is never an independent database
transaction or UI identity.

Large documents, web pages, health exports, and binary artifacts cross the
protocol as immutable `DataReference` objects containing blob identity, digest,
media type, byte count, provenance, observation time, and authorization scope.
Core requests bounded chunks or searches through Host effects. Raw large
payloads do not enter checkpoints, Run configuration snapshots, or trace logs.

Performance correctness is measured by ownership-preserving work avoided:

1. one configuration serialization per new Run, not per delta;
2. one hot Core runtime per active Run, not one restore per event;
3. one SQLite transaction per semantic batch, not per token;
4. one canonical projection for every client, not client-side transcript copies;
5. bounded chunk reads and caches keyed by content/config/catalog revisions.

### Implementation status

The 2026-08-29 implementation establishes the first production slice of this
boundary:

- Application stores the immutable Run configuration and clients retain only
  its digest after Run creation. Active commands reject unknown full-config
  payloads and digest mismatches.
- Embedded Application runtimes retain a hot `AgentCore` after a successful
  commit. Reuse is guarded by the canonical Store checkpoint revision; a
  mismatch, failed commit, terminal Run, or process restart discards the cache
  and restores from Store.
- The Application runtime now returns small text, reasoning, tool-input, and
  progress transitions to presenters without advancing SQLite on every delta.
  It retains their typed Core events in the active Run actor and atomically
  commits the bounded batch at `64 KiB`, a terminal Host response, or an active
  submission boundary such as cancellation. Internal checkpoint effects are
  never exposed as executable successor work. Direct, stateless Application
  calls remain immediately durable for recovery and administrative tooling.
- Core publishes assistant text at stable Markdown/sentence/size boundaries.
  Clients consume the canonical Part projection directly and no longer replay
  provider tokens through a second character or phrase queue.
- iOS, macOS, Android, CLI, and Web reducers preserve append-only Part identity
  and reject stale recovery projections before they can replace visible text.

The full `DataReference` migration for every large payload remains follow-up
work. It must not be represented as complete until each Host path has bounded
read/search evidence and checkpoint/trace absence tests.
