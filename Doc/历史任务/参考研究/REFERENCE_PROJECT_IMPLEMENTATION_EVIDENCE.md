> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Reference Project Absorption Evidence

Updated: 2026-08-06

## Scope

This document compares Nola with the checked-in reference projects under:

- `参考资料/codex-main`
- `参考资料/claude-code`
- `参考资料/claude-code-system-prompts-main`
- `参考资料/opencode-dev`

It covers two separate surfaces:

1. System-prompt and instruction composition.
2. Runtime capabilities and behavioral contracts.

The status column describes current source evidence. It does not imply that a
build, unit test, or end-to-end test has passed unless validation evidence is
listed explicitly.

## Reference strengths

| Project | System-prompt strength | Runtime strength | What Nola should absorb |
|---|---|---|---|
| Codex CLI | Stable base instructions separated from dynamic environment, project instructions, tool schemas, and turn state | Typed tool calls, approval/sandbox policy, resumable sessions, compaction, cancellation, event-driven UI | Keep policy in Core, keep Host execution mechanical, preserve stable prompt prefixes, and make every effect recoverable or explicitly non-replayable |
| Claude Code | Strong instruction hierarchy, concise behavioral guidance, Skills/subagents/hooks as explicit extension concepts | Long-running task delegation, user steering, permission scopes, project memory and lifecycle hooks | Make extension activation model-selected rather than keyword-routed, persist task guidance, and keep user steering available during execution |
| OpenCode | Provider-neutral prompt/runtime separation and configurable agents | Client/server session model, event bus, provider adapters, permission requests, plugins and MCP integration | Treat clients as projections over one canonical runtime and expose extensions through typed catalogs rather than client-owned routing |

## System-prompt comparison

| Concern | Reference behavior | Previous Nola risk | Current Nola evidence | Status |
|---|---|---|---|---|
| Instruction hierarchy | Base policy, project instructions, environment and turn context have explicit precedence | Platform prompts could drift or concatenate multiple system messages | Shared prompt components and Core turn snapshots are used by the agent runtime; macOS rejects multiple or misplaced system messages at the provider boundary | Absorbed, validation pending |
| Stable versus dynamic prefix | Stable instructions and tool schemas are kept separate from volatile turn data | Dynamic content could invalidate provider prompt caches | `core/src/prompt_cache.rs`, `core/src/agent/model_capability.rs`, and Host model request digests separate stable prefix, invocation policy and request snapshot | Absorbed, validation pending |
| Project instructions | Repository instructions are discovered and scoped, not copied into every client implementation | Different clients could interpret repository guidance independently | Core owns context components; clients provide platform data rather than rewriting user input | Partially absorbed |
| Prompt templates | Templates influence response behavior without replacing the user's request | Template expansion previously copied or rewrote user content | iOS/macOS template reads use Core resource visibility; macOS injects templates as context components rather than rewriting the user message | Absorbed, validation pending |
| Tool selection | The model chooses from schemas; policy evaluates the concrete call | Keyword routing caused multilingual and ambiguous-query failures | Repository guardrail and runtime design prohibit keyword routing; Core freezes a capability projection for each turn | Absorbed |
| Provider-specific reasoning | Provider continuation/reasoning state is passed back using provider protocol | Clients could strip or synthesize reasoning state | `ModelProviderContinuationState` is staged on the active turn and committed only when model completion succeeds | Absorbed, validation pending |

## Runtime capability comparison

| Capability | Codex / Claude Code / OpenCode lesson | Nola implementation evidence | Remaining gap | Status |
|---|---|---|---|---|
| Canonical session state | UI must project events rather than reconstruct lifecycle from text chunks | `core/src/agent/runtime_event.rs`, `core/src/agent_trace.rs`, `ConversationProjection` | Full cross-platform projection parity still needs E2E evidence | Implemented, unverified |
| Input idempotency | Retried client submissions must not duplicate turns | `AgentJournalState` stores bounded accepted submission identities; `AgentRuntime::submit` is the only idempotency boundary | Need checkpoint migration and duplicate-submit E2E coverage | Implemented, unverified |
| Host effect correlation | Every model/tool operation needs stable identity and ownership | `HostRequestEnvelope` carries request/effect/Host/lease/session/turn/catalog/epoch identity and monotonic response sequence | Need real remote-host reconnect tests | Implemented, unverified |
| Transport failure | Driver failure must become runtime state, not disappear as a function error | `cli/src/core_host_loop.rs` now submits `HostDisconnected`, publishes projections and leaves effects journaled | Other platform Host loops need equivalent runtime evidence | Implemented in CLI Host loop |
| Replay safety | Retrying a mutation blindly is unsafe | `HostReplaySafety` distinguishes idempotent, recoverable and explicit-decision effects | Recovery-decision UI remains to be proven on every client | Implemented, UI evidence missing |
| Cancellation | Cancellation requires an acknowledgement boundary | Core models `Cancel`, `CancelTask`, `CancellationAcknowledged`, cancellation linkage and task cancellation trees | Physical interruption latency needs real Host tests | Implemented, unverified |
| Live steering | Parent input during a delegated task must be durable and acknowledged | Task guidance has stable message IDs, monotonic sequence, ownership-token validation, bounded queues and completion gates | Platform composer UX and reconnect delivery need E2E evidence | Implemented, unverified |
| Background tasks | Long work needs leases, heartbeats, retries and ownership | `TaskDurability` persists worker/token/lease/retry/input/completion watermarks | Scheduler integration across all platforms is not yet proven | Implemented, partially integrated |
| Context management | Compaction must combine deterministic selection with model summarization | Core owns context window state and compaction requests; provider Host supplies structured summary output | Real-model compaction quality and provider-window discovery require E2E evidence | Implemented, unverified |
| Memory | Long-term memory needs typed lifecycle, not prompt concatenation | Core memory runtime owns extraction/consolidation jobs, leases, retry and projection | Cross-session recall quality and all-client enablement remain unproven | Implemented, partially integrated |
| Skill activation | Skills should be discoverable resources and model-selected capabilities | Core Skill runtime plus CLI/Web/macOS/Android catalogs; no language-keyword routing | Activation behavior needs multilingual real-model sessions | Implemented, unverified |
| MCP | MCP discovery is not execution authorization | MCP metadata/content access is separated from tool capability grants | Remote MCP reconnect and credential-redaction E2E coverage is incomplete | Implemented, partially verified by source |
| Managed resource access | Read, edit, delete and execution are separate permissions | `core/src/agent/resource_access.rs` owns read metadata, read content, edit, delete and sharing decisions; execution authorization is always false | Sharing UI and remote-account principals remain incomplete | Implemented, unverified |
| Data sources | A source must persist how it was validated and how it can be read again | macOS registry stores validation/read contracts and resource identity; iOS URL sources persist access metadata; CLI/Web reconcile sources through Core | Android custom-source authoring and cross-device sharing are incomplete | Partially absorbed |
| Prompt cache | Cache identity must diagnose why reuse failed without leaking prompt content | Core emits bounded digest-based prompt-cache diagnostics | Provider-specific cache hit E2E evidence is missing | Implemented, unverified |

## Concrete changes absorbed into Nola

| Change | Authoritative source |
|---|---|
| Bounded, transactional submission idempotency | `core/src/agent/session.rs`, `core/src/agent/runtime/submission.rs`, `core/src/agent/protocol.rs` |
| Bounded queued input and deterministic current-turn delivery | `core/src/agent/input_queue.rs`, `core/src/agent/runtime/turn_lifecycle.rs` |
| Durable Host effect journal, response dedupe, lease/epoch checks and replay rebinding | `core/src/agent/recovery.rs`, `core/src/agent/runtime/effect_dispatch.rs`, `core/src/agent/runtime/host_response.rs` |
| Driver failure converted to Core Host-disconnect state | `cli/src/core_host_loop.rs` |
| Durable task guidance, ownership, heartbeat, retry and cancellation tree | `core/src/agent/task.rs`, `core/src/agent/protocol.rs` |
| Core-owned resource access ABI | `core/src/agent/resource_access.rs` |
| CLI/Web Skill, MCP and data-source reconciliation | `cli/src/resource_access.rs`, `cli/src/store/skill.rs`, `cli/src/web.rs` |
| iOS prompt-template and URL-source resource access | `Nola/Services/PromptTemplateService.swift`, `Nola/Services/URLFetcherService.swift` |
| macOS Skill/MCP/template/data-source resource boundaries | `NolaMac/Services/MacCoreResourceAccess.swift`, `NolaMac/Services/MacExtensionsService.swift`, `NolaMac/Services/MacDataSourceRegistry.swift`, `NolaMac/Services/MacWebSourceStore.swift` |
| Android Skill/MCP resource projection and atomic reconciliation | `android/app/src/main/java/com/nola/core/AgentCoreResourceAccess.kt`, `android/app/src/main/java/com/nola/ui/screen/chat/ChatViewModel.kt`, `android/app/src/main/java/com/nola/ui/screen/intelligence/IntelligenceViewModel.kt` |

## Deliberately not copied

| Pattern | Decision |
|---|---|
| Language-specific keyword routing | Rejected. Model tool selection and concrete Core policy evaluation remain authoritative. |
| Treating Skill or MCP visibility as execution permission | Rejected. Resource access decisions always report `tool_execution_authorized = false`. |
| Client-specific system prompts that redefine the agent | Rejected. Platform Hosts provide capabilities and context; Core owns behavior. |
| Blind replay of mutating tools | Rejected. Replay safety is declared per capability and checked against the current Host catalog. |
| Hiding tool/task lifecycle in prose-only status | Rejected. Core emits structured events and trace projections for clients to render. |

## Verification debt

The following evidence is still required before this absorption effort can be
called complete:

1. Compile Core, CLI/Web, iOS, macOS and Android after the cross-platform ABI changes.
2. Run Core tests for submission idempotency, Host response correlation, replay safety, task guidance and resource grants.
3. Run real-model multilingual sessions proving model-selected Skill/Web/MCP behavior without keyword routing.
4. Run remote Host disconnect/reconnect sessions covering streamed model output, tools, permissions and cancellation.
5. Run cross-client E2E cases proving identical tool traces, context usage, permission persistence and resource deletion behavior.
6. Verify prompt-cache diagnostics and model-assisted compaction against providers that expose real usage and continuation metadata.
