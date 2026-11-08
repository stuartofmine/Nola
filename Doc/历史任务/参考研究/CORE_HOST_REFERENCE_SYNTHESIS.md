> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Core / Host Reference Synthesis

## 1. Purpose

This document records the architecture decisions derived from studying:

- `参考资料/codex-main`
- `参考资料/opencode-dev`
- `参考资料/claude-code`
- `参考资料/open-webui-0.10.2`

The goal is not to copy any one implementation. Nola keeps its local-first,
multi-platform product model while adopting the strongest lifecycle,
context-management, tool, recovery, and transport mechanisms from the four
projects.

The resulting rule is:

> Core owns deterministic Agent state. Host owns every side effect. Product UI
> consumes one ordered event stream and never runs a second Agent loop.

## 2. What Nola adopts

### 2.1 From Codex CLI

Adopt:

- A long-lived Thread containing multiple Turns.
- An immutable TurnContext captured at Turn start.
- Items and Parts as stable protocol entities rather than reconstructed text.
- Steering only at a model boundary.
- Compaction before model sampling, based on the actual model-visible context.
- A single ordered event stream for model output, reasoning, tool activity,
  permissions, usage, compaction, and terminal state.
- Explicit cancellation and interruption.
- Host effects that can remain pending across process or network boundaries.

Reject:

- Terminal UI implementation details.
- A CLI-specific tool catalog as the product capability model.
- Any assumption that the Agent and the tool executor are in one process.

### 2.2 From OpenCode

Adopt:

- Stable message Parts for text, reasoning, tool input, tool output, artifacts,
  and errors.
- Tool lifecycle states: pending, running, waiting, completed, failed, and
  cancelled.
- Streaming tool arguments without recreating the tool call.
- A doom-loop guard based on repeated tool name, normalized arguments, and
  equivalent results.
- Preservation of recent Turns during compaction.
- Pruning or externalizing large tool results while preserving references.
- Model-window metadata as the source of context limits.

Reject:

- Provider-specific message shapes leaking into product state.
- UI reducers that infer tool state from display strings.

### 2.3 From Claude Code

Adopt:

- LLM-assisted compaction combined with deterministic code-side selection.
- Preservation of plans, decisions, constraints, recent files, unresolved
  work, background tasks, and tool outcomes.
- Prompt-too-long recovery by removing complete oldest model round groups, not
  arbitrary individual messages.
- Explicit cancellation propagation into model, tool, and background work.
- Tool calls as first-class visible work rather than hidden status text.

Reject:

- Opaque transcript mutation.
- Compaction that replaces the durable transcript.
- Platform prompts that silently override the shared product prompt.

### 2.4 From Open WebUI

Adopt:

- Explicit session identity and request correlation.
- Heartbeats and leases for remote or child-process Hosts.
- Bounded session pools and idle expiry.
- Separate transport completion from Agent completion.
- Resume messages bound to the original request, session, Host, client, nonce,
  and payload digest.

Reject:

- Treating a socket reconnect as a new Agent request.
- Replaying the user's original message to resume a waiting tool or permission.

### 2.5 Audited source anchors

The synthesis above is based on executable paths rather than product-level
descriptions:

| Reference | Source paths inspected | Boundary adopted by Nola |
| --- | --- | --- |
| Codex CLI | `codex-rs/core/src/tools/{registry,router,lifecycle}.rs`, `codex-rs/core/src/session/input_queue.rs`, `codex-rs/core/src/tasks/mod.rs`, `codex-rs/core/src/compact.rs` | separate model-visible specifications from registered executors; centralize lifecycle; inject queued input only at a named boundary |
| OpenCode | `packages/opencode/src/session/{message-v2,processor,compaction}.ts`, `packages/opencode/src/permission/index.ts`, `packages/opencode/src/bus/index.ts` | stable discriminated Parts; suspended permission requests; typed events; bounded tool-output pruning |
| Claude Code | `src/hooks/{useCanUseTool,useCommandQueue}.tsx`, `src/commands/{compact,resume,rewind}`, `src/components/tasks/renderToolActivity.tsx` | explicit permission adapters and aborts; immutable queue snapshots; compaction hooks and recovery; validate input before presentation |
| Open WebUI | `backend/open_webui/utils/context_compaction.py`, `backend/open_webui/socket/main.py`, `backend/open_webui/utils/middleware.py` | correlated remote events; compaction checkpoints; preserve tool-call/result boundaries; stable streamed output identities |

Nola does not copy the references' provider-specific messages, UI state
machines, Tokio ownership, opaque transcript mutation, or server-owned
conversation semantics. The adopted ideas are expressed through Nola's
deterministic Core contracts.

### 2.6 Prompt ownership and late-bound capability activation

Reference systems are most reliable when stable policy, environmental facts and
retrieved content have different owners and lifetimes. Nola therefore uses this
composition boundary:

```text
Core policy prefix                 cacheable, product-owned, exactly once
  + shared identity/style          cacheable product voice
  + Host surface facts             session/Turn facts, never authorization
  + dynamic reference extensions  non-cacheable, explicitly untrusted
  + user messages                  model protocol, never system fragments
```

`compose_agent_prompt_extension` is deliberately separate from the full Turn
composer. A memory, Skill, attachment or retrieval component cannot recursively
add the Core policy prefix. Host prompts no longer import the shared Agent
policy in addition to Core; they retain only platform identity and mechanics.

Capability discovery follows the same ownership rule. `tool_search` returns a
typed `nola.capability_activation.v1` result. Core validates every returned ID
against the immutable Turn registry, records the activation source, and derives
the next model Step from that state. A Skill invocation activates its declared
required capabilities through the same Core method. Hosts do not scan message
history or user words to reconstruct an expanded tool set.

Core publishes `CapabilitiesActivated` with the owning session, Turn, cursor,
source and canonical capability IDs. CLI/Web, macOS, iOS and Android may adapt
the envelope for transport, but must preserve those identities and display the
event under a stable process/tool item. The shared contract fixture is
`core/tests/fixtures/capability_activation_v1.json`.

## 3. Target layers

```text
Product UI
  |
  | AgentSubmission / AgentEvent
  v
AgentCore
  |
  | private deterministic reduction
  v
AgentRuntime
  |
  | HostRequestEnvelope
  v
Host
  |
  | HostResponseEnvelope
  v
AgentCore
```

### 3.1 AgentCore

`AgentCore` is the only public mutable Agent API.

It owns:

- durable append-only transcript;
- current model-visible context;
- session configuration;
- Turn queue;
- active Turn;
- permission state;
- pending Host effects;
- recovery metadata;
- event cursor;
- context-window state.

It does not:

- call a model;
- read files;
- execute commands;
- request OS permissions;
- read clocks or random generators;
- open network connections;
- write databases;
- render UI.

### 3.2 Private AgentRuntime reducer

`AgentRuntime` is internal and deterministic.

Given the same checkpoint and the same ordered submissions, it must produce:

- the same state;
- the same Host requests;
- the same Agent events;
- the same event cursors;
- the same Core-allocated IDs.

Session identity and imported item identity are supplied by Host. IDs created
inside a session use a checkpointed monotonic allocator.

### 3.3 Host

Host provides:

- model sampling and streaming;
- tool execution;
- permission UI and OS authorization;
- context collection;
- compaction model calls;
- persistence and blob storage;
- clocks, random values, and run identity;
- heartbeat and lease renewal;
- background process lifecycle;
- platform capability catalogs.

Host must not:

- choose the next Agent action;
- append assistant output directly to the transcript;
- replay user input to resume a pending effect;
- infer semantic events from display strings;
- maintain a second Turn counter;
- mutate model-visible context independently.

## 4. Canonical lifecycle

### 4.1 Thread

A Thread is a recoverable conversation session.

```text
create/restore
  -> idle
  -> submit user input
  -> active Turn
  -> waiting for Host or model
  -> active
  -> completed/failed/cancelled
  -> idle
```

Only an idle Thread accepts session-configuration changes. A busy Thread may
accept:

- steering input;
- permission response;
- Host response;
- heartbeat;
- interrupt.

### 4.2 Turn

A Turn captures two typed Item sets with different lifetimes: transcript input
is appended to the Session, while turn-context input is immutable and
model-visible only for that Turn. Every model/tool continuation in the Turn
reuses the same context snapshot. Core, not the Host, composes both sets for
sampling and accounts tokens against the exact composed input.

Source-aware Context Components are submitted separately from both Item sets.
Core validates authorization and freshness, applies platform-aware priority and
budgeting, and freezes the component metrics plus a payload digest. Checkpoint
recovery reuses that exact snapshot and never refetches a source mid-Turn.

Web and CLI Hosts pass raw typed history and components to this boundary. They
must not call a summarizer before dispatching a user Turn; otherwise the same
history is transformed once by the Host and again by Core, doubling latency and
creating two incompatible continuation states.

A Turn captures one immutable `AgentTurnContext`:

- model and provider-facing settings;
- prompt and response style;
- context-window limits;
- tool catalog revision;
- permission policy revision;
- Host identity and lease;
- workspace and data-source selection;
- language and locale.

Changing global settings while a Turn is active must not alter that Turn.

### 4.3 Item and Part

The transcript is not a list of arbitrary display messages.

Canonical Items include:

- user message;
- assistant message;
- reasoning;
- tool call;
- tool result;
- compaction summary;
- system/developer instruction.

Items contain stable Parts:

- text;
- reasoning summary/content;
- tool arguments;
- tool output preview;
- full-output reference;
- artifact reference;
- validation result.

Streaming updates a Part by ID. It never replaces an older assistant message
and never creates a second message for the same Part.

## 5. Host effects

The Host Catalog is authoritative for environmental capability facts, including
availability, replay safety, cancellation behavior, and the permission domain.
The model may request a capability, but it cannot declare that the capability is
safe or choose its permission scope. Static local file reads constrained by the
Host sandbox can be catalogued without a permission domain. Input-dependent
operations such as shell execution are classified by the Host against the real
environment; Core remains the only owner of the resulting allow/ask/deny policy
decision and its durable permission Part.

Every external operation is represented by a `HostRequestEnvelope`.

The envelope binds:

- session ID;
- Turn ID;
- effect ID;
- request-attempt ID;
- request kind;
- capability ID;
- catalog revision;
- Host ID;
- lease ID;
- replay-safety policy;
- cancellation mode;
- payload digest.

The Host returns an ordered stream of `HostResponseEnvelope` values for that
effect and request attempt. Every response repeats the owner tuple and advances
a strictly monotonic sequence.

The pending effect lifecycle is:

```text
dispatched -> accepted -> streaming -> terminal
     \---------------------------> cancelling -> acknowledged
```

Cancellation uses the same envelope path. A cancel request names its target
effect; the Host acknowledgement repeats that target. A driver-private cancel
method would create a second lifecycle and is therefore forbidden.

### 5.1 Permission

Permission is a pending Core state, not a failed tool result.

```text
tool selected
  -> Core emits PermissionRequested
  -> Host displays permission UI
  -> Host submits PermissionResponse
  -> same tool call continues or is denied
```

Choosing "allow for this conversation" updates the checkpointed session grant.
It must not be implemented by caching a UI boolean only.

### 5.2 Remote device tool

A remote device tool remains one pending Host effect:

```text
Core ToolCall
  -> Web/remote Host emits device request
  -> original Turn stays open
  -> device executes capability
  -> device POSTs bound result
  -> original effect resumes
  -> Core emits one ToolCompleted
```

The client must not send the user's message again. Replaying the message creates
a second Turn, duplicate model latency, duplicate context usage, and duplicate
assistant output.

### 5.3 Heartbeat and disconnect

In-process Hosts may omit a lease. Remote and child-process Hosts must provide
one.

Core accepts a Host heartbeat containing:

- Host ID;
- availability epoch;
- lease ID;
- observed time supplied by Host;
- extended expiry supplied by Host.

When the lease expires, pending effects become recoverable or failed according
to replay safety. Core never guesses that a disconnected Host completed work.

## 6. Context model

Nola maintains two different collections:

### 6.1 Transcript

- append-only;
- durable;
- replayable;
- contains full semantic history;
- not destroyed by compaction.

### 6.2 Model context

- derived from transcript and context sources;
- bounded by the selected model's total window;
- compactable;
- may replace large results with summaries and references;
- records whether token usage is exact or estimated.

Compaction runs before sampling when the prompt budget is insufficient.
Manual compaction uses the same Core state machine. When no Turn is active,
success updates the session checkpoint without model sampling; failure is a
session-level compaction event rather than a synthetic Turn failure.

The algorithm:

1. Measure the exact model-visible request when provider token accounting is
   available; otherwise use an explicitly marked estimate.
2. Preserve system/developer instructions, active plans, unresolved work,
   permission decisions, recent Turns, recent file references, and background
   work.
3. Externalize large tool results.
4. Ask the model for a structured summary of the selected old round groups.
5. Validate that required facts remain.
6. Commit one compaction Item.
7. Rebuild model context and remeasure.

The durable transcript remains unchanged.

Host-collected context crosses the boundary as typed `ContextComponent` values
with source, authorization, freshness, and budget metadata. Generic
role/content context-message arrays are not part of the Core runtime contract.

Platform executors receive already-bound capability requests. They validate the
active Host Catalog/tool-plan receipt, execute the OS effect, and return one
correlated Host response. Re-encoding a typed request into a legacy UI event and
parsing it back is prohibited.

Remote clients transport current input and context components in separate
fields. A remote Host must not turn platform context into synthetic system
history before it reaches Core.

Checkpoint persistence is a Host capability (`journal.checkpoint`), not a Core
filesystem concern. Each product surface supplies its own writer and storage
location while Core defines checkpoint contents, revision, cursor, reason, and
recovery semantics.

## 7. Event and Trace boundary

Imported history and current-turn input cross the boundary as typed `AgentItem`
values. A Host must not flatten prior system, developer, user, assistant, tool
call, and tool result items into a synthetic user prompt. Existing process
sessions ignore duplicate history imports and accept only the incremental
current-turn items.

`AgentEvent` is the canonical semantic journal emitted by Core. The same
`AgentTransition` carries `trace_events`, projected inside Core from the
sequenced event envelopes.

The Core-owned projection must preserve:

- Core event cursor as stable identity;
- session and Turn identity;
- Part and tool-call identity;
- ordering;
- visibility;
- terminal state.

Each Turn owns one Trace run and one monotonic Trace sequence. Its
`AgentTraceContext` is persisted in the Session checkpoint, so reconnect and
replay cannot renumber already published UI events. Host may attach permission
nonce, expiry, argument digest and routing metadata to the wire envelope, but
must not recreate or renumber Trace events.

Every `PartUpdated` event carries the Part snapshot immediately after that
specific reducer update. When one internal event produces several updates,
later updates must not retroactively replace the earlier snapshots. This is the
rendering invariant that prevents permission/tool state flicker.

Clients must not accept untraced `text`, `reasoning`, or `tool_result` frames
once the migration is complete. `[DONE]` means transport EOF only; it is not an
Agent completion event.

## 8. Platform mapping

### CLI

- `CoreHostLoop` is the only Agent loop.
- Terminal rendering consumes Trace/Event projection.
- Tool executors are Host adapters.
- `Esc` submits interrupt and propagates cancellation.

### macOS

- Local mode launches `nola-agent-host`.
- Remote mode connects to the Web Host.
- Both modes consume the same Trace contract.
- SwiftUI reducers only project events into presentation state.

### iOS and Android

- Local mode executes Host requests from Rust Core v2.
- Remote mode holds one Web Host Turn open.
- Permission and device results are submitted through dedicated HostResponse
  endpoints.
- The client never resumes by resending the original conversation.

### Web

- The Rust CLI binary is the Web Host.
- SSE is an event transport, not an Agent state machine.
- Permission and device result endpoints wake pending effects.
- Session/Turn state survives temporary transport boundaries according to the
  checkpoint and lease policy.

## 9. Code removal policy

The migration intentionally has no compatibility mode.

Remove:

- legacy `run_agent`;
- raw `text/reasoning/tool_result` semantic decoders;
- permission resume loops that resend user messages;
- device-tool "required" fake error results;
- message-count context heuristics;
- platform-owned Turn iteration counters;
- UI-generated synthetic completion events;
- duplicated prompt stacks;
- legacy Core v1 FFI/runtime entry points.

Temporary adapters are acceptable only while a product path is actively being
migrated. They must not remain reachable from a shipping route.

## 10. Required contract tests

The completed architecture needs cross-platform fixtures proving:

1. Same checkpoint + submissions produce the same Core events.
2. Streaming updates one stable Part.
3. Permission resumes the original tool call exactly once.
4. Device tool result resumes the original effect exactly once.
5. Replayed nonce or mismatched digest is rejected.
6. Host disconnect follows replay-safety policy.
7. Interrupt cancels model and tool work.
8. Compaction preserves required facts and the durable transcript.
9. Context usage is identical in the input footer and detail panel.
10. Mac local, Mac remote, iOS, Android, CLI, and Web render the same fixture in
    the same semantic order.

These tests are evidence of the boundary. UI snapshots or narrow parser tests
alone are not sufficient.

## 11. Project-by-project deep comparison

This section records the implementation-level comparison behind the synthesis.
It distinguishes mechanisms worth adopting from product-specific behavior that
must not leak into Nola.

### 11.1 Codex CLI

Codex has the strongest separation between model configuration, runtime
context, task lifecycle, and terminal projection:

- Model metadata owns base instructions, reasoning support, context limits, and
  auto-compaction thresholds instead of asking each client to infer them.
- Base, developer, compact, environment, permission, plugin, Skill, and selected
  Skill instructions are independent prompt fragments.
- A Turn captures one immutable request snapshot before sampling. The advertised
  tools, context accounting, and actual tool execution therefore refer to the
  same capability state.
- Regular, compact, review, shell, and delegated work use one cancellable task
  lifecycle rather than inventing separate completion rules.
- The terminal renderer keeps an immutable rendered region plus a mutable
  Markdown tail. Incomplete code blocks and tables can evolve without replacing
  already stable output.

Nola adopts typed prompt components, immutable Turn snapshots, one task
lifecycle, and the stable-region/mutable-tail rendering model. Nola does not
adopt Codex's coding-oriented product prompt, provider-specific remote
compaction, terminal styling, or model-name substring routing.

### 11.2 OpenCode

OpenCode provides the clearest end-to-end stable Part implementation:

- It creates and persists the assistant message before model output begins.
- Reasoning, text, tool input, tool progress, tool result, retry, and compaction
  update stable message and Part identifiers.
- Permission requests are suspended promises. `once`, `always`, and `reject`
  update a structured rule set, and a newly accepted session rule releases
  matching pending requests.
- Session, Message, Part, Todo, and Permission records are persisted separately,
  allowing a running or interrupted lifecycle to be reconstructed without
  parsing display text.
- Context compaction preserves recent complete Turns, protects selected Skill
  output, and updates an anchored structured summary.

Nola already has stable `AgentPart`, `PartUpdate`, Core permission events, and
pair-safe compaction. The missing adoption boundary is client conformance: every
client must create an assistant shell on submission and apply updates by
`message_id + part_id`. Nola does not adopt model-ID-based prompt selection,
arbitrary step limits, or UI animation state inside Agent state.

### 11.3 Claude Code

Claude Code is strongest in tool protocol recovery, Skill execution modes, and
long-running work:

- The runtime starts a tool as soon as a streamed `tool_use` block is complete;
  it does not wait for the entire assistant response.
- Every tool call is paired with a terminal result, including cancellation,
  fallback, timeout, malformed output, and unexpected failure paths.
- Skills are model-selected tools and may run inline or in an isolated Agent
  with their own context, token budget, allowed tools, and visible progress.
- Permissions use structured source, behavior, tool, resource, and persistence
  scopes. MCP tools use fully qualified names.
- Background tasks expose task identity, progress, usage, output references,
  cancellation, waiting, and exactly-once terminal notification.
- Context reduction progresses from large-result snipping through micro
  compaction and full structured summary while retaining the durable transcript.

Nola adopts the terminal-result invariant, cancellation-safe suspended
permissions, inline/fork Skill semantics, and a unified observable TaskHandle.
It does not expose hidden chain-of-thought, copy CLI-specific prompts, or let an
LLM permission classifier override deterministic Core policy.

### 11.4 Open WebUI

Open WebUI is not the target Agent Core, but it has valuable compatibility and
source-presentation mechanisms:

- Tools, MCP, direct servers, terminal tools, and built-ins are merged at
  request time, then adapted to native or legacy provider function calling.
- Web, file, and knowledge retrieval results become structured sources. The
  pre-RAG state is restored before each subsequent tool round so the same source
  is not repeatedly injected.
- Search and retrieval results carry citation metadata that the UI can group,
  count, expand, and associate with the current assistant message.
- Malformed tool arguments produce an explicit terminal result rather than
  silently corrupting the conversation.
- Web background work has explicit task identity and distributed cancellation.
- Memory is browsable through a path namespace, while write guidance separates
  durable preferences and standing instructions from transient activity.

Nola adopts a first-class citation/source projection, source de-duplication,
provider function-call adapters, malformed-call recovery, and path-oriented
memory management UI. Nola does not adopt random one-to-three-character stream
chunking, whole-message cloning, fixed tool-call iteration limits, message-count
compaction, or Open WebUI's coarser permission model.

## 12. Capability matrix

| Concern | Codex CLI | OpenCode | Claude Code | Open WebUI | Nola target |
| --- | --- | --- | --- | --- | --- |
| Prompt composition | typed fragments and model capability metadata | model prompt + environment + runtime catalog | dynamic base/user/system/tool/Agent layers | middleware composition | Core prompt-component plan; no client keyword rewrite |
| Turn ownership | session task + immutable step context | one session loop | query loop with concurrent tool scheduling | middleware loop | one deterministic `AgentCore` Turn |
| Stream identity | stable output items and mutable tail | message ID + Part ID | message/tool/task IDs | call ID plus message mutation | stable Item/Part IDs only |
| Tool lifecycle | typed begin/end and cancellation | pending/running/result/error | terminal pairing on every path | in-progress/result events | exactly-once terminal Core lifecycle |
| Skill model | structured inventory and policy | catalog plus Skill tool | inline or isolated Agent | prompt/tool plugins | unified Tool envelope with inline/fork execution |
| Permission | approval policy + granular request | suspended once/always/reject rules | structured scope and source | feature/user toggles | Core rule on normalized resource and scope |
| Context | model metadata and pre-sample compaction | anchored summary + recent Turns | multi-stage reduction | summary checkpoint + heuristic tail | durable transcript + measured model projection |
| Background work | cancellable SessionTask | session loop/subtask | persistent observable task | Web/Redis cancellation | unified TaskHandle; Host-selected execution |
| Retrieval evidence | Web events | tool Parts | visible tool activity | structured citations and RAG de-duplication | first-class Citation Part |
| UI update | immutable region + mutable Markdown tail | reducer updates one Part | grouped stable activity | structured tokens, some whole-message cloning | local Part update; stable Markdown blocks |

## 13. Nola gap matrix

### P0: correctness and single ownership

1. All shipping clients must consume the same Core Item, Part, permission,
   retry, context-usage, compaction, and terminal events. A platform-private
   Agent loop or context counter is a correctness defect.
2. A submitted user message must synchronously create one persisted assistant
   shell. The first provider delta only updates its first Part.
3. Every tool, permission, task, model request, and Turn must reach exactly one
   terminal state on success, failure, denial, cancellation, timeout, process
   disconnect, and provider fallback.
4. Conversation grants must live in the checkpointed Core permission rule set.
   A UI cache or Host-only string match cannot implement "allow for this
   conversation".
5. Footer and inspector context usage must project the same Core context-window
   record, including total window, used tokens, reserved output, exact/estimated
   status, and last update.

### P1: capability composition and recovery

1. Replace surface-owned prompt concatenation with typed prompt components:
   product identity, Agent contract, response style, immutable Turn environment,
   capability catalog, enabled Skill catalog, selected Skill, memory/context,
   and compaction instruction.
2. Unify built-in tools, Skills, MCP, browser/device capabilities, and internal
   tools under one runtime descriptor and lifecycle. Preserve source metadata
   for policy and UI, not for divergent execution semantics.
3. Add typed retry scheduling and provider fallback events. Retry text must not
   masquerade as thinking, a tool call, or assistant content.
4. Add malformed tool-call recovery and synthesize terminal results for orphaned
   tool calls during checkpoint recovery.
5. Add `CitationPart` with URL, title, excerpt, provider, retrieval time, and
   relevance metadata. De-duplicate sources while rebuilding each model-visible
   request.

### P2: product quality

1. Render Markdown as a stable region plus mutable tail. Closed semantic blocks
   stay mounted; only newly stable phrases receive the restrained appearance
   animation.
2. Add inline/fork Skill execution with independent budgets and visible
   progress, while keeping every resulting tool event in the parent transcript.
3. Give typed memory a browsable namespace and provenance-aware management UI.
4. Use one TaskHandle for long Skills, scheduled jobs, data-source refreshes,
   and delegated Agents. Do not automatically background ordinary tool calls.
5. Aggregate pending approvals across conversations without converting them
   into assistant text or losing their owning Turn.

## 14. Changes adopted during this audit

The shared Agent contract in `Nola/SharedPrompts/nola_agent_system.txt` is now
included by the CLI, local Agent Host, Web Host, and macOS Host prompt builders.
The CLI retrieval guidance was changed from a language-specific trigger list to
a semantic evidence rule. iOS text-agent requests now also use the typed Core
prompt composer instead of the legacy local prompt builder. The request keeps
identity, safety, execution policy, host facts, reference context, and response
style as distinct fragments with explicit trust and cacheability. User text is
kept out of system fragments, and available tools are derived from the concrete
schema rather than inferred from words in the request.

iOS context compaction now supplies a dedicated, text-only execution-policy
fragment. The summarizer must preserve decisions, constraints, unresolved work,
paths, tool outcomes, and durable preferences; it may not invent facts or call
tools. Android's unused `LocalProvider` bridge no longer calls a second local
prompt builder: the builder, its C ABI, JNI export, DTOs, and tests were removed
together. Future local inference must consume canonical messages, concrete tools,
and the Core prompt digest directly.

No client may repair routing with localized keyword or exclusion lists. The
model selects a capability from the current schema; Core validates the concrete
call, normalized resource, policy revision, and result lifecycle.

## 15. System prompt architecture comparison

System prompt quality is not measured by one large instruction string. The
important properties are ownership, composition order, trust boundaries,
cache stability, per-turn immutability, and whether special-purpose model calls
can accidentally inherit unrelated tools or authority.

| Project | Strong mechanism | Lesson for Nola |
| --- | --- | --- |
| Codex CLI | Builds prompts from explicit base, developer, environment, Skill, plugin, and compaction inputs; captures the effective turn configuration and supports a stable cacheable prefix. | Core must own typed composition and produce one immutable prompt snapshot and digest for every turn. Clients should provide facts and capabilities, not concatenate policy strings. |
| Claude Code | Uses separate prompts for the main harness, safety and truthful reporting, memory, permission decisions, and no-tools summarization. Recalled memory must be checked against current state. | Compaction, memory consolidation, and permission reasoning need narrow contracts and tool scopes instead of inheriting the full chat prompt. Durable corrections should be written deliberately with provenance. |
| OpenCode | Keeps provider/model prompt selection separate from its runtime permission engine. Pending permission decisions are suspended state, and conversation-scoped approval rules release matching work without rewriting the user prompt. | Authorization must remain typed runtime state. Prompt text may explain policy but must never grant permission or emulate a pending approval. |
| Open WebUI | Provides extensible server-side middleware, filters, and session correlation around model requests. This is useful for deployment customization but permits policy divergence if middleware owns semantics. | Preserve middleware extensibility for transport and observability, while keeping prompt order, trust, and authorization invariants in Core. |

### 15.1 Root cause found in Nola

Nola now has one typed Core prompt composer. The obsolete
`build_local_agent_prompt` path has been removed from Core, its native ABI, and
Android, so local inference cannot silently regain prompt ownership. That path had flattened system strings, inserted its own identity and
tool policy, and encoded conversation history into an ad hoc user payload.
Consequently the same turn could have different policy order, trust semantics,
and caching behavior even when both clients claimed to use Core.

The iOS production path now builds typed fragments and delegates composition to
Core. Its regression tests prove that user queries do not enter system
fragments, typed conversation roles survive, policy fragments are preserved,
and multilingual or ambiguous wording does not force a tool. The compaction
request also carries an explicit no-tools policy instead of a synthetic system
message embedded in history. Android main turns use the same typed composer;
the obsolete local builder and every exported access path were deleted rather
than retained as a compatibility fallback.

### 15.2 Remaining prompt work

1. Align iOS and Android treatment of Core-provided system context so identical
   inputs produce identical layer, trust, cacheability, ordering, and digest.
2. Extend typed provider history with structured media/tool-result payloads on
   transports whose SDK supports them. iOS now preserves canonical roles and
   tool identity; macOS MLX performs an explicit final-edge text degradation
   because its current Swift history API accepts only role/content.
3. Give memory applicability, contradiction checking, consolidation, and
   retention their own typed prompts and restricted tool catalogs.
4. Represent input origin and approval provenance in Core. Stored scheduled-task
   text must never count as fresh interactive authorization.
5. Add a single cross-platform parity fixture for prompt composition identity.
   Core debug traces now emit only the redacted digest and fragment IDs, and
   Android plus iOS both populate `turn_identity.prompt`.
6. Keep model-visible policy separate from UI localization. Localized labels
   may describe a state, but must not change routing, permissions, or tool
   contracts.

## 16. Acceptance sequence

Implementation should proceed in this order, because later UI work otherwise
hides lifecycle defects instead of fixing them:

1. Fake-Host Core test for model stream, permission suspension, tool execution,
   terminal pairing, checkpoint, cancellation, reconnect, retry, and compaction.
2. One cross-platform fixture proving identical semantic event order in CLI,
   Web, macOS, iOS, and Android.
3. Real-model E2E sessions covering natural follow-ups, multilingual requests,
   ambiguous words, unknown tools, permission reuse, malformed tool arguments,
   cancellation, overflow, retrieval citations, and process reconnect.
4. Platform UI tests for immediate assistant shell, stable Part identity,
   incremental Markdown, permission presentation, task progress, and consistent
   context usage.
5. Visual QA only after the state-machine fixtures pass.

## Shared memory worker and accounting boundary

Long-term memory now follows the same ownership rule as tools and permissions: Core decides; Host executes.

1. A completed turn creates a Core extraction job from a frozen transcript snapshot.
2. Core exposes only jobs that are claimable at the current clock boundary.
3. The Host claims a job, launches an isolated extraction or consolidation worker, and writes a result atomically to the session inbox.
4. At the next safe turn boundary, the Host submits the result against the original ownership token and input watermark.
5. Core validates semantic quality, provenance, citations, selected IDs, and output kind before Host persistence and completion.
6. Core chooses consolidation candidates from live global records, excluding superseded rows and ranking actual use before importance and confidence.
7. Successful `memory_search` tool results carry internal usage metadata. Core records the ledger entry and strips that metadata before normal tool-result projection.

This avoids a second hidden model call in the foreground turn, prevents a background worker from overwriting an active checkpoint, and keeps CLI, Web, and macOS Agent Host behavior aligned without prompt rewriting or keyword routing.
