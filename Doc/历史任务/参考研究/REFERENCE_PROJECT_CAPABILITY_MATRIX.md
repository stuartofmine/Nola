> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Reference Project Capability Matrix

## Purpose

This document compares Nola with the checked-in Codex, Claude Code, OpenCode, and Open WebUI sources. It separates system-prompt design from runtime capabilities so prompt text is not used to compensate for missing protocols, storage, or authorization.

## System-prompt architecture

| Area | Codex | Claude Code | OpenCode | Open WebUI | Nola current state | Direction |
|---|---|---|---|---|---|---|
| Composition | Layered base instructions, environment, tool policy, skills, developer and user instructions | Many dedicated sections and tool prompts with cache-boundary handling | Session assembler combines provider/model/agent instructions, tools and compaction state | Resolves one model/system prompt through variables and middleware | Core owns product policy and prompt ordering; Host prompts now retain only identity, platform facts and surface-specific operating guidance | Bind the same Core stable-prefix digest to every supported provider and keep all volatile material in typed extensions |
| Tool guidance | Tool schemas and runtime policy are authoritative | Tool-specific prompts explain exact constraints | Tool catalog and permission rules are runtime inputs | Tool exposure is gated by global config, model capability, session feature and user permission | Core freezes the executable Turn catalog and exact Step projection; clients consume typed activation and permission events rather than inferring tools from prompt words | Finish cross-client diagnostics and request-capture fixtures without reopening client-local routing |
| Skills | Metadata is discoverable; full skill instructions load only when selected | Bundled skills and dedicated skill workflows | Agent/config instructions are progressively loaded | Skill list omits content; `view_skill` loads full content on demand | Core owns immutable Skill identity, owner, budget, catalog revision and run state; CLI/Web and Apple clients project that state while Hosts materialize exact content references | Add equivalent Android presentation and resource-sharing policy without making Skill access an execution grant |
| Context compaction | Preserves rollout semantics and reconstructable events | Dedicated compact/continuation behavior | Structured summary template preserves goal, constraints, exact paths, commands and errors | Configurable compact threshold and provider middleware | Core compaction exists; UI/token accounting has historically diverged by client | Use model-declared limits, reserve output, persist structured summary and retained tail |
| Memory instructions | Memory has explicit mode and citation protocol | Extraction/consolidation are isolated background workflows | Primarily session and project context | Memory tools are exposed conditionally | Core eligibility policy exists; mobile/macOS legacy stores are still being migrated | Keep retrieval, applicability and consolidation as separate contracts |
| Scheduled input | Thread/task lifecycle retains origin | Scheduled prompts enter a lower-priority queue | Session commands/events retain source | Automations are a gated tool category | Core now freezes input origin; macOS scheduled turns declare it | Route every automation through the same Agent/Core lifecycle without live-user authority |
| Prompt caching | Stable sections and provider-aware behavior | Explicit cache-break detection and cache-safe fork parameters | Provider transform and session boundaries | Mostly middleware/model configuration | Core owns the stable digest; CLI/Web, Android, iOS, and the macOS Agent Host main-turn path bind it to supported provider requests | Keep auxiliary classifier, compaction, title, and summary requests isolated from the main-turn identity |

Current prompt-cache audit:

- Core already separates `stable_prefix` from `dynamic_suffix`, rejects cacheable reference fragments, and returns `stable_prefix_sha256`.
- CLI/Web main turns and Android now attach a provider/model-namespaced identity derived from that Core digest; explicit configured keys remain overrides and unsupported providers receive no field.
- iOS passes the Core `stable_prefix_sha256` into the Apple provider cache key, while macOS delegates its main turn to `nola-agent-host`, which consumes the same Core `model_prefix_digest`. Auxiliary classifier, compaction, title and summary requests remain isolated from the main-turn identity.
- Hashing the final rendered system prompt remains wrong because it includes Host facts and per-turn reference context.

#### Prompt ownership audit (2026-08-03)

The reference projects converge on section ownership rather than a single large
prompt. Nola now applies the same principle with stricter trust boundaries:

- `compose_agent_prompt` inserts Core's product-wide action-scope, truthful
  outcome, semantic tool-selection and investigate-before-question policies
  exactly once for the full Turn;
- `compose_agent_prompt_extension` wraps dynamic memory, Skill, attachment,
  retrieval and platform data without reinserting the Core policy prefix;
- shared chat text owns identity, language and general response quality;
- product Hosts own only surface identity and concrete platform facts;
- the CLI retains terminal-specific engineering workflow, but no Host loads the
  shared Agent policy a second time;
- reference material remains explicitly wrapped as data and cannot become
  instructions or authorization.

This absorbs Codex/OpenCode's layered assembly and Claude's cache-boundary
discipline without copying their coding-only prompt prose into consumer clients.

## Functional architecture

### Tool execution

**Codex strengths**

- Strong typed protocol between model, Core, Host, and UI.
- Tool results remain replayable events rather than being flattened into assistant prose.
- Sandboxing and approval are concrete execution decisions.

**Claude Code strengths**

- Tool-specific permission handlers.
- Background work is abortable and reports bounded progress.
- Tool restrictions for internal consolidation are explicit and narrower than the interactive session.

**OpenCode strengths**

- A permission request is pending runtime state; UI responds with once, always, or reject.
- Context overflow uses actual token accounting and model limits.

**Open WebUI strengths**

- Built-ins pass four gates before schema exposure: global configuration, model capability, session feature, and user permission.
- Tool name collisions are resolved explicitly when aggregating external servers.

**Nola gap and action**

- Core already evaluates concrete calls and permission provenance.
- Platform tool catalogs and process-event rendering remain inconsistent.
- Introduce one per-turn capability projection containing schema digest, enabled source, Host support, and denial reason. Clear it at the next turn while preserving bounded call/result history.

#### Tool exposure freeze audit (2026-08-03)

The checked-in implementations make the remaining Nola boundary more precise:

- Codex builds a `ToolRegistry` with one canonical `ToolName`; duplicate
  registration is an error rather than last-writer-wins behavior. A runtime has
  an explicit exposure state, parallel-call support, cancellation behavior,
  typed model output, pre/post hooks, and an optional streamed-argument diff
  consumer. Dispatch resolves the same registry entry that produced the model
  schema.
- OpenCode assembles built-in, plugin and custom tools through one registry and
  evaluates permissions against the concrete call. Its implementation is less
  strict than Nola's signed/expiring Host manifest boundary, but its session UI
  benefits from consuming the same call identity and pending permission state.
- Nola already has the stronger low-level primitives: Host catalog revision and
  availability epoch, Host leases, Core-owned input validation, explicit
  exposure, permission domains, streaming/cancellation/parallelism metadata,
  deterministic policy and manifest digests, identity-bound device egress
  receipts, and fail-closed unknown tools.

The unresolved issue is composition, not another permission rule. The
`ToolPolicyOutput`, configured model schemas and `HostCapabilityCatalog` are
currently related inputs instead of one immutable Turn artifact. Consequently,
different clients can still describe a capability set that differs from the
schemas actually sent to the model.

The Core implementation contract is therefore:

1. At `TurnStarted`, resolve one `TurnCapabilityProjection` containing the full
   executable registry from the Core policy result and exact Host catalog
   revision/availability epoch.
2. For each model sampling step, derive a `ModelToolProjection` from that frozen
   registry. This preserves Codex-style deferred discovery and Skill activation
   without consulting a changed live catalog.
3. Include canonical IDs, schema digest, policy digest, manifest digest, Host
   identity, disabled IDs with machine-readable reasons, and runtime
   streaming/parallelism/cancellation traits.
4. Build every model request from its Step projection and dispatch only a call
   present in the Turn registry; do not independently re-filter by localized
   client state.
5. Project the same revisioned objects to CLI/Web/native UI. Display names remain
   localized presentation data and never become dispatch IDs.
6. A catalog or authorization update during an active Turn applies to the next
   Turn unless it invalidates the active Host lease; it must not silently mutate
   an already-issued model Step.
7. Preserve bounded argument diffs, progress, typed result preview and artifact
   references under the stable call ID. Unknown or stale calls return a
   recoverable model-visible protocol result, never a guessed fallback tool.

This is intentionally narrower than copying Codex hooks or OpenCode plugins.
Nola should retain its multi-device Host and result-egress guarantees while
making the resolved capability surface equally coherent and observable.

Implementation status:

- Core now defines checkpointable `AgentTurnCapabilityProjection` and
  `AgentModelToolProjection` contracts.
- Turn creation freezes the full available Host registry plus directly visible
  definitions, disabled IDs and machine-readable reasons.
- Canonical projection and model-schema digests are stable across configured
  tool ordering.
- `AgentTurnContextSnapshot` retains the projection/schema digests and enabled
  canonical IDs; `TurnContextCaptured` publishes the full revisioned projection.
- The public `tool_count` now means capabilities actually exposed by the frozen
  projection, not the number of unvalidated configuration entries.
- Model planning derives every request's exact tool subset and digest from the
  frozen registry. Skill/recovery additions may select another frozen descriptor
  but cannot read a newer session catalog.
- Tool batching, schema validation and permission-subject construction now use
  the reconstructed frozen catalog rather than the live Host catalog.
- First-dispatch model, permission, tool and compaction envelopes now carry the
  Turn-frozen Host ID, lease, catalog revision and availability epoch. Explicit
  recovery remains the only path allowed to rebind a replay-safe pending effect
  to a replacement Host.
- Model-stream settings carry the Step tool projection alongside the exact
  schema list so an authenticated Host can attest what it received.
- The Turn checkpoints the last issued Step projection. A model-produced call
  must be present in that exact canonical ID set before schema validation,
  permission evaluation or Host execution. Merely existing as a deferred
  registry capability is not authorization to dispatch it.
- Tool discovery and Skill invocation now activate only canonical capabilities
  already frozen in the Turn registry. Core records their source, derives the
  next Step projection from that state, and emits a typed
  `CapabilitiesActivated` event; it never reconstructs activation by parsing
  conversation text or localized display strings.
- CLI/Web, macOS, iOS and Android project that event into the existing visible
  tool/process history under a stable Turn/source identity. A shared fixture at
  `core/tests/fixtures/capability_activation_v1.json` defines the authoritative
  Core envelope plus the explicit iOS and macOS Host projections.
- CLI/Web Host model requests now forward the exact Core-issued Step schema list
  without filtering or appending a Host-private projection. Deferred built-ins
  remain discoverable in the frozen catalog and become callable only through
  Core's typed `CapabilitiesActivated` transition.
- Core now validates every normalized Host response against the persisted
  request kind before advancing its response sequence or mutating Turn state.
  Effect, request, Host lease, Turn, catalog, availability epoch, and sequence
  identity are checked first; a mismatched payload is rejected as a protocol
  error without consuming the valid response's sequence.
- Remaining integration work is to buffer an activation event that arrives
  before its UI run exists, expose the same protocol diagnostics on every
  client, and expand real-model validation across all clients.

### Permissions

| Requirement | Reference evidence | Nola status |
|---|---|---|
| Concrete action/resource evaluation | Codex sandbox and OpenCode permission request | Implemented in Core tool policy |
| Pending request survives UI delay | OpenCode permission dock | Implemented for several CLI/macOS paths, not yet proven on every client |
| Scope is explicit | once/always; user/group/resource grants | Core provenance distinguishes product, global, conversation and turn grants |
| Automation cannot borrow interactive authority | Claude scheduled queue and forked jobs | Implemented for Core input origin and macOS Agent Host |
| Resource sharing | Open WebUI access grants for user/group/public and read/write | Missing as a shared Skill/MCP/data-source collaboration contract |
| Ownership-safe completion | Codex job ownership tokens | Added to Core task durability and CLI task store/runner |

Nola should add resource grants only for shareable resources. Local destructive-operation approval must remain a separate execution permission; sharing a Skill must never authorize its tools.

### Skills and MCP

Open WebUI demonstrates a useful split between `SkillResponse` and `SkillModel`: list responses exclude full content, while an explicit tool loads it. Nola should use the same progressive-disclosure property without copying its server-centric ownership model.

Required Nola contract:

- gallery projection: ID, localized name, localized description, category, source, enabled state, version and capability requirements;
- content projection: full `SKILL.md`, supporting files and editability;
- execution projection: selected Skill IDs, content digest and compatible tool IDs;
- authorization projection: who may read/edit the Skill, separate from which tools may run;
- MCP projection: server identity, transport, health, available schema digest and per-tool availability;
- deterministic name-collision handling using server-qualified IDs while retaining localized display names.

#### Hooks, MCP and Skill isolation audit (2026-08-06)

The inspected reference projects solve different parts of extension reliability:

- **Codex** normalizes third-party MCP schemas before exposing them, retains
  structured output schemas, keeps discovery separate from execution, and can
  defer a large extension catalog behind `tool_search`. Its main advantage is
  that the executor resolves the same canonical tool definition that produced
  the model-visible schema.
- **Claude Code** has the richest Hook surface and strong timeout/cancellation
  behavior. Hooks can participate in more lifecycle phases, but that power also
  creates a larger mutation and authorization boundary.
- **OpenCode** makes custom tools and Skills easy to add. Plugin modules are
  dynamically imported, and a selected Skill injects its full instructions plus
  sampled supporting files. This is productive, but direct plugin execution and
  prompt injection are less isolated than Nola's revisioned Host/Core contract.
- **Open WebUI** provides the clearest resource-management UI and separates
  list metadata from full Skill content, but its server-centric ownership model
  is not appropriate for sensitive local files, health data, or device grants.

Nola should retain its stricter design rather than copy the broadest extension
surface:

- MCP servers and tools use stable server-qualified IDs, transport identity,
  availability, input and output schemas, catalog revision, deferred exposure,
  bounded output, cancellation and replay-safety metadata.
- A transport failure never causes an automatic retry of the same remote tool
  call because the server may already have committed a side effect.
- Skill discovery is metadata-first. Invocation returns an immutable
  `instructions_ref` containing version and content SHA-256; Core then owns the
  SkillRun owner, mode, budget, usage, missing-tool recovery and terminal state.
- Loading a Skill does not authorize its required tools. Concrete calls still
  pass the normal Core permission evaluator.
- Resource visibility uses a separate Core decision with `read_metadata`,
  `read_content`, `edit_content`, and `manage_sharing` scopes. Grants bind the
  resource kind/ID, principal, optional Host, revision, content digest and
  expiry; every decision explicitly leaves tool execution unauthorized.
- Lifecycle Hooks are deliberately observation-only in schema v1. Untrusted or
  modified registrations fail closed, matching uses exact canonical tool IDs,
  and commands, URLs, credentials and prompt bodies remain Host-owned.

The remaining extension gaps are precise:

1. Hook schema v2 now binds every running and terminal execution projection to
   the installed registry's `revision_sha256`. Remaining work is presenting
   that provenance consistently in platform diagnostics and replay tooling.
2. Core now has a separate, version/digest/Host-bound read/edit/share access
   contract for Skill, MCP, data-source and prompt-template resources. Host
   stores and gallery editors still need to consume it before shared resources
   are exposed.
3. Android now binds Core-owned Skill runs to their assistant Turn and presents
   stable status, usage, result and failure details without duplicating the
   compatibility tool event. Cross-platform replay fixtures and real-model
   execution evidence remain incomplete.
4. Active delegated Tasks now have a Core-owned ordered guidance queue and a
   CLI Host safe-boundary delivery adapter. macOS, iOS and Android route native
   controls to either the active local coordinator or the paired Host, and all
   three consume authoritative projections. Cross-platform replay fixtures and
   mobile local-worker claim/checkpoint parity remain incomplete.

#### Cross-platform execution boundary audit (2026-08-06)

- macOS launches `nola-agent-host` with a Core-issued policy receipt and frozen
  Host capability manifest. Native browser, computer-use, memory and data-source
  operations are bridges owned by the Mac Host, not a second Agent runtime.
- Web reuses the CLI Agent Host and `run_agent_core_turn`; it does not maintain a
  separate prompt/tool router.
- iOS and Android use typed Core FFI tool plans with identity-bound, expiring
  manifests and platform permission brokers. The brokers translate an approved
  Core capability into OS authorization UI; they do not select tools.
- The Web tool-policy API no longer accepts raw user turn text. Catalog exposure
  depends only on candidate capabilities, platform, network policy, explicit
  denials and result-egress authorization. This removes an otherwise-unused
  interface that could have reintroduced language-specific keyword routing.

### Memory

| Property | Codex | Claude Code | OpenCode | Open WebUI | Nola |
|---|---|---|---|---|---|
| Per-source extraction | Yes, Stage 1 | Yes | No dedicated durable pipeline | No | Planned; Core record exists |
| Consolidation job | Global Phase 2 | AutoDream lock/fork | No comparable durable memory pipeline | Batch operations only | Not yet implemented |
| Lease/retry/watermark | Yes | Lock and rollback | No | No | Core and CLI primitives added |
| Provenance/confidence | Rollout metadata | File/session provenance | Session provenance | User ID/path/meta only | Core supports both |
| Supersession/expiry | Consolidation semantics | Managed files/version handling | No | In-place replace/delete | Core selection supports both |
| Citation/usage | Memory citations and usage counters | Visible consolidation files | No | No explicit citation usage | Missing UI and usage tracking |
| Hierarchical paths | Workspace/thread metadata | Memory files | Project/session organization | Strong path browsing | Namespace exists; path browsing not standardized |

Nola should combine its stronger record semantics with Open WebUI's legibility: browse by namespace/path, but never reduce records to path/content alone.

### Background tasks and automation

Reference behavior converges on six requirements:

1. enqueue is separate from execute;
2. a worker atomically claims work;
3. a lease and heartbeat establish current ownership;
4. permission waiting is suspended, not failed;
5. cancellation reaches the executing worker;
6. only the owner commits terminal state and advances a watermark.

Nola now has these primitives in Core and the CLI SQLite store. Remaining work is to connect iOS, Android, macOS daemon, and Web Host to the same envelope and persist checkpoints for permission suspension.

Current integration status:

- Android, iOS and macOS now map platform-owned structured memory records into the Core selection contract before prompt injection. An unavailable Core gate fails closed instead of injecting every stored record.
- macOS context estimation and the real request use the same selected long-term-memory set, preventing inspector totals from describing a different prompt.
- Web SSE now forwards typed Core Part, Task, Skill and context-usage projections without requiring a trace wrapper that those projection events do not carry.
- Web refresh and `session show --json` now restore the same typed Skill projection from the canonical Core checkpoint, including cursor and session/run ownership validation; refresh no longer depends on a Web-private JSON reconstruction path.
- macOS, iOS and Android now keep remote Task observation independent from the
  original answer stream. Mobile clients use the encrypted Host endpoint,
  resume after the last session cursor and merge only non-stale Core projection
  revisions; reconnecting observation never replays a Task command.
- The remaining memory gap is user-visible provenance/citation rendering, correction/version history, and consolidation jobs on every client.
- macOS launchd automation now uses an atomic owner lease, heartbeat, stale-owner takeover and a scheduled-success watermark while preserving its bounded retry/timeout policy.
- The remaining task gap is equivalent claim/checkpoint implementations on iOS and Android, plus suspended-permission checkpoint parity across every Host.
- Android scheduled actions now fetch through the data-source reader, attach the result as typed reference context, and submit only the configured instruction through `DeviceAgentCoordinator` as Core `automated_input` with `origin=scheduled_action`. HTTP success is no longer Agent success.
- Android permission requests now persist the exact request, stable session and checkpoint as `needs_attention` rather than being auto-denied. Remaining work is same-call resume UI, stable occurrence idempotency, once-only delay, local-time recurrence and crash reconciliation.
- iOS `ChatManager` and `DeviceAgentCoordinator` now expose the same typed origin boundary, reject unknown origin, and encode scheduled input as Core `automated_input`; the scheduler now calls this path for scheduled URL reports.
- iOS scheduled URL reports now keep the configured action prompt separate from fetched reference data, pass the fetch as a typed document component, and execute the final report through `ChatManager -> DeviceAgentCoordinator -> Core` with scheduled origin. Quiet hours, dedupe, backoff, run history and notifications remain Host responsibilities.
- Web presentation now rejects non-canonical raw tool/status events instead of synthesizing run IDs, trace IDs, event IDs or sequence numbers. Typed Core projections and canonical trace envelopes are the only accepted live protocol.

### Data sources

Nola is stronger than the reference coding agents because it has user data sources, HealthKit, scheduled refresh, local-first storage, and multi-device clients. The weakness is that source configuration, successful test evidence, refresh policy, authorization, normalized output schema and context usage are not yet one shared contract.

Each configured source should persist:

- connector type and stable source ID;
- enabled-by-default versus conversation-selectable mode;
- authorization state and required platform capability;
- tested-at timestamp, test result and schema digest;
- read method and normalized content schema;
- refresh strategy, maximum age and last successful watermark;
- sensitivity classification and allowed memory namespaces;
- structured failure state and next retry;
- usage citations back to the source snapshot.

### UI and observability

Codex and Claude Code preserve intermediate events as first-class history. OpenCode treats permissions as a docked pending interaction. Open WebUI exposes share/access state at the resource layer.

Nola's unified UI event model should render:

- assistant placeholder immediately after user submission;
- stable thinking/activity title without exposing hidden chain-of-thought;
- tool/Skill/MCP/data-source events in chronological order;
- bounded arguments and results, expandable without changing message identity;
- permission request attached to the exact pending call;
- background-task progress, lease/retry state and cancellation;
- memory and data-source citations;
- incrementally parsed Markdown blocks with stable IDs.

Clients must not reconstruct these states from log strings such as `agent running_turns=x` or localized tool names.

## What not to copy

- Do not copy Open WebUI's in-place memory replacement as Nola's only history; preserve versions and supersession.
- Do not copy server-wide public-read defaults for sensitive local files, health data or memories.
- Do not place Claude/Codex coding-specific instructions into consumer chat prompts.
- Do not implement OpenCode-style behavior by matching translated prompt keywords.
- Do not expose every tool simply because a model supports function calling.
- Do not let Skill access grants become execution grants.

## Implementation priority

### P0: correctness boundary

- Finish Core memory-selection C/JNI/Swift/Kotlin boundary.
- Finish structured memory migration on every client.
- Connect every background runner to claim, heartbeat, suspended permission and ownership-safe finish.
- Make Web and clients render the same Agent events.

### P1: capability projection

- Standardize per-turn tool/Skill/MCP/data-source projection and digest.
- Add model capability and Host support gates before schemas are exposed.
- Add resource access grants for shared Skills, MCP configurations and data sources.

### P2: durable intelligence

- Implement Stage 1 extraction and Stage 2 consolidation with leases, retries, watermarks and validation.
- Add memory citations, correction, version history and usage counters.
- Persist tested data-source schemas and refresh watermarks.

### P3: performance and refinement

- Add stable prompt-fragment cache identities.
- Use provider token accounting and declared context/output limits everywhere.
- Add deterministic event replay and cross-client rendering snapshots.

## Evidence locations

- Codex memory store: `参考资料/codex-main/codex-rs/state/src/runtime/memories.rs`
- Codex memory model: `参考资料/codex-main/codex-rs/state/src/model/memories.rs`
- OpenCode compaction: `参考资料/opencode-dev/packages/opencode/src/session/compaction.ts`
- OpenCode overflow: `参考资料/opencode-dev/packages/opencode/src/session/overflow.ts`
- OpenCode permission dock: `参考资料/opencode-dev/packages/app/src/pages/session/composer/session-permission-dock.tsx`
- Claude consolidation: `参考资料/claude-code/src/services/autoDream/autoDream.ts`
- Claude scheduled tasks: `参考资料/claude-code/src/hooks/useScheduledTasks.ts`
- Open WebUI prompt payload: `参考资料/open-webui-0.10.2/backend/open_webui/utils/payload.py`
- Open WebUI tool projection: `参考资料/open-webui-0.10.2/backend/open_webui/utils/tools.py`
- Open WebUI access grants: `参考资料/open-webui-0.10.2/backend/open_webui/models/access_grants.py`
- Open WebUI skills: `参考资料/open-webui-0.10.2/backend/open_webui/models/skills.py`
- Open WebUI memories: `参考资料/open-webui-0.10.2/backend/open_webui/models/memories.py`

## Nola implementation evidence

- Core memory eligibility and C ABI: `core/src/memory.rs`, `core/src/lib.rs`, `core/swift_shims/nola_core.h`.
- Core managed-resource access contract and C ABI: `core/src/agent/resource_access.rs`, `core/src/agent/mod.rs`, `core/swift_shims/nola_core.h`.
- Android memory boundary and Host integration: `android/app/src/main/java/com/nola/core/AgentCoreMemorySelection.kt`, `android/app/src/main/java/com/nola/data/local/datastore/MemoryProfileDataStore.kt`, `android/app/src/main/java/com/nola/data/repository/ContextRepository.kt`.
- iOS memory boundary and profile projection: `Nola/Services/AgentCoreMemorySelection.swift`, `Nola/Services/ContextManager.swift`, `Nola/Services/ChatManager.swift`.
- macOS memory boundary and prompt integration: `NolaMac/Services/MacCoreMemorySelection.swift`, `NolaMac/Services/MacAssistantPreferences.swift`, `NolaMac/UI/MacChatWorkspaceView.swift`.
- CLI durable task ownership: `core/nola-store/src/schema.sql`, `cli/src/store/task.rs`, `cli/src/task_runtime.rs`.
- macOS launchd ownership and watermark: `NolaMac/Services/MacAutomationStore.swift`.
- Web canonical projection transport and restore: `cli/src/web.rs`, `cli/src/session.rs`, `cli/src/main.rs`, `web/app/static/js/agent-trace-runtime.js`.
# Prompt composition evidence audit (2026-08-03)

> This section is the current source of truth for prompt composition and prompt-cache identity. It supersedes any earlier statement in this document that the CLI still derives its default cache key only from a configured prefix and model name.

## Evidence matrix

| Dimension | Codex | OpenCode | Nola current state | Evidence-based conclusion |
|---|---|---|---|---|
| Prompt hierarchy | Keeps model base instructions separate from typed context items. `TurnContext` owns turn policy and environment state; `StepContext` freezes the exact capability roots, MCP runtime/tool snapshot, and loaded `AGENTS.md` used by one sampling request. | Builds provider/model/agent system material, discovered instructions, reminders, conversation messages, and resolved tools in the session prompt pipeline. Synthetic reminders are inserted into the latest user message for mode transitions. | Core composes ordered fragments with explicit `layer`, `trust`, and `cacheable` metadata. Policy/identity/host/reference content is not accepted as an undifferentiated string, and ordinary user messages remain outside the fragment composer. | Keep Nola's typed composition contract. Adopt the reference projects' request snapshot discipline, not their exact prompt strings. |
| Stable versus dynamic boundary | Base instructions and advertised tools are asserted stable across repeated requests. Turn/step state is snapshotted so a request cannot observe a changing MCP catalog or a different `AGENTS.md` halfway through construction. | No equivalent typed stable-prefix contract was found in the inspected session layer. System text and tools are assembled per request; provider transforms may add provider-specific behavior later. | Cacheable fragments form `stable_prefix`; non-cacheable fragments form `dynamic_suffix`. Reference-trust fragments are rejected if marked cacheable. Core returns `stable_prefix_sha256`. CLI stable identity and tool-transport policy are cacheable; workspace facts, exposed tool names, runtime instructions, skills, memories, attachments, and retrieved content are dynamic. | Nola has the clearest semantic boundary, but some conservative dynamic inputs, especially project instructions, cannot benefit from caching until they carry a revision/content identity. |
| Project instruction discovery | The canonical loaded `AGENTS.md` is part of `StepContext`. Its value is bound to the same environment snapshot as capabilities and MCP tools. | Discovers global `AGENTS.md`, optional `CLAUDE.md`, and deprecated `CONTEXT.md`; also resolves configured local globs and HTTP(S) instruction sources. When a file is read, it walks upward from that file toward the worktree root and injects nearby instruction files once per message. | Core owns deterministic scope selection and a checkpointed instruction manifest. CLI discovery supplies canonical origin/scope, content digest, observed file version and selection reason; the selected content is frozen in Turn identity. Mobile wrappers submit an explicit empty manifest when no project workspace exists. | Extend the same typed discovery contract to any non-CLI Host that exposes a project workspace, and add fixture-driven replay parity. Do not add keyword routing. |
| Permission and tool injection | Permission profile, filesystem roots, sandbox state, approval policy, and environment facts are rendered as typed context. The exact MCP tool list is frozen for a step; tool schemas remain request fields rather than prose-only promises. | Resolves registry and MCP tools before sampling. Each execution asks the permission service with the merged agent and session ruleset; plugin hooks wrap execution. A user-message `tools` field is deprecated because tools and permissions are converging on session policy. | Core prompt policy explains tool transport, while runtime catalogs and permission evaluation remain authoritative. CLI injects current workspace/tool facts dynamically. Android also supplies available tool names as dynamic Host facts. Prompt text cannot grant permission or make an unavailable tool executable. | Preserve the separation between model guidance and enforcement. Add a canonical capability snapshot ID to each request and audit event so prompt facts, advertised schemas, permission policy, and executed tools can be proven to come from the same snapshot. |
| Cache and request identity | API-key root and child-agent requests use the session ID as `prompt_cache_key`, even though child requests have different thread/request IDs. Tests also assert stable instructions and tool ordering across repeated requests. | No session-layer stable-prefix digest or comparable cache-identity contract was found in the inspected files. Any provider cache behavior is downstream of session prompt assembly. | Core computes the provider prefix identity from stable prompt, exact Step tool schema, and model invocation policy. Separately, every model effect carries a canonical request-snapshot digest over the frozen Turn context, exact Step tool projection, and invocation semantics. CLI/Web, Android, and Apple Agent Host consumers receive these Core-owned identities; auxiliary requests remain isolated. | Nola now has stronger request-world identity than either inspected session layer. Remaining work is provider-native cache-breakpoint mapping and privacy-safe provider telemetry, not another client-local hash. |

## Capabilities already absorbed by Nola

- **Least-authority tool selection.** Following the reference agents' preference for dedicated file/search/edit tools over a generic shell, Core now tells every Host model to choose the narrowest structured capability that directly represents the operation. This is semantic model guidance only; the catalog and permission evaluator remain authoritative, and no keyword routing was introduced.
- **Provider adaptation without policy forks.** OpenCode selects a substantially different full system prompt for GPT, Claude, Gemini, Kimi, and other families. Nola keeps identity, safety, permission, Skill, memory, completion, and response policy identical across providers; Core freezes only a small typed `ModelInstructionProfile` plus a `ModelInvocationPlan` for native tool calls, reasoning continuity, continuation, streaming, and cache behavior. This keeps protocol compatibility without making product behavior provider-dependent.
- **Explicit Anthropic cache configuration.** Anthropic cache enablement and TTL now come from the same per-session `LlmConfig` used by the OpenAI-compatible path. The provider adapter no longer reads hidden `NOLA_CLI_PROMPT_CACHE*` process state while constructing a request, so the Core invocation snapshot and provider wire behavior remain reproducible.
- **Durable delegated work rather than a UI-only Task wrapper.** Nola promotes Agent work into Core `TaskProjection` with parent/child lineage, child-session identity, permission and cancellation scopes, leases, heartbeats, retry state, input/completion watermarks, cancellation trees, and terminal result references. CLI adds persisted workers, event-driven waits, a single-writer workspace guard, and a code-enforced child tool allowlist that removes recursive delegation and user-input tools. Delegated work inherits explicit conversation grants but not parent-Turn grants, matching the Core permission contract.

- Core owns deterministic prompt ordering instead of allowing each client to concatenate arbitrary system strings.
- `AgentPromptTrust` distinguishes policy, Host facts, and untrusted reference material.
- Reference material fails closed when incorrectly marked cacheable.
- Stable-prefix content has a deterministic SHA-256 identity exposed through the FFI result.
- CLI main-turn cache identity now derives from the Core digest and includes provider/model namespace isolation; an explicit configured key is still honored.
- Auxiliary classifier and compaction calls are isolated from the main-turn cache identity.
- Runtime tool availability and permission decisions remain outside prompt prose, preventing model text from escalating authority.

## Remaining gaps

1. **Provider-native cache boundaries are not unified.** An OpenAI-compatible cache key is not equivalent to Anthropic cache-control breakpoints. Core should expose stable/dynamic segments and digest; each provider adapter should map them without rebuilding semantic boundaries.
2. **Project instruction I/O remains Host-specific.** Core now owns the typed manifest, scope selection, precedence, deduplication, provenance and invalidation identity; CLI supplies real filesystem evidence. Other project-capable Hosts still need equivalent discovery plus cross-wrapper replay fixtures.
3. **Cross-wrapper contract coverage is incomplete.** Add fixture-driven tests that feed the same fragments through Rust, Swift, and Kotlin and assert rendered text, fragment order, rejection behavior, and digest equality.
4. **Cache observability is partially implemented.** Core emits stable-prefix, model-prefix, request-snapshot and cache-diagnostic identities; the CLI adapter records provider/model cache identity and provider-reported usage. Native inspectors still need a consistent privacy-safe presentation and request-capture fixtures.
5. **Instruction cacheability is conservative but inefficient.** Project instructions should become cacheable only after their canonical scope and content revision are represented in the stable identity; do not cache them solely because they usually change slowly.
6. **Live steering is implemented at the Core/CLI Host boundary but not yet complete across clients.** Core owns bounded ordered Task guidance, stable message identities, acknowledgement watermarks, parent ownership and checkpoint state. The CLI Host injects both delegated guidance and interactive user steering only at model/Host response boundaries, and `send_agent_message` derives session/Turn identity from the trusted Host request rather than model arguments. Mac/Web still need explicit composer/task UI entry points, and Swift/Kotlin replay fixtures have not yet proved parity.

## Recommended implementation order

1. Add cross-language golden fixtures and provider request-capture tests for
   prompt, request-snapshot and capability-projection parity.
2. Extend typed project-instruction discovery to every Host that exposes a
   project workspace.
3. Surface the lifecycle Hook registry digest in diagnostics and replay tooling
   without granting Hooks mutation authority.
4. Add privacy-safe cache telemetry and expose cache misses caused by changed
   prompt/tool/policy revisions in debug logs.
5. Finish native and Web steering UI, surface queued/applied acknowledgement
   state, and add replay/race fixtures for CLI/Web/macOS before treating live
   steering as cross-platform complete.

### Active-turn and delegated-task steering update (2026-08-06)

- **Codex reference:** `turn/steer` validates the expected active Turn, rejects
  non-steerable Turns, and records multiple user messages inside one Turn.
  Collaboration tooling separately supports follow-up guidance and interrupts
  for delegated Agents.
- **OpenCode reference:** asynchronous session prompts and abort endpoints make
  session control explicit, while the task tool persists a child session for
  later continuation.
- **Nola adoption:** `AgentInputQueue` remains the single model-visibility
  boundary. A process-local `AgentBoundarySubmissionSource` may supply input
  while a Host effect is streaming, but `CoreHostLoop` alone mutates Runtime
  state. Interactive steering retains `InteractiveUser` authority; delegated
  guidance uses `DelegatedAgent` and cannot manufacture fresh authorization.
- **Delivery invariant:** parent guidance is durably queued before child
  delivery. Child acceptance is idempotent by submission ID, and the parent is
  acknowledged by stable message ID only after the enclosing Host/checkpoint
  flow completes. Parent command FIFO therefore orders queue, acknowledgement,
  and terminal Task update without guessing a Core-assigned sequence.
- **Remaining evidence:** no build or end-to-end run has yet proved the new
  callback signature, mid-stream queue behavior, crash replay, or Mac/Web UI.

### Stable system-policy adoption update (2026-08-06)

| Reference strength | Nola comparison | Adoption decision |
| --- | --- | --- |
| Claude Code explicitly prohibits unnecessary features, refactors and defensive handling | Nola already prohibited extra deliverables and gold-plating, but the implementation boundary was less concrete | Strengthened `nola.core.task-completion` with smallest-coherent-change, concrete-consumer compatibility, and real-boundary validation rules |
| Claude Code researches cheaply before asking and continues approved work without repeated confirmation | Nola already has `investigate-before-question`, scoped durable grants and end-to-end task completion | Keep the existing typed policies; do not duplicate them in model- or client-specific prompts |
| Codex and Claude coordinate delegated work without having the parent redo it | Nola had spawn/wait/stop semantics but no stable parent non-duplication rule | Strengthened `nola.core.tool-execution-order`: continue independent work, guide only with new constraints, and wait only on a blocking dependency |
| Reference coordinators treat worker output as input to review, not authority | Nola had provenance-bearing Task state but the stable prompt did not state the trust consequence | Strengthened `nola.core.truthful-outcomes`: delegated conclusions retain provenance and consequential claims require source verification |
| OpenCode ships large provider-specific prompt bodies | Nola composes short typed Core fragments plus model capability profiles | Do not copy monolithic provider prompts. Keep shared safety/execution semantics stable and vary only capability-specific guidance |
| Several reference prompts repeat tool and permission rules in natural language | Nola evaluates concrete calls through Core capability and permission contracts | Do not adopt keyword routing or prose authorization. Prompt text guides selection; Core remains the enforcement boundary |

The adopted fragments remain cacheable because they are versioned Core policy,
contain no workspace data, and participate in the stable-prefix digest. Project
instructions, retrieved context, memory records and tool results remain separate
typed layers with explicit provenance.

## Prompt audit evidence paths

### Codex

- `参考资料/codex-main/codex-rs/core/src/context/mod.rs`
- `参考资料/codex-main/codex-rs/core/src/context/user_instructions.rs`
- `参考资料/codex-main/codex-rs/core/src/context/permissions_instructions.rs`
- `参考资料/codex-main/codex-rs/core/src/context/environment_context.rs`
- `参考资料/codex-main/codex-rs/core/src/context/available_skills_instructions.rs`
- `参考资料/codex-main/codex-rs/core/src/session/step_context.rs`
- `参考资料/codex-main/codex-rs/core/tests/suite/prompt_cache_key.rs`
- `参考资料/codex-main/codex-rs/core/tests/suite/prompt_caching.rs`

### OpenCode

- `参考资料/opencode-dev/packages/opencode/src/session/instruction.ts`
- `参考资料/opencode-dev/packages/opencode/src/session/prompt.ts`

### Extension systems

- `参考资料/codex-main/codex-rs/tools/src/mcp_tool.rs`
- `参考资料/codex-main/codex-rs/tools/src/tool_discovery.rs`
- `参考资料/codex-main/codex-rs/tools/src/tool_executor.rs`
- `参考资料/claude-code/src/schemas/hooks.ts`
- `参考资料/claude-code/src/services/tools/toolExecution.ts`
- `参考资料/claude-code/src/services/tools/StreamingToolExecutor.ts`
- `参考资料/opencode-dev/packages/opencode/src/tool/registry.ts`
- `参考资料/opencode-dev/packages/opencode/src/tool/skill.ts`
- `参考资料/opencode-dev/packages/opencode/src/server/routes/instance/mcp.ts`

### Nola

- `core/src/agent/prompt_composer.rs`
- `Nola/Services/AgentCorePromptComposer.swift`
- `android/app/src/main/java/com/nola/core/AgentPromptComposer.kt`
- `cli/src/core_agent_runtime.rs`
- `cli/src/agent_host_driver.rs`
- `cli/src/llm.rs`
- `core/src/agent/capability.rs`
- `core/src/agent/capability_projection.rs`
- `core/src/lifecycle_hooks.rs`
- `cli/src/mcp_host.rs`
- `cli/src/builtin_skills.rs`
- `cli/src/tool_policy.rs`
## Capability activation ordering closure

Core capability activation is an authoritative protocol event, not transient UI state. A Host may receive it before the corresponding assistant run or message projection exists. Clients therefore must not discard it based on the currently selected conversation or on whether a presentation row has already been created.

- Web accepts both canonical Core envelopes and flattened client events, stages early activations in a bounded `session + turn` journal, and replays them into every matching run projection.
- Android stages activations per conversation, keeps only the newest Core cursor for each `turn + source`, restores them when the conversation is selected again, and clears staged entries after the assistant response is persisted.
- Stable presentation identity is `core-capability:{turn}:{source}`; duplicate or stale cursors update neither order nor visible state.
- iOS and macOS already create/own the active assistant turn before consuming activation events, but remain covered by the shared capability-activation fixture.

Validation now includes early arrival, stale and duplicate delivery, conversation switching, canonical-envelope ingestion, durable message completion, and a real HTTP SSE stream on Android.
## Host response rejection taxonomy

Codex maps internal failures to stable client protocol categories, while OpenCode persists tool errors as typed session events. Nola now applies the same separation at its stricter Core/Host boundary:

- completed Host effects retain a bounded journal of 256 identity tombstones without retaining result bodies;
- a response for a completed, identity-matching effect is `duplicate_response`, while an unrecognized effect is `unknown_effect`;
- wrong request, response family, Host/session owner, Turn, catalog revision, lease, availability epoch, and non-monotonic sequence remain distinct codes;
- codes are grouped as `benign_replay`, `stale_authority`, or `protocol_violation`, but terminal responses remain exactly-once and every post-terminal envelope is rejected;
- rejection text includes stable `code` and `category` fields plus correlation identity, never the response payload.

This prevents retry noise from being confused with forged ownership or a Host implementation bug. The native ABI now exposes `error_code` and `error_category`; Swift and Android preserve them as typed protocol errors instead of reparsing display text. Remaining work is consistent CLI/Web diagnostic presentation and localized debug surfaces without leaking protocol metadata into ordinary assistant prose.
