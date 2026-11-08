> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# macOS UI information architecture

This document defines the product boundaries and visual hierarchy of the native macOS client. It is a contract for implementation, not a list of independent screens.

Current desktop interaction (source checked 2026-11-08) follows the [conversation contract](CONVERSATION_UI_DESIGN.md) and [inline inspector contract](CONVERSATION_INSPECTOR.md): one 48px global bar, browser-style content tabs, an independent sidebar/list, and a right-side work inspector. The page destination table below defines ownership, not a second global navigation row or large canvas title. The static prototype is a visual reference, not proof of native implementation or acceptance; outdated prototype page switching does not override the inline inspector contract.

## Primary navigation

The primary sidebar contains user destinations, not implementation concepts.

| Destination | User question | Owns | Does not own |
| --- | --- | --- | --- |
| Chat | What am I working on now? | Conversations, current workspace, model and host selection, live agent progress | Global provider setup, permission administration |
| Data | What information may Nola use? | Data sources, validation, freshness, default/session availability, provenance | Model configuration, operating-system permissions |
| Automations | What should happen later or repeatedly? | Schedules, triggers, runs, failures and outputs | Data-source setup, Skill editing |
| Models | Which intelligence runs the task? | API providers, local models, parameters, lifecycle and health | Host connection and tool authorization |
| Skills & MCP | What workflows and external capabilities are available? | Skill/MCP gallery, enablement, inspection, editing and connection health | macOS privacy permissions |
| System Access | What can this Mac permit? | Accessibility, screen recording, notifications and other OS grants | Tool-by-tool conversation policy |

Activity, diagnostics, notifications, prompt templates and data management are secondary destinations reached from the destination that owns them or from Settings. They should not compete with daily work in the primary navigation.

## Conversation and saved assistants

The current target is [unified conversations and saved Agents](MULTI_AGENT_NAVIGATION.md).
Chat has one public conversation type and one history directory. A conversation
with one assistant can gain participants in place; there is no separate Room
page, window, creation flow or history list.

The sidebar orders daily actions first: New conversation/Search, pinned
assistants, the unified conversation list, then workspace management destinations.
New conversation is the sole creation entry; menu and keyboard equivalents
invoke that same action. Its unified draft starts with Nola, and participants are
selected inside the conversation. Clicking an assistant opens configuration
details only, with no composer, chat draft, automatic history filter or new
session. Clicking a historical topic resumes that conversation. Drafts survive
navigation; history filtering is a separate list action.

The assistant section supports create, save, edit, duplicate, pin/reorder and
archive. Its row action opens configuration details. Saving stays in those details;
assistant rows, search results and editors never offer a second chat entry.
Saved defaults include separate public responsibilities and working instructions.
Participants capture configuration revisions; later profile edits do not silently
change existing conversations or active Runs. Model controls belong to the single
participant, or to an explicit member in a multi-participant conversation.

These are implementation targets, not claims that the current native sidebar or
Room runtime already implements the unified design.

## Page hierarchy

Every primary page uses the same four layers:

1. **Page header**: sidebar control, destination icon, title, one-line purpose and page-wide actions.
2. **Summary**: status and metrics that answer whether the destination is healthy. It is omitted when it adds no decision value.
3. **Content**: sections, gallery or master-detail browser. One page uses one dominant browsing pattern.
4. **Detail**: selection-specific editing and diagnostics. Destructive actions live here, not in the page header.

## Layout and component authority

Use `MacDesignTokens`/`MacDesignColor` in `NolaMac/UI/MacPageChrome.swift`, theme assets and the responsive workspace layout for actual widths, gutters, targets and motion. Do not keep a second hard-coded 232/244-point sidebar or 300/360-point inspector specification in design prose. Native window layout adapts to available space; the central transcript keeps a readable width.

Borders and keyboard focus follow semantic theme tokens. Hover and pressed fills must remain visible in dark and light mode without changing control size. Reply copy actions have no permanent border or filled surface; touch targets and text insets remain usable.

## Interaction contract

- Hover may change fill or emphasis but never size or surrounding layout.
- Press feedback is brief and local to the control.
- Disclosure rotates one stable chevron and reveals content without replacing the row.
- Streaming content owns its reveal animation; ancestor layouts must not animate every delta.
- Loading, running and waiting states retain their layout footprint to prevent jumps.
- Reduced Motion removes translation, scaling and continuous decorative movement while retaining state changes.

## Ownership rules

- A setting appears in one canonical destination. Other pages may show its current value and deep-link to the owner.
- Conversation-specific overrides and prompt-template selection belong in model settings and remain persisted with the conversation. Moving the picker does not turn it into a global model default.
- The inspector contains independent inline disclosures, not secondary pages. Toolbar actions reveal and scroll to a section; knowledge, Wiki and recall can coexist without replacing each other.
- The composer shows declared native reasoning efforts directly, without a parallel three-style control.
- Global defaults belong in the owning management page.
- Runtime progress belongs beside the result it affects. Historical execution details belong in Activity.
- Permissions are evaluated from concrete operations. UI labels or user-language keywords never authorize or route tools.
