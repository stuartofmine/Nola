> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Nola for macOS Product and Architecture Design

Source alignment: 2026-11-08, limited to the current Mac shell, composer, inspector, model configuration and runtime deployment. The current [conversation](CONVERSATION_UI_DESIGN.md), [inspector](CONVERSATION_INSPECTOR.md) and [client architecture](../软件架构设计/模块/13-客户端投影/CLIENT_INTERACTION_AND_CONFIGURATION.md) contracts govern these areas. Dated evidence later in this document retains its original scope and does not establish full product acceptance.

## 1. Product position

Nola for macOS is the desktop control surface for Nola, not a scaled iOS app and not a terminal wrapped in a window.

It has two equal responsibilities:

1. Provide every user-facing assistant capability available on iOS where macOS has an equivalent system API.
2. Provide desktop agent capabilities such as repository work, shell and process execution, files, Git, browser automation, computer use, skills, sessions, plans, and tool permissions through native application services and the shared Rust Core.

The CLI remains an independent terminal product. Capability parity does not mean that native Chat wraps, launches, or depends on the CLI executable.

The application opens directly into chat. Data, automation, memory, and settings support the conversation instead of competing with it as top-level destinations.

## 2. Experience principles

- **Conversation is the workspace.** Launch, deep links, restored windows, and new windows all land in a conversation.
- **Desktop-native density.** Information is compact and calm. Avoid iOS-sized cards, persistent tab bars, and oversized rounded controls.
- **Progressive disclosure.** The composer exposes only model, style, execution location, attachments, and send. Advanced controls live in the inspector.
- **Execution is legible.** Agent work appears as a structured timeline, not terminal noise mixed into the answer.
- **Power without clutter.** Keyboard navigation, command palette, drag and drop, multiple windows, and menus provide depth without crowding the primary UI.
- **Local and remote are explicit.** A run always shows whether tools execute on this Mac, on a connected host, or without host tools.
- **System materials are structural.** Use vibrancy and translucent materials for chrome and selection, not as decoration over every surface.

## 3. Main window

The workspace adapts between three columns and compact overlays. Actual width constraints and breakpoints belong to `MacChatWorkspaceView` and `MacDesignTokens`; do not maintain separate fixed window dimensions here.

### 3.1 Conversation sidebar

Sidebar sizing follows the responsive workspace and shared design tokens.

Content order:

1. Compact search field and new conversation button.
2. Pinned conversations.
3. Recent conversations grouped by relative date.
4. A quiet footer containing local/host connection state and the user menu.

Each row is one line for the title with a short secondary preview only when space permits. Selection uses a low-contrast tint and a slim leading accent. Hover reveals rename and more actions. Delete is never a permanently visible red control.

The sidebar can collapse with `Command-Control-S`. When collapsed, the chat remains visually centered rather than stretching text across the full window.

### 3.2 Conversation canvas

The center is the visual anchor. It consists of:

- A unified toolbar.
- A readable transcript column.
- A bottom composer attached to the window safe area.

Transcript readable width follows `MacDesignTokens.Layout.chatProseMaxWidth`. Assistant messages do not use large colored bubbles. They sit on the canvas with strong typography and restrained tool/result containers. User messages use a subtle elevated fill with a maximum width around 72 percent.

Streaming updates must be coalesced to display cadence rather than triggering a layout pass for every token. The scroll position follows output only while the user is already at the bottom. A single circular down-arrow returns to the latest message.

Every run owns one stable assistant message from the moment the user submits it. Pending state, reasoning, long-task phases, tool activity, incremental Markdown, completion, failure, and cancellation are projections inside that same message; they are not separate transcript rows or global status bars. While a tool is running, the collapsed projection shows its current action and sanitized target summary, and the expanded projection shows sanitized parameters and affected resources. Completion updates that same tool row with the real bounded result or error. A generic `completed` label must never replace a useful result.

### 3.3 Inspector

The right inspector is one scrollable surface with independent inline disclosures: overview, review, artifacts, progress, goal, context, execution and participants. There is no page picker or secondary navigation stack. Toolbar actions reveal a section and scroll to it; expanding one section does not replace another or reset its draft. Knowledge, Wiki and recall also expand in place.

Presentation state belongs to `MacInspectorNavigationState`; durable facts come from the current conversation and committed projections. Prompt-template selection has moved to conversation model settings. Provider connections, credentials and global defaults remain in the model center.

## 4. Toolbar and composer

### 4.1 Toolbar

Use the current single-bar workspace layout: navigation controls, browser-style content tabs and task/artifact/context tool groups. Do not add a second conversation title bar or a nested inspector page picker. Actual controls and compact overlays are composed by `MacChatWorkspaceView` and `MacConversationCanvas`.

### 4.2 Composer

The composer is one composed surface. Text, conditional attachments and compact model/execution controls retain a stable layout. Send, requested stop and confirmed terminal state are distinct; changing a control does not mutate an admitted Run.

The reasoning menu exposes the current model's declared native efforts directly, with the selected value and checkmark. It does not coexist with the old concise/balanced/detailed composer menu. Exact preference is saved in the per-model conversation override and reaches the local runtime without an implicit balanced tier. Connected-host capability declarations remain owned by that Host; do not manufacture local options for a remote profile.

Preset prompt selection lives in `MacConversationModelSettingsView`. It is staged until Save and discarded on Cancel, while the template library remains the owner of template content. Conversation selection does not become a global default merely because its control moved into model settings.

Reply copy controls have no permanent border or filled background. Hover/pressed and keyboard focus states use semantic theme colors in both light and dark mode; text controls retain insets and do not change size on hover.

## 5. Motion and interaction

Motion should communicate state changes and preserve spatial continuity.

- Sidebar and inspector use the system split-view transition, 180 to 240 ms.
- New messages enter with 6 pt vertical travel and a short opacity transition; existing transcript content does not shift unnecessarily.
- Tool activity expands in place and uses a restrained progress pulse. No continuous decorative animation.
- Streaming Markdown uses one renderer for the whole message lifetime. It preserves committed semantic blocks and updates only a mutable tail; completion drains the tail without replacing the renderer, the message, or already rendered nodes.
- Permission sheets originate from the tool activity that requested them.
- Reduce Motion removes travel, parallax, and pulsing while preserving opacity and state transitions.

## 6. Desktop capabilities

### 6.1 Window model

- Main chat window: multiple instances, each restoring its selected conversation.
- Conversation window: `Open in New Window` creates a dedicated chat window.
- Settings scene: global configuration only.
- Optional compact agent activity window for long-running tasks.

### 6.2 Menus and shortcuts

| Action | Shortcut |
| --- | --- |
| New conversation | `Command-N` |
| New window | `Command-Shift-N` |
| Focus composer | `Command-L` |
| Command palette | `Command-K` |
| Open file | `Command-O` |
| Open folder as workspace | `Command-Shift-O` |
| Search conversation | `Command-F` |
| Toggle sidebar | `Command-Control-S` |
| Toggle inspector | `Command-Option-I` |
| Stop current run | `Command-.` |
| Settings | `Command-,` |

The command palette searches conversations, tools, skills, agents, models, and application commands.

### 6.3 Files and Finder

- Drag files or folders onto the composer.
- Paste images and rich text.
- Use Quick Look from attachment and tool-result rows.
- Reveal generated or modified files in Finder.
- Store durable folder access with security-scoped bookmarks.
- Display extraction/indexing status before a file is sent to the model.

## 7. Code sharing boundary

Do not add conditional compilation throughout existing iOS views. Establish explicit shared and platform layers.

### 7.1 Shared code

The following belong in a shared application layer and should be members of both Apple targets:

- Conversation and message models.
- Persistence schema and repositories.
- LLM providers and model selection.
- Prompt/context construction.
- Attachment parsing and indexing engines that use cross-platform frameworks.
- Rust core FFI and agent event types.
- Memory, retrieval, localization, logging, and host connection protocols.
- View models that do not import UIKit, AppKit, HealthKit, BackgroundTasks, or platform permission frameworks.

### 7.2 iOS adapters

- `NolaApp` lifecycle and onboarding.
- `IOSAgentPermissionBroker`.
- `IOSAgentHostExecutor`.
- BackgroundTasks scheduling.
- HealthKit and iOS-only data sources.
- UIKit wrappers and mobile navigation.

### 7.3 Current macOS composition

```text
NolaMac/
  App/NolaMacApp.swift
  UI/MacChatWorkspaceView.swift
  UI/MacPageChrome.swift
  Features/Chat/MacConversationCanvas.swift
  Features/Chat/MacComposerView.swift
  Features/Chat/MacConversationModelSettingsView.swift
  Features/Inspector/MacChatInspectorView.swift
  Features/Inspector/MacInspectorKnowledgeView.swift
  Services/MacApplicationPort.swift
  Services/MacCanonicalConversationRepository.swift
  Services/MacLocalAgentRuntime.swift
  Services/MacRemoteHostService.swift
```

These are existing source owners, not a proposal for additional similarly named protocols or adapters. Application commands/queries, canonical storage, Run observation and window presentation remain separate; source sharing must not introduce a second reducer or a direct View-to-Store write path.

## 8. Native agent and CLI boundary

### 8.1 Current deployment boundary

Normal native Chat does not drive the interactive `nola` TUI. The current desktop deployment does use bundled service/Host processes: `MacApplicationPort` talks to Application, and `MacAgentHostRuntime` in `MacLocalAgentRuntime.swift` launches the owned agent Host executable. This is distinct from opening a terminal CLI session. The previous description of every local Agent as an entirely in-process Swift/FFI loop is not current.

Model inference may be local or an external provider independently of tool execution location. Connected-host execution uses `MacRemoteHostService` and source Host identities. Credentials are resolved by the appropriate Host; menu selection and process exit do not determine canonical completion.

### 8.2 Shared runtime shape

```text
nola-domain / nola-protocol       identities, invariants and typed contracts
nola-agent-engine                deterministic Agent reducer and state
nola-application                 admission, commands, queries and transactions
nola-store                       persistence through the canonical Store boundary
nola-runtime-driver / Host       committed work and physical execution
NolaMac/Services                 Application/Host transport and observation
NolaMac/Features + UI            drafts, projections and native presentation
cli                             independent terminal and Rust Web adapters
```

Mobile embedded Application and desktop service-backed Application reuse business boundaries, not necessarily the same process topology. Reusable deterministic decisions remain below all client renderers; no client owns another Agent loop.

### 8.3 Shared Agent Trace contract

macOS must not define a second application event vocabulary. Local Core, local-model, cloud-provider, and connected-host execution all normalize at the host boundary into the versioned shared `AgentTraceEvent` envelope defined by Core and documented in `Doc/历史任务/实施与审计/AGENT_TRACE_ARCHITECTURE.md`.

The envelope carries stable trace, conversation, run, message, span, event, and tool-call identities together with a monotonic sequence and timestamp. Its typed payload covers run lifecycle, phases, reasoning, assistant deltas, tool lifecycle, permissions, usage, validation, and terminal outcomes. Swift adapters may derive localized presentation models, but they must not invent parallel event names or infer state from display strings.

The UI applies events through one deterministic Trace reducer:

- A user submission immediately creates one assistant message and binds it to the run.
- Pending, reasoning, tool activity, parameters, resources, bounded results, incremental Markdown, and terminal state are projections within that message.
- Tool start and completion update one row keyed by `toolCallID`; completion preserves the real sanitized, truncated result or error.
- Duplicate and out-of-order events are idempotent. A sequence gap is buffered and recovered from the Journal before later events become visible.
- An accepted event that does not change the visible projection advances the reducer and Journal cursor without publishing a SwiftUI update.
- Streaming Markdown keeps stable committed blocks plus one mutable tail. It never changes renderer or replays the answer at completion.

The append-only Trace Journal is the recovery source for conversation reopen, application restart, host reconnect, and interrupted SSE or native streams. Replay must reconstruct the same projection without replaying entrance animation or duplicating tools and text. A provider or host event must never replace earlier messages. Process supervision belongs only to explicit external tools or the independent CLI Console, not to normal Chat startup.

### 8.4 Execution locations

The same conversation independently selects a model and an execution host:

- **This Mac:** native application services and the shared Rust Core execute available tools locally.
- **Connected Host:** the configured remote bridge executes remote tools after an explicit host selection.
- **Local model:** inference stays on this Mac and cannot be paired with a remote execution host.
- **Provider chat:** the configured provider answers directly; host tools are advertised only when the selected conversation mode requires them.

When no connected host is selected, Nola must not request a host skills endpoint. Remote skills and capabilities are queried only after a compatible connected-host profile is active. Failure of remote discovery must not affect native/local chat.

## 9. Permissions and entitlements

macOS permissions are capability-specific and should be shown as part of the requested action, not as an onboarding checklist.

| Capability | macOS mechanism | Product behavior |
| --- | --- | --- |
| Read/write selected files | App Sandbox user-selected files + security-scoped bookmarks | Ask for a file/folder when a durable scope is needed |
| Broad workspace access | User-selected folder bookmark | Display current workspace in toolbar/inspector |
| Shell/process | In-app permission policy before helper execution | Allow once, allow for run, allow matching command |
| Network | App Sandbox outgoing network | Conversation or global domain policy |
| Clipboard | `NSPasteboard` | Action-level disclosure for agent reads |
| Calendar/reminders/contacts | EventKit/Contacts TCC | Request only when a tool is invoked |
| Notifications | UserNotifications | Global preference after system grant |
| Microphone/camera | AVFoundation TCC | Request from explicit capture action |
| Screen capture | Screen Recording TCC | Required for screenshot/computer-use tools |
| UI automation | Accessibility TCC | Required for input and Accessibility API tools |
| Apple Events | Automation TCC | Request per controlled application where needed |
| Full disk access | System Settings, optional | Never required for basic operation; show only for workflows that need it |
| Health | No macOS parity assumption | Show iPhone-originated synced summaries or an unavailable state; do not report zero as live data |

Initial entitlements:

- App Sandbox.
- User-selected files read/write.
- Network client.
- Application group shared with iOS where signing supports it.
- Keychain access group for provider configuration.

The bundled helper must be code signed, embedded, version matched, and launched by the app. Whether it runs sandboxed or with user-approved broader access is a release architecture decision that must be validated before distribution; it must not silently inherit permissions the UI did not explain.

## 10. Data and iOS parity

Feature parity means a shared feature model with correct platform presentation:

- Conversations, providers, models, attachments, memory, retrieval, scheduling state, notifications, contacts, calendar, and reminders share application concepts.
- iPhone-only live health collection remains on iPhone. macOS can display records synchronized through the existing local/cloud data layer with source and freshness labels.
- The Data experience is accessible from the inspector and application menu, not a persistent tab that displaces chat.
- Unsupported sources display `Available on iPhone` rather than stale values, zeros, or misleading authorization states.

## 11. Settings information architecture

Settings is global and contains:

1. **General:** launch behavior, appearance, language, updates.
2. **Models:** providers, credentials, default model, local model storage.
3. **Agent:** default execution profile, permission defaults, CLI host status.
4. **Connections:** paired hosts and sync.
5. **Data:** sources, freshness, indexing, memory and retrieval defaults.
6. **Notifications and Automations:** schedules and delivery.
7. **Advanced:** logs, diagnostics, protocol versions, reset actions.

Conversation-specific model override, style, context limit, attachments, retrieval use, memory use, and execution profile belong only in the conversation inspector. Long-term memory and user preferences must have one global entry rather than duplicate switches in `More`.

## 12. Accessibility and localization

- Full keyboard operation and visible focus rings.
- VoiceOver labels describe tool state, permission scope, attachment extraction state, and execution location.
- Respect Dynamic Type where macOS supports it, Increase Contrast, Differentiate Without Color, and Reduce Motion.
- Use localization keys from the start. Do not construct translated sentences by concatenating fragments.
- Support English and Simplified Chinese in the first macOS milestone, then inherit the existing localization set.

## 13. Target and build design

Add a new macOS application target named `NolaMac` to `Nola.xcodeproj`.

- Product name: `Nola`.
- Bundle identifier: `com.gaojc.Nola.mac`.
- Deployment target: macOS 15.0.
- SwiftUI lifecycle with `WindowGroup`, `Settings`, and application commands.
- Separate `Info.plist` and entitlements.
- Share package dependencies only when used by macOS code.
- Build `nola_core` for macOS architectures and link the static library through a target-specific build phase.
- Build and bundle the current `nola-service` and `nola-agent-host` runtime artifacts through the project scripts; do not reintroduce the removed `NolaCLIHost` name.
- XCTest uses the `Test` configuration and `Products/Test`. Unsigned tests must never replace the signed daily `Products/Debug` app.

Do not make the iOS target a multiplatform target. Separate application targets keep platform lifecycle, entitlements, resources, and release behavior understandable while sharing source membership deliberately.

## 14. Implementation and remaining acceptance

The native shell, service-backed runtime, canonical conversation repository and inline inspector already have current implementations. They are not future milestones that authorize temporary in-memory storage or a CLI bridge. Follow [current implementation mapping](../软件架构设计/总体设计/CURRENT_IMPLEMENTATION_MAP.md) and [current work](../当前任务/README.md) for scope.

Compilation and isolated behavioral tests establish their own paths only. Full-window accessibility, interrupted workflows, real providers, connected Hosts and platform grants require separate evidence. Historical milestone checklists are not a second architecture plan.

## 15. Required tests

- Shared application contracts: conversation, context, attachments, persistence, host events.
- Shared `AgentTraceEvent` canonical fixture parity across Core, macOS, iOS, Android, Web, and CLI adapters.
- Reducer tests for duplicate events, semantic no-op events, out-of-order delivery, sequence gaps, terminal events, and run/conversation isolation.
- Journal recovery tests for reopen, restart, reconnect, cursor replay, and a page or window closing during a live run.
- Stable assistant-message tests proving pending, reasoning, tool parameters/resources/results, Markdown, and completion remain in one bubble with one message identity.
- Streaming Markdown tests proving committed block identities survive later deltas and the mutable tail drains without renderer replacement.
- CLI host protocol framing, version mismatch, cancellation, crash recovery, and permission round trips.
- macOS permission outcome tests with unsupported and denied states.
- UI tests for launch-to-chat, sidebar, inspector, composer, drag/drop, shortcuts, and multiple windows.
- Snapshot coverage for window widths, appearances, localization, accessibility sizes, and Reduce Motion.
- A semantic no-op gate proving an accepted Trace event advances sequence/cursor but causes zero presentation publication and zero rendered-node mutation.
- A sequence-gap gate proving later events remain buffered until missing events are replayed from the Journal.
- Canonical long-task fixture E2E proving sanitized running parameters and resources are visible on expansion and the completed row retains the real truncated result.
- End-to-end capability matrix comparing CLI and `This Mac` execution.

## 16. Architecture decisions

The following are fixed for implementation unless a concrete build constraint invalidates them:

1. macOS is a new application target in the existing Xcode project.
2. Chat is the root experience; there is no macOS tab bar.
3. iOS views are not bulk-shared. Platform-neutral services and view models are shared deliberately.
4. Reusable Agent capabilities remain implemented in the shared Rust Core; native Chat does not launch or depend on a CLI subprocess.
5. Local, connected-host, and chat-only execution profiles are explicit and capability-driven.
6. Conversation settings and global settings have separate, non-duplicated homes.
7. Health data on macOS is synchronized data with provenance, not a direct iOS HealthKit assumption.

## 17. Dated implementation acceptance matrix

The entries below preserve earlier evidence and outstanding acceptance at the time they were written; they are not a 2026-11-08 rerun. Current source owners are listed above. This matrix records evidence separately from implementation. A compiled feature is not treated as runtime-accepted when it depends on external credentials, downloaded models, another device, or macOS privacy grants.

| Area | macOS implementation | Automated evidence | Remaining live acceptance |
| --- | --- | --- | --- |
| Chat-first launch | Main `WindowGroup` opens `MacChatWorkspaceView`; no desktop tab bar | Current `scripts/test_nola_mac.sh real` delegates to the production Mac runtime acceptance; current architecture changes await unified verification | Exercise window restoration across logout/restart |
| Conversation UI | Three-region workspace, searchable sidebar, transcript, compact composer, inspector, down-arrow, streaming cadence | Debug light/dark snapshot harness renders sidebar and composer; clean build passes | Full-window visual capture needs Screen Recording permission because fallback capture cannot render split/scroll descendants |
| Conversation persistence | Archive repository, rename, delete, search, multiple conversation windows | Build and startup smoke pass | Manual round trip with real messages and multiple windows |
| Cloud chat | OpenAI-compatible configuration, streaming, thinking, model parameters, prompt/context composition | Compiles in clean target build | Requires a configured provider and a live request |
| Local model chat | MLX model download/selection and native streaming generation | MLX packages and shared `MLXService` compile in clean target build | Requires a downloaded compatible model and generation run |
| Attachments | File picker, drag/drop, extraction, chips, local paths for CLI execution | Application runtime harness extracts and verifies real TXT, DOCX and PDF fixtures through `MacAttachmentPipeline` | Image UI, large-file and security-scoped bookmark fixtures |
| This Mac native agent | Shared Rust Core ABI plus native macOS runtime; streaming, sessions, cancellation, permissions and activity timeline do not require a CLI process | Contract tests cover host events, message identity/order, context accounting and local runtime behavior | Live tool runs for web, browser, skills and plans |
| CLI Console | Explicit PTY-backed access to a bundled or discovered CLI, isolated from native Chat | CLI executable discovery/help checks are separate from native Chat launch tests | Interactive prompt and long-running process exercise |
| Connected Host | Explicit host profile, token configuration, remote event stream and permission challenge UI | Clean build passes | Requires a compatible live host for protocol and cancellation tests |
| Models | Model Center, local model manager and direct numeric model parameter editor | Clean build passes | Provider discovery/download errors and credential validation |
| Data | Contacts, calendars, reminders and notification authorization/freshness; selectable chat context | Clean build passes | TCC allow/deny/not-determined matrix on a signed app |
| iPhone Health | iCloud KVS aggregate sync with source device, activity date and freshness; never presented as live zero data | Clean build passes | Requires signed iOS/macOS apps on the same iCloud account |
| Memory and retrieval | One global memory/preferences location, per-conversation use controls, local prompt and web source stores | Clean build passes | Retrieval quality and persistence round trip with real sources |
| Notifications | Persistent notification history, unread badge, scheduled/delivered/history states and deep links | Clean build passes | UserNotification authorization, delivery and deep-link click |
| Automations | Persistent schedules, duplicate-run prevention, launchd registration, run state and history | Clean build passes | Scheduled launchd execution and failure recovery |
| Workspace/Git | Bookmark-backed workspace selection, status/diff review and Finder actions | Clean build passes | Real repository with staged, unstaged and untracked changes |
| Backup/reset | Versioned export/import for conversations, activity, automations, prompts, sources, notifications and settings | Isolated runtime harness proves conversation messages and numeric settings survive export, removal and import | Destructive clear plus full multi-store restore on a disposable signed test account |
| Desktop integration | Menus, shortcuts, Settings scene, login item, diagnostics/support bundle | GUI startup smoke passes | Signed distribution build, login item restart and support bundle inspection |

### 17.1 Current acceptance entry points (not yet rerun after architecture changes)

```bash
bash scripts/test_nola_mac.sh contracts
bash scripts/test_nola_mac.sh host-e2e
bash scripts/test_nola_mac.sh real
```

The former standalone mock-provider/CLI smoke script has been removed. `contracts` and `host-e2e` use the shared `test_mac_layers.sh` suites, including the production-Host Skill fixtures; `real` (also the default) uses `test_real_mac_agent_host.sh`, an explicitly configured real provider, isolated runtime data, and structured canonical evidence. These commands no longer claim CLI help/file/shell/Git coverage. Unknown modes fail instead of silently retaining obsolete options. All unsigned acceptance builds use `Products/Test`, never the developer's signed Debug app. Evidence is retained after failure.

For the automation chain, set `NOLA_MAC_RUNTIME_ACCEPTANCE_WEB_SOURCE_URL` to a controlled fixture source and `NOLA_MAC_RUNTIME_ACCEPTANCE_AUTOMATION_SOURCE_TOKEN` to its expected content, then run `bash scripts/test_nola_mac.sh automation`. Configure the provider using `NOLA_CONFIG_PATH` and the explicit execution declaration required by `scripts/e2e/model_execution_fixture.py`; a missing declaration fails rather than inventing capabilities. Build the canonical audit-reader example as documented by the real runner before this final verification phase.

The Mac harness uses the same `MacAgentHostRuntime` and occurrence admission as production. Each automation scenario registers a real model profile in an isolated database with only an in-memory credential reader, never a production Keychain edit or an arbitrary profile ID. It does not apply orchestration mutations itself or cancel a Run upon tool completion. Success requires canonical Run results, the exact occurrence receipt and terminal seal, canonical Skill evidence, and independent JSON artifact readback. `verify_mac_automation_acceptance.py` compares that report with persisted binding/receipt/seal in a read-only transaction; it does not implement another executor or replace the Application seal validator. Exit code zero and log marker strings are not business results. Current script and fixture changes are source-only pending final unified testing.

### 17.2 Embedded executable naming

The app executable is `Nola`. If the independent CLI Console bundles a CLI, its executable is named `nola-cli`, not `nola`: default macOS volumes are case-insensitive, so `Nola` and `nola` would resolve to the same bundle path. External installations at `/opt/homebrew/bin/nola` and `/usr/local/bin/nola` remain optional CLI Console fallbacks; none is used by normal native Chat.
