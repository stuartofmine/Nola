> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Changelog

## 2026-09-19 — Run-owned recovery and cancellation boundary

- Raised the canonical Store schema epoch to 4. Development clients select a fresh `nola-v4.db`; no prior database is migrated or rewritten.
- A Turn remains nonterminal in `cancelling` while its idempotent Host cancellation effects are outstanding. `TurnCancelled`, assistant terminal projection, and Run closure now commit only after all cancellation acknowledgements.
- Foreground CLI and Agent Host requests no longer perform an all-conversation orphan sweep. Dead-owner recovery is scoped to the owning session so one invalid Run cannot block unrelated conversations.
- macOS validates every Host event against the Run and assistant message established by the request, and no longer kills the Host process in response to a presentation terminal event.
- Recovery of a predecessor Run is isolated from the successor Run transport. Canonical projection remains the path for updating the predecessor message.

## 2026-09-13 — Typed text-tool protocol recovery and visible model fallback

- macOS Agent Host session snapshots now cross a bounded Foundation JSON boundary instead of recursively re-decoding the full dynamic payload with `JSONDecoder`. Excessively nested protocol data is surfaced as a visible projection failure rather than overflowing a cooperative task stack and terminating the app.
- Raised the canonical Store schema epoch to 3 and moved macOS development state to `State/nola-v3.db`. The app does not migrate, rewrite, or read the incompatible V2 model-attempt ledger; the previous database remains untouched on disk.
- Replaced scattered DSML/tool-envelope string checks with a registered text-tool protocol dialect parser in the provider stream adapter. Transport fragments are classified as ordinary text, a valid typed tool call, or an invalid control frame without enumerating malformed spellings.
- Invalid or incomplete control frames now fail closed as `protocol_error`; their raw bytes never enter assistant text, canonical history, or client-side protocol detection.
- Added structured Host recovery progress for rejected model tool output. macOS presents the failure and native-protocol retry explicitly and preserves it across canonical projection refreshes.
- Preserved Host retry summaries so macOS can show the failed model, failure reason, and selected fallback instead of silently collapsing a recovered attempt.
- Kept model purpose as routing metadata rather than tool authorization; concrete catalogs, grants, and permission profiles remain authoritative.
- Added regression coverage for valid, fragmented, malformed, and misspelled protocol frames, ordinary XML-like text, natural multilingual follow-ups, and Mac recovery projection.
- Made the canonical Attempt ledger retain a bounded typed failure before retry/fallback, and folded failed attempts plus successor identity directly into conversation projections on macOS, iOS, Android, and Web.
- Kept raw provider response bodies in diagnostics while projecting only sanitized error categories and messages; refreshed clients no longer lose model failure history or duplicate transient retry rows.
- Added turn-scoped provider health so a quota-exhausted endpoint is omitted from later model invocations in the same turn.
- Changed Code Memory provider reuse to cache only successful initialization, allowing recovery from an early state-directory or migration failure.
- Unified `tool_search` schema discovery over the frozen Host capability catalog, so native Code Memory tools are discoverable by the same typed protocol as built-in, device, and MCP capabilities.
- Kept typed capability activations and successfully executed capabilities in subsequent model-step projections, including completion-reserve rounds; a Code Memory operation can no longer disappear during the same evidence chain.
- Made CLI model retries durable visible status rows with sanitized failure text and explicit `failed provider/model → fallback provider/model` identity; endpoint URLs, API keys, and raw provider diagnostics remain hidden. Core Trace now preserves typed Host progress metadata so native clients no longer parse retry control state from presentation strings.
- Deferred assistant copy actions until terminal output on macOS, iOS, Android, and Web; streaming output no longer exposes an action surface that can copy a partial response.
- Clarified compaction prompts that the summarized input is only an archived prefix, preventing unsupported “no active user task” conclusions when the current request lives in the retained tail.
- Refined process UI using the state hierarchy demonstrated by [Libraries.dev](https://github.com/Jakubantalik/Libraries.dev): motion only for active work, static high-contrast failure cards, theme-native colors, and reduced-motion support.
- Reworked the macOS conversation surface around a single readable column: wide-screen prose and user bubbles are bounded, successful process history recedes after completion while failures remain expanded, hover no longer repaints entire messages, completed copy controls remain reachable at low emphasis and strengthen on hover, and the composer uses one quieter opaque surface.
- Unified macOS disclosure motion around one reduced-motion-aware 220 ms curve and a restrained top-anchored transition across chat traces, task cards, permission guidance, inspector recall, data-source details, and Model Center tabs. Removed nested animation suppression that made expanded content flash into place.
- Reworked macOS streaming Markdown around Streamdown's stable-block and bounded-backlog model: unfinished prose is visible immediately, a restrained 420 ms reveal head flows through each new phrase glyph-by-glyph, fast streams compress within a 320 ms visual horizon, and layout-sensitive code blocks continue updating without animation churn. Terminal canonical text is promoted to document-mode Markdown so syntax spanning streamed phrase boundaries is rendered instead of left literal.
- Extended the same motion language to iOS, Android, and Web: disclosures use one reduced-motion-aware 220 ms top-anchored transition, streaming Markdown keeps stable content visible with a restrained 180 ms reveal, and the Web phrase scheduler compresses pending work into a 320 ms visual horizon instead of draining a long animation queue.
- Moved the macOS reasoning timeline into the assistant response bubble and made reasoning/tool mutations participate in follow-to-latest, so live process output grows in one scrollable surface instead of above a separately anchored answer.
- Grouped repeated macOS tool operations into one active summary while retaining every canonical audit entry behind disclosure. Tool parameters and results now stay folded by default in running, waiting, failed, and completed states.
- Reduced the first model request to protocol entry points plus the real Code Memory capabilities (`repo_map`, `code_search`, `code_navigation`, and `failure_context`). Raw file/edit/process tools remain available through typed discovery instead of consuming the initial context window.
- Released the first safe semantic stream unit at a smaller boundary while preserving larger durable batches afterward, reducing time from provider text arrival to the first rendered phrase without weakening malformed-tool-frame quarantine.
- Raised the default remote-model context budget from 16K to 128K and aligned recovered development profiles with it. This avoids model-powered pre-compaction caused by fixed instructions and tool schemas before a short conversation can answer.
- Deferred AppKit composer bindings and transcript geometry state to the next main-actor turn, preventing SwiftUI state publication from inside active view updates.
- Replaced blocking model-management error alerts with a four-level inline notice language. Informational and success feedback recedes after four seconds; warnings and failures remain visible, while destructive endpoint deletion still requires an explicit confirmation.
- Extended the same non-blocking feedback language across macOS Extensions, Chrome integration, Data Management, Prompt Library, Web Sources, and Automations. Routine operation failures now stay in page context; only destructive actions and unsaved-change decisions interrupt the flow.
- Replaced the macOS conversation-sidebar rename alert with focused inline editing, including Return-to-save, Escape-to-cancel, explicit controls, and focus-loss commit behavior.
- Standardized destructive macOS confirmations on lightweight confirmation menus for model deletion, notification removal, and history/data clearing, keeping their consequences visible without a full blocking alert.
- Removed the decorative diagonal gradient shared by macOS management pages. A stable system canvas now lets content, status, and empty-state hierarchy provide the visual structure without background noise.
- Tightened the macOS API-model form: manual model-ID edits now synchronize the catalog picker, catalog changes update auto-filled display names, disabled submission explains the missing requirement, and the displayed output-token bound matches the 32K runtime limit.
- Surfaced previously hidden Notification Center and Activity Center store failures as persistent inline notices with an immediate retry action.
- Added visible missing-field guidance and first-field focus to macOS prompt, web-source, and automation editors; hidden personal-data age input no longer blocks automation saving when that data source is disabled.
- Replaced silent model-parameter clamping with explicit range validation. The editor now shows the actual context/output bounds, blocks an invalid save with a persistent explanation, and keeps save failures in document flow instead of overlaying the form.
- Removed the final operational macOS alerts: settings failures now stay inside Settings, and compaction-source lookup failures remain visibly attached to the transcript instead of interrupting the conversation.
- Moved API-model sensitive-data authorization into its own orange-accented form section so it remains visually distinct from ordinary routing uses even while disabled.
- Added a responsive single-pane Setup/Output mode to macOS Model Test below 820 points, automatically switches to Output on run, removes its decorative gradient, and presents failures with retryable inline feedback.
- Made the macOS Settings sidebar vertically scrollable while preserving bottom alignment when space permits; changing categories now keeps the selected row visible in short windows.
- Simplified the compact macOS chat header to sidebar, active model, conversation title, and one overflow menu; execution unit, host, workspace, search, and inspector remain available without competing for the narrow title row.
- Converted macOS Notifications and Prompt Library from compressed split panes to animated list-to-detail navigation below 760 points. Activity, notifications, and prompts now share an inset, low-chroma selected-row treatment instead of AppKit's full-width inactive gray highlight; the flat Activity feed uses a lazy scroll stack rather than an unnecessary table bridge, eliminating its reentrant `NSTableView` update warning.
- Turned “Add API Model” into a complete setup sheet with provider discovery/manual identity, routing uses, visually distinct sensitive-data permission, precise/balanced/creative presets, and expandable context/output/sampling controls. Newly selected parameters are committed with the endpoint instead of requiring a save-and-reopen detour.
- Made enabled API-model purposes visually explicit under graphite system accents, and applied the same real parameter-range validation used by the model editor to the add flow. Invalid advanced values now remain visible with a concrete explanation and block creation instead of being silently clamped.
- Made automatic context compression visible end to end: a semantic `phase=compacting` trace appears in the live process area even when Core owns durable content, then yields to a persistent checkpoint card with the summarized item count, digest, summary, and original-context review action.
- Added a rendered macOS compaction lifecycle regression covering the in-bubble running state, retained failure explanation, and durable checkpoint card together, so future styling changes cannot silently collapse the user-visible transition into a hidden backend event.
- Removed duplicate terminal-error cards when a failed or cancelled canonical Trace already presents the same failure inside the response bubble. A separate fallback card remains for transport failures that have no visible Trace, preserving explicit failure reporting without implying two failures.
- Status-only response timelines now use a stable “Status” heading rather than the generic “Activity” label, while preserving the concrete compaction or recovery operation in the row below.
- Made macOS notification authorization observable instead of self-reverting silently: Settings now distinguishes the app preference from the live macOS permission, keeps denial feedback visible, disables unavailable event controls, and links directly to System Settings. Permission copy is localized across every supported Mac language.
- Split “Add API Model” into two compact, animated steps so request behavior is no longer buried below identity and sensitive-data controls. Presets now expose their effective context, output, temperature, and Top P values at a glance; advanced fields expand with the shared reduced-motion-aware disclosure transition.

## 2026-09-12 — Broad native Code Memory language coverage

- Expanded the in-process Code Memory parser registry from seven extensions to 66 statically linked language and engineering-format grammars without runtime downloads or companion processes.
- Applied a 0.1% GitHub developer-activity floor to programming languages while retaining common repository formats independently, reducing the release Host from 215 MiB to 119 MiB.
- Added explicit declaration extraction for 18 mainstream languages while exposing parser, semantic, and support-policy metadata in the graph schema.
- Initialized and reused parsers only for languages present in a project, and moved file reads, hashing, and parsing to a bounded worker pool while preserving atomic SQLite publication.
- Preserved the validated Kotlin and existing deep-language parsers, and added offline grammar initialization plus cross-language symbol extraction tests.

## 2026-08-28 — Native managed-browser and unified browser tools

- Added a pinned Playwright runtime to the macOS app, including a standalone Node executable and a long-lived, session-scoped Chrome bridge.
- Replaced model-visible vendor-specific browser paths with unified `browser_*` tools and explicit `target=managed|attached` routing.
- Added ARIA snapshots, revision-bound element refs, screenshot artifacts, stale-ref rejection, browser diagnostics, and product-bundle E2E coverage.
- Added explicit `vision_input` model capability. Screenshot bytes enter provider context only for models configured to accept images; text-only models continue to receive semantic page evidence.
- Preserved the Chrome extension path for current tabs and signed-in Chrome sessions while making isolated Playwright automation a first-class native capability.

## 2026-08-24 — Canonical Web conversation-title ownership

- Replaced implicit title updates during message persistence with a typed ownership state: `automatic`, `first_message`, or `user`.
- The Web Host now commits the first visible user message as canonical metadata before Core execution; attachment-expanded model context cannot leak into titles.
- Explicit rename and branch titles are user-owned and cannot be overwritten by later messages. The browser refreshes canonical metadata only after durable message/Trace reconciliation.
- Added Rust contract coverage and a real-model browser E2E proving persisted titles, multi-turn recovery, rendered Markdown, tool traces, screenshots, and clean host logs.

## 2026-08-24 — Typed tool-result presentation ownership

- Split Core tool presentation into a Host-owned human summary, bounded technical preview, structured output, full-output reference, and typed artifacts.
- Removed serialized tool-result envelopes from the Web message surface. Tool progress remains visible while arguments and technical output live in an expandable detail layer.
- Updated realtime and replay rendering together, and added reducer plus real-model browser assertions with screenshot and log review.

## 2026-08-21 — Remove dead macOS preference state

- Removed the non-functional “Default execution” setting; execution location remains authoritative per conversation.
- Deleted eight obsolete model, connected-host, local-model, and data-context preference keys from backup/reset. Removed state is no longer exported, restored, or treated as product configuration.

## 2026-08-21 — Canonical macOS connected-host identity

- Removed the macOS connected-host URL, expected Host ID, and client ID UserDefaults writers.
- URL, Host ID, and stable client identity now commit atomically in one versioned canonical ProductSetting; the access token remains in Keychain.
- Host configuration and client-ID writes now fail explicitly, and settings/data-reset flows surface failures.
- Added a real SQLite cold-reopen and clear E2E proving obsolete preference keys are ignored and cannot revive a removed Host connection.

## 2026-08-21 — Reliable canonical macOS local-model writes

- Local-model selection and parameter changes now commit canonical `ModelProfile` state before changing conversation or UI selection.
- Removed silent `try?` writes. Store, encoding, and unknown-model failures are surfaced in Model Center, the parameter editor, or the chat activity stream.
- Dynamic installed models now use the same canonical selection path as bundled models.
- Added a production-service SQLite cold-reopen E2E covering parameters, selection, identity, provider, and runtime.

## 2026-08-21 — Strict macOS model identity

- Removed macOS local-model resolution by display name, repository basename, whitespace normalization, and the old `local:` alias.
- Conversation model selection now accepts only the current typed `configured:<model-id>` or `local:<model-id>` envelope; raw and unknown-prefixed values fail closed.
- Added regression coverage for exact canonical IDs and rejection of every removed alias.

## 2026-08-21 — Canonical macOS remote model configuration

- Removed the macOS remote endpoint and model-name UserDefaults store. Remote API models now persist only as canonical `ModelProfile` records; API secrets remain in Keychain.
- Configuration writes now fail explicitly when the canonical store or encoding fails, and settings/model UI surfaces the error instead of reporting a false success.
- Added real SQLite cold-reopen and clear coverage plus a poisoned-obsolete-key regression proving removed preference values cannot revive configuration.

## 2026-08-21 — Reject obsolete persisted state

- Removed prompt-template UserDefaults keys, macOS split backup fields, CLI protocol defaults,
  and pre-release stream/context fallbacks; only current canonical contracts are accepted.
- Personal intelligence state now requires the current schema version. Existing malformed or
  unsupported state returns an error instead of silently becoming an empty memory store.
- URL history appends now distinguish a missing file from invalid persisted content and refuse to
  overwrite malformed or obsolete history shapes.

## 2026-08-21 — Delete iOS model and URL history shadows

- Deleted RustInterop model catalog, last-selection, provider, generic persistence, and URL-history UserDefaults compatibility APIs.
- iOS model catalog and selection now read and write canonical `ModelProfile` records only; Web Host tokens and trusted identities remain in Keychain.
- Moved URL fetch history to canonical ProductSettings with real deduplication and cold-reopen E2E coverage.
- Fixed the iOS Store adapter to accept successful Core commands whose acknowledgement result is JSON `null`; previous `try?` callers could silently hide rejected writes.

## 2026-08-21 — Canonical model identity

- Model generation parameters now live directly on the canonical `ModelProfile` identified by its stable model ID on iOS, macOS, and Android.
- Removed display-name-derived `api:` and `model-parameters:` compatibility profiles. Display names are presentation-only and no longer participate in persistence identity.
- Parameter reset preserves model entities instead of deleting catalog entries.
- Verified canonical cold reopen through the real Core ABI on iOS and macOS; Android main and instrumentation sources compile against the same contract.

## 2026-08-21 — Canonical mobile assistant state

- Deleted the unreleased iOS UserDefaults memory-profile writer and Android `memory_profile`
  DataStore. No migration or fallback path remains.
- Made canonical Memory, Skill, MCPServer and ProductSetting records the only durable mobile
  assistant state; model API secrets remain in Keychain/Keystore.
- Added cold-reopen coverage for iOS memory and uploaded context plus Android assistant settings,
  model configuration and resource identity.
- Deleted the unused Xcode-template CoreData `Item` store and its app environment injection; iOS
  now starts only product-owned persistence.
- Removed Android's embedded English response-style prompt fallback. The packaged shared prompt
  asset is required and a missing/empty asset fails explicitly instead of silently changing policy.
- Moved iOS/macOS/Android per-model generation parameters into canonical ModelProfile records.
  Deleted the iOS UserDefaults and Android DataStore parameter writers without migration paths,
  and removed Swift's missing-field compatibility decoder.
- Added real iOS/macOS SQLite cold-reopen tests for the client parameter repositories and compiled
  the equivalent Android device E2E through the production JNI Store adapter.

## 2026-08-21 — Removed the unreleased in-memory Agent runtime loop

- Deleted `CoreHostLoop`, direct dispatch, staged submission, deferred execution, and their in-memory pseudo-E2E tests.
- CLI/Web now reduce every new turn, active-run command, memory lifecycle update, and manual compaction through Application transactions before executing committed Outbox effects.
- Runtime retains no reducer or uncommitted queue; its process-local state is a read-only projection rebuilt from the canonical checkpoint.
- Removed the CLI presenter's provider-shaped shadow transcript and its legacy checkpoint/message reconstruction helpers. CLI rendering now consumes canonical Core Part events without maintaining a second message history.
- Removed `SessionStore`'s mutable `SavedSession` transcript cache. Resume, compaction preparation, external-turn titling, and append conflict checks now reread the canonical Application Service projection; appends no longer update a process-local message copy.
- Added real SQLite rollback/reopen coverage for raw Host responses and active submissions, plus committed-only Runtime streaming coverage.

## 2026-08-21 — Removed public Store transaction primitives

- Removed public Application commands for arbitrary Run creation, raw transition commits, pre-reduced Host-event injection, checkpoint writes/deletes, pending-effect reads, and direct effect acknowledgement.
- Removed the CLI helper and fake footer test that wrote hand-authored checkpoint JSON; context recovery must now originate from a real Core/Application transaction.
- Added protocol rejection coverage so removed command names fail closed instead of decoding through a fallback.

## 2026-08-21 — Removed macOS conversation metadata side storage

- Deleted the unreleased `MacConversationMetadataStore` UserDefaults writer and its isolated tests. Conversation title and pinned state now have one durable owner: canonical Conversation fields/settings through Application Service.
- Removed backup/reset cleanup for the deleted side store and kept inline rename/sidebar synchronization on the canonical repository path.
- Made the Mac AppendMessages result decode the current snake_case service contract explicitly and updated the Core Part E2E fixture to send the required typed envelope instead of relying on an obsolete untyped shape.

## 2026-08-21 — Canonical macOS assistant defaults

- Moved user guidance, retrieval default, memory default, and context-window default into versioned canonical ProductSettings. The settings UI now writes through Application Service instead of `@AppStorage`.
- Removed the three corresponding UserDefaults keys from backup/export and made ProductSettings the only backup representation.
- Deleted the removed `begin_turn` and `commit_transition` Swift commands, DTOs, and obsolete ABI test rather than restoring protocol aliases. Remote execution retains only the current typed remote transaction contract.
- Made SessionProjection decoding strict: `source` and `permission_rule_set` are required current-protocol fields, and the unused schema-version facade was removed.
- Added real SQLite reopen coverage proving all assistant defaults survive through the production Mac service boundary.

## 2026-08-21 — Removed unreleased data-source compatibility

- Removed the legacy reminder-shaped `URLDataSource` format, reminder URI inference, fetch path, and `UserTask` mirroring.
- One-off URL-backed automations now persist only the canonical `scheduledDate` field; old field names are not decoded or migrated.
- Removed stale FastAPI compatibility claims from the repository overview. Cloudflare Worker + D1 is the only backend implementation.

## 2026-08-21 — Removed the legacy native-app side channel

- Removed the Python/AppleScript native-app daemon and `/api/native-app/*` queue. Native application and computer-use execution now has one authority boundary: the versioned Agent Host protocol and signed client callback.

## 2026-08-21 — Canonical conversation projection transactions

- Removed the remaining Web-only message-hiding write path. Conversation revert now enters Core as a typed submission and commits its checkpoint plus message-visibility projection through Application in one immediate SQLite transaction.
- Kept reverted messages in the immutable Message/Part ledger while exposing only the current projection to product views. Stable operation identity makes replay idempotent; invalid revisions or foreign message IDs roll back both checkpoint and projection.
- Added real SQLite reopen, replay, and rollback coverage. The full Rust workspace and the production token-session Web Host-to-Core-to-real-LLM path pass with stable `user, assistant, user, assistant` persistence order.

## 2026-08-15 — Cross-platform execution and channel acceptance

- Verified installed Codex, Claude Code, and OpenCode through the production Agent Host boundary on Mac, iOS, Android, CLI, and Web. Real two-turn tests preserve each executor's native session identifier instead of replaying a synthesized Nola transcript.
- Verified Telegram Bot, WhatsApp Cloud API, WeChat Official Account, and WeCom webhook-to-Worker-to-Host-to-real-LLM delivery. The acceptance run completed five real Codex turns, including Telegram conversation resume and streaming draft delivery.
- Verified the production iOS execution-unit picker beside the conversation title, incremental Markdown/tool rendering, restart persistence, and Host messaging-management card; Android device tests cover the execution picker and all four messaging providers.
- Added startup recovery for dead session-run locks and removed orphaned per-user background scheduler jobs left by deleted Xcode DerivedData bundles. Development builds do not register persistent jobs; installed Mac apps register the bundled `nola-agent-host` scheduler.
- Unified Mac diagnostics around `Application Support/Nola/State/nola.db` and `Application Support/Nola/Logs/nola.log`, preventing test paths and stale registrations from being mistaken for production state.

## 2026-08-13 — External execution units and messaging channels

- Added per-conversation Nola, Codex, OpenCode, and Claude Code execution backends across Mac, iOS, Android, CLI, and Web, including installed-runtime discovery and native session resume.
- Added Telegram, WhatsApp Cloud API, WeChat Official Account, and WeCom channel ingestion and delivery through signed Worker webhooks and a leased Nola Host inbox.
- Added real Codex, Claude Code, and OpenCode two-turn Web/Host E2E coverage, including persisted external-session resume, plus real channel-to-Agent-Host LLM coverage; unavailable runtimes are still reported explicitly rather than simulated.
- Prevented terminal authentication and authorization failures from consuming scheduled-automation retries, preserved valid context usage on failed runs, and isolated test diagnostics from the user's production log.

## 2026-08-12

- macOS production pages now use tighter responsive reading widths and card density: Settings is centered, Model Center actions no longer drift to the card bottom, the Skill/MCP gallery avoids tiny four-column cards, and unauthorized data sources show one authorization action instead of repeated unknown contract fields.
- macOS transcript scrolling now observes the AppKit clip view instead of mutating SwiftUI scroll geometry multiple times per frame.
- The built-in PowerPoint workflow now preserves and validates native hyperlinks, while temporary specification and validation files are not surfaced as conversation attachments.
- Local MLX model downloads validate required artifacts before activation and preserve the original download error when cleanup also fails. Qwen3.5 0.8B download/cache/start/inference was exercised through the real macOS model path.
- Core now emits `InputApplied` for every newly started Turn, including queued follow-ups. Hosts persist one authoritative event stream instead of separately pre-writing the first user message.
- Typed `full_output_ref` activation now exposes bounded output-navigation tools on the next model step; untyped text markers still cannot activate capabilities.
- Windows remains a thin Electron shell over `nola web`. Its TypeScript build now uses Node16 module resolution, strict child-process types, and a single trusted IPC sender check; the Rust Web Host reports Unix-only shell-script capability explicitly on Windows.

## 2026-08-21 — Android native GGUF runtime completed

- Added a pinned official `llama.cpp` Android binding with statically linked CPU backend for arm64 and x86_64 builds.
- Separated custom API configuration (`Cloud`) from real on-device GGUF models (`Local`) and removed the unreleased compatibility aliases and implicit background fallback.
- Added app-private atomic downloads, SHA-256 verification, installed-model manifests, explicit start/stop/delete controls, runtime state, progress, RAM estimates, and 64K context metadata.
- Passed temperature, top-p, top-k, context size, and maximum output into the native runtime; fixed the native output stop position and context-shift capacity calculations.
- Added localized model lifecycle UI in English, Simplified Chinese, German, Spanish, French, Japanese, and Russian.
- Added real Android 35 arm64 E2E using the 428,970,080-byte Qwen3 0.6B Q4_0 GGUF with its publisher LFS SHA-256. Both direct JNI inference and the production `LocalLlamaProvider` lifecycle generated non-empty text.
- Updated the canonical conversation persistence instrumentation test to use the current canonical store instead of removed Room conversation/message compatibility DAOs.

## 2026-08-12 — Local-model runtime and responsive macOS pages hardened

- Tightened Agent Host artifact projection: ordinary file mutations remain visible in the tool timeline but no longer promote input paths such as Skill specs or helper scripts into chat attachments; only explicit, existing output artifacts are presented as deliverables.
- Removed the Windows desktop chat surface's fixed 440px center-column floor so the shared responsive sidebar and inspector breakpoints can collapse without horizontal overflow.
- Fixed Web stream recovery so journal replay resumes from the latest Core Part prefix, deduplicates the replay boundary, appends the missing tail, and cannot be rolled back by a late stale Part snapshot.
- Fixed duplicate post-tool final synthesis: a new answer now completes the Turn immediately, while exact tool-round replay or embedded tool protocol still triggers a clean retry.
- Regenerated the Core canonical Part lifecycle fixture and updated engineering-loop contracts for one-pass final answers without weakening validation or recovery obligations.
- Unified CLI/Web user-home resolution across `NOLA_HOME`, `HOME`, Windows `USERPROFILE`, and `HOMEDRIVE + HOMEPATH`.
- Removed Android's unimplemented placeholder on-device provider and unavailable onboarding choice; custom OpenAI-compatible endpoints remain distinct from the planned `llama.cpp` GGUF runtime.
- Corrected Qwen3.5 0.8B/4B text chat on macOS and iOS to use `LLMModelFactory` instead of the multimodal VLM path.
- Added a real macOS Qwen3.5 0.8B end-to-end test covering complete weights, explicit startup, authenticated loopback model discovery, actual chat generation, and shutdown.
- Kept API and local model sources independent; opening the local-model manager no longer refreshes the configured remote provider catalog.
- Tightened local-model cards and host-connection controls, including explicit start/stop lifecycle, parameter access, context/runtime estimates, and safer credential presentation.
- Made shared macOS split pages stack their selector above details below 760pt and made diagnostic logs wrap within compact windows.
- Re-rendered regular and compact Automation, Activity, Data, Diagnostics, Extensions, Model, Prompt, and Web Source pages; localization/catalog/navigation contracts pass.
- Verified the Rust Web host module tests and a real token login/session flow. Windows cross-build remains environment-blocked on macOS by missing MinGW/MSVC Windows C toolchains; no native Windows build claim is made.

## 2026-08-11 — Auditable real-model system testing

- Reworked real E2E acceptance around product entrypoints, real provider traffic,
  durable persistence, canonical event streams, semantic judgement, bounded
  process-group timeouts, and periodic phase heartbeats. Executable acceptance
  scenarios are no longer miscollected as pytest fixture-based unit tests.
- Added real iOS and Android uploaded-Skill lifecycle coverage for enable, edit,
  invoke, persist, and disable behavior. Android loopback provider overrides now
  establish and clean up an `adb reverse` mapping automatically.
- Fixed Core policy projection that incorrectly classified internal Skill and
  memory capabilities as filesystem access and consequently removed every iOS
  model tool when no device-result egress receipt existed. Sensitive device data
  tools still require their existing scoped authorization and egress receipts.
- Fixed the iOS Host catalog contract so a tool's argument schema, rather than
  its complete function definition, becomes `function.parameters`.
- Added OpenAI-compatible serialization for Core's provider-neutral flat tool-call
  records, including `type: function`, nested function metadata, and JSON-string
  arguments; this restores the post-tool model round trip with real providers.
- Strengthened central evidence typing so semantic response audits cannot be
  mistaken for persistence audits. The macOS parallel-search/natural-follow-up
  scenario now requires evidence-backed current eligibility and rejects unknown,
  expired, or otherwise unverified recommendations.

## 2026-07-29 — Browser intent, permission clarity, and file artifacts

- Added semantic browser-interaction qualification so explicit visible-Chrome requests
  must attempt the native Chrome bridge instead of silently degrading to web search.
- Normalized generated files from tool results into Agent Host artifacts, allowing
  clients to persist, preview, locate, and reveal real outputs.
- Shortened macOS permission headlines to a localized action and target while retaining
  complete commands and paths in expandable details.
- Removed the legacy macOS Skill router that exposed DerivedData bundle paths and
  instructed models to call `skill_context`; Skill selection now stays Host-owned.
- Expanded the test evidence standard for requested tool paths, conversation approval
  reuse, permission legibility, artifact propagation, cleanup, and Finder reveal.

## 2026-07-19 — Shared Agent Trace and stable streaming presentation

- Established `AgentTraceEvent` and `AgentRunProjection` as the single execution
  and presentation contract across macOS, iOS, Android, Web, and CLI.
- Added append-only Trace Journal semantics, monotonic sequence replay,
  idempotent event handling, gap recovery, and conversation/run isolation.
- Unified long-running activity inside one stable assistant bubble, including
  processing state, thinking, tool parameters, affected resources, permission
  requests, validation, and truncated results.
- Replaced completion-time Markdown swaps with incremental committed blocks and
  one mutable tail, preserving bubble and node identity throughout streaming.
- Defined semantic no-op behavior so cursor-only or duplicate events advance
  internal state without causing observable UI/DOM mutations, flicker, or scroll.
- Expanded cross-platform E2E evidence to cover real Host execution, intermediate
  mounted UI states, Journal recovery, incremental Markdown, and permission paths.

## 2026-07-14 — Cross-platform chat, data, and build reliability

- Documented the current iOS, native macOS, Android, Web/CLI, backend, and shared Rust Core architecture, including the rule that native macOS chat is not a CLI subprocess wrapper.
- Unified the iOS Data tab and Health detail screen on one dashboard snapshot, made successful direct HealthKit reads authoritative over opaque read-authorization states, and wired observer-driven refresh without notification loops.
- Hardened document attachments: iOS/macOS parse supported files locally, DOCX extraction includes body, tables, headers, footers, footnotes, and endnotes, and attachment text is isolated as reference context rather than executable user intent.
- Fixed Android Rust builds for current `cargo-ndk` platform arguments, retained dynamic Cargo/NDK discovery, and validated all four Android ABIs plus device-level Compose tests.
- Expanded macOS contract coverage for attachment archives, conversation metadata, token estimation, host events, localization, personal intelligence, and production visual snapshots.
- Replaced the macOS assistant's single-`Text` Markdown path with stable block-level rendering for headings, paragraphs, lists, quotes, tables, code and Mermaid, and added parser plus production-snapshot coverage.
- Clarified the macOS execution boundary: native Chat uses provider/local-model services or the shared Rust Core directly, remote discovery occurs only for an explicitly selected host, and the independent CLI Console is not a Chat dependency.
- Localized dynamic macOS Chat controls that bypass SwiftUI's literal-key localization, including model/prompt/context/workspace rows and tool capability names.
- Removed the obsolete hard-coded `iPhone 16` test destination from repository guidance; developers must select a simulator available in their installed Xcode runtime.

## 2026-05-23 — Web and CLI computer-use tools

- Added Web Agent computer-use tools backed by the native app host: desktop status, full or cropped screenshots, cursor position, window listing, mouse move/click/drag, text entry, key press, scroll, and app activation.
- Extended `web/native_app/host/nola_native_app_host.py` with macOS screenshot, Quartz mouse/scroll events, and System Events keyboard/text actions for `app=computer`.
- Registered native app tools in the Web tool registry and updated prompts/default skills so computer-use is used only for desktop/non-browser flows with confirmation.
- Added matching Rust CLI `computer_*` tools using a local Python helper, plus CLI docs, help text, tests, and validation.

## 2026-05-16 — iOS chat context data-source enrichment

- Updated `DefaultContextManager` so conversation-attached data sources inject real readable items (title/summary/content), not only source-name history titles.
- Added URL live-fetch fallback for context building: when a selected URL data source has no local history, context assembly can fetch and inject current URL preview items.
- Added scheduled-output context support: selected scheduled sources now pull recent `ScheduledActionRunRecord` + related journal content from `LocalJSONStorage` and inject them as data-source context items.
- Wired `LocalJSONStorage` into `DefaultContextManager` through `ServiceContainer` injection so scheduled run outputs are available during context assembly.

## 2026-05-16 — Web Agent generic native app host control (internal TODO)

- Added a generic native app bridge (`web/app/agent/native_app_bridge.py`) with host queueing, preferred host routing, and token-authenticated host poll/result APIs.
- Added Web API endpoints `/api/native-app/status`, `/api/native-app/command`, `/api/native-app/host/poll`, and `/api/native-app/host/result`.
- Added Agent tools `native_app_status` and `native_app_invoke` to control host-native software via generic `app/action/params` (no fixed action whitelist), with sensitive-operation confirmation checks.
- Added a local host daemon under `web/native_app/host/nola_native_app_host.py` plus installer script `web/native_app/install_host.sh`; includes generic AppleScript driver and NetEase Cloud Music aliases as a reusable sample.
- Native app host now proactively requests macOS Accessibility/Screen Recording permissions at startup (when missing) and surfaces permission-specific guidance on AppleScript failures.
- Added regression coverage for native app tool registration/validation, bridge roundtrip, token enforcement, command execution, and host-id forwarding.
- This capability is now kept as internal TODO by default: user-facing registration and `/api/native-app/*` endpoints are gated behind `NOLA_ENABLE_NATIVE_APP_CONTROL` and remain disabled unless explicitly enabled.

## 2026-05-15 — Scheduled task model/skill/workflow customization

- Removed backend/worker reliance on `PUSH_LLM_MODEL`; scheduled-task defaults now read `MODEL`.
- Added per-source scheduled-task fields `llmModel`, `workflowPrompt`, and `skills` (with `model`/`skill` compatibility aliases) to sync/list/debug payloads and DB persistence.
- Added scheduled execution skills runtime with whitelist validation (`time.now`, `search.serper`) and injected skill outputs into LLM context.
- Updated run-now/scheduler execution path to prioritize source-level model + workflow prompt and record effective diagnostics.
- Added regression tests for source field persistence, unsupported skill rejection, and run-now source-level model/workflow/skills behavior.

## 2026-05-14 — Backend auth/CORS hardening

- Fixed JWT verification to use fixed algorithm allowlists instead of trusting token header `alg` in FastAPI Apple/Google auth paths and App Store notification JWS verification.
- Hardened Worker provider token verification by rejecting non-`RS256` identity-token algorithms explicitly.
- Removed Worker wildcard CORS (`access-control-allow-origin: *`) so the API backend no longer permits arbitrary browser origins by default.
- Replaced dynamic SQL fragment assembly in Worker auth/notification queries with explicit fixed-column / fixed-query variants.
- Added scan-guard protections in both FastAPI and Worker backends to rate-limit repeated unknown-path probes and temporarily ban abusive scanner IPs.
- Disabled FastAPI Swagger/ReDoc/OpenAPI endpoints by default (`ENABLE_API_DOCS=false`) for API-only deployment posture.
- Optimized gSerper retrieval flow: added per-user short-TTL cache, moved quota charging to successful upstream fetches only, and reduced duplicate Serper calls for repeated same-query lookups.

## 2026-05-12 — Notification center sync

- Added backend notification records and authenticated `/v1/notifications` sync/read endpoints.
- Added iOS unread red dot behavior, app badge updates, backend notification sync, and tap-to-mark-read handling.
- Added a Web消息 page with navbar unread badge and read/read-all actions.

## 2026-05-12 — Subscription Google search quota

- Added monthly Google (Serper) search quota tracking for subscription users: default 100 requests/month (`SUBSCRIPTION_MONTHLY_SEARCH_LIMIT`).
- Enforced the same quota across both `/search/serper` and chat retrieval (`/v1/chat/completions` with `enable_retrieval=true`).
- Added backend regression tests covering successful Serper access, quota exhaustion on `/search/serper`, and quota exhaustion in chat retrieval flow.

## 2026-05-10 — Atomic monthly usage accounting

- Added a unique `(client_uuid, period_start)` invariant for backend monthly usage records and startup deduplication for legacy hyphen/hex UUID rows.
- Switched monthly usage increments to database-level upserts on PostgreSQL/SQLite so concurrent requests accumulate token/request counters instead of overwriting each other.
- Added regression coverage for legacy UUID-format deduplication and parallel usage increments.

## 2026-05-10 — Official OAuth Relay for local Web

- Added official backend OAuth Relay endpoints (`/oauth/login`, `/oauth/start/{provider}`, `/oauth/callback/{provider}`, `/oauth/relay/exchange`) so local Web clients can use Apple/Google/GitHub without registering `127.0.0.1` redirect URIs.
- Updated Web OAuth start/callback flow to enter the backend `/oauth/login` page via `OAUTH_RELAY_BASE_URL` and exchange one-time relay codes for Nola access tokens.
- Unified OAuth response contracts across backend, Web, and iOS: provider auth responses always include `accessToken`, `expiresIn`, `userId`, `authProvider`, and `providerUserId`; Apple additionally includes `appleUserId`.
- Hardened provider auth edge cases: Apple now accepts configured Web/iOS audiences via `APPLE_SIGNIN_AUDS` and issues provider-prefixed Nola JWT subjects; Google unverified emails no longer block sign-in and instead use a synthetic provider-local email.
- Documented provider callback URLs and relay environment variables.

## 2026-05-09 — Official backend OAuth-only accounts

Summary:
- Disabled official backend email/password registration and login endpoints; `/v1/auth/register`, `/v1/auth/login`, and official mobile password login now return 410.
- Added Google and GitHub provider auth endpoints alongside Apple, with provider identity verification, automatic `UserAccountRecord` creation/reuse, and Nola JWT issuance.
- Updated Web `/login` and `/link-backend` to use Apple, Google, and GitHub OAuth redirects/callbacks for official backend linking instead of collecting a backend password.
- Updated docs and tests for the provider-only official backend account model.

## 2026-05-08 — Web Agent native Chrome browser bridge

Summary:
- Added a Chrome MV3 extension plus Python Native Messaging host bridge under `web/native_browser`, allowing the local Web Agent to operate the user's real Chrome tabs instead of only headless Playwright.
- Added token-protected Web host polling/result endpoints and an in-process command queue for native browser commands.
- Registered `chrome_native_*` Agent tools for status, open page, click, type text, key press, and scroll, with prompt/skill guidance to prefer them when the user asks for real Chrome or existing login sessions.
- Hardened the macOS host installer so Chrome points at an executable launcher in `~/.nola`, and the launcher starts the Python host with an absolute interpreter path; verified an end-to-end `chrome_native_open_page` call against real Chrome.
- Added mandatory confirmation for browser automation involving account login, credentials, verification codes, authorization, payment, or other sensitive-information flows across both `browser_*` and `chrome_native_*` tools.

## 2026-05-07 — Chat retrieval, context, memory, and Web UI refinements

Summary:
- Made official backend Serper retrieval configurable via `SERPER_API_URL` and added local HTTP integration coverage for `/search/serper` plus `enable_retrieval` chat injection.
- Web plain/custom chat now injects persisted user preferences, long-term memory, session memory, and auto-retrieved compact snapshots into the system context, matching the Agent context path; it also exposes the same hidden memory tool protocol so durable memory is persisted only when the main LLM explicitly requests it.
- Web auto-compact now uses the configured local OpenAI-compatible model for summary generation when available, falling back to deterministic local summaries otherwise.
- iOS ordinary non-Web-host chat now exposes a hidden memory tool protocol; the main LLM only requests persistence when it judges a stable long-term memory or user preference should be saved, and the hidden request is stripped from UI/history before `ChatMemoryProfileStore` is updated.
- Polished the Web chat toolbar so model/style controls stay on one row with horizontal overflow on narrow screens instead of wrapping the style selector vertically.

## 2026-05-06 — Claude Code-inspired Web Agent refinements

Summary:
- Added a Web Agent `update_agent_todos` tool inspired by Claude Code's `TodoWriteTool`, giving complex multi-step Agent requests a session-local checklist with one active task and explicit verification guidance.
- Updated the shared Agent system prompt to tell the model when to use the todo tool and when to skip it for simple one-step work.
- Borrowed Claude Code's command-exit semantics for the Web Agent bash tool so `rg`/`grep` no-match, `diff` differences, and partial `find` traversal are not rendered as hard tool failures.
- Tightened bash validation to reject additional shell substitution forms (`${}` / `$[]`) before execution.

## 2026-05-06 — Official backend compatibility refresh

Summary:
- Added official FastAPI compatibility endpoints for current Web/iOS needs: `/v1/models`, Agent settings/permission rules/tools, chat/custom/Agent streaming aliases, `/v1/subscribe/*` aliases, and `/api/mobile/*` / `/v1/mobile/*` mobile bridge routes.
- Added backend mobile AES-256-GCM envelope helpers matching the Web host bridge protocol, plus encrypted mobile attachment parsing and encrypted Agent event SSE for iOS clients.
- Added `AgentSettingRecord` and `AgentPermissionRuleRecord` persistence for official backend Agent configuration and permission state.
- Kept official backend Agent execution tool-safe: it can stream `text` / `reasoning` / `done` events, but does not expose server-side shell, filesystem, or browser automation tools.

## 2026-05-06 — iOS data/model quick UI fixes

Summary:
- Expanded the iOS Data tab scheduled-task row hit area so tapping anywhere on the row opens details.
- Removed the duplicate Web-host choice from the inner “添加云端模型” flow by locking it to OpenAI-compatible cloud model setup.
- Clarified “模型名称 / API model” versus “本地称呼” in cloud model setup copy.

## 2026-05-06 — iOS chat More Settings hub

Summary:
- Replaced the iOS chat composer's standalone max-iterations pill with a “更多设置” entry.
- Added `ChatMoreSettingsSheet` to centralize AI conversation parameters: response style, temperature/top-p, max context/output, thinking/stream toggles, Agent mode/max iterations, retrieval, auto-compact threshold, session/long-term memory, user preferences, and Skill enablement.
- Made the iOS auto-compact trigger threshold configurable via `chat.auto_compact.trigger_percent` instead of hard-coding 80%.

## 2026-05-06 — Web/iOS prompt response styles

Summary:
- Added shared response-style prompt guidance for 严谨专业、日常聊天、俏皮活泼 under `Nola/SharedPrompts/`.
- iOS model parameters now store a default response style, ChatView exposes a current/new conversation style selector, and OpenAI/local/Web-host providers receive the selected style through prompt/userPrefs routing.
- Web chat and `/models` now expose a shared style selector and apply it to normal chat, custom LLM streaming, Agent v2, and encrypted mobile Agent bridge requests.

## 2026-05-05 — Web/iOS Agent mode

Summary:
- Added an explicit Agent mode switch to Web and iOS chat for complex multi-step problem solving.
- Web Agent mode now routes through Agent v2 with the shared complex workflow prompt; normal chat can stay on the plain streaming path.
- iOS Agent mode injects the same shared workflow prompt while enforcing the no-code local boundary; host tools are only used through a connected Nola Web host with host-side permissions.
- iOS Web-host payloads forward `agent_mode=true` without sending iOS-side model, endpoint, or API key configuration.
- Added a `disk_usage` local-dev tool for host disk-space questions, with tool-level confirmation required before broad full-disk or large system-directory scans; partial/timeout folders are reported explicitly.
- Refined Web Agent permissions toward the Claude Code model: rules now support `allow` / `deny` / `ask`, tools expose content-specific match values, and wildcard allow rules no longer bypass content-level `ask` decisions.
- Added inline interactive permission prompts for Web and iOS Web-host Agent mode: `permission_request` events pause the stream, the current AI reply shows an attached confirmation card, and approved actions retry with a one-shot permission decision instead of changing permanent rules.

## 2026-05-05 — Web generic readable data sources

Summary:
- Added Web scheduled-action support for local `format=file` and `format=folder` readable data sources, constrained to Agent allowed directories and stored local-first.
- Local file/folder run-now execution now writes `local_file_snapshot` events into Web execution history without syncing file paths or contents to the backend.
- Updated Web chat context management so browser-selected files and folders are saved as local readable context assets with source-kind metadata.
- Updated Web scheduled-actions and chat UI copy from URL-only/document-only language to generic readable data-source language.

## 2026-05-05 — iOS generic readable data sources

Summary:
- Added shared iOS metadata for generic readable data-source kinds and capabilities, covering websites, search results, APIs, feeds, files/folders, system sources, and custom sources.
- Kept existing `URLDataSource` storage backward-compatible while inferring a generic kind for legacy records and exposing readable context items for chat/scheduled operations.
- Added local file/folder source selection with on-device validation/reading and bookmark persistence for Files-picked URLs.
- Updated iOS Data Sources, custom source management, and Chat context UI copy so sources are presented as any readable data, not only URLs.

## 2026-05-05 — iOS DashScope thinking toggle enforcement

Summary:
- Cloud chat now sends DashScope/Qwen thinking-capable models the request-level `enable_thinking=false` flag when the model parameter toggle is off, instead of relying only on prompt instructions.
- Streaming output now suppresses both `reasoning_content` and literal `<think>...</think>` content when thinking is disabled, preventing hidden reasoning from leaking into ChatManager/UI state.
- Added OpenAIProvider regression coverage for provider-specific thinking request bodies.

## 2026-05-05 — iOS UI card system refactor

Summary:
- Added shared iOS UI primitives for compact inline empty states, icon badges, and standard row/icon sizing so cards and list-like pages share one visual language.
- Migrated scheduled actions, scheduled run history, notifications, notification detail fallback, URL data sources, model management, model picker, model download, model testing, and chat bubbles/empty state to the shared primitives and design tokens.
- Preserved intentionally centered hero/marketing layouts while tightening functional pages toward leading alignment, compact cards, consistent badges, and tokenized button heights/radii.

## 2026-05-05 — iOS UI alignment polish

Summary:
- Standardized leading alignment for same-level text in high-traffic iOS screens, including Home summaries, Data Sources, URL sources, Model Management, Settings rows, scheduled run history, and medication search.
- Updated shared empty-state cards to use leading text alignment inside full-width cards, reducing accidental centered/ragged text in list-style pages.
- Verified the iOS app builds and the existing iOS test scheme passes after the UI-only changes.

## 2026-05-05 — Chat title persistence hardening

Summary:
- Default chat titles now use the first non-empty user query, so accidental whitespace-only messages no longer leave conversations permanently named "新对话".
- Web chat now treats message persistence as the source of truth for first-query titles instead of issuing a separate pre-message PATCH; the message response returns the persisted conversation title for immediate UI refresh.
- Web local chat titles now use text storage to preserve long first-query titles, and iOS ChatManager tests avoid XCTest-time `@MainActor` deallocation instability while covering the new title edge case.

## 2026-05-04 — Chat titles use the first user query

Summary:
- Backend, Web local-first chat APIs, and iOS ChatManager now name default/new conversations from the first user message instead of an LLM-generated or truncated summary.
- Existing explicit conversation titles are preserved, and later user messages do not overwrite the first-query title.
- Added backend, Web, and iOS regression coverage for first-query title behavior.

## 2026-05-04 — Dependency alignment and real integration test hardening

Summary:
- Aligned backend SQLModel/SQLAlchemy/python-dotenv pins with the Web test environment so backend and web tests can run from the same Python environment without import conflicts.
- `/purchases/tier/{uuid}` now returns `reported_tier` in addition to the effective `tier`, letting Web show expired subscriptions as current `free` while preserving the original paid tier for UX.
- Hardened the real no-mock integration script and tests by disabling unrelated background outbox work, increasing real backend registration timeout tolerance, and ignoring generated dependency stamp files.

## 2026-05-03 — Real no-mock integration test pipeline for subscription/token state

Summary:
- Added backend real integration suite (`backend/tests_integration/`) that boots a real Uvicorn process against SQLite and verifies auth registration, `/v1/auth/me`, `/purchases/tier/{uuid}`, and `/purchases/usage/status/{uuid}` without monkeypatch/mocks.
- Added web real integration suite (`web/tests/test_account_status_integration.py`) that starts a real backend process and verifies `/api/me` behavior for token-expired, subscription-expired, and active-subscription flows.
- Added `scripts/run_real_integration_tests.sh` to create isolated backend/web virtualenvs, install pinned dependencies, and run real backend+web+iOS integration checks end-to-end, avoiding global Python dependency conflicts.
- Web `/api/me` now actively validates linked backend token via `/v1/auth/me` and returns explicit `auth_status=expired` guidance; subscription-expired detection now relies on real backend `expires_at` semantics.

## 2026-05-03 — iOS Web-host skills fetch timeout storm fix

Summary:
- ChatView now only fetches `/api/mobile/agent/settings` when the active model is a Web backend, instead of falling back to any stored Web host model.
- Added in-flight dedupe and cooldown/backoff for host-skill refresh requests, preventing concurrent timeout storms when conversation/model state updates rapidly.
- Host-skill fetch tasks are now cancelled on view teardown to avoid stale retries after leaving chat.

## 2026-05-03 — MCP/Skill context interaction hardening

Summary:
- Agent settings normalization now deduplicates MCP servers by endpoint and uploaded skill/document items by name+content to reduce duplicate prompt/context payloads.
- Agent context assembly now applies MCP/Skill/Document injection caps and shared content budget, preventing oversized uploads from crowding out MCP guidance.
- Added full-flow regression coverage for `/api/chat/agent/v2/stream` and `/api/mobile/chat/agent/v2/stream` to verify MCP + Skill + auto-retrieved memory are injected together correctly in conversation mode.

## 2026-05-03 — Context compaction raw-memory retention/search

Summary:
- Web compact now persists raw compact snapshots (`LocalCompactedMemoryEntry`) alongside session-memory summaries, so compact no longer relies on truncated summary text as the only memory carrier.
- Added Web API `GET /api/chat/conversations/{id}/memory/search` and context-panel UI search to retrieve raw compact snapshots and archived messages by keyword.
- iOS compact now stores `Conversation.compactedMemoryEntries`, and `ContextManager.searchRawMemory(...)` + Chat context sheet provide local raw-memory lookup after compaction.
- Web and iOS now default to automatic context operations: auto-compact runs when context usage crosses threshold, and raw-memory lookup is applied automatically during agent context assembly; manual context controls are no longer primary UX.

## 2026-05-03 — Web/iOS context suggestions refined

Summary:
- Borrowed Claude Code-style context diagnostics into both Web and iOS: suggestions now carry severity (`high/medium/low/info`), prioritized ordering, and estimated token savings.
- Web `/api/chat/conversations/{id}/context` now returns `suggestionDetails`, and chat context panel renders structured suggestion cards instead of plain text hints.
- iOS `ContextManager` now generates the same structured suggestion model and `ChatContextSheet` shows severity-tinted recommendation cards with estimated savings.

## 2026-05-03 — Stripe subscription flow validated end-to-end

Summary:
- Documented a full local run of Stripe subscription flow: checkout creation, post-payment verify endpoint, webhook update, and final tier reconciliation.
- Verified final backend state for test UUID reaches `tier=subscription`, with webhook-driven period-end update reflected in `/purchases/tier/{uuid}`.

## 2026-05-02 — Web Stripe checkout verification hardening

Summary:
- Added backend endpoint `POST /purchases/stripe/checkout/verify` so Web can submit Stripe `session_id` for server-side verification and immediate subscription upsert.
- Stripe default success redirect now includes `session_id={CHECKOUT_SESSION_ID}` for deterministic post-payment reconciliation.
- Web Settings page now auto-verifies Stripe success callbacks, surfaces sync status to users, and refreshes subscription display after verification.
- Added backend/web regression tests covering checkout verification, ownership checks, and Web proxy behavior.

## 2026-05-01 — Cross-platform account merge UX

Summary:
- Added an iOS Settings account-merge guide so users understand how Web backend linking, iOS Sign in with Apple, App Store subscriptions, official-model quota, and push devices converge on one account.
- Updated iOS privacy disclosure copy to explain that Apple identity tokens, client UUIDs, and subscription receipts are used for explicit account merge and subscription sync.
- Kept the Web Settings merge guide and backend account-sync semantics aligned with the same product language.

## 2026-05-01 — Web settings account merge guidance

Summary:
- Added a Settings section that explains cross-platform account merging from the user's perspective: Web backend link, iOS Sign in with Apple, subscription verification, and manual/support fallback.
- The guide adapts its CTA based on whether the Web account is already linked to the backend.

## 2026-05-01 — Cross-platform subscription account sync

Summary:
- Apple sign-in now migrates pre-merge client data from the submitted iOS client UUID to the canonical Web/Apple account when an existing account is matched.
- Account migration now recognizes both hyphenated UUIDs and hex UUIDs, and merges UsageRecord counters into the canonical hex usage row.
- Manual Apple linking and auth profile tier reporting now use active, unexpired purchase semantics, so expired paid records do not block safe merges or appear as active tiers.

## 2026-05-01 — Account usage and subscription hardening

Summary:
- Subscription tier checks now require an active, unexpired purchase; inactive or expired purchase records are treated as free.
- Usage status is read-only, UUIDs are normalized to hex for usage records, and usage responses include request/prompt/completion/total token breakdowns.
- Web settings now shows local/backend account state, official-model quota, remaining tokens, request count, and clearer subscribe/link/configure-model prompts.

## 2026-05-01 — Scheduled Actions: host-local script automation

Summary:
- Added Web-local script scheduled actions (`format=script`) for user-provided or LLM-generated bash scripts.
- LLM/Agent-created scripts are saved disabled and require explicit Web UI approval before they can run.
- Script execution is host-local only, never synced to backend, and is gated by local developer tools, allowed directories, bash prefix whitelist, timeout/resource limits, and stdout/stderr execution logs.

## 2026-04-30 — Scheduled Actions: harden URL automation boundary

Summary:
- Web scheduled-actions UI now removes manual creation forms and directs users to create URL/search automations through AI chat.
- Web and backend scheduled-source APIs reject local reminder-shaped payloads (`format=reminder` or `nola://reminders/*`) so reminders stay in local task/notification storage.
- iOS scheduled-action copy and paid push sync now only describe/sync URL automations, not local reminders.

## 2026-04-30 — iOS Tests: remove false signals

Summary:
- Removed skipped, placeholder-only, mock/substitute-driven, and stale UI automation tests that produced false positives, false negatives, or simulator hangs.
- Kept deterministic product-boundary tests for URL automations vs local reminders, run-now record semantics, notification stores, context memory, model response parsing, and Web Agent payload parsing.
- Extracted run-now error presentation into a pure helper so tests assert production code instead of duplicating classification logic.

## 2026-04-30 — iOS Scheduled Actions: separate reminders from URL sources

Summary:
- Local reminders created from AI chat now stay in `UserTaskService` and local notifications instead of being mirrored as `URLDataSource` scheduled actions.
- Legacy reminder-shaped URL sources are filtered out of URL data-source lists, scheduled automation lists, and scheduler execution candidates to avoid duplicated local reminders.
- The Data tab now surfaces locally created reminders in the local schedule card, while scheduled automation remains focused on URL-backed periodic fetch/report tasks.

## 2026-04-29 — Web Agent: default browser operation Skill

Summary:
- Added a built-in “浏览器操作” Skill covering local browser workflow, LAN/private-page handling, form filling, login verification, and safe result reporting.
- The default Skill is automatically included in Agent settings and chat context, while user-uploaded Skills continue to work alongside it.
- Context token accounting avoids double-counting the default Skill in scaffold estimates.

## 2026-04-29 — iOS Notifications: clear unread dot on tap

Summary:
- Tapping a scheduled-action push notification now marks matching in-app notification records as read by notification id, dedupe key, or push result id.
- Push result notifications fetched asynchronously after a notification tap are also marked read, so the top-right unread dot does not stay visible after opening the result.

## 2026-04-29 — iOS Chat: composer file attachments

Summary:
- iOS chat composer plus button now imports local files instead of toggling retrieval.
- Selected image/document attachments appear as removable chips above the input; images are sent as OpenAI-compatible multimodal `image_url` content, while PDF/text/RTF/HTML files are text-extracted locally and injected into the next turn.
- Attachment prompts are persisted in conversation context while chat history shows a compact attachment summary instead of dumping full extracted file content.

## 2026-04-29 — iOS/Web Agent: browser state visibility fixes

Summary:
- iOS browser screenshot previews now fetch host artifacts through the WebAgentProvider resource path instead of SwiftUI `AsyncImage`, so screenshot loading follows the same host URL handling as Agent streaming.
- iOS persists Web Agent host tool events in conversation message metadata and restores browser tool cards, final URLs, and screenshot previews when reopening chat history.
- iOS Web host setup now accepts bare LAN addresses, strips copied `/api/mobile/...` paths, rejects phone-unreachable loopback/all-interface hosts, and turns host connection failures into actionable LAN setup guidance.
- iOS chat connection/provider failures no longer render as a global inline status above the composer or keep the input locked while the user switches/retries another service.
- Web local chat messages now support metadata and persist Agent tool events so browser screenshots/tool cards can be rendered again from Web history.
- Web Agent browser sessions are kept per local user for a short bounded window, allowing follow-up turns to continue operating the same headless Chromium page instead of losing page state at request end.

## 2026-04-28 — iOS/Web Agent: iOS as host-input surface

Summary:
- Web mobile Agent bridge now uses host-side `LocalUserAgentSettings` for LLM endpoint/API key/model, MCP, browser, and tool permissions; iOS requests no longer override those host settings.
- LAN exposure is the development default: `dev.sh` defaults `NOLA_LAN=1`, binds Web/backend to `0.0.0.0`, and uses HTTP for easier iOS local-network connection; `NOLA_LAN=0` keeps loopback-only startup.
- iOS now allows local-network HTTP for self-hosted Web Agent connections while keeping AES-GCM encryption for Agent request bodies and SSE events.
- iOS Web Agent configuration can store a pinned certificate SHA-256 fingerprint; WebAgentProvider pins that certificate for login, token validation, and Agent streaming.
- iOS/Web mobile Agent traffic now encrypts request JSON and streamed SSE events with AES-256-GCM using the bridge access token as key material; plaintext Agent requests are rejected.
- iOS Web Agent provider parses structured SSE `tool_use` / `tool_result` events, resolves browser artifact URLs, and exposes host tool events to chat UI separately from assistant text.
- iOS chat bubbles now render host tool cards with status, tool name, parameters/results, final URL, and screenshot link/preview for host browser operations.
- Web Agent model setup on iOS treats the model field as host-owned and only requires Web base URL + bridge token.

## 2026-04-28 — Web Agent: host-side browser tool

Summary:
- Added `browser_open_page`, `browser_click`, and `browser_type_text` to the Web Agent tool registry so the Web host can open public pages with headless Chromium, read rendered visible text, click specified page areas/selectors, type text, and save PNG screenshots.
- Added `browser_press_key` and `browser_scroll`, and browser results now include an interactive element summary to make follow-up clicks/typing more reliable.
- Screenshot artifacts are stored locally under Web static artifacts and rendered inline in Agent tool-call cards.
- Browser URLs reuse public http(s) SSRF protections and reject localhost/private/file targets, including unsafe redirects and page subresource requests.

## 2026-04-28 — iOS: scheduled actions moved into Settings

Summary:
- Moved scheduled actions out of the primary tab bar and into Settings under a new automation/computer-control section.
- Added a Settings entry for connecting a self-hosted Web service so iOS can use the Web Agent bridge to control the user's own computer.
- Improved one-off reminder scheduling so local reminders can appear as scheduled actions without being treated as URL/LLM report jobs.
- Added native natural-language update/pause/resume/delete handling for local scheduled actions while preserving Web Agent bypass for Web backend models.

## 2026-04-27 — Chat: real MCP runtime, uploaded skills, and documents

Summary:
- Web and iOS chat context profiles now support local-first MCP endpoints, uploaded skills, and uploaded documents.
- Web Agent can now actually list/call MCP tools and list/read MCP resources through JSON-RPC over configured HTTP endpoints.
- iOS now queries configured MCP endpoints while building chat context, including live tools/resources and a small sample of resource content.
- Uploaded skill/document content is injected into chat context and counted in context usage reporting.

## 2026-04-27 — Chat: Claude-style context management added to Web and iOS

Summary:
- Web chat added context usage reporting, editable session memory, and manual compact APIs/UI backed by local conversation state.
- iOS chat extended local conversation state with session memory, archived-from-context markers, context usage estimation, and manual compact controls.
- Both clients now use “context management” to mean Claude Code style context window management rather than only prompt-template or data-source attachments.

## 2026-04-27 — Chat: Agent mode made always-on across Web and iOS

Summary:
- Web chat removed the explicit 🤖 Agent toggle and now sends messages through `/api/chat/agent/v2/stream` by default.
- iOS chat removed the persisted Agent mode flag and now always routes task-like input into the local task parsing flow.
- Updated chat-facing documentation and settings copy so Agent behavior is described as default-on with no user-visible mode switch.

## 2026-04-27 — Web: Claude Code 风格 Agent v2 集成

Summary:
- 在 Web 端新增 `app/agent/` 模块：Tool ABC、Registry、PermissionRule、ToolUseContext、
  OpenAI 兼容流式 LLM 客户端（`stream_chat`，处理 `tool_calls` 与 `reasoning_content`）、
  asyncio 多轮工具调用循环 `arun_agent` 与 Flask 同步桥 `run_agent_sse`。
- 工具实现覆盖：web_safe（now / calc / web_fetch+SSRF / web_search / 5 个定时任务工具 /
  get_task_history）+ local_dev（read_file / write_file / edit_file / multi_edit_file / list_dir / glob / grep / bash，
  默认禁用，需在设置启用并配置目录与 bash 前缀白名单）。
- 新增 SSE 接口 `/api/chat/agent/v2/stream` 与 `/api/agent/{settings,tools,permission_rules}`。
- 聊天页右上角加入 🤖 Agent 开关（**默认开启**），新增工具调用卡片 UI；
  Agent 直连 LLM **复用聊天页「连接设置」中的本地 LLM 配置**，不再单独配置端点。
- 新增 `LocalUserAgentSettings` 与 `AgentPermissionRule` 表，启动自动建表。
- 新增 `web/tests/test_agent_basics.py`（4 项；已通过）。
- 文档：`Doc/软件架构设计/总体设计/REQUIREMENTS.md` §6.5、`Doc/软件架构设计/总体设计/SOFTWARE_DESIGN.md` §9.1、`web/README.md`。

## 2026-04-27 — iOS UI shell rewritten around Web layout system

Summary:
- Added a shared SwiftUI design system in `Nola/UI/DesignSystem.swift` with Web-inspired page headers, cards, badges, stat cards, empty states, spacing, radius, and color tokens.
- Reworked the iOS main shell to expose Web-aligned primary sections: Home, Chat, Scheduled Actions, Results, and Settings.
- Reframed Home, Settings, Scheduled Actions, and Chat toolbar surfaces around local-first cards and explicit optional-backend messaging while preserving existing services/data/LLM layers.
- Kept ServiceContainer, storage, model, scheduled action, and chat execution logic intact; this change is UI/interaction-layer focused.

Files touched:
- `Nola/UI/DesignSystem.swift`
- `Nola/UI/MainTabView.swift`
- `Nola/UI/HomeView.swift`
- `Nola/UI/SettingsView.swift`
- `Nola/UI/ScheduledActionsView.swift`
- `Nola/UI/ChatView.swift`
- `Nola/UI/ModelManagementView.swift`

## 2026-04-23 — Backend: scheduled actions opened to all authenticated users

Summary:
- Removed subscription gate from scheduled-actions management APIs in `backend/app/push.py`:
  - `/v1/sources`, `/v1/sources/sync`, `/v1/sources/{source_id}`, `/v1/sources/{source_id}/run_now`
  - `/v1/results`, `/v1/results/{result_id}`, `/v1/results/{result_id}/ack`
  - `/v1/devices/register`, `/v1/auth/apple`
- Kept LLM usage subscription-restricted by enforcing subscription check inside scheduled job execution before calling upstream LLM.
- Added regression tests to ensure scheduled-actions endpoints no longer invoke `_require_subscription`.
- Updated requirements/design docs to reflect: **only LLM call paths are subscription-gated**.

## 2026-01-26 — UI: Prompt templates & Scheduled Actions / Data Sources changes

Summary:
- Move Prompt templates editing/management out of the Data Sources page and into Scheduled Actions.
- Data Sources page now shows an inline list of custom URL sources; each row is tappable to edit name/URL/description using `AddURLDataSourceSheet`.
- Scheduled Actions page shows a single-line "Manage Prompt Templates" entry (`PromptTemplatesCard`) instead of an inline template list; left options and top-right "+" removed.
- `ScheduledActionBasicEditorView` retains prompt template picker and TextEditor for per-action prompt customization.

Files touched:
- `Nola/UI/URLDataSourcesView.swift`
- `Nola/UI/MainTabView.swift`
- `Nola/UI/ScheduledActionsView.swift`
- `Nola/UI/ScheduledActionBasicEditorView.swift`

Notes:
- Acceptance: Data Sources only lists sources; editing opens basic editor; Scheduled Actions hosts the templates management entry; builds and core interactions verified locally.
- Follow-ups: Visual polish (long-press menus, row-level quick actions), simulator screenshots on request.
- Core now terminates a Turn after three consecutive model completions that make no progress on a pending validation or recovery obligation. Useful tool rounds remain unlimited, while empty/prose-only loops can no longer consume CPU indefinitely.
# 2026-08-12

- Fixed conversation-scoped tool approval so the approval lifetime no longer becomes a wildcard resource grant. Agent Host clients now reuse approval only for the normalized operation family and resource covered by the original request.
- Fixed the CLI Agent Host permission bridge to return Core's frozen permission resource instead of reducing Bash approvals to only their working directory.
- Corrected the macOS permission E2E fixture to use an executable destructive command (`unlink`) when verifying that a prior `rm` grant does not authorize a different command family; unsupported shell forms remain preflight failures rather than permission prompts.
- Bounded model retries when a turn repeatedly attempts to finish without satisfying a pending validation or recovery obligation, while preserving unlimited useful tool progress.
