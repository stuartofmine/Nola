> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Cross-platform UI and model configuration parity

Status: implementation in progress. This checklist is a completion gate, not a
claim that current clients already satisfy it.

Current interaction (checked 2026-11-08) is defined by [conversation design](CONVERSATION_UI_DESIGN.md), [inline inspector behavior](CONVERSATION_INSPECTOR.md) and [client configuration ownership](../../architecture/模块/13-客户端投影/CLIENT_INTERACTION_AND_CONFIGURATION.md). The [prototype](../prototype/README.md) remains a visual/demo reference. “Parity” here means equivalent product semantics, permissions, data scope, and recovery; it does not require identical page composition. Phone starts with conversation browsing and separates activity, library and settings; desktop uses a single 48px bar, browser-style content tabs, an independent sidebar, and a right-side work inspector. The prototype demonstrates these choices locally. It is not production implementation evidence; use the [feature coverage matrix](../prototype/FEATURE-COVERAGE.csv) to distinguish source evidence, demo behavior, and validation.

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

## 当前逐页验收补充（2026-11-09）

设置分类入口使用主题语义色提供悬停、按下和足够内边距；嵌入式页面沿用宿主导航标题及转场。数据刷新不插值整页文本换行。共享组件的静态文案和动态状态都必须进入各客户端的本地化覆盖检查。

资料页的返回操作保留实际来源：从设置进入返回设置，从信息入口进入返回信息；显示返回标签和系统返回使用相同路径，不清除来源页面来硬编码目标。清单读取或保存回执不完整时保留编辑状态并显示可恢复错误；未确认保存不自动重复提交。

本轮覆盖、失败记录和未验证的平台边界见逐页验收记录（本地工作记录，不随仓库分发）。该记录不把所有验收清单勾选为完成。

## 历史证据

此前日期化进度、旧模板运行时差距和验证流水已移入历史证据（本地工作记录，不随仓库分发）。不再把其中“下一步”当作现行待办。
