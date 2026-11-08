> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Reference Project Adoption Audit

## Background task ownership and recovery

Nola's Core task state machine already models durable leases, heartbeats, retries, lineage,
permission scope, cancellation scope, and child sessions. The remaining legacy Bash bridge did
not preserve the same authority boundary: persisted shell jobs had no owner session, so the first
session that rediscovered a job could mirror it as its own runtime task. The background task store
now persists the originating session owner, every chat/runtime list-read-wait-stop-notification
path applies that owner boundary, and recovered jobs resume completion notification instead of
being silently marked as already notified.

This adopts the strongest common properties of the reference implementations: Claude Code's
session-scoped durable output, Codex's stable process identity and explicit lifecycle, and
OpenCode's parent/child session ownership. Nola now also exposes independent stdout/stderr byte
cursors: a first read returns the latest bounded chunk, while subsequent reads pass
`stdout_after_seq` and `stderr_after_seq` and receive only new bytes. Results include total byte
counts, next cursors, skipped-history markers, exit state, and exit code. This adopts Codex's
non-repeating `after_seq` contract while retaining Claude Code's file-backed source of truth.
Full migration of shell execution into a Core-owned worker remains explicit follow-up work.

## Consolidated decision matrix

| Reference | System-prompt advantage | Runtime/product advantage | Adopted in Nola | Deliberately not copied |
|---|---|---|---|---|
| Codex CLI | Explicit instruction precedence, trusted environment facts, bounded tool guidance, verification as a separate phase | Typed turn lifecycle, sandbox/approval boundary, resumable tasks, MCP and Skills, provider-aware continuation | Core-owned prompt fragments and trust levels, frozen turn snapshot, canonical Parts, capability catalog, bounded execution, cancellation and checkpoints | A terminal-first product model or a single desktop filesystem as the user's world |
| Claude Code | Compact behavioural contract, clear plan/tool discipline, model-family-specific guidance, stable cached prefix | Subagents, permission modes, Skills, hooks, compaction, durable tool results | Versioned Skills, Agent tasks, scoped grants, structured compaction, memory quality gates and family overlays | One monolithic public prompt, implicit prompt-side authorization, or client keyword routing |
| Claude Code system-prompt archive | Separates invariant policy from model/provider overlays and exposes cache-sensitive prompt composition | Useful evidence for prompt evolution across model generations | Stable-prefix digest, provider capability profile and overlay selection | Mixing every historical or localized prompt into one template library |
| OpenCode | Small provider-aware agent prompts; tool availability is schema-driven rather than inferred from user wording | Session Parts/event bus, permission rules, plugin/MCP catalogs, background tasks | Canonical Part reducer, versioned MCP catalog, unknown-tool refresh/retry, exact-once Host journal | JavaScript-centric ownership of Core semantics or silent same-revision catalog mutation |
| Open WebUI | Explicitly projects selected tools, knowledge and workspace state into the request | Connector lifecycle, model/workspace administration, knowledge/RAG surfaces and Web delivery | Tested data-source contracts, secret-free read recipes, output schemas, model center, Skill/MCP galleries and CLI-hosted Web | Server-centric data ownership, automatic cloud disclosure, or treating cached connector metadata as verified data |

MCP initialization instructions are now retained as bounded typed descriptor data instead of
being discarded after handshake. Unlike direct system-prompt insertion, Nola projects at most 8K
characters per server and 24K total as non-cacheable Reference context with an explicit
non-authority label. They cannot alter the frozen tool catalog, grant permission, or contaminate
the stable prompt prefix.

Core Prompt Composer now also enforces the cross-Host envelope: at most 256 fragments, 64K
characters per fragment, and 256K characters of fragment content in total. Oversized input is
rejected with stable structured error codes instead of being silently truncated differently by
each client. Source-aware trimming remains the Host/context engine's responsibility.

The shared CLI/Web/macOS Agent Host OpenAI adapter now mirrors another reference reliability
boundary: it repairs tool-call/result ordering before transport, records reorder-only repairs in
diagnostics, and drops the oldest media blocks when a request exceeds 80 media items while
preserving text and the newest media. This prevents provider-limit failures without rewriting the
user's text or treating media content as instructions.

### Resulting Nola boundary

Nola's differentiator is not a larger system prompt. Core owns instruction
precedence, capability and permission semantics, context accounting, Parts,
memory quality, task state and recovery. Hosts own platform I/O, rendering,
credential storage and native permission presentation. The model chooses from
the structured catalog; clients do not rewrite natural-language requests or
authorize calls from localized keyword lists.

Context compaction now follows the same boundary: Core sends one versioned
loss-minimizing instruction contract with the selected archive and target
budget, while CLI, iOS, and Android only adapt those inputs to provider-native
messages. Hosts no longer maintain divergent summary prompts.

Lifecycle extension follows the same boundary. Nola has adopted exact event
names, typed progress, cancellation, bounded previews, timeouts, deterministic
ordering, and Trace projection. It does not copy Claude Code's ability to alter
permission decisions or tool input/output, nor OpenCode's mutable system-prompt
plugin Hook. Those would create policy and authorization paths outside Core.
The unfinished work is registry discovery, handler binding, native Hook
execution, and cross-Host execution evidence, not another Hook matcher. Core now
validates exact trusted rule-to-handler bindings and emits a stable registry
digest. ABI v3 publishes the registry builder through the C/Swift and Android
JNI boundaries; configuration discovery and concrete Host bindings remain
unfinished. The persistent CLI/Web Driver now emits `session_start` once before
the first user-prompt Hook without resetting that state between Turns. Normal
shutdown now uses a Core-owned `close_session` submission: Core persists
`session_closing`, emits the exact `session_end` event as a correlated Host
effect, rejects new Turns while closing, and only persists `session_closed`
after the terminal Host response. CLI/Web execute the registered Hook; iOS and
Android explicitly acknowledge the event because they do not yet bind native
handlers. Session-end effects are not automatically replayed after a crash;
their possible side effects require an explicit recovery decision, matching
Codex's shutdown-and-wait discipline without pretending at-most-once delivery.

Permission resource extraction is also schema-driven. External capabilities
derive resource shape from their declared permission domain; only versioned
Nola built-ins use exact capability IDs. Arbitrary names containing words such
as read, write, memory, browser, or search cannot change authorization scope.

Tool availability is no longer duplicated as a comma-separated system-prompt
sentence in the CLI Host. The frozen structured tool schema and capability
catalog are authoritative, which reduces prompt tokens and prevents textual
availability claims from drifting away from executable tools.

Reference-data safety is likewise defined once in the shared identity policy
and enforced again by Core's typed reference-context wrapper. The native Mac
Host no longer injects a duplicate system component for attachment safety;
attachments, retrieval, memory, Skills, and data sources remain lower-trust
components or explicitly delimited user-provided reference blocks.

The reference projects are therefore adopted as mechanisms, not copied as
products: Codex contributes rigor, Claude Code contributes disciplined agent
workflows, OpenCode contributes event/resource composition, and Open WebUI
contributes connector/product surfaces. Nola keeps local-first ownership,
cross-device data sources, scheduled automation, long-term memory and native
clients as its distinct product layer.

## System-prompt composition matrix

Nola now enforces the same trust separation used by the strongest reference
implementations at the native macOS-to-Host boundary:

- product-wide stable policy is composed by Core and participates in prompt
  identity and cache invalidation;
- Host identity is typed as fact, while Host workflow is wrapped as a
  subordinate `host_instruction` that cannot grant permission, add tools, or
  override Core policy;
- the public native ABI and CLI Host composer reject any caller-supplied
  `policy` fragment; only Core can install product policy, including for
  auxiliary compaction requests;
- the default public Rust composition APIs are Host-safe as well; the
  unrestricted policy composer is private to Core's prompt module, so safety
  does not depend on importing a specially named alternative;
- native Swift and Kotlin Host types do not expose a `policy` constructor, so
  ordinary clients cannot accidentally promote workflow text even before the
  defensive ABI validation runs;
- user-selected templates remain `UserInstruction` rather than Host policy;
- memory, attachments, retrieval, MCP output, and platform data remain
  reference context;
- runtime workspace state contains facts only and cannot smuggle additional
  behavioral instructions into the `HostFact` layer.

The Mac client therefore sends typed context components directly. It no
longer flattens every component to a `system` message or injects a second
English workspace-intent prompt. CLI workspace policy retains only
filesystem/platform workflow details; the shared Core action-scope fragment
owns the product-wide distinction between an inline answer and an external
side effect.

| Concern | Codex | Claude Code | OpenCode | Open WebUI | Nola target |
|---|---|---|---|---|---|
| Instruction precedence | Explicit system/developer/user and repository scope | Compact invariant behavior plus reminders | Small agent/provider prompts | Workspace/model configuration projected into requests | Core trust levels with one precedence contract; Hosts cannot promote reference text |
| Stable prefix | Stable operating policy and tool discipline | Cache-sensitive common prefix | Provider-specific minimal prompt | Mostly server/workspace assembled | Core-owned product policy plus model-family overlay, each included once |
| Dynamic suffix | Environment, repository instructions, current tools | Current task state, reminders and modes | Current agent/tool schema | Selected tools, knowledge and workspace | Frozen Turn/Step capability projection, Host facts, user instructions and delimited context components |
| Tool selection | Tool schemas and explicit execution rules | Schema-driven tool use with permission modes | Tool set attached to the selected agent | Selected tool resources | Model selects from exact structured schemas; no client keyword router or textual allowlist |
| External content | Scoped and treated as data | Tool results/reminders remain distinguishable | Parts preserve source and tool identity | Knowledge/connectors are visible resources | Typed reference components retain provenance, authorization, freshness and trust |
| Provider differences | Provider protocol normalized below the agent | Model-generation overlays and cache behavior | Dedicated provider transforms | Model/provider settings in administration | Core model-capability profile chooses overlay and continuation policy; clients do not fork product behavior |
| Localization | Product UI, not duplicated policy variants | Prompt variants are version/model-specific, not a mixed language library | UI localization separate from agent contracts | UI/resource metadata localized | Common agent policy has one semantic source; localized UI strings never become competing system instructions |

### Prompt adoption rules

1. Stable policy belongs in Core and participates in the model-prefix digest.
2. Host overlays may describe surface identity and observed runtime facts, but
   may not restate retrieval, permission, memory, or tool-selection policy.
3. User custom instructions remain `user_instruction`; they cannot grant a
   capability or change the trust of attachments and retrieved data.
4. Skills, MCP, tools, data sources, and memory are projected from typed state.
   Human-readable summaries help the model, but are never the authority.
5. Provider/model differences are selected through a capability profile rather
   than a growing universal prompt full of contradictory clauses.
6. Prompt-cache reuse is valid only when stable policy, model-family overlay,
   exact Step tools, and invocation semantics share the same identity.

## Functional mechanism matrix

| Mechanism | Reference advantage | Nola position | Remaining work |
|---|---|---|---|
| Turn and Item lifecycle | Codex exposes durable started/updated/terminal identities | Core Turn, canonical Parts, Tasks and terminal envelopes | Real disconnect/cancel/replay sessions on every Host |
| Permission suspension | Claude Code keeps the pending operation resumable | Core permission identity and scoped decision projections | Cross-platform custom-response and conversation-grant E2E |
| Capability discovery | OpenCode discovers live MCP tools and reacts to list changes | Core typed catalog plus Host-owned Streamable HTTP and stdio sessions | OAuth and server-originated list-change refresh on every Host |
| Tool effect semantics | Codex separates concrete approval policy, execution, and verification evidence instead of treating command prose as a capability contract | Core consumes a versioned effect declaration from the frozen Host catalog; fixed tools declare their effect, polymorphic CLI commands use a schema-constrained `operation_kind`, and iOS/Android normalize the existing `x_nola.effect` manifest fact. Command text, script names, file extensions, risk labels, and user wording are not classification inputs. Host catalog revision 2 and Turn capability projection schema 2 prevent old checkpoints from masquerading as the same execution contract. | Run adversarial cross-Host E2E proving the declaration cannot grant permission or hide a real mutation |
| Skills | Claude/OpenCode use metadata discovery and load detailed instructions on demand | Built-in and explicit local Skill roots, Core activation projection, and metadata-first native invocation | Connected-host edits/removal, live revision evidence, and real-model cross-platform selection |
| Context and compaction | Codex/Claude preserve resumable history while summarizing | Core accounting, compaction planner, provider continuation and replay | Long real-model conversations with tool results and repeated compaction |
| Memory | Codex separates bounded rollout extraction from single-flight consolidation; Claude distinguishes identity, feedback, project, and reference memory, skips redundant background extraction after an explicit write, and verifies mutable recall | Core applicability/durability/legibility, provenance, citation, lease, retry, usage, and supersession gates; successful structured `memory_save`/`memory_delete` results suppress duplicate extraction for that Turn; CLI/Web/Mac Agent Host has a real background worker; hidden assistant-text memory protocols have been removed from iOS, Android, and Web | iOS and Android worker execution plus cross-device conflict/supersession E2E |
| Background work | Reference agents keep long work observable and cancellable | Core Tasks plus CLI/Mac schedulers, durable receipts, and a SQLite-backed per-session FIFO command inbox shared by claim, heartbeat, progress, terminal update, and cancellation | Reuse the same durable inbox adapter in native Hosts and prove disconnect, process restart, duplicate replay, and descendant cancellation end to end |
| Knowledge/data sources | Open WebUI makes managed knowledge visible | Nola adds local-first authorization, Core-sealed read contracts, freshness and usage provenance | First-connect and persisted conversation-override conformance on all clients |
| Hooks/plugins | Claude/OpenCode expose extensibility around lifecycle events; Codex waits for explicit session shutdown | Core observation-only planning/execution projections, durable close state, correlated `session_end`, CLI/Web execution, and explicit mobile acknowledgement. Explicit conversation deletion on iOS, Android, macOS, CLI, and Web closes the Core session before destroying local data; process exit, cache eviction, branch, and truncate remain resumable Host-detach operations. | Add trusted native handler binding, recovery UI for interrupted side-effecting Hooks, and cross-Host execution evidence before exposing configuration |
| UI observability | Codex Items/OpenCode Parts keep intermediate work inspectable | Native/Web Part renderers and trace journal | Stable, non-flashing progress identity and artifact/file deep links across clients |

This document is the implementation audit for ideas adopted from the reference
projects under `参考资料/`. It distinguishes inspiration from behavior that is
actually owned by Nola Core and projected consistently by every Host.

## Evaluation rules

- A prompt sentence is not a capability. A capability requires a typed contract,
  runtime state, Host execution, a visible result, and regression evidence.
- Core owns model-visible Agent policy, Turn context, Step tool projection,
  permission decisions, checkpoint semantics, and normalized events.
- Hosts own platform I/O and presentation. They must not silently add Agent
  policy, infer tools from words, or authorize operations from natural language.
- User instructions are preferences, not policy and not authorization.
- A feature is marked adopted only when its behavior is represented in the
  current architecture. Tests and real-model sessions remain separate evidence.

## Codex

### System-prompt strengths

- Clear instruction hierarchy: product policy, developer instructions, user
  request, repository instructions, and tool results have distinct authority.
- Tool use is schema-driven. Tool discovery expands the active tool surface
  without treating capability names in a user request as routing commands.
- Operational guidance is concrete but does not claim that work happened before
  a tool result exists.

### Functional strengths

- The Turn is a state machine rather than a single prompt/response call.
- Tool calls, approvals, results, cancellations, compaction, and resumable work
  are first-class events.
- Repository instructions and working-directory scope are explicit inputs.

### Nola adoption

- Core prompt layers and trust levels now distinguish `policy`, `host_fact`,
  `host_instruction`, `user_instruction`, and `reference`; Host-facing
  composition APIs reject `policy` rather than trusting caller labels.
- Turn/Step capability projections are typed and Core-owned.
- Unknown-tool recovery and dynamic Skill/tool activation are protocol behavior,
  not keyword routing.
- Free-form Skill validation steps are workflow guidance only. Structured
  `required_tools` entries can activate catalog capabilities, but neither field
  bypasses the frozen catalog or concrete permission policy. Validation prose
  cannot grant, select, or silently complete a capability; completion follows
  the observed successful tool result and Core's engineering policy.
- The FastAPI compatibility proxy no longer scans message text for retrieval
  markers, rewrites the user's query, or starts an implicit search. Retrieval is
  selected by the model through the active `web_search` schema and evaluated by
  Core like every other concrete tool call; the gateway remains transparent.
- Core capability activation now accepts only the typed Host response field.
  Tool output text, nested `text/output/result/data` values, and legacy marker
  lines are retained only as adversarial test inputs and cannot mutate the
  active catalog.
- Tool execution recovery likewise accepts only a root-level structured
  `tool_execution_skipped: true` receipt. String markers, JSON encoded inside
  strings, arrays, and unrelated nested result values cannot alter Core's
  engineering state.
- Permission evaluation happens on the concrete call, resource, and runtime
  identity.
- Permission denial is Core-owned runtime feedback: an identical or equivalent
  operation cannot be retried or routed through a different tool to bypass it.
- `runtime_state` is represented by the same ABI value on iOS, macOS, Android,
  and Core, keeping workspace, mode, budget, and capability facts outside the
  cacheable policy prefix.

### Remaining evidence

- Real-model sessions must prove natural follow-ups, multilingual requests,
  ambiguous words such as “search” and “current”, and unknown-tool recovery.
- Cancellation, permission suspension, resumption, and compaction must be
  replayed through the same Core protocol on all Hosts.
- Like OpenCode's per-message instruction claims, nested repository guidance
  is now attached on demand to a concrete mutation Step rather than permanently
  inflating every later Turn. A first file mutation against a new instruction
  digest is not executed: the Host returns the applicable directory-chain
  rules as a structured tool result, and a compliant retry may proceed. This
  is resource-driven and does not inspect words in the user's request.

## Claude Code

### System-prompt strengths

- Stable policy is separated from volatile repository and Turn context.
- Safety, truthfulness, tool use, and output style are modular rather than copied
  into every caller.
- User-configured instructions remain subordinate to product policy.

### Functional strengths

- Prompt caching is designed around the exact provider prefix: tools, system,
  then messages. Dynamic UUIDs, timestamps, and tool lists are kept out of the
  stable prefix.
- Permission modes are durable and scoped; an approval is not confused with a
  natural-language confirmation.
- Long tasks expose intermediate work and keep the parent transcript resumable.

### Nola adoption

- Core has a stable prompt prefix and a dynamic Turn suffix.
- Core now owns language/response behavior and visual-content safety as stable,
  independently identified fragments. Host identity is a small surface fact;
  CLI and macOS append only their actual execution workflow instead of sending
  a universal prompt that mixes retrieval, memory, permission, UI, and tools.
- Mobile Hosts no longer recompose Core system messages for main Agent turns.
- Android now asks the Core composer for one complete Agent system prompt before
  starting a Turn. Its surface identity is a `host_fact`; response style and
  custom instructions are `user_instruction`. The Android Runtime forwards the
  real model identifier and Core stable-prefix digest instead of the synthetic
  `android-active-model` identity.
- iOS no longer re-adds the legacy chat policy or memory-tool policy as Host
  `policy` fragments. Core inserts those invariants once; iOS contributes only
  its surface identity, selected templates and response preferences, while
  memory, health, attachments and retrieved data remain `reference`.
- Core Context Engine now renders those trust classes into the actual
  model-visible prompt instead of relying on typed metadata alone. Prompt
  templates, user preferences and activated Skill instructions are enclosed as
  subordinate user-configured instructions; memory, documents, attachments,
  retrieval, MCP resources and platform data are enclosed as non-authoritative
  reference context. iOS consumes that already-scoped output directly rather
  than wrapping it a second time, and it no longer duplicates tool schemas as
  prose in the system prompt.
- The same Core composition now covers tool-free iOS and Android chat paths,
  not only Agent turns. Provider fallbacks no longer define a second product
  policy: surface identity stays a Host fact, selected style/custom prompts are
  subordinate user instructions, and assembled personal/context data is
  reference material.
- Conversation-first behavior, capability-manifest authority, external retrieval,
  memory evidence, and cross-surface continuity are now stable Core policy
  fragments. Web, macOS, iOS, Android, and CLI receive them through the same
  prompt composer exactly once; Host prompts contain only identity and
  surface-specific facts.
- Custom instructions use `user_instruction`; they cannot grant capabilities.
- Core checkpoint and memory state are independent of transient UI state.
- Automatic memory extraction and consolidation now cross a Core-owned quality
  gate: each candidate must be assessed as applicable, durable, and legible,
  while Core independently verifies long-term scope, retention, frozen-source
  provenance, citations, duplicate ids, and supersession boundaries.
- Recalled mutable-state memories remain past snapshots. Core marks them for
  current-source verification instead of presenting them as current truth.

### Contract evidence

- Core derives `model_prefix_digest` from the exact stable system prefix, the
  selected model-family instruction overlay, the active Step tool-schema
  projection, and the model invocation-plan semantic digest.
- CLI, iOS, and Android forward that Core identity to cache-capable providers;
  mobile Hosts no longer fall back to a system-only digest when the complete
  identity is absent.
- Core contracts prove that a one-byte policy change and a changed tool subset
  invalidate the identity, while conversation forks, compaction, projection
  revision changes, and tool-order noise preserve it when the model-visible
  prefix is unchanged.

## OpenCode

### System-prompt strengths

- Provider/model capability differences are expressed by adapters instead of
  leaking into a single universal prompt.
- Session, message Part, tool state, permission state, and compaction are
  modeled separately.
- Project instructions are discovered and scoped as context, not elevated into
  hardcoded product policy.

### Functional strengths

- Message Parts allow thinking, text, tool calls, tool results, errors, and
  lifecycle state to update independently.
- Permission rules are resource-aware and reusable across a session.
- Provider transforms normalize reasoning and tool-call differences.

### Nola adoption

- Core emits Part lifecycle/update events and task/skill/capability projections.
- Hosts render tool calls and thinking from normalized events.
- CLI/Web Host tool results now use bounded model previews plus a typed
  `full_output_ref`. This applies to built-in and MCP tools, so large results do
  not silently consume the Turn window and clients can expose the retained
  output without parsing presentation text.
- Permission scopes and conversation-level decisions are separated from the UI
  prompt that collects the decision.
- Canonical Parts are the sole response-body transport for Agent Hosts. Legacy
  text/reasoning deltas are not emitted alongside Core-derived traces; traces
  remain an independent audit/progress stream.
- Native provider tool streams are keyed by the provider's call index until a
  call id is available. Missing provider ids use a deterministic
  `turn + model iteration + index` identity, and completion reuses that same
  identity. Clients therefore update one Core Tool Part instead of rebuilding
  cards from presentation text or transient provider ids.
- Non-streaming native completions use the same deterministic fallback instead
  of UUIDs, so checkpoint replay, permission suspension, and ToolResult
  correlation remain stable after process restart.
- Provider usage remains lossless across Swift and Kotlin Hosts: input,
  cached-input, cache-write, output, and total token evidence reaches Core with
  explicit reported/not-reported semantics. Context pressure, compaction, and
  cache diagnostics therefore do not depend on client-side message counts.
- Model completion seals the existing Text and Reasoning Parts. If a provider
  buffered a final suffix, Core emits only that suffix before completion; it
  does not replace or recreate the rendered node. This keeps Markdown state,
  expansion state, scroll anchoring, and reveal animation stable.

### Contract evidence

- The shared canonical Part fixture is consumed by CLI, Web, macOS, iOS, and
  Android reducers and covers ordering, revisions, terminal states, tools,
  sources, citations, cancellation, and late events.
- macOS now rejects foreign-session, foreign-Turn, duplicate-cursor,
  out-of-order, and post-terminal Part snapshots before they reach SwiftUI.

### Recovery contract

- CLI Web persists canonical trace events with an append-only sequence and
  resumes from `trace_id + after_sequence` after a broken stream.
- The Web reducer explicitly switches from live Core Part presentation to the
  durable trace journal during recovery. It preserves already visible text,
  removes overlap at the transport boundary, ignores duplicate event IDs, and
  prevents late Part snapshots from rolling the recovered answer backward.
- `web/tests/agent_trace_journal_recovery_test.js` models a live partial Part,
  disconnect, duplicate boundary event, continued text, tool completion, late
  Part, and terminal replay. A real-model disconnect session remains a release
  validation gate rather than an inferred guarantee.

## Open WebUI

### System-prompt strengths

- Workspace configuration, model configuration, tools, prompts, and knowledge
  are visible user-managed resources.
- Extensions have discoverable metadata and explicit enablement state.

### Functional strengths

- Galleries and administrative surfaces make tools, prompts, and knowledge
  inspectable rather than hidden implementation details.
- Provider/model configuration is separated from conversation content.

### Nola adoption

- Skill and MCP resources have catalog/projection contracts and can be exposed
  as editable resources by native Hosts.
- Data sources, scheduled actions, memory, and Skills are treated as composable
  inputs to an Agent Turn.
- A successful data-source test now persists a Core-verifiable, secret-free
  read recipe: method, stable operation id, required configuration keys, JSON
  output schema, freshness, redacted sample, and validation digest. Legacy or
  modified display-only contracts are unusable until the Host retests them.
- Core, CLI/Web, macOS, iOS, and Android now enforce that same v2 usability
  boundary. Persisted v1 records remain decodable for migration, but no Host
  may inject them into model context before a real read reseals the contract.
- Interactive Host catalogs are no longer filtered by capability-name prefixes.
  MCP catalogs require a positive revision, declared protocol versions, exact
  descriptor identity, and rejection of same-revision content drift; unknown
  tools remain a recoverable catalog-refresh checkpoint rather than a prompt fix.
- Mobile native tool streams now preserve the same ordered calls in both delta
  Parts and the final model completion, so Core execution, replay, and recovery
  do not depend on transient UI events.
- The CLI-hosted Web opt-in real-model suite reuses the saved CLI configuration
  and covers multilingual natural requests, an ambiguous cross-language
  follow-up, autonomous tool selection, injected unknown-tool recovery, tool
  result return, cancellation, persistence, and the final answer.
- Local-first execution and platform permission brokers remain Nola-specific;
  Nola does not adopt Open WebUI's server-centric ownership model.
- macOS now distinguishes discovery from activation for workspace/global
  Skills. A discovered `SKILL.md` is inactive by default; explicit enablement
  is persisted and the exact enabled directories are frozen into the next
  Agent Host request alongside built-in Skills. The gallery and executable
  Skill roots therefore use the same state instead of maintaining shadow
  catalogs.
- CLI-hosted Web derives each MCP catalog revision from canonical enabled
  descriptor content. Editing a server, tool, resource, template, or prompt
  changes Core's catalog identity, while JSON object key order does not. This
  preserves same-revision drift rejection without a constant revision.
- A local MCP configuration file discovered by macOS is shown as discovered,
  not as an enabled callable capability. It must complete a typed Host
  handshake and enter the Core catalog before the product may claim it is
  active.

### Remaining evidence

- Native galleries must still prove connected-host MCP handshake, inspection,
  enable/disable, removal, and live catalog-refresh behavior. macOS does not
  own a second Swift MCP executor: it freezes explicitly enabled local config
  into the `nola-agent-host` launch request, then inherits the retained
  HTTP/stdio session, discovery, invocation, and Turn-boundary refresh path
  used by CLI/Web. Connected MCP activation remains a release gate rather than
  a UI toggle claim.
- Data-source tests must prove initial connection testing, persisted read shape,
  freshness policy, default enablement, and per-conversation overrides.

### MCP lifecycle comparison and required boundary

The Rust Web Host now completes the first executable remote-MCP vertical
slice rather than stopping at discovery:

- settings save performs `initialize`, `notifications/initialized`, and
  paginated capability discovery;
- endpoint and credential headers remain in a Host-only connection registry;
- Core receives only typed, versioned descriptors and opaque
  `connection_ref` values;
- each available MCP tool is projected to a stable provider-safe capability
  identity, while Host freezes the exact identity-to-server/tool route;
- model calls execute through `tools/call`; no user text, localized keyword,
  or tool-name prefix is parsed to choose the route;
- destructive MCP annotations create an explicit permission boundary, while
  read-only calls can execute concurrently and replay idempotently.

The Host retains one isolated Streamable HTTP session per configured server.
Calls to the same session are serialized while different servers remain
independent. Editing an endpoint, timeout, or credential header atomically
replaces the connection and discards its old session. A transport or protocol
failure invalidates only that server session; the next independent invocation
may initialize a new session, but the failed tool call is never automatically
replayed because its remote side effect may already have happened.

Server-originated `tools`, `resources`, and `prompts` list-change notifications
mark retained HTTP and stdio catalogs stale. The active Turn keeps its frozen Step
projection; CLI-hosted Web re-handshakes before the next Turn, derives a new
canonical revision, and gives Core the refreshed descriptors. SSE decoding
selects a response by JSON-RPC id instead of assuming the final event is the
response, so a trailing notification cannot corrupt a valid call. OAuth renewal
and equivalent native-client refresh evidence remain follow-up work; they must
build on this typed route rather than introduce a second protocol.

Local stdio MCP follows the same Host-owned lifecycle. Claude Code and
OpenCode configurations that declare `command`, `args`, `env`, and an optional
working directory are launched directly as argv, never through a shell. The
Host retains one isolated child process per enabled server, performs the same
typed initialize/discovery handshake, bounds stderr diagnostics, and routes
`tools/call` through the frozen connection identity. macOS freezes the config
root as `cwd` when the server does not declare one, so relative commands do not
depend on the app's launch directory. A broken process is discarded without
replaying the in-flight tool invocation.

macOS now uses the same boundary rather than adding a Swift-only MCP stack.
The native Extensions gallery discovers workspace and user-level Claude,
OpenCode, and `.mcp.json` registries, including JSONC comments and trailing
commas. Discovery remains inactive by default. Enabling one exact server
persists that resource identity; a new chat run freezes only the enabled raw
configs into the authenticated desktop Host request. `nola-agent-host` then
performs discovery, builds the typed Core catalog, and owns invocation. Secret
headers do not enter prompts, context components, Core checkpoints, or UI
extension models.

The reference implementations expose an important distinction that a gallery
alone cannot provide:

| Stage | Claude Code / OpenCode mechanism | Current Nola mechanism | Decision |
|---|---|---|---|
| Configuration | Scoped local/remote server configuration | Web endpoint records and discovered macOS config files | Keep configuration separate from runtime truth |
| Connection | Explicit connect, disconnect, reconnect and timeout paths | No complete native Mac/Web MCP connection manager | Host must own a real connection state machine |
| Authentication | OAuth discovery, callback state, expiry and re-auth states | Typed descriptors can carry metadata, but there is no complete Host auth lifecycle | Add Host-owned credential/auth flow; never put secrets in Core |
| Discovery | List tools, prompts, resources and templates only after connection | Core accepts already-discovered typed descriptors | Preserve Core descriptor validation; populate it only from a successful handshake |
| Hot update | Tool-list-changed notification refreshes the live catalog | Core supports catalog refresh and same-revision drift rejection | Host must translate server notifications into a new content revision and availability epoch |
| Invocation | A live client is retained and the call has an in-progress/completed/failed item | Core can model MCP capability identity and Parts, but a saved endpoint is not itself an executor | Route calls only through a live Host client bound to the descriptor revision |

This yields a strict product rule: **configured**, **connecting**,
**needs authentication**, **connected**, **failed**, and **disabled** are
different states. Only `connected` servers with a successful capability-list
handshake may contribute callable descriptors to Core. A UI toggle may request
connection; it cannot directly manufacture an enabled capability. macOS and
CLI-hosted Web still need this Host broker before their local MCP galleries can
claim end-to-end MCP support.

Until that broker is present, CLI-hosted Web creates endpoint records disabled
and rejects an attempt to persist an enabled record that is not already a
typed, handshake-derived descriptor. This is deliberately fail-closed: it
prevents a settings-only URL from poisoning the next Agent Turn while keeping
the missing connection lifecycle visible as product work.

CLI-hosted Web now owns the first half of that broker for remote Streamable
HTTP servers. Enabling or editing MCP settings performs a real `initialize`
handshake, sends `notifications/initialized`, follows bounded cursors for
tools, resources, resource templates, and prompts, accepts JSON or SSE
responses, captures `Mcp-Session-Id`, classifies authorization separately, and
persists the resulting typed descriptor only after discovery succeeds. Live
invocation and session retention are implemented in the shared Agent Host,
including the path used by macOS. The remaining work is OAuth completion,
broader server-notification coverage, and real connected-server product E2E.

### Model metadata and provider adaptation

Codex treats model metadata as a versioned contract and makes an unknown-model
fallback visible. OpenCode likewise adapts provider wire formats at one
boundary instead of assuming every OpenAI-compatible endpoint supports the
same reasoning and tool protocol. Nola previously violated both principles:
the CLI declared every configured model to support reasoning continuity and
parallel tool calls, while Core could later reinterpret an intentionally
generic profile from a model-name substring.

Nola now freezes one Core-owned model capability profile per Turn. Exact known
provider identifiers take precedence, bounded model-family prefixes are used
only as a compatibility fallback, and unknown deployments receive a named
`generic.fallback` profile. That fallback preserves basic streaming and serial
native tool calls but does not claim reasoning, reasoning continuity, or
parallel tool support. The resolution source and revision are part of the
invocation identity, and the runtime is forbidden from guessing again after
the Turn starts. Provider-specific message repair remains a Host transport
responsibility; user language never participates in model capability
selection.

The persisted provider fact now travels request by request in `LlmConfig` and
cannot be overridden by `NOLA_LLM_PROVIDER` or `NOLA_CLI_LLM_PROVIDER`.
Crucially, Nola no longer conflates that value with model ownership:
`provider=openai` may mean the OpenAI-compatible transport for a DashScope,
DeepSeek, or local deployment. Core selects the instruction family from a
specific vendor declaration or bounded model family before falling back to the
generic transport. The Host independently selects the wire flavor from a
specific provider declaration and endpoint before its model fallback. This
keeps prompt compatibility and request serialization aligned without forcing
them into one misleading enum.

Like Codex's versioned model metadata, Nola now also accepts an explicit
`llm.capabilities` contract from persisted configuration. It can override the
instruction family, native/parallel tool support, reasoning continuity,
request continuation bounds, and stream channels without adding another model
name heuristic. An explicit contract receives its own profile id and revision,
so cache identity, replay, and debug traces distinguish configured metadata
from provider/model fallbacks. The main Host's thinking request flag is derived
from the same capability resolution, so an unknown or explicitly
`reasoning=false` model cannot receive vendor thinking parameters while Core
claims reasoning is disabled.

### Item and Turn lifecycle comparison

Codex exposes Thread, Turn, and Item as separate identities. Command execution,
file changes, MCP calls, reasoning, Web search, plans, and errors each have
their own started/updated/completed or failed state; Turn completion carries
provider usage rather than replacing the Item history. Nola's Core Parts and
Task projections now follow the same useful mechanism while remaining more
general across native clients. The remaining release criterion is behavioral:
Hosts must render one stable Part identity through progress and completion,
not synthesize a second final bubble or collapse intermediate work into a
status strip.

## Nola target architecture

| Concern | Authoritative owner | Host responsibility |
|---|---|---|
| Stable Agent policy | Core | Supply typed platform facts |
| User preferences | Core prompt composer | Persist and submit as user instructions |
| Turn context | Core context coordinator | Collect authorized source payloads |
| Active Step tools | Core capability projection | Execute only frozen projected calls |
| Permission | Core decision + Host broker | Present request and return typed decision |
| Memory/checkpoint | Core runtime | Durable storage and synchronization |
| Tool/thinking UI | Core normalized events | Render without inventing state |
| Provider differences | Provider adapter | Transport exact canonical input |
| Prompt cache identity | Core | Forward unchanged to provider |
| Prompt cache evidence | Provider adapter -> Core Trace | Render reported read/write tokens and bounded diagnostics without client inference |

macOS, iOS, Android, and CLI Web now consume that row literally: the Core
status is shown as the diagnosis, while read/write token counts appear only
when reported by the provider. macOS stores the evidence in the conversation
context snapshot; iOS and Android store it in assistant-message metadata, so
restoring a conversation does not replace evidence with a client estimate.
The terminal CLI reads the same diagnosis from the Core checkpoint and exposes
it only in the wide debug footer; it does not infer status from counters. All
Hosts still need cross-provider, cross-session evidence.

## Acceptance matrix

The adoption is complete only when all rows have contract and real-session
evidence.

| Scenario | Required proof |
|---|---|
| Ordinary answer | No tool is forced by words in the request |
| Dynamic Skill | Activation changes the Step projection exactly once |
| Unknown tool | Recoverable protocol path; no language-specific fallback |
| Permission once/conversation | Later matching calls do not reprompt |
| Custom response | Turn suspends and resumes through normal user input |
| Compaction | LLM summary plus deterministic retention; transcript replayable |
| Long task | Thinking/tool progress visible; final text ordered once |
| Cache reuse | Identical system/tools prefix reuses one Core identity |
| Cache invalidation | Tool projection or stable policy change alters identity |
| Cross-platform replay | Same fixture yields equivalent Parts and terminal state |
| Memory | Cross-session recall is relevant, inspectable, and independently mutable |
| Memory write quality | Weak/session-only candidates fail before job completion; citations resolve to the frozen input |
| Data source | Authorization, freshness, schema, and conversation override persist |

## Non-adoptions

- No prompt-keyword tool router.
- No Host-specific shadow Agent runtime.
- No approval inferred from “yes”, a localized phrase, or a capability name.
- No opaque tool success synthesized from model prose.
- No server-only architecture that makes local core behavior unavailable offline.
- No duplicated product policy in CLI, Web, macOS, iOS, or Android prompts.
- No platform prompt loader may shadow Core retrieval, memory, permission, or
  capability-selection policy; platform overlays are identity and presentation
  facts only.

## 2026-08-04 incremental protocol adoption

### Model-visible trust boundaries

- Core now renders user-configured instructions and untrusted reference material as distinct, explicit scopes. Prompt templates, preferences, and Skills cannot silently become platform policy; memories, documents, attachments, Web results, MCP content, and platform data remain reference context rather than commands.
- Ordinary iOS and Android chat and Agent turns now use the same Core prompt composer. Platform identity remains a Host fact, while response style and custom prompts remain user instructions. The legacy shared prompt is fallback-only when Core FFI is unavailable.
- The separate `build_local_agent_prompt` stack has been removed from Core, the native ABI, and Android. A future local inference runtime must accept the same canonical messages, concrete tool schema, invocation plan, and Core digest as remote providers; it cannot regain prompt ownership by flattening the Turn.
- Prompt composition now emits a redacted internal `prompt.composed` Trace. It contains only a validated stable-prefix SHA-256 digest, bounded fragment identifiers, and the original fragment count; prompt bodies, user text, and secrets are never serialized or shown in ordinary chat UI.
- A raw `System` context component remains a privileged Host channel for a complete Core-composed prompt. Hosts must not place retrieved or user-authored content in that channel.

### Frozen MCP catalogs with recoverable refresh

- Retained HTTP and stdio MCP clients recognize `notifications/tools/list_changed`, `notifications/resources/list_changed`, and `notifications/prompts/list_changed`.
- Catalog changes are applied before the next Turn, never during the active Turn. The active Turn keeps its frozen capability projection and exact catalog revision.
- HTTP list-change notifications are currently observed on JSON-RPC POST responses. A dedicated long-lived Streamable HTTP GET notification channel remains a future interoperability improvement.

### Native model stream identity

- OpenAI-compatible native tool streams are keyed by the protocol call index, not by an initially optional call ID. A stable fallback ID is preserved for the full logical call.
- Arguments that arrive before the capability name are buffered and emitted once the capability can be validated against the frozen tool projection. This prevents split calls, duplicate tool Parts, lost arguments, and completion-time surprises caused by provider-specific chunk ordering.
- The regression contract covers a first chunk with no call ID or capability name followed by a later identifying chunk.

### Executable MCP resources and prompts

- Core exposes three bounded aggregate capabilities, `mcp_resources_read`,
  `mcp_resource_templates_read`, and `mcp_prompts_get`, rather than expanding
  every advertised resource, template, or prompt into a model tool.
- Requests use typed, deny-unknown-fields contracts. The Host resolves `server_id + resource_id` or `server_id + prompt_name` against the frozen catalog before translating them to MCP URI/name calls.
- Prompt arguments are checked for required and unknown keys; unknown capabilities produce recoverable catalog errors instead of being repaired through text matching.
- Execution dispatches through the transport recorded in the typed server descriptor and supports retained Streamable HTTP and stdio sessions. Credentials and connection details remain Host-owned.
- Resource templates are executable through bounded simple `{variable}` URI
  expansion and the same retained Streamable HTTP or stdio session used for
  static resource reads. Unsupported RFC 6570 operators fail explicitly;
  Host connection configuration now remains separate from the discovered
  descriptor. In particular, stdio command/args/cwd/env survive discovery so a
  persisted server can reconnect after process restart; `connection_ref` alone
  is never treated as durable connection configuration.
- CLI-hosted Web and macOS Connected Host presentation now retain the actual
  discovered tools, resources, resource templates, prompts, protocol version,
  and availability state. The catalog snapshot is inspectable but read-only;
  users edit Host connection configuration, then discovery replaces runtime
  truth. A local macOS config that has not completed a handshake is explicitly
  shown as configured rather than connected.
- iOS no longer opens MCP endpoints or injects an ad-hoc capability summary as
  reference context. It consumes the Agent Host's frozen descriptor, including
  tools, resources, resource templates, prompts, availability, and protocol
  version, and presents that catalog read-only. This removes a native routing
  and permission bypass while preserving inspectability.
  Legacy SSE and in-process transports remain unsupported.

## 2026-08-04 Runtime and E2E hardening update

This validation pass closed the remaining gaps between the adopted Core model and the CLI/Host adapters:

- **Canonical child output:** background and delegated agents now consume `agent_trace` (`assistant.delta` / `assistant.reset`) as the authoritative result stream. The suppressed legacy delta is retained only as a compatibility fallback, preventing both missing child results and double delivery.
- **Evidence-driven progressive disclosure:** broad-repository guidance is activated after a real structured `project_overview` result crosses the size threshold. It is not selected from user-language keywords or a separate semantic classifier.
- **Explicit provider capabilities:** E2E fixtures that return parallel tool calls declare `parallel_tool_calls`; runtime behavior no longer silently contradicts the model capability profile.
- **Core-owned permission normalization:** user-facing shell command globs are converted by Core into a versioned canonical resource matcher. A rule such as `pwd` or `git status*` therefore matches the structured `{command,cwd}` resource consistently across CLI, Web, and native Hosts without serialized-JSON heuristics.
- **Deterministic terminal barriers:** PTY tests wait for `command output -> subsequent composer activation` ordering. Fixed sleeps and global redraw counts are not valid readiness signals because background task notifications may redraw the composer independently.

## 2026-08-05 Core/Host projection ownership update

### Canonical presentation ownership

Codex keeps commands, MCP calls, reasoning, messages, file changes, and errors
as stable Items. OpenCode uses stable message Parts and tool-call identity. Both
avoid rebuilding the visible response from a second, append-only diagnostic
stream. Nola now applies the same boundary across its Hosts:

- Core Part is the only source for visible assistant text, reasoning, tool
  state, permission rationale, sources, citations, and artifacts.
- Core Part is also the source persisted into the conversation transcript.
- Agent Trace remains a sequenced diagnostic/control stream for run phase,
  usage, cancellation, permission transport, recovery evidence, hooks, and
  debugging. A Trace delta must not race or duplicate a Part snapshot.
- macOS and Web project live content directly from Part identity. iOS and
  Android retain Core Part lifecycle events from Agent Host and reduce them
  locally. The CLI terminal now renders text and tool transitions from the
  same Part updates used by native Hosts rather than from Trace.
- iOS compatibility summary streaming now reduces Core Part snapshots and
  uses prefix deltas only as an output adapter. It no longer ignores Part and
  reconstructs the answer from Trace.
- Waiting tool cards on macOS, iOS, and Android use the permission rationale
  carried by the canonical ToolPart; they do not derive user-facing detail
  from a transient Trace label.

This is a protocol ownership rule, not a UI optimization. Trace-only body
reconstruction is permitted only when inspecting legacy diagnostic journals;
it is not a live response path.

All client reducers now default Trace presentation projection to disabled.
Callers cannot accidentally regain a second visible-content path merely by
omitting an option; Trace continues to update diagnostics, usage, run terminal
state, and permission control state.

### Host-owned MCP visibility

iOS and Android no longer open configured MCP endpoints or inject locally
assembled MCP summaries. Agent Host owns MCP connection, initialization,
catalog revision, availability, and execution. Mobile clients receive a
read-only catalog containing tools, resources, resource templates, and prompts,
including stdio services without an HTTP endpoint. Mac and CLI-hosted Web use
the same configuration/catalog separation.

The CLI/Web Agent Host no longer rebuilds or widens the model-facing tool set
after Core has issued an `AgentModelToolProjection`:

- Core is the sole owner of the per-Step schema list, activation state, and
  `model_prefix_digest`; the Host forwards that exact schema list to the provider.
- Built-in capabilities are declared once as initially visible or deferred in
  the frozen Host catalog. This is static capability metadata, never a decision
  derived from user words, language, or conversation history.
- `tool_search` and Skill execution return typed activation metadata. Core
  applies `CapabilitiesActivated` to the next Step; the Host no longer keeps a
  parallel set of discovered tool IDs or parses tool output to infer activation.
- Protocol capabilities needed by Core-owned workflows, including output
  navigation, planning, goals, background-task control, and workspace status,
  are explicitly visible at Turn start. Domain tools remain deferred until Core
  activates them.
- Regression coverage proves catalog visibility/deferred discovery, rejects
  Host-side schema reconstruction, passes all 23 PTY product-path scenarios,
  and passes both CLI binary suites (`660 passed`, `1 ignored` each).
- The macOS product-path suite passes all four selected tests through the Agent
  Host. Its test target was also repaired to import the actual `Nola` product
  module and to pass structured context components instead of reconstructing
  obsolete Host messages.
- Core now closes the response side of the effect protocol. A model request may
  emit only model deltas, tool-call deltas, continuation state, retry progress,
  completion, acceptance, or failure; each other request kind has an equally
  explicit response family. A correlated but semantically wrong response is
  rejected before sequence or Turn state advances.
- Envelope ownership validation precedes payload-family validation, so a stale
  lease or wrong request cannot probe the pending effect's legal response set.
  The regression test also proves that rejecting a wrong payload does not
  consume the sequence number needed by the later valid response.
- Full verification after this change passes the Core suite, all 23 CLI PTY
  scenarios, both CLI binary suites (`660 passed`, `1 ignored` each), and all
  four selected macOS Agent Host product-path tests.

Validation completed on this revision:

- Core command-pattern contract: passed.
- Focused delegated-agent, task-scoped permission, and unified background-shell E2E scenarios: passed.
- CLI PTY suite: `23 passed, 0 failed` (serial execution, about 34 seconds after build).
- CLI full Rust suite: `660 passed, 0 failed, 1 ignored`, followed by all integration targets and the PTY suite passing.

The earlier hour-scale execution was not a single long inference. It combined first-build cost, multiple real-LLM/skill workflows, and tests that waited on UI symptoms without per-stage readiness barriers. The current suite isolates compile time from scenario time and reports exact protocol stages, so a stalled provider, tool, permission, background task, or composer transition is attributable instead of appearing as one opaque timeout.

## 2026-08-05 Typed provider history and prompt identity update

- Codex keeps messages, reasoning, tool calls, and tool outputs as distinct `ResponseItem` variants with stable item/call identity. OpenCode likewise retains message Parts and `toolCallId` through provider conversion, applying provider-specific degradation only at the transport edge.
- Nola's iOS Host now sends the actual Core prompt composition digest and ordered fragment IDs in `turn_identity.prompt`; model identity is no longer `null`, and turns without prepared context receive the same composed system identity rather than an untyped user-only request.
- The macOS Agent Host remains the sole owner of production prompt composition. The native MLX loopback now decodes canonical OpenAI messages with tool call IDs, names, arguments, reasoning metadata, and tool-result correlation. Because MLX Swift history currently exposes only role/content, assistant tool calls are converted only at that final adapter into the model's standard tool-call text form instead of disappearing as empty assistant messages.
- No prompt body, user query, reasoning text, or secret is added to prompt-identity traces. Routing and authorization remain Core decisions over concrete typed calls; none of these changes use language-specific keyword matching.
## Adopted: out-of-order capability activation

Reference agents treat authoritative runtime events as journal entries whose delivery order is independent from view construction. Nola now applies the same rule to Core capability activation:

- no client-side keyword inference or prompt rewriting is involved;
- Core cursor and stable protocol identity determine idempotency;
- early events are retained in bounded client journals rather than dropped;
- Web's public reducer now recognizes canonical Core envelopes directly instead of accepting only flattened transport projections;
- Android conversation switching no longer loses capability/tool disclosure before final message persistence.

The expanded regression run also repaired an SSE transport contract: an injected Ktor client without `HttpTimeout` can now execute streaming requests instead of failing before the request is sent. Timeout overrides are applied only when that client actually provides the plugin.
## Adopted: precise Host protocol rejection

The reference implementations separate protocol failures from model-visible tool failures and user-facing terminal outcomes. Nola previously returned `UnknownOrDuplicate`, which made a network replay indistinguishable from a response for an effect Core never issued.

Core now retains a bounded identity-only tombstone for completed Host effects and emits stable rejection codes. Duplicate, unknown, wrong-request, wrong-response-type, wrong-owner, wrong-turn, stale-catalog, stale-lease, and non-monotonic-sequence paths are independently testable. The terminal exactly-once contract is preserved: classification improves recovery and diagnostics but does not silently accept a second terminal response.

Verified against the full Core suite and both CLI binaries. The native ABI now emits stable rejection codes/categories, and Swift plus Android retain them as typed errors. CLI/Web diagnostic presentation remains open; clients must not recreate Host policy by parsing localized display strings.

## 2026-08-05: Durable memory lifecycle adoption

The Codex-style two-phase memory lifecycle is now implemented through the shared Core/Host boundary rather than as a CLI-only heuristic.

- Core owns claimability, lease/retry state, single-flight consolidation, supersession filtering, usage-aware candidate ordering, immutable input watermarks, and output validation.
- Hosts execute provider I/O only. Extraction and consolidation run in an isolated worker process; API credentials travel through stdin and results return through an atomic per-session inbox.
- One-shot Hosts place workers in a separate process group so a slower consolidation survives foreground process exit. Mac uses the same path because `nola-agent-host` includes the shared Host entrypoint.
- `memory_search` results carry hidden typed usage metadata. Core records stable memory IDs, citations, time, session/turn identity, and a payload digest, then removes the metadata before the result reaches model context or UI.
- Consolidation preserves old rows and writes `supersedes` edges. Retrieval excludes superseded records, so rollback remains possible and provenance is retained.
- Host persistence is preceded by Core's non-mutating output validation. Completion uses the frozen job input watermark, preventing invalid output or half-committed lifecycle state.

Validation evidence:

- Core full suite: 150 primary tests plus all auxiliary suites passed.
- CLI full suite: 663 passed, 1 ignored.
- Real-provider recall E2E: a fresh conversation autonomously called `memory_search`; the Core checkpoint recorded the exact memory ID, citation ID, payload digest, and usage count.
- Real-provider consolidation E2E: Core selected 8 live records, the worker produced 2 consolidated records superseding 6 inputs, the next safe Host boundary imported them, and the inbox result was consumed exactly once.
- Negative E2E: with tools explicitly disabled, the model could not retrieve memory and Core recorded no usage, preventing fabricated recall.
## 2026-08-05 source-verified corrections

This section supersedes older status statements when they conflict. It is based on the current reference sources and the current Nola worktree rather than feature names or UI presence.

### System-prompt architecture

| Project | Source-verified strength | Nola status | Adoption decision |
| --- | --- | --- | --- |
| Codex | A concise role prompt is paired with code-owned tool registration, sandbox assessment, approvals, compaction, and session events. Safety is not delegated to prose alone. | Core owns immutable safety, evidence, permission-feedback, semantic-tool-selection, retrieval, memory, continuity, completion, and response-style fragments. Hosts add identity and surface workflow only. | Keep Core policy small and deterministic; put enforcement in typed runtime state and Host effects. |

Android no longer falls back to a client-authored shadow Agent policy when the
Core prompt-composer ABI is unavailable; composition fails explicitly. iOS and
Android classify user custom instructions as scoped user execution instructions,
not as product policy or mere visual style. iOS also no longer prepends a magic
`ENABLE_RETRIEVAL` phrase to the user's message. Retrieval enablement remains
typed turn state, and the original query is preserved byte-for-byte.

The CLI Host also removes the dormant `force_named_tool` helper and its
browser-specific forced-sequence test. Browser connectivity is an execution
result, not a signal for the Host to narrow the model's schema from user text.
The model selects from Core's frozen projection and receives recoverable,
typed failure feedback when the Chrome bridge is unavailable.

The Host no longer carries an always-false `validation_requested` semantic
flag or rejects an already-selected Bash call as `validation_not_requested`.
That flag had no typed authorization source and made explicit validation
requests impossible. Core now evaluates the concrete command purpose,
resource, permission, and result; the Host executes the approved call without
reclassifying the user's intent.
| Claude Code | Prompt behavior is assembled from narrowly scoped fragments for action safety, tool use, task agents, memory, background work, browser use, and Skills. | Nola now has a Core-owned task-completion fragment in addition to the existing evidence and action-scope fragments. Model-family overlays remain separate from product policy. | Continue modular fragments; do not copy the large prompt corpus or expose hidden reasoning. |
| OpenCode | The provider/model selects a compatible base prompt; the active Skill catalog is described to the model, and the model invokes the Skill tool. | The obsolete Agent Host scope classifier and its Skill/browser/mutation preselection have been removed. The main model selects only from the Core-frozen tool projection. | Do not reintroduce a preflight routing model or language-specific routing lists. |
| Open WebUI | Prompts are configurable around a multi-user model/knowledge platform, with server-side policy and RAG configuration. | Nola keeps product policy immutable and treats user templates, retrieved data, memory, attachments, and Skills as scoped non-authoritative inputs. | Adopt inspectability and provenance, not mutable replacement of Core safety policy. |

### Functional architecture

| Capability | Reference evidence | Current Nola evidence | Remaining work |
| --- | --- | --- | --- |
| Permission suspension | OpenCode stores pending requests as deferred state and resolves `once`, durable approval, rejection, and corrected feedback. Codex separately tests sandbox and approval policy. | Core checkpoints pending requests, explains rule matches, distinguishes turn/conversation/global sources, rejects interactive grants for scheduled/replayed/delegated origins, and resolves matching concurrent requests after a conversation grant. | Run the same real interaction suite through CLI, Mac Agent Host, iOS, Android, and Web; UI presence alone is not proof. |
| Skill selection | Claude Code and OpenCode expose Skill descriptions and load exact Skill content only after model selection. | Core freezes the capability projection and Skill revision. Agent Host no longer performs a second LLM classification or forces a selected Skill schema. | Verify multilingual and natural follow-up selection with real models; keep direct atomic tools available when no Skill workflow is needed. |
| MCP resources | Claude Code exposes exact MCP resource reads and prompts. MCP discovery includes parameterized resource templates. | Core now projects static resource read, parameterized resource-template read, and prompt-get capabilities from the frozen catalog. Agent Host validates exact server/template IDs, bounds variables, percent-encodes simple `{variable}` expansion, and calls `resources/read`. | Add full server-notification catalog refresh, OAuth lifecycle, and broader RFC 6570 operator support before claiming complete MCP parity. |
| Knowledge and data sources | Open WebUI provides server-side knowledge collections, directories, embeddings, user/group grants, and read-only external knowledge. | Nola provides local-first source policies, per-conversation overrides, Core-sealed reader recipes, authorization/freshness decisions, redacted samples, schema identity, and usage provenance. Mac, iOS, and Android now consume the Core resolver rather than reimplementing the decision rules. | Prove first-connect, persisted conversation overrides, stale-data refresh, and fail-closed behavior with cross-platform fixtures and real Host reads. |
| Prompt caching | Claude Code documents byte-stable prefixes and cache breakpoints; OpenCode selects stable provider-family prompts. | Core separates a deterministic stable policy prefix from runtime/reference suffixes, computes cache identities, records provider-reported read/write tokens, and emits one bounded diagnostic through the canonical usage Trace. iOS, Android, CLI/Web, and Mac preserve that evidence. | Finish product-facing presentation and prove provider cache reads in real sessions; a stable digest or `stable_candidate` diagnosis is still not proof of a hit. |
| Long-term memory | Claude Code distinguishes identity, feedback, project, and reference memory, rejects derivable/ephemeral content, and verifies mutable recall. Codex runs bounded per-rollout extraction separately from single-flight consolidation with leases, retry backoff, secret redaction, watermarks, and usage-ranked retention. | Core owns memory selection, provenance, freshness, usage ledger, extraction jobs, consolidation candidates, supersession, validation, and completion watermarks. The CLI worker uses structured tool output, bounded redacted transcripts, atomic result files, leases, and retries; CLI-hosted Web and Mac inherit that implementation. Core's stable memory policy rejects derivable workspace/history facts, unconfirmed assistant claims, secrets, and transient work, preserves why/how context, respects disabled/ignored memory, and requires current-state verification before relying on mutable recall. Every Host now supplies the Turn observation time; Core records each typed memory that survives budgeting exactly once, while legacy text context cannot enter the usage ledger. iOS and Android retain terminal checkpoints as durable Session state. Core exposes claimable jobs and builds each Worker Plan directly from its private canonical transcript and selected records; Hosts pass only `job_id`. Hidden reasoning and raw tool arguments are excluded, secret-shaped values are redacted centrally, and completion atomically updates both Job and recallable memory projections. Semantic candidates no longer control IDs, provenance, retention, citations, or supersession; Core materializes those fields and requires extraction evidence to include a frozen user message. Native mobile Hosts now retain a bounded per-Session Runtime authority outside the UI Coordinator and rebuild it from the latest checkpoint only when configuration changes. Their Runtime bridges expose claim, heartbeat, completion, and retry submissions with Core ownership tokens and watermarks. | iOS and Android still need an independent scheduler/model Worker that acquires the retained Runtime's single-writer lease and drives those transactions. Then run cross-device conflict, supersession, lease-expiry, crash-recovery, and real-model quality E2E before claiming parity. |

### Explicit non-adoptions

- Do not copy Open WebUI's server-centric multi-user ACL into the local personal data path. Use it only for optional shared/team knowledge services.
- Do not copy reference prompts wholesale. Their product identity, tool names, sandbox assumptions, and UI protocol are not Nola policy.
- Do not select tools, Skills, memory, Web retrieval, MCP, or permissions from keywords, file extensions, platform names, or a separate preflight classifier.
- Do not treat a rendered catalog, a test fixture, a stable digest, or a successful discovery request as proof that execution, recovery, persistence, or UI replay works end to end.

### Evidence gates for the next phase

1. Compile Core and CLI/Agent Host after the prompt and MCP contract changes.
2. Run Core MCP catalog/model-projection tests and Agent Host template-route tests.
3. Exercise one real stdio or Streamable HTTP MCP resource template through the Mac Agent Host and CLI/Web host.
4. Capture provider request counts to prove ordinary Skill selection uses one main model transaction and no hidden scope-classification request.
5. Run persisted cross-language data-source fixtures against Core ABI v3,
   including first-connect sealing, conversation overrides, stale refresh, and
   unavailable-Core fail-closed behavior.

## 2026-08-05 prompt protocol and deferred capability adoption

This update compares mechanisms rather than prompt wording. The authoritative
reference points are Codex's generated permission instructions and deferred
tool search, Claude Code's cacheable system-prompt sections and structured tool
orchestration, and OpenCode's provider overlays plus catalog-driven Skill tool.

| Mechanism | Reference strength | Nola adoption in this update | Evidence status |
| --- | --- | --- | --- |
| Permission interaction | Codex derives model instructions from the effective sandbox and approval configuration instead of asking the model to infer policy. OpenCode suspends the concrete request and resumes it from a typed decision. | Core stable policy now requires the model to submit the concrete structured call, forbids conversational permission negotiation, and defines pause, allow-once, denial, and custom-response continuation semantics. Actual authorization remains exclusively in `PermissionRuleSet`. | Core contract assertions were added; compilation and interaction E2E remain pending. |
| Skill loading | Claude Code and OpenCode advertise bounded Skill metadata, then load exact instructions only after a model-selected Skill call. | Core stable policy now defines Skills as discoverable workflow references that cannot add tools, permission, authority, or success evidence. Selection must use the active structured catalog and a declared Skill capability. | Prompt-composition assertions were added; multilingual real-model selection remains pending. |
| Deferred tools | Codex constructs searchable text from names, descriptions, and JSON Schema and returns bounded deferred schemas. | `HostCapabilityCatalog.search_available_tools` now ranks only the frozen Host catalog using capability ID, title, schema field names, and schema descriptions. `Visible` and `Deferred` tools are discoverable; `Internal` and `Hidden` tools are excluded. Unicode text is preserved without language-specific routing rules. | Core unit cases cover Chinese metadata, exact-ID precedence, result bounds, deferred inclusion, and internal exclusion; tests have not yet been run. |
| CLI activation | Codex separates discovery from activation; a search result does not silently execute the tool. | Agent Host injects Core-ranked IDs only after Core validates the `tool_search` input. CLI merges static and native dynamic schemas in Core order, returns activation metadata, and Core promotes those IDs for the next model step. Explicit `select:` and structural filters keep their existing deterministic path. | Implementation is connected; CLI build, protocol replay, and real-model E2E remain pending. |
| Prompt ownership | Claude Code memoizes stable sections and invalidates them at lifecycle boundaries; OpenCode keeps model-family prompts distinct from environment and Skill sections. | Nola retains its deterministic stable prefix, dynamic suffix, schema digest, invocation-policy digest, and provider overlay. iOS now fails explicitly if Core composition fails instead of substituting a client-authored shadow policy. Response style and user execution instructions use distinct trust layers. | Static contract changes are present; native builds and provider cache-hit evidence remain pending. |
| Model capability truth | OpenCode selects a compatibility prompt by model family, while provider adapters determine actual wire behavior. Codex similarly treats reasoning, tool transport, continuation, and cache support as model/provider capabilities rather than prose assumptions. | Nola now uses provider/model identity only for the small instruction overlay. The fallback profile no longer invents parallel tools, reasoning, reasoning continuity, or reasoning streaming. Explicit runtime/provider metadata may enable each capability independently and the resulting invocation-plan digest records the decision. | Core assertions cover conservative DeepSeek fallback and explicit Qwen capability enablement; provider request/response E2E remains pending. |

### Invariants retained

- The original user message is not rewritten to trigger retrieval, Skill,
  memory, MCP, browser, file, or permission behavior.
- Tool search consumes the model's structured query. It is not a classifier
  over the user's message and does not grant permission.
- Discovery returns only catalog entries; execution still requires a later
  concrete call, Core input validation, permission evaluation, and Host result.
- Prompt policy, tool schemas, permission state, and visible Part state remain
  separate identities. A matching search result cannot mutate any of them.

### Required verification before this row is considered complete

1. Build Core and both CLI binaries after the catalog-search extraction.
2. Run Core prompt-composer and Host catalog-search tests.
3. Replay CLI `tool_search` with mixed static/native results, Chinese queries,
   exact selection, filters, no matches, and a stale Host catalog.
4. Run real-model CLI/Web and Mac Agent Host sessions proving that discovery
   activates a schema on the next step without a hidden classifier request.
5. Verify native iOS and Android behavior either exposes the same Core search
   ABI or deliberately keeps a bounded directly visible catalog; client-local
   ranking is not an acceptable fallback.
6. Exercise Qwen and DeepSeek both with and without explicit reasoning
   continuity metadata. Only the explicit profile may return provider
   reasoning state on the next request; a model-family name alone must not.

## 2026-08-05 native Host convergence update

This update closes additional gaps found by comparing the reference projects'
runtime boundaries rather than their surface feature lists.

- Android no longer injects its composed Agent system prompt as a dynamic
  per-Turn context component. Like iOS and the reference agents, a new Session
  receives the stable prompt as its initial system message, while checkpoint
  restore retains the frozen prompt identity.
- iOS and Android foreground Turns now acquire a Host-owned single-writer lease
  by Session id before reading a checkpoint and retain it through the final
  durable write. Atomic file replacement alone was insufficient because two
  runtimes could otherwise fork the same revision.
- Android removed the process-global `activeToolPlan`. Tool execution receives
  the exact frozen plan explicitly; permission presentation consumes a
  one-shot `requestId -> toolPlan` binding. Concurrent Sessions cannot overwrite
  or borrow each other's authorization state.
- Android Skill handling now follows Claude Code/OpenCode's metadata-first
  mechanism. Context contains only enabled Skill id and a bounded description.
  A model-selected `skill_invoke` call resolves one exact enabled Skill, bounds
  its contents, derives a SHA-256 content identity, and returns an
  `invoked_skill` payload for Core-owned Turn/session activation. Skill text is
  no longer pre-injected into every unrelated request.
- iOS now uses the same boundary. Enabled Skill context contains bounded JSON
  metadata rather than the complete instruction body. `skill_invoke` is a
  Core-planned, permission-free read capability; the native Host resolves one
  exact enabled Skill from a Session-frozen snapshot and returns a digest-bound
  `invoked_skill` payload. Unknown, disabled, ambiguous, and oversized Skills
  fail explicitly instead of falling back to prompt injection.
- iOS, Android, and Web no longer parse hidden assistant-text tags to create
  long-term memory. Semantic extraction output is a structured worker result;
  only Core can materialize a recallable record.

Evidence status: these source changes have not yet passed native compilation or
real-model cross-platform E2E. They must not be reported as verified parity
until the build and interaction gates below run successfully.

### Native Memory Worker lifecycle gap

The remaining mobile Memory gap is not an LLM prompt adapter. iOS currently
owns its Core Runtime inside one foreground `DeviceAgentCoordinator` and clears
that Runtime when the Turn stream finishes. Running extraction synchronously
before that finish would add a second model request to user-visible completion;
restoring an unrelated Runtime afterward would fork checkpoint authority.

The reference-aligned fix begins with a durable, per-Session Runtime owner below
the UI Turn. That owner is now present on iOS and Android: foreground Turns use
the retained Runtime under the same single-writer lease, changed configuration
restores from the latest durable checkpoint, and an LRU bound closes inactive
native handles. Clearing an active Session is rejected instead of invalidating
a live handle. The native Runtime bridges now expose the same claim, heartbeat,
completion, and retry submissions used by the CLI Worker, including ownership
tokens, leases, operation IDs, and completion watermarks. Background Memory
Workers must next acquire that same lease and use Core-authored Worker Plans.
The Host may perform provider I/O and scheduling
only; it must not rebuild transcript selection, redaction, quality gates,
identity, provenance, citation, retention, or supersession. A synchronous
Turn-tail Worker remains product-invalid even though it could satisfy the data
contract.
# Durable permission suspension adoption

Nola now persists each concrete background-Agent permission request before the
worker waits. This follows the useful part of OpenCode's deferred permission
model without copying its UI or prompt text: Core evaluates the call, SQLite
preserves the pending request and decision, and the Host only renders and
wakes. Conversation grants resolve all matching pending calls, while Custom
returns control to ordinary user input rather than manufacturing a failed tool
result.
# Tool, Skill, MCP, and Part comparison

## Codex

Codex exposes command execution, file changes, MCP calls, dynamic tools, and
collaboration as strongly typed thread items. Its main advantage is not the
number of item variants, but that callers never need to recover command,
server, status, duration, content items, or file changes from display text.
Nola adopts that invariant through one extensible `ToolPart`: stable ownership,
typed input, permission state, progress, bounded structured output, artifacts,
sources/citations, and `full_output_ref`. Nola deliberately does not copy a new
UI item type for every tool family because that would multiply client logic.

## Claude Code

Claude Code's streaming executor validates arguments before execution, starts
parallel-safe calls as soon as their streamed input is complete, preserves
result order, separates progress from terminal results, and cancels sibling
work with a child abort controller. Nola's Core scheduler and Part projection
already cover the same lifecycle. The remaining adoption rule is that
concurrency and interrupt behavior must be descriptor metadata evaluated after
schema validation, never inferred from a tool name or user wording.

## OpenCode

OpenCode has a single registry contract for built-ins and plugins: schema,
execution context, permission callback, title, metadata, output, attachments,
abort signal, and automatic truncation. Its Skill tool first exposes concise
metadata, then loads full instructions and bundled files only after a concrete
model call. Nola keeps its metadata-first Skill activation and now preserves
the structured Host result in `ToolPart`, so Skill and MCP metadata no longer
need to be embedded in prose for clients to recover.

## Nola adoption invariants

- Core owns Part identity, ordering, terminal state, permission projection,
  sources, citations, artifacts, and structured bounded results.
- Host owns execution and declares schema, effect, concurrency, replay,
  interrupt, provider, and presentation metadata.
- Clients render the Core projection and may localize labels; they do not parse
  output prose, capability names, or user queries to reconstruct behavior.
- Skill and MCP are capability providers behind the same Tool lifecycle, not
  hidden prompt injection paths and not client-specific routing shortcuts.
- Large results contain a preview plus a typed `full_output_ref`; the reference
  is never discovered by parsing a sentence appended to the output.
# Host-declared tool provenance

Nola now follows the reference projects' metadata-first boundary for tool identity. A Host declares whether a capability comes from a built-in provider, Skill, MCP server, device, extension, or Core-internal service. Core captures that immutable declaration in the Tool Part at creation time, and clients consume it as presentation and audit data.

- Tool source is never inferred from a tool name, user text, or localized label.
- Exposure, cancellation, parallelism, and replay safety remain independent typed catalog fields.
- Provider identity and namespace survive catalog refreshes because they are copied into conversation history.
- Legacy Parts decode as `unknown`; clients must not repair missing provenance with keyword rules.
# Sub-agent Task identity

Codex collaboration agents and Claude Code Tasks are durable execution identities rather than generic tool invocations. Nola's Core Task model already owns lineage, child session identity, leases, heartbeats, retries, permission/cancellation scopes, progress, and result references. The remaining cross-layer mismatch has been removed: a spawned child Agent is now promoted as `TaskKind::Agent` instead of `TaskKind::Tool`, and every native client decodes the same wire value.

The tool call that starts an Agent remains a Tool Part, while the resulting long-lived execution is an Agent Task. Keeping both identities prevents duplicate or black-box UI and gives cancellation/recovery the correct target.

## Core-owned Skill execution

Reference-grade agents separate loading a Skill from owning its execution. Nola now follows that boundary: a Host materializes immutable Skill content and returns an explicit `invoked_skill` payload, while Core creates the `SkillRun`, validates declared capability ids against the active Host catalog, accounts model turns and tool calls, and completes, fails, or cancels the run with its owning Turn. Clients must not infer a Skill from user wording or maintain a second client-local Skill state machine.

## System-prompt comparison (current worktree)

| Project | Strong boundary | Nola adoption | Remaining gap |
| --- | --- | --- | --- |
| Codex | Freezes model base instructions, developer instructions, user/repository instructions, model identity, and tool surface as distinct Session/Turn inputs. | Core owns product policy, instruction hierarchy, model compatibility overlay, tool-manifest identity, and the stable-prefix digest. Restored client history can no longer replace the current Core system policy. iOS ordinary cloud chat is unpacked from an explicit Core Provider envelope; Android cloud/Web chat now composes Host policy and user custom instructions through the Core composer before context budgeting, while local Agent turns retain those instructions only in the Turn request to prevent duplicate injection. | Move remaining auxiliary model calls onto explicit typed prompt fragments where their task policy is still authored ad hoc, and keep Provider adapters limited to canonical-message wire conversion. |
| Claude Code | Builds ordered prompt sections, keeps mode/runtime reminders separate from the stable base, and detects prompt-cache breaks. | `AgentPromptFragment` provides ordered layers, trust labels, cacheability, fragment ids, a stable digest, and runtime-state rejection from the stable prefix. `prompt_cache` diagnoses provider, model, prefix, tool manifest, and cache-control changes. | Expose section-level cache diagnostics to developer UI instead of only an aggregate cache miss reason. |

Section-level cache diagnostics are now implemented without prompt leakage. `PromptCacheIdentity` carries the stable fragment digest map, Core reports only fragment IDs and `added` / `removed` / `changed`, CLI summarizes the number of changed sections, and the macOS context inspector lists the affected fragment IDs. No client compares prompt text or reconstructs section ownership.
| OpenCode | Selects a small provider/model-family base, publishes Skill metadata separately, loads repository instructions through a hierarchy, and injects mode reminders as synthetic Turn data. | Core selects bounded model compatibility profiles, CLI loads scoped repository instructions, Skill catalogs remain metadata-first, and current-Turn facts are non-cacheable runtime fragments. | Replace remaining surface-specific legacy prompt builders rather than expanding the Core base prompt. |
| Nola | Must support one personal-assistant identity across native, CLI, and Web surfaces while preserving platform capabilities. | Product policy is Core-owned; Hosts may provide facts, bounded workflow guidance, user instructions, and references but cannot manufacture policy. iOS and Android now replace stale history system messages with the current composition every Turn. | Complete the migration of older direct chat/summarization paths and add cross-platform contract tests before declaring parity. |

### Adopted invariants

1. User input and restored conversation history are never promoted into system policy.
2. Model-family adaptation is small, Core-owned, cache-identified, and cannot grant capabilities.
3. Runtime state, retrieved context, memory, attachments, and Skill instructions remain a dynamic suffix or typed Turn context.
4. Tool schemas and the Host catalog, not prompt vocabulary, define the callable surface.
5. Prompt cache identity includes provider, model, stable prompt prefix, tool manifest, and cache-control policy.
6. Stable prompt cache breaks are attributable to individual typed fragments: Core records each stable fragment's ID and SHA-256 identity in the Turn snapshot, never its prompt text.
7. Agent Trace never infers resource operations from capability names. Explicit Host artifact metadata wins; legacy events retain their URI with an `unknown` operation.
8. Built-in capability compatibility recognizes only versioned exact IDs. Prefixes such as `web_*`, `chrome_*`, `computer_*`, or `files.*` never grant an unknown Host, Skill, MCP, or extension tool an authorization family.
9. Host capability metadata declares how validated arguments become a stable permission resource (`constant`, `first_of`, `composite`, or `full_input`). Runtime permission and conversation-grant code consume the frozen declaration rather than reclassifying the call.
10. Loading or declining a Skill is an internal catalog operation and grants no environmental authority. Every required tool remains a separate frozen capability call. Dynamic MCP mutations bind permission to the complete canonical invocation and frozen server origin; MCP reads remain ordinary read-only discovery.

## Memory and permission comparison

| Boundary | Reference strength | Nola decision |
| --- | --- | --- |
| Memory storage | Claude Code exposes explicit user/project/local memory-file scopes and conflict choices; Codex keeps durable session state separate from instructions. | Keep Nola's typed records, source snapshots, citations, quality assessments, extraction/consolidation jobs, watermarks, and usage ledger. Add equivalent scope visibility and conflict UX; do not collapse records back into one editable prompt string. |
| Memory injection | Reference clients account memory separately in context and do not let retrieved text grant authority. | Memory remains a typed context component and dynamic prompt suffix. Its citations and use are recorded per Turn; it cannot add tools or permissions. |
| Memory visibility | Claude Code makes durable instruction scopes inspectable; Codex exposes authoritative item identity instead of reconstructing hidden state in each UI. | Every memory that survives context budgeting now creates a redacted, typed `SourcePart` owned by the Turn. Scope, namespace, provenance, observation time, truncation, and current-state verification travel with the Part without exposing memory content. `memory.used` remains an audit event only; clients use the common Source lifecycle instead of maintaining a second status-card projection. |
| Permission suspension | Claude and OpenCode suspend the concrete call and resume it by request identity; Codex evaluates structured tool metadata under sandbox/approval policy. | Core owns the durable permission projection and exact call/resource subject. Client continuations only render and deliver decisions; they are not a second authorization store. macOS retains the original validated request inside the live Agent Host runtime, so lost presentation state can recover the exact pending request without re-running the tool or forging arguments and digests. A terminated Host remains non-actionable until the protocol provides a reconnectable permission channel. |
| Permission resource | Reference systems bind approval to an executable action and resource, not words in the user prompt. | `permission_domain` is the Host-declared authorization subject while the concrete tool id remains the audit/presentation identity. Core no longer parses domain-label vocabulary. Unknown Host, MCP, and Skill calls use the complete canonical invocation until a typed catalog resource declaration exists. |
| Session grants | OpenCode persists structured session rules so repeated matching calls do not prompt again. | Nola conversation rules persist the exact Host-declared permission domain and frozen resource while concrete tool ids remain audit identities. Pending sibling calls are re-evaluated against that same subject, so a conversation grant does not immediately prompt again. Core does not widen grants from tool names or command text. Scheduled/replayed work cannot inherit a foreground Turn grant. |

### Remaining memory/permission work

1. Host-declared permission domains now flow directly into Core permission subjects; concrete tool ids remain separate audit identities. The CLI-only terminal policy keeps its explicit code-owned family mapping because that surface owns its own rule file. Missing resource declarations fail closed to `full_input`; no checkpoint path re-enters name-based resource inference.
2. Core Source evidence carries memory scope, namespace, provenance, supersession, usage count, and last-used time through macOS, iOS, Android, and Web source cards. Hosts now supply `submitted_at_epoch_ms`; Core records each typed memory that survives Turn budgeting once, without reading wall clock time or treating legacy text context as a durable memory record.
3. Prove restart recovery for suspended foreground and background permission requests with real-model end-to-end sessions.
4. Mobile model requests now carry an optional typed `requiredToolName`. iOS and Android OpenAI-compatible providers encode it as native `tool_choice`, and model clients reject a completion that omits the required function. This is the shared structured-output boundary needed by independent mobile Memory Workers; unsupported iOS provider paths fail explicitly instead of guessing JSON from prose.
5. iOS and Android now share equivalent Memory Worker model executors. Each consumes only a Core-authored plan, exposes exactly the plan's response schema through `submit_memory_worker_output`, forces that native call, and returns only its arguments to the caller. The executors do not write memory or invent lifecycle state; scheduler, lease, checkpoint, Core validation, and durable materialization remain separate required work.

### Task continuation and sub-agent lineage

OpenCode's resumable Task contract and Codex's typed parent/child trace graph expose a gap in Nola's earlier CLI behavior: durable Agent tasks had a child session and Core lineage fields, but every delegation created an unrelated child session. The CLI tool contract now accepts an optional completed `task_id`. Continuing it creates a new immutable task record, reuses only that task's child session, and records the prior task as `parent_task_id` in both persistence and Core projection. Project, conversation, terminal-state, task-kind, child-session, and agent-profile checks prevent cross-boundary takeover. Recursive delegation remains disabled until task visibility and control are scoped transitively rather than only by project.

Runtime task discovery and control are now scoped by both project and owner conversation. `list_tasks`, `/tasks`, `read_task`, `wait_task`, and `stop_task` no longer expose another conversation's Agent or Shell task merely because it shares a workspace. Unknown or foreign durable task ids also cannot fall through to the process-local Shell registry. This brings the operational boundary closer to Codex's thread graph rather than treating a workspace as one global task authority.

The CLI task view now treats Core projection as the canonical representation and uses the Runtime database only for tasks Core has not projected. A promoted task is no longer rendered or counted twice merely because both durable layers contain it.

macOS task cards now cover the Agent task kind and expose parent_task_id as a localized “Continued from” detail. Existing empty localization placeholders for background Agent, Skill, and Tool labels were replaced with translations across the Mac catalog's supported languages.
## Mobile long-term-memory worker transaction

- iOS and Android now expose a reusable Host Executor operation for Core-authored long-term-memory jobs.
- Background memory work shares the foreground Session writer lease, restored Core Runtime, native model client, and atomic checkpoint store. It cannot fork an independent memory state machine.
- The transaction is `claim -> checkpoint -> plan -> structured model tool call -> heartbeat -> complete/fail -> checkpoint`.
- Background execution fails closed if Core exposes any Host effect other than `persist_checkpoint`; normal model, tool, permission, and context effects remain owned by foreground Turn coordination.
- The model executor receives only Core's worker instructions, model input, and response schema, and must answer through the required native `submit_memory_worker_output` tool.
- Runtime configuration is persisted atomically beside each checkpoint, allowing a background launch to restore the exact Runtime contract instead of reconstructing it from UI state.
- iOS queues memory processing after the visible response and uses the existing refresh/processing BGTasks as recovery opportunities.
- iOS background recovery enumerates persisted Runtime configurations and processes a bounded cross-Session batch instead of assuming that the most recently restored conversation is the only authority.
- Android queues a unique per-Session WorkManager chain after the foreground stream completes; retries are bounded and use the same injected Agent service. WorkManager does not impose a network constraint because the configured model may be local.
- Platform cancellation remains cancellation: it is not converted into a retryable Memory Worker failure or persisted as misleading model evidence.
- Cross-Session recovery isolates corrupt iOS Runtime sidecars instead of allowing one damaged conversation to block all memory work. Android WorkManager identities use a full SHA-256 Session digest rather than collision-prone process-language hash codes.
## Source-verified follow-up: Skill dependencies and mobile Memory Workers

### Skill dependency recovery

Codex detects missing MCP dependencies, prompts at most once per session, can install or authenticate the dependency, and refreshes the MCP catalog before retrying. OpenCode injects a selected Skill as typed context with an explicit base directory and sampled files.

Nola Core already implements the stronger combined contract rather than relying on client routing:

- Skill selection is a structured model tool call; user-language keywords never activate a Skill.
- Skill content is digest-attested before execution and is checkpointed with the Session.
- Missing capabilities move the run to `WaitingForCatalog`, emit a recoverable catalog refresh request, and resume only after the Host supplies a newer catalog.
- Inline and forked Skill budgets are independent, terminal updates are exactly-once, and late terminal updates are absorbed.

No additional client-side Skill dependency router should be added. The remaining work is real Host/catalog integration coverage, not keyword matching.

### Mobile Memory Worker recovery boundary

iOS and Android now use the foreground Session authority for deferred memory extraction: the same Runtime checkpoint, runtime configuration, model client, writer lease, heartbeat, retry, cancellation, and completion protocol are reused. iOS background recovery scans persisted Session configuration sidecars; Android queues a per-Session WorkManager chain with a SHA-256 identity and no forced network constraint because the selected model may be local.

`AgentMemoryWorkerHostRecoveryTests` adds the missing iOS transaction boundary: a real Core Turn completes, Core queues extraction, the checkpoint is persisted, a new Host Executor restores it, the structured Memory Worker model tool completes, and the final checkpoint contains the completed job with no claimable residue. Android's ordinary local JVM suite cannot honestly claim this boundary without loading the native Core library; its JVM coverage therefore remains focused on deterministic scheduling identity and structured model-call contracts, while native/device E2E remains a separate validation lane.
## MCP catalog recovery: discovered capabilities, not cached echoes

Reference implementations treat an external capability registry as a live dependency: a recoverable lookup failure must refresh the provider-owned catalog before retrying. Nola already discovered MCP tools, resources, resource templates, and prompts over Streamable HTTP and stdio, but its explicit Core refresh effect returned the Host's cached catalog unchanged. That made the recovery protocol structurally correct but operationally inert.

The CLI Agent Host, which is also used by Web and the native macOS host path, now retains the reconnectable MCP server registry. A Core `RefreshCapabilityCatalog` effect forces bounded MCP discovery, installs the newly discovered routes atomically, removes stale MCP routes and catalog-access capabilities, and returns a monotonically newer catalog revision. Server `list_changed` notifications still apply only at normal Turn boundaries; an in-Turn refresh remains an explicit, checkpointed Core recovery operation rather than an implicit Host mutation.
## Prompt composition: one typed pass per request

Claude Code memoizes named system-prompt sections, Codex separates stable policy from sourced dynamic context, and OpenCode keeps environment, Skills, and repository instructions as independently assembled inputs. Their shared advantage is not longer prompt prose; it is preserving the identity and lifecycle of each source instead of flattening one completed system prompt into another prompt builder.

Nola Core already enforces a stronger trust boundary and produces stable-prefix and per-fragment digests. The Android cloud Agent path nevertheless composed the complete Core prompt once, relabeled that rendered result as a Host instruction, and composed it a second time. This duplicated Core product policy, erased the original fragment identities, and destabilized prompt caching. Android now passes the original typed Host identity, response-style, and user-instruction fragments through Core exactly once. The macOS Core ABI wrapper now also preserves Core's per-fragment digests, matching iOS and Android observability.
## Effective-prompt observability without prompt leakage

Codex exposes a debug path that builds the exact model-visible input, while Claude Code tracks named system-prompt sections and their cache lifecycle. Nola already checkpoints prompt identity, but its runtime debug log did not provide one authoritative receipt for the effective Turn composition.

The shared CLI Agent Host runtime now emits a `prompt_composition` debug record after Core composition. It includes fragment identifiers, the stable-prefix digest, per-stable-fragment digests, stable/dynamic character counts, and the repository-instruction manifest digest/count. It deliberately excludes prompt bodies, memory text, attachments, retrieved data, and instruction paths. Because Web and native macOS use this Host runtime, the same receipt is available across all three surfaces without duplicating client logging logic.

## Deferred tool schemas: bounded model surface, complete Host catalog

Claude Code and Codex avoid sending every connected tool schema in every model request. Their important invariant is that discovery and execution use the complete authoritative catalog while the model receives only an active subset; a discovered tool is added in a later model step rather than being inferred from user-language keywords.

Nola Core already implements that provider-neutral contract:

- each Turn freezes the complete available Host catalog for dispatch, validation, Skill activation, and `tool_search`;
- only `Visible` tool descriptors enter the initial model projection;
- `Deferred` descriptors remain searchable from declared identifiers, titles, metadata, and JSON Schema;
- a successful structured discovery result produces an activation receipt, and Core adds only those exact capability schemas to the next Step projection;
- the Host validates the projection digest and cannot rebuild or widen it from message history.

The integration gap was MCP exposure. Built-in tools used the visible/deferred split, but dynamically discovered MCP tools were marked `Visible`, so a large MCP registry still expanded the first request. MCP tools are now `Deferred` by default. They remain present in the frozen catalog and are discoverable through the same structured protocol, while their full schemas are omitted until explicitly activated. This preserves permission and execution semantics and removes the need for language-specific routing or client-side query rewriting.

## Session recovery is not conversation rollback

Codex reconstructs resumed and forked threads from persisted rollout items and records rollback markers so later replay can recompute effective Turn settings and reference context. OpenCode couples message-level revert state with a workspace snapshot and provides an explicit fork operation from a selected message.

Nola currently has two strong but different primitives:

- Core checkpoints restore the authoritative Agent state, pending effects, permission lifecycle, memory jobs, context accounting, and execution cursor after interruption;
- workspace checkpoints can preview, diff, and restore file changes around risky tool execution.

These primitives provide crash recovery and file rollback, but they do not yet provide a conversation fork or message-level rollback contract. A client deleting visible messages would leave summaries, activated capabilities, Skill runs, memory usage, permission evidence, and context accounting inconsistent. Therefore clients must not implement local-only conversation rollback.

The Core-owned branch operation now names a persisted message boundary, creates a new Session identity, retains only authoritative completed Items and Parts through that boundary, clears live Tasks, pending permissions, Skill runs, and memory jobs, recomputes context usage, and records lineage to the parent Session. In the same way Claude Code preserves content-replacement records, a branch through the active compacted range inherits Core's exact compacted `model_context`; an older boundary is rebuilt from raw history with synthetic compaction summaries removed. This prevents both prompt-cache loss and duplicated summarized/raw content. Historical regeneration now also submits a Core-owned `RevertConversation` operation by durable message identity. Core trims transcript, ownership, compacted model context, provider continuation, and token accounting together; Web mirrors the accepted boundary and restores its original message list if Core rejects the operation. This revert deliberately does not undo workspace files, durable memory, or other external side effects. A future explicit workspace rollback remains a separate user-visible transaction rather than an implication of hiding transcript text.

The shared Agent Host debug log now emits a `model_tool_projection` receipt only after the Host has validated Core's Step projection. It records catalog revision/epoch, projection revision/digest, active tool count, available deferred count, and the exact deferred capability ids activated for that Step. It does not log tool arguments, schemas, prompt text, or result content. This makes schema deferral and activation observable in CLI, Web, and macOS without allowing those clients to reconstruct or widen the projection.

## Auxiliary model calls keep role boundaries

Reference implementations use separate, narrowly scoped prompts for compaction, title generation, classification, and other auxiliary model work. The reusable lesson is not to wrap those calls in the full interactive Agent identity, and never to duplicate their system instructions inside a user message.

Nola's iOS task parser had exactly that duplication: the task-parsing system template was prepended to the user message and also passed as the provider system prompt, including on JSON recovery. It now sends the task policy once through the system channel and sends only the user request or bounded recovery request through the user channel. This auxiliary prompt remains independent from the Core interactive-Agent policy; it cannot grant tools, memory, or permissions.

Android's Chat migration also left an unused string-building helper that could reconstruct a complete `AndroidSystemPrompt` outside the typed Chat path. The helper had no callers and is removed rather than retained as an attractive bypass. Interactive Android Chat now has one supported composition route: original typed Host fragments enter the Core composer once.

Claude Code and OpenCode enforce structured auxiliary results through a synthetic function/tool rather than searching free-form prose for JSON. Nola's mobile model contract now exposes the same boundary through a reusable `AgentStructuredOutputExecutor`: callers provide bounded system instructions, typed model input, a JSON Schema, and one required submission tool. The executor disables parallelism and reasoning, forces the exact tool, locally validates arguments including nested required fields, enums, constants, bounds, and closed objects, and gives the model at most three schema-specific correction attempts. Memory Worker execution and task parsing both use this contract. Task parsing no longer searches marker-wrapped prose or silently falls back to a text-only provider; an unsupported provider or invalid tool submission fails closed and returns to normal chat rather than executing an inferred task.

Android now uses the same contract and failure semantics. Its Memory Worker no longer selects the first matching tool call from a potentially ambiguous completion; the shared executor rejects zero or multiple matching submissions and rejects missing arguments. This keeps auxiliary structured-output behavior aligned across both mobile Hosts.

The CLI goal-completion verifier was another high-impact free-text JSON parser. It could accept a JSON-shaped substring from ordinary prose and used a relatively high sampling temperature for a binary completion gate. It now exposes only `submit_goal_verification`, forces tool use, uses a bounded low-temperature request, requires exactly one matching call, and validates its arguments. Missing, duplicate, or malformed calls fail closed to `completed=false`; verifier text can no longer change goal state by merely resembling JSON.

iOS task parsing now attempts the same native structured-output path through `submit_task_parse`, with reasoning and parallel calls disabled. The resulting typed arguments are decoded by the existing task-domain validator. Providers without native tool support enter an explicitly logged compatibility path that retains the legacy marked-JSON parser; this is a compatibility boundary, not silent structured-output inference. Recovery prose is no longer the primary remote-model protocol.
## Core-owned conversation branching and durable item ownership

Nola now records persistent `item_id -> {turn_id, message_id}` ownership in Agent Core rather than asking each UI to infer grouping from rendered rows. Core can create a branch checkpoint only at the final durable item of a completed Turn. The branch retains canonical transcript, terminal Parts, model context, provider continuation, permission rules, and explicit lineage while leaving the parent runtime unchanged and clearing transient work.

This follows the strongest shared property of Codex rollout replay and OpenCode session fork/revert: branching is a state-machine operation over canonical history, not a client-side copy of visible text. The C ABI, Swift wrapper, and Android JNI wrapper expose the same checkpoint operation so platform clients do not invent incompatible trimming rules.

Imported history identities are now deterministic across CLI, iOS, and Android. Hosts derive Item IDs only from the stable Session identity, persisted message position, item kind, and tool-call position; Core deterministically groups those Items into Turns. Previous random-on-restore UUIDs made identical history look new after every launch, invalidating prompt-cache identity and making branch boundaries impossible to reproduce.

Web truncation now evicts the cached Core Runtime and removes the stale checkpoint before replacing durable messages. This prevents a later request from restoring Turns the user already removed. Full Web branch lineage remains intentionally blocked on a persisted `WebMessage.messageId -> Core message_id` mapping; matching by text or array position would recreate the exact ambiguity this migration removes.

The Web history adapter now consumes the Core `message_id` already persisted in `meta.agentTrace.messageId`. User messages retain their durable Web message ID; assistant messages retain their Core trace message ID; subsequent imported tool Items inherit that assistant ownership in Core. This closes the identity gap without schema-specific text matching or a second message table.

Web branching now derives the child from the persisted Core checkpoint even when the parent Host is not resident. The selected Web message must resolve to a real Core `message_id`; Core locates the final durable Item owned by that message, enforces the complete-Turn boundary, and writes a child checkpoint with lineage before the Web transcript is cloned. Cloned rows receive new UI primary keys but retain `meta.coreMessageId`, keeping UI identity separate from canonical Agent identity.

All imported Web roles now prefer `coreMessageId` over the cloned UI primary key. If conversation persistence fails after the child checkpoint is created, the orphan checkpoint is removed; a failed branch cannot silently appear later as a restorable Agent session.
# 2026-08-06: Authoritative memory management control plane

Nola now follows the strongest reference-project pattern of keeping memory ownership behind the Agent Host rather than duplicating it in each UI. The local Host exposes a bounded JSON `memory` control plane for listing, creating, revising, and deleting structured records. Revisions preserve provenance through `supersedes`; macOS reads this same store in Data Center and no longer injects its legacy `UserDefaults` shadow copy into chat context. This closes the product gap between a capable Core memory model and a user-auditable correction/version-history workflow.
