> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Cross-platform UI and model configuration parity

Status: implementation in progress. This checklist is a completion gate, not a
claim that current clients already satisfy it.

Current interaction (checked 2026-11-08) is defined by [conversation design](CONVERSATION_UI_DESIGN.md), [inline inspector behavior](CONVERSATION_INSPECTOR.md) and [client configuration ownership](../软件架构设计/模块/13-客户端投影/CLIENT_INTERACTION_AND_CONFIGURATION.md). The [prototype](../../docs/design/nola-ui-prototype/README.md) remains a visual/demo reference. “Parity” here means equivalent product semantics, permissions, data scope, and recovery; it does not require identical page composition. Phone starts with conversation browsing and separates activity, library and settings; desktop uses a single 48px bar, browser-style content tabs, an independent sidebar, and a right-side work inspector. The prototype demonstrates these choices locally. It is not production implementation evidence; use the [feature coverage matrix](../../docs/design/nola-ui-prototype/FEATURE-COVERAGE.csv) to distinguish source evidence, demo behavior, and validation.

The current Web conversation uses Svelte components (`web/frontend/src/pages/Chat.svelte`), with one mounted model/settings owner and responsive drawers. Historical template-migration notes are archived below. This does not prove every product route or every provider workflow is complete.

Current deltas include declared native effort menus, one mobile composer surface, inline independent inspector sections, prompt-template controls in model settings on Mac/iOS, themed focus/hover states, and cancellable native/browser navigation. Platform limits are documented in the configuration ownership contract; do not mark a parity checkbox complete merely because shared Core tests pass.

## Product contract

Mac is the desktop interaction reference. Web must expose equivalent product
information, actions, configuration scope, defaults, validation, and execution
semantics, while using the accepted responsive composition in the current
prototype. Mobile adapts navigation to conversation-first work and
conversation browsing; it does not copy the desktop three-column shell.
Browser restrictions must be explained with an equivalent supported action;
silently hiding functionality is not parity. iOS and Android adapt navigation and
touch targets, not the underlying model or permission semantics.

The visual direction follows the restrained system typography, neutral surfaces,
consistent spacing, outlined icons, and accessible hierarchy described in
https://developers.openai.com/plugins/concepts/ui-guidelines . Nola retains its
identity; OpenAI branding and host-specific widget behavior are not copied.

## Required implementation and acceptance

- [ ] [Unified conversations and saved Agents](MULTI_AGENT_NAVIGATION.md): one
      conversation identity/history/composer for single and multiple participants;
      no separate Room destination or public Context conversations.
- [ ] One New conversation action (including its keyboard/menu equivalents),
      with participants selected in the unified draft. Assistant navigation and
      create/save/edit/archive/pinning only manage configuration, never open chat.
      Separate history filtering, unsent drafts without empty sessions,
      in-place participant changes, and
      immutable per-participant configuration snapshots with explicit updates.

- [ ] Model center: Local / API tabs, consistent page header and action hierarchy.
- [ ] Svelte migration: chat, model center, overview, data sources, capabilities,
      automations, results, settings and authentication; build/packaging and
      client routing preserve Rust authentication and authorization boundaries.
- [ ] API endpoints: name, URL, credentials, explicit wire protocol, enabled state,
      model discovery, ordering, validation, save/cancel, and deletion confirmation.
- [ ] Model profiles: identity, display name, priority, allowed uses, sensitive-data
      opt-in, parameters, declared thinking capability, and availability state.
- [ ] Protocol-aware thinking: Responses declared efforts; other protocols map
      presets to actual effort/budget. No prompt rewrites or invented capability.
- [ ] Sparse per-conversation/per-model overrides win over model/global defaults;
      zero is explicit. Inheritance, reset, persistence, and scope are visible.
- [ ] Effective settings freeze at submission and reach the actual provider request
      in local and connected-host execution on all platforms.
- [ ] Composer: opaque surface, aligned hint/caret, permission/thinking footer,
      no context meter; transcript is the sole location for streaming output.
- [ ] Overview: expanded sections, useful model/workspace actions, compact context
      usage, friendly sandbox name without exposing internal paths.
- [ ] Files: PDF and common document preview, clear unreadable/unsupported errors,
      security-scoped lifetime where applicable, safe browser-host authorization.
- [ ] Markdown: tables, math, code, emphasis, long paths, wrapping, and spacing.
- [ ] Lifecycle: progress, permissions, cancellation, errors, retry, scroll anchor,
      conversation switching, restore and relaunch preserve canonical state.
- [ ] Consistent neutral surfaces, spacing, typography, focus, contrast, dark mode,
      reduced motion, text scaling, localization, and narrow-width layout.

Each row requires executable behavioral coverage and observed rendered UI on
the affected platform. Request semantics require captured HTTP assertions, not
only view snapshots. Web/Mac parity requires matched fixtures and actions.
Window-level conversation acceptance follows VISUAL_ACCEPTANCE_CONTRACT.md.

## 历史证据

此前日期化进度、旧模板运行时差距和验证流水已移入[历史证据](../历史任务/实施与审计/CROSS_PLATFORM_UI_PARITY_EVIDENCE_20260922.md)。不再把其中“下一步”当作现行待办。
