> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Nola Documentation

This directory separates current normative contracts from implementation guides,
verification evidence, platform notes, and historical records. When documents
overlap, use the authority order below instead of copying a second version of a
protocol into another file.

## Normative product and architecture

- `../实施与审计/ARCHITECTURE.md`: highest-authority system contract for data ownership,
  Nola Store, Agent Core, Host effects, canonical events, cross-platform modules,
  and migration. When another architecture document conflicts with it, this
  document wins.
- [Executable architecture](../../软件架构设计/README.md): module contracts, lifecycle,
  evidence/outcome semantics, audit ledger, performance measurement and vertical
  acceptance. Target contracts and verified implementation are explicitly separate.
- `../../软件架构设计/总体设计/REQUIREMENTS.md`: current product behavior and release acceptance requirements.
- `../../软件架构设计/总体设计/SOFTWARE_DESIGN.md`: current implementation inventory and integration details;
  it is subordinate to `../实施与审计/ARCHITECTURE.md`.
- `../../软件架构设计/总体设计/CORE_HOST_ARCHITECTURE.md`: normative Core/Host ownership, Submission/Event,
  effect lifecycle, capability, permission, compaction, and recovery contract.
- `../../软件架构设计/总体设计/CORE_HOST_LAYERING.md`: concise ownership rules and invariants distilled from
  Codex CLI, OpenCode, Claude Code, and Open WebUI.
- `../实施与审计/AGENT_CORE_REWRITE.md`: current single-`AgentRuntime` rewrite decision and
  removal invariants.
- `../参考研究/CORE_HOST_REFERENCE_SYNTHESIS.md`: evidence and adopted/rejected mechanisms
  from the four reference projects; informative where the normative architecture
  document is more specific.
- `../实施与审计/AGENT_TRACE_ARCHITECTURE.md`: historical trace-driven design; not authority for
  business state or chat recovery. Use canonical Store/Application projections.
- `../../软件架构设计/模块/12-记忆数据源与自动化/data-source-centered-architecture.md`: data-source, freshness, scheduling, and
  context boundaries.
- `../../软件架构设计/模块/13-客户端投影/MOBILE_SHARED_CORE.md`: iOS/Android shared Core and FFI integration.
- `../../软件架构设计/模块/13-客户端投影/mobile-agent-architecture.md`: mobile Host, trust, permission, and transport model.
- `../../UI和交互设计/macos-app-design.md`: macOS-native product and platform integration decisions.

## UI and interaction contracts

- [Conversation and tool UI](../../UI和交互设计/CONVERSATION_UI_DESIGN.md): cross-client presentation,
  disclosure, approval boundaries, implementation stages, and acceptance checks.
- `../../UI和交互设计/PROMPT_UI_STREAMING_SOP.md`: platform-neutral streaming conversation,
  incremental Markdown, stable identity, animation, scrolling, and visual QA rules.
- `../../UI和交互设计/tool-activity-presentation.md`: tool and long-running activity presentation
  inside the assistant message.

The streaming SOP explains the general engineering method. Product implementations
must consume committed canonical Application projections; they must not create a
platform-specific execution state machine.

## Verification and release evidence

- [CLI Runtime and Interaction Audit (2026-09-02)](../实施与审计/CLI_RUNTIME_AND_INTERACTION_AUDIT_2026-09-02.md)

- `../../软件架构设计/模块/15-观测与验收/TESTING.md`: evidence levels and release gates. A reducer fixture is not E2E,
  and a successful build is not runtime evidence.
- `../../软件架构设计/模块/15-观测与验收/mobile-agent-test-plan.md`: mobile execution-mode, permission, lifecycle, and
  real-device verification matrix.
- `../../软件架构设计/模块/15-观测与验收/privacy-release-checklist.md`: privacy manifests, credential handling, transport,
  exports, and store-review evidence.

Cross-platform Agent tests use
`core/tests/fixtures/long_task_trace_v1.json` as the canonical fixture and add at
least one real Host execution per client. Platform copies may not diverge
semantically.

## APIs and persistence

- `../../软件架构设计/模块/01-领域与协议/API.md`: API reference.
- `../../软件架构设计/模块/01-领域与协议/API.zh-CN.md`: Chinese API reference.
- `../../软件架构设计/模块/04-存储与索引/local-sqlite-store.md`: local persistence boundaries and schema ownership.

## History

- `../实施与审计/CHANGELOG.md`: dated user-visible and architectural changes.
- `../实施与审计/DISTRIBUTED_FIXES_2026_02_14.md`: historical distributed-backend remediation.

Historical entries describe the implementation at that date. They do not override
`../实施与审计/ARCHITECTURE.md`, current requirements, protocol contracts, or the test evidence
standard.
