> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Nola Architecture V2 Implementation Plan

Status: Accepted implementation baseline\
Scope: Core, Application Service, Runtime Host, macOS, iOS, Android, CLI, Web\
Related: `Doc/历史任务/实施与审计/ARCHITECTURE_V2.md`

## 1. Purpose

This document turns Architecture V2 into an executable refactor plan. It addresses the failures repeatedly observed in real sessions:

- old messages are overwritten, duplicated, reordered, or partially lost;
- thinking, tool calls, and final answers disagree between clients;
- clients and background processes write the same durable state through different paths;
- permission decisions are not durable at the correct scope and are requested repeatedly;
- long-running work is tied to a chat stream or a process lifetime;
- local services and browser workers remain alive without useful work and consume excessive memory;
- context, transcript, memory, and UI preview are treated as interchangeable data;
- platform clients implement different agent behavior and gradually diverge;
- integration tests exercise helpers but do not prove the real command-to-projection path.

The refactor is not a compatibility exercise. Nola has not shipped this storage architecture, so obsolete stores, adapters, and dual-write paths must be deleted instead of migrated indefinitely.

## 2. Architectural diagnosis

The recurring defects are not isolated UI bugs. They are consequences of ambiguous ownership.

| Symptom | Structural cause |
|---|---|
| Duplicate or overwritten answers | More than one component assembles or persists the assistant message |
| Missing thinking/tool history | Ephemeral host events are not committed as canonical message parts |
| Slow first response | Client startup, service startup, environment discovery, and model request are serialized on the send path |
| Permission prompts repeat | Grant state is split across UI, host process, CLI store, and transient tool execution |
| Long tasks disappear or loop | Chat turns, workers, leases, and durable task state share no single state machine |
| High service memory | Full transcripts, tool output, browser state, and projections are retained in one long-lived process |
| Different behavior per platform | Agent policy and context construction live in platform clients rather than the shared application boundary |
| Tests pass while products fail | Tests bypass IPC, persistence, projection subscriptions, rendering, or real process lifecycle |

The central correction is to assign one owner to each kind of state and prohibit all alternative write paths.

## 3. Reference-derived principles

The useful pattern shared by mature coding-agent systems is not their terminal UI. It is the separation between protocol, durable session state, execution runtime, and presentation.

### 3.1 Protocol-first boundary

Clients submit typed commands and consume typed events or projections. They do not call persistence helpers, interpret host-private strings, or reconstruct canonical messages from console text.

### 3.2 One canonical session transcript

The transcript is an append-only sequence of typed message parts. Streaming deltas are temporary delivery events. Only the finalized, canonical part is durable conversation content.

### 3.3 Harness owns execution, not product state

The runtime harness executes committed effects, reports observations, and renews leases. It does not own conversations, permissions, memory policy, or the final answer.

### 3.4 Server/client symmetry

Terminal, desktop, mobile, and Web are views over the same application protocol. Platform-specific capabilities are exposed by Hosts through typed capability descriptors rather than by forking agent logic.

### 3.5 Bounded process lifetime

Browser engines, MCP servers, model processes, and task workers start on demand, publish health, obey idle timeouts, and can be reclaimed independently of the Application Service.

## 4. Target topology

```text
macOS / iOS / Android / CLI / Web
                |
       Command / Query / Subscribe
                v
        Nola Application Service
        - sole durable writer
        - command receipts
        - projection journal
        - policy coordination
        - Runtime Supervisor
                |
          pure decisions/effects
                v
            Nola Kernel
                |
          committed Effect jobs
                v
       Platform / Remote Runtime Host
       - files, shell, browser, MCP
       - model transport, local model
       - platform data sources
       - no canonical persistence
```

### 4.1 Kernel

The Kernel is deterministic and performs no I/O. It owns:

- state transitions;
- effect planning;
- tool selection contracts;
- context budgeting decisions;
- continuation and completion decisions;
- typed validation independent of any platform.

The Kernel must not know SQLite paths, SwiftUI state, terminal dimensions, browser processes, API keys, or platform permission APIs.

### 4.2 Application Service

The Application Service is the only component allowed to open the canonical database. It owns:

- command validation and idempotent command receipts;
- Unit of Work transaction boundaries;
- canonical conversations, messages, and parts;
- context checkpoints and memory records;
- permission grants and pending interventions;
- data source, Skill, MCP, model, and automation configuration;
- long-task records, leases, output cursors, and task events;
- projection snapshots, suffix journals, and subscription cursors;
- Runtime Supervisor coordination.

No platform client, CLI mode, Web route, worker, or Host may directly write these tables.

### 4.3 Runtime Supervisor

The Runtime Supervisor is an Application Service component. It owns runtime lifecycle, but not tool implementation:

- claim and lease arbitration;
- bounded concurrency and fairness;
- cancellation and timeout propagation;
- crash recovery and uncertain-effect reconciliation;
- Host health and capability routing;
- idle shutdown of expensive runtimes;
- task progress and resource-budget enforcement.

### 4.4 Runtime Host

A Host executes only committed Effects and reports typed observations. A Host owns:

- platform-specific tool implementations;
- local or remote model transport;
- filesystem handles granted by the operating system;
- Chrome extension transport and browser sessions;
- accessibility, screen capture, HealthKit, calendar, and equivalent platform adapters;
- MCP process or remote-transport management.

A Host must not invent messages, silently retry non-idempotent effects, decide durable permission scope, or update canonical task rows directly.

### 4.5 Clients

All clients are thin. They own:

- local presentation state;
- draft input and selection;
- incremental rendering caches;
- commands initiated by the user;
- projection subscriptions and pagination;
- platform-native interaction and accessibility.

Clients must not assemble a second canonical transcript, infer completion from a quiet stream, or persist final model text outside the Application Service.

## 5. Canonical data ownership

| Data | Owner | Durable | Notes |
|---|---|---:|---|
| Conversation/message/part | Application Service | Yes | Append-only identity and stable ordering |
| Streaming delta | Delivery layer | No | May be dropped and resumed from cursor |
| Final assistant part | Application Service | Yes | Committed once with source run identity |
| UI animation state | Client | No | Never changes canonical text |
| Context checkpoint | Application Service | Yes | Exact inputs used by a run |
| Long-term memory | Application Service | Yes | Typed provenance, status, revision, policy |
| Permission grant | Application Service | Yes | Explicit scope and expiry |
| Pending permission/input request | Application Service | Yes | Suspends execution without ending it |
| Runtime task | Application Service | Yes | Independent from any chat window |
| Worker process state | Runtime Supervisor | Partly | Lease and health durable; process handle ephemeral |
| Tool output preview | Application Service | Yes | Bounded and safe for UI |
| Full large output | Artifact store | Yes | Referenced by content handle, not embedded |
| Markdown layout/cache | Client | No | Derived from committed or preview text |

## 6. Conversation and streaming protocol

### 6.1 Stable identities

Every command, run, message, part, effect, tool call, task event, and artifact has a stable identifier. Every stream event carries:

```text
conversation_id
run_id
message_id
part_id
sequence
revision
event_kind
payload
```

The client deduplicates by event identity and applies only a contiguous sequence. It requests a suffix from the last committed cursor after reconnecting.

### 6.2 Message lifecycle

The user command atomically appends the user message and creates an empty assistant message shell. The shell is visible immediately, before model output begins.

```text
created -> planning -> executing -> streaming -> finalizing -> completed
                                                -> failed
                                                -> cancelled
```

Thinking, tool calls, permission waits, and answer text are distinct typed parts within the same assistant message. The final answer never replaces earlier assistant messages.

### 6.3 Delta versus canonical text

The delivery path maintains a preview buffer per `part_id`. A delta extends that preview only when its sequence is new. Finalization provides the canonical full text and its revision.

The Application Service commits final text exactly once. A client may replace the preview of that same part with the canonical final value, but it must never append both values or replace another part.

Continuation after an output limit creates a new segment linked to the same answer part. The Application Service joins segments using stable segment identifiers, overlap detection, and model-provided finish reasons. Blind text continuation based on suffix similarity is prohibited.

### 6.4 Incremental Markdown

Rendering is a client concern with a shared behavioral contract:

- parse only the changed tail and the nearest open block;
- keep stable blocks keyed by source ranges and part revision;
- render an open fenced code block immediately after its language marker;
- update the code block in place until the fence closes;
- never switch the entire response from raw text to Markdown at completion;
- animate only newly stabilized semantic spans;
- preserve layout identity so hover, expansion, and stream updates do not move existing blocks.

The desired reveal is a restrained opacity and vertical-depth transition applied once to a newly stabilized phrase. Existing text must not blur, flash, or reanimate.

## 7. Context, transcript, and memory

These are separate concepts and must remain separate in storage and APIs.

### 7.1 Transcript

The transcript is the full durable user-visible history. It is paged and never inserted wholesale into the model request.

### 7.2 Context checkpoint

A context checkpoint records the exact bounded inputs selected for a run:

- recent message slices;
- conversation summary revision;
- selected memory record revisions;
- selected data-source observations;
- Skill instructions and MCP capability descriptions;
- tool-result summaries;
- token estimates and model window budget;
- provenance, authorization, freshness, and content references.

The input composer and inspector read the same projection of the current checkpoint. They must never calculate context percentage independently.

### 7.3 Long-term memory

Memory is semantic state, not copied transcript. Automatic writes do not require a user-facing permission dialog, but they require policy and quality checks. Records include:

- typed subject and content;
- provenance and source message/run identifiers;
- confidence and quality assessment;
- lifecycle status;
- revision and timestamps;
- scope, including whether cross-conversation recall is allowed.

Recall results are candidates. Context planning decides whether they enter a checkpoint. Deleting or disabling memory changes future checkpoints without mutating historical transcripts.

### 7.4 Compaction

Compaction combines deterministic code and an LLM-generated semantic summary:

- code selects immutable source ranges and preserves tool/permission/task facts;
- the model summarizes only the selected range;
- code validates coverage metadata and stores the summary revision;
- recent messages and unresolved work remain verbatim;
- the transcript remains available for replay and audit;
- no message-count heuristic may masquerade as token usage.

## 8. Tool, Skill, MCP, and SOP model

### 8.1 Tool

A Tool is a typed executable capability with schema, risk class, platform requirements, and result contract.

### 8.2 Skill

A Skill is a durable, editable instruction-and-resource package that helps the Kernel plan and use Tools. It is not a keyword router and it does not execute by itself.

### 8.3 MCP

An MCP server contributes tools and resources through a managed transport. Its process lifecycle, health, schemas, and user configuration are visible independently from Skills.

### 8.4 SOP

An SOP is represented as a Skill with explicit workflow stages, expected artifacts, and verification criteria. It is not pasted into every prompt and must not appear as an unrelated attachment list below every message.

### 8.5 Selection

Selection is model- and schema-driven:

1. Context planning exposes the bounded relevant capability catalog.
2. The model requests a typed Tool or Skill operation.
3. The Kernel validates capability, arguments, policy, and current state.
4. The Application Service commits an Effect.
5. A compatible Host claims and executes the Effect.

Hard-coded natural-language keyword routing is prohibited except for explicit slash commands and deterministic UI actions.

## 9. Permission and intervention model

Permission policy is centralized and typed.

### 9.1 Modes

| Mode | Default behavior |
|---|---|
| Recommended | Allow reversible read/write work; ask for destructive, irreversible, credential, privacy, or external side effects |
| Safe | Ask for writes, execution, external communication, and sensitive data access |
| Unrestricted | Allow supported operations except operating-system consent that cannot legally or technically be bypassed |

### 9.2 Grant scopes

```text
once
conversation
workspace
capability
session
persistent
```

A grant contains normalized operation, resource scope, mode, expiry, origin, and revocation state. Shell wrappers such as `bash -lc`, display labels such as `cmd=`, and localized tool names must not affect matching.

### 9.3 Suspension

When permission or user input is needed, the task enters a durable suspended state. `custom response` returns control to the normal composer and waits for the next user message; it is not interpreted as an allow/deny decision.

The worker releases expensive resources while suspended. Resumption uses the same task/run identity and the same committed effect boundary.

Operating-system access, such as a folder picker, Accessibility, Screen Recording, or HealthKit, is represented separately from Nola policy. The Host reports the real OS authorization state; the Application Service must not infer it from a stale prompt result.

## 10. Long-running task architecture

A long task is a first-class durable aggregate, not a long chat callback.

### 10.1 State machine

```text
queued -> claimed -> running -> completed
                         |       failed
                         |       cancelled
                         v
                 waiting_permission
                 waiting_input
                 waiting_dependency
                 retry_scheduled
```

### 10.2 Commands

The public command surface is explicit:

```text
CreateTask
ClaimTask
RenewTaskLease
ReportTaskProgress
AppendTaskOutput
SuspendTask
ResumeTask
ProvideTaskInput
AuthorizeTaskPermission
RequestTaskCancellation
CompleteTask
FailTask
AcknowledgeTaskNotification
```

Generic `SaveTask` and arbitrary row replacement are prohibited.

### 10.3 Ownership

Every worker mutation requires the active ownership token. Progress, resource use, output cursors, and terminal transitions are monotonic. A stale worker can report an observation, but it cannot overwrite the active task.

### 10.4 Output

Task output is append-only and cursor-addressable. Small summaries are stored inline. Large logs and artifacts are stored externally and referenced by typed content handles. Chat projections may show task milestones, but task truth does not depend on a chat message being visible.

### 10.5 Recovery

On service restart:

- queued tasks remain claimable;
- valid leases remain owned;
- expired leases enter reconciliation before retry;
- non-idempotent uncertain effects are never repeated silently;
- suspended tasks remain suspended;
- completed output remains readable without the original Host process.

## 11. Process and memory design

### 11.1 Application Service process

The service stays small and durable. It may retain:

- database connection pool;
- bounded projection journal cache;
- bounded active-run metadata;
- lightweight Host health records;
- subscription cursors.

It must not retain full transcripts, full tool outputs, Playwright page objects, model weights, or unbounded event histories.

### 11.2 Runtime processes

| Runtime | Start | Stop |
|---|---|---|
| Local model | Explicit user start or claimed task requiring it | Explicit stop, memory pressure, or configured idle timeout |
| Chrome bridge | Existing extension connection or claimed browser effect | Disconnect/idle policy; Chrome itself is not owned by Nola |
| Playwright sidecar | Claimed automation effect requiring Playwright | Page/context cleanup then short idle timeout |
| MCP server | Enabled configuration plus active demand | Disable, crash policy, or idle timeout when supported |
| Task worker | Claimed task | Completion, suspension, cancellation, lease loss, or timeout |

All runtimes publish resident memory, child count, last activity, current owner, and shutdown reason. Orphan cleanup is part of Runtime Supervisor recovery.

### 11.3 Backpressure

- subscriptions have bounded buffers and resumable cursors;
- tool output has inline byte limits and artifact spillover;
- streaming coalesces delivery updates without coalescing canonical parts;
- transcript pages are loaded on demand;
- UI caches are bounded by visible conversations and evict on window switches;
- four concurrent LLM requests are a configurable scheduler limit, not an unconditional fan-out rule.

## 12. Data Source, Skill, and Automation composition

Data-driven behavior is a planned composition rather than implicit prompt injection.

```text
Automation trigger
    -> select enabled Data Sources
    -> validate authorization and freshness
    -> collect typed observations
    -> create Context Checkpoint
    -> select Skill/SOP and capability catalog
    -> create durable Task/Run
    -> execute through Runtime Host
    -> commit result, artifacts, and optional memory candidates
```

The first successful Data Source setup stores:

- connector configuration;
- authorization state and scope;
- tested read operation;
- typed content schema;
- sample shape with sensitive values removed;
- freshness policy;
- last validation result and time.

Users configure default-enabled sources globally and override allowed sources per conversation. The conversation override is durable and visible in the inspector.

## 13. Client responsibilities

### 13.1 macOS

- native SwiftUI/AppKit client;
- communicates with the Application Service and the selected Host;
- does not launch CLI for ordinary chat;
- exposes model and Host selectors as independent controls;
- keeps workspace/context inspector settings per conversation;
- uses Chrome extension transport for browser communication;
- displays task progress in the message timeline with expandable details;
- never opens the canonical SQLite database.

### 13.2 CLI

- terminal client and optional Web Host entry point;
- renders the same projections and interventions;
- uses normalized tools rather than shell/display-name matching;
- preserves terminal scrollback and keeps the input cursor in the composer;
- debug mode changes detail density, not context-format semantics;
- never opens the canonical SQLite database.

### 13.3 Web

- served by the CLI Web Host where configured;
- authenticated by an explicit token session;
- subscribes to Application Service projections;
- does not create its own session store or duplicate agent policy.

### 13.4 iOS and Android

- use the same commands, projections, memory policy, context semantics, and tool event model;
- expose only Hosts/capabilities available on the platform;
- keep permission and onboarding text localized;
- do not fork prompt policy or agent state transitions.

## 14. Protocol and compatibility policy

The protocol uses versioned typed envelopes. Unknown optional fields may be ignored within one protocol epoch, but semantic compatibility shims and dual storage paths are not retained.

Because this refactor precedes release:

- recreate obsolete canonical databases instead of adding migration branches;
- delete legacy direct-Store APIs when their callers are moved;
- delete CLI subprocess paths from native clients;
- delete stringly typed permission, task, and tool-event fallbacks;
- delete parallel prompt implementations;
- fail visibly on a schema-epoch mismatch rather than silently deleting user state.

## 15. Implementation sequence

### Phase 1: Protocol and dependency direction

- establish stable IDs, envelopes, commands, events, and projections;
- make Kernel pure and prevent it from depending on Application or clients;
- make service-client depend only on protocol contracts;
- add automated dependency-direction guards.

### Phase 2: Sole writer and durable journal

- give the Application Service exclusive database ownership;
- add command receipts and idempotency keys;
- add fixed-revision projection snapshots and suffix journal subscriptions;
- remove direct Store access from macOS, CLI, Web, iOS, Android, and Hosts.

### Phase 3: Runtime Supervisor and long tasks

- replace generic task row saves with explicit commands;
- enforce ownership tokens and leases;
- separate progress/output/events from the task snapshot;
- recover expired and uncertain work deterministically;
- add bounded runtime lifecycle and resource budgets.

### Phase 4: Context, memory, permissions, and capabilities

- persist exact context checkpoints;
- route all memory through typed Application commands;
- centralize permission grants and interventions;
- unify Tool, Skill, MCP, SOP, Data Source, and Automation catalogs;
- remove keyword routing and platform-specific prompt policy.

### Phase 5: Thin clients and incremental rendering

- move every client to commands, queries, and subscriptions;
- make message-shell creation immediate;
- render typed thinking/tool/task parts consistently;
- implement stable incremental Markdown and semantic reveal animation;
- load transcript and inspector data through bounded projections.

### Phase 6: Delete obsolete architecture

- remove legacy Store wrappers and schema compatibility code;
- remove native-client CLI coupling;
- remove duplicate transcript assemblers;
- remove orphan process managers and legacy browser open tools;
- remove tests that validate only obsolete helper paths.

## 16. Verification strategy

Static helper tests are necessary but not sufficient. Verification is layered.

### 16.1 Contract tests

- command/event round trips;
- stable IDs and ordering;
- schema-epoch rejection;
- permission normalization;
- task transition invariants;
- context and memory provenance.

### 16.2 Application Service integration tests

- command receipt idempotency;
- atomic state plus event publication;
- reconnect from projection cursor;
- stale worker rejection;
- crash between effect commit and Host report;
- permission suspension and resumption;
- compaction with transcript preservation.

### 16.3 Host integration tests

- tool result and artifact contracts;
- Chrome extension connection and browser effects;
- Accessibility and Screen Recording authorization refresh;
- MCP startup, health, timeout, and shutdown;
- local-model startup progress and cancellation;
- long command output spillover and process cleanup.

### 16.4 End-to-end tests

Required scenarios use the real service/client protocol and real persistence:

1. Send a message and observe the assistant shell before the first model delta.
2. Stream thinking, tool calls, open Markdown code, and final prose without duplicates or layout replacement.
3. Reconnect during streaming and resume exactly once from a cursor.
4. Allow a write for the conversation and perform multiple matching writes without another prompt.
5. Request a destructive operation and verify a durable suspended permission intervention.
6. Run a multi-hour simulated task across worker restart and service restart.
7. Create an artifact through a Skill and open it from the committed message part.
8. Trigger an Automation that reads a Data Source, applies a Skill, calls a Tool, and records the result.
9. Switch among multiple conversations while each contains ten million simulated characters.
10. Keep four concurrent model streams ordered and isolated by run/message/part identity.
11. Start and stop local model, MCP, and Playwright runtimes and verify memory returns below the idle budget.
12. Run the same canonical conversation assertions through macOS, CLI/Web, iOS, and Android adapters.

### 16.5 Visual and performance tests

Screenshots and interaction traces are first-class evidence. Tests must inspect:

- alignment, typography, hierarchy, localization, empty/loading/error states;
- expansion without flicker or delayed body layout;
- stable message positions during stream updates;
- input composer occlusion and smooth bottom-follow behavior;
- memory before, during, and after browser/model/task workloads;
- first-shell, first-status, first-token, and completion latency separately.

## 17. Budgets and acceptance gates

Initial budgets are explicit and adjusted only with measured evidence.

| Metric | Gate |
|---|---:|
| Assistant shell after accepted send | <= 100 ms |
| Local status transition visible | <= 150 ms |
| Projection reconnect without duplicate | 100% |
| Stale ownership-token mutation accepted | 0 |
| Full transcript retained in service memory | 0 pages beyond bounded cache |
| Inline tool output | bounded; overflow becomes artifact |
| Playwright after idle timeout | no owned browser sidecar remains |
| Permission grant repeated within matching scope | 0 prompts |
| Ten-million-character conversation switch | no full synchronous decode/render |
| Canonical final answer commits per part | exactly 1 |

Architecture V2 is not complete until all gates below are true:

- only the Application Service opens the canonical Store in production normal operation;
- every durable mutation is a typed command with an idempotent receipt;
- every worker mutation is authorized by ownership and state transition rules;
- all clients render the same canonical message-part ordering;
- transcript, context, memory, and preview are distinct in code and storage;
- no long task depends on one chat stream, window, or worker process surviving;
- expensive runtimes are demand-driven, observable, bounded, and reclaimable;
- direct Store APIs, generic task saves, duplicate prompt policy, and compatibility paths are deleted;
- required end-to-end, visual, restart, and performance scenarios pass.

## 18. Immediate implementation decisions

The next implementation work follows these decisions:

1. Finish replacing `RuntimeTaskCommand.Save` with explicit lifecycle commands.
2. Require ownership tokens for all worker progress, metadata, output, and terminal reports.
3. Add append-only task output cursors and typed artifact references.
4. Make terminal state plus terminal event one Unit of Work operation.
5. Remove remaining CLI/Web direct Store access, including test-only wrappers after equivalent Application Service tests exist.
6. Move platform Hosts to committed Effect execution only.
7. Then migrate client transcript rendering and inspector state to canonical projections.

No UI polish should be used to conceal a violated ownership rule. Presentation work begins after the underlying command, event, and projection path is singular and testable.
