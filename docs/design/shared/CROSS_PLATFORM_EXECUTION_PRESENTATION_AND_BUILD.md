> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Cross-platform execution presentation and build optimization

## Scope

This document records the macOS changes completed in August 2026 and defines
the equivalent contract for CLI, Web, iOS, and Android. The goal is not to make
every surface visually identical. It is to give every client the same execution
semantics while respecting the interaction model of that platform.

## macOS changes

### Progressive code intelligence

- A missing repository index no longer blocks the current answer.
- Native `repo_map`, `code_search`, or `symbol_search` provides a coarse result
  immediately while a fast Codebase Memory index runs in parallel.
- Indexing is a Supervisor-owned durable RuntimeTask. It has session ownership,
  progress, cancellation, failure, retry, and reconciliation state.
- Completion of the background index improves later turns; it never replaces or
  delays the answer already being composed.

### Conversation execution presentation

- Reasoning remains visible as the stable execution narrative.
- The conversation bubble shows only the currently running tool, collapsed to
  its operation, state, and concise progress by default.
- Completed tool calls do not accumulate in the bubble. Their arguments,
  truncated results, affected resources, and ordering remain in the canonical
  trace for audit and dedicated detail views.
- A pending assistant bubble appears before the first provider delta and uses a
  breathing status indicator to reduce uncertainty without introducing a second
  competing status bar.
- Tool failures are committed as failures rather than briefly appearing as
  successful completion.

### Incremental Xcode builds

- macOS Debug/Test builds use Cargo's `dev` profile and shared incremental
  cache. macOS Release/Archive builds use Cargo's `release` profile.
- iOS device and simulator builds retain Cargo's `release` profile because the
  current FFI runtime reproducibly faults in `AgentRuntime::try_new` when linked
  from an unoptimized Rust archive. Its Release archive is still incrementally
  reused across unchanged Xcode builds.
- The derived Core archive records its Cargo profile, so a Debug build cannot
  silently reuse an older Release archive or vice versa.
- Embedded `nola-agent-host` and `nola-service` executables follow the same
  Xcode-to-Cargo profile mapping.
- Xcode dependency files contain source files only. Directory timestamps and
  nested Cargo `target` trees are excluded.
- Consecutive unchanged macOS builds skip both Cargo Core and Runtime builds.
- macOS validation uses `ARCHS=arm64` and `ONLY_ACTIVE_ARCH=YES`.

## Shared presentation contract

Every client must keep two separate projections:

1. **Canonical execution history** stores every reasoning, tool, permission,
   result, resource, and task event in authoritative order.
2. **Live conversation focus** contains the latest reasoning state and at most
   one current tool. It is derived from canonical state and is never persisted
   as a replacement for history.

The current tool is the latest non-terminal tool belonging to the active turn.
When it becomes terminal it leaves the live focus. A dedicated details or audit
surface may render historical calls on demand.

This separation prevents a long task with hundreds of calls from producing a
hundreds-row chat bubble while preserving complete observability.

## Platform application

### CLI

- The transient footer owns current tool/progress state.
- The scrollback contains meaningful milestones, permission decisions, failures,
  and final results rather than a completed line for every low-level call.
- `--debug` may expose the complete event journal without changing normal-mode
  conversation layout.
- CLI and CLI-hosted Web share the progressive code-indexing implementation.

### Web

- The trace reducer retains `tools` as canonical history and exposes a separate
  nullable `currentTool` presentation field.
- The live card renders reasoning plus `currentTool`; history belongs in the
  expandable run details surface.
- Terminal runs expose no current tool.

### iOS

- Canonical message parts and Agent Trace retain all tool parts.
- Chat bubbles select only the active tool and keep reasoning visible.
- Durable background work is rendered from Core Task projection, not inferred
  from tool text.
- The shared Xcode Core dependency fix applies to iOS builds as well.

### Android

- `AgentTraceProjection.tools` remains the audit collection and
  `currentTool` is the only collection member eligible for the live chat card.
- Compose state keys use stable trace/span identity; tool history changes must
  not recreate the assistant bubble.
- Gradle Rust inputs cover all Core crates and workspace manifests while
  excluding Cargo output directories, so unchanged builds remain up to date.

## Verification requirements

- Replay a fixture with hundreds of completed tools and one running tool.
- Assert canonical history remains complete and live focus contains one tool.
- Complete the final tool and assert live focus becomes empty without losing
  history.
- Verify reasoning remains visible throughout the transition.
- Run two consecutive unchanged platform builds and assert the second build does
  not invoke Cargo.
- Verify missing-index recovery returns a coarse result while the durable index
  task is still running.

## Canonical host and automation corrections

- macOS uses the authenticated native Agent Host bridge as the only live agent
  execution path. Native macOS tools are advertised only while that bridge is
  available; a generic CLI process does not impersonate Accessibility, Screen
  Recording, browser, or application-owned capabilities.
- A custom `MacAppEnvironment` now opens the Application Store belonging to
  that environment. Isolated tests and secondary app environments no longer
  display an isolated database path while accidentally writing through the
  global `MacApplicationPort.shared` connection.
- Scheduled actions enter the same canonical assistant-turn transaction as
  user submissions. `AutomatedInput` carries a stable submission identity and
  is accepted by both Rust Core and the Application Service instead of failing
  before host execution.
- Automation receipts use a real lease interval and record a success watermark.
  The native permission suspension/resume path executes orchestration exactly
  once and canonical readback proves that persistence succeeded.
- Built-in document and data Skills declare produced artifacts through typed
  `output_artifacts` metadata. Validation uses the typed `operation_kind`
  contract (`validate`, `mutate`, or `mutate_and_validate`) rather than parsing
  shell command text.

## Verified build behavior

The macOS Core subproject now receives an explicit Cargo profile from Xcode:

| Xcode configuration | Rust Core | Embedded Agent Host / Service |
| --- | --- | --- |
| Debug | `dev` | `dev` |
| Test | `dev` | `dev` |
| Release / Archive | `release` | `release` |

The build script logs the selected platform, architecture, configuration, and
Cargo profile, and invokes Cargo with `--locked`. Validation on macOS arm64 used
the same DerivedData directory twice:

- First Debug build logged `platform=macosx`, `archs=arm64`, and
  `cargo_profile=dev`, then succeeded.
- The second unchanged Debug build succeeded without executing either the Core
  script or Cargo.
- Native automation, data-analysis Skill, and scheduled-Skill end-to-end tests
  completed successfully with canonical database readback.

The corresponding retained logs are
`.artifacts/mac-core-profile-first.log`,
`.artifacts/mac-core-profile-second.log`,
`.artifacts/mac-automation-final.log`,
`.artifacts/mac-data-analysis-final.log`, and
`.artifacts/mac-schedule-final.log`.

## 2026-08-29 cross-platform completion audit

### Shared product contract

The audit treats the durable Core projection as authoritative and keeps presentation state separate:

- The canonical transcript retains reasoning, tool lifecycle, parameters, bounded results, citations, and terminal state.
- Live conversation surfaces show reasoning plus only the currently active tool. Completed historical tool calls remain available through the canonical transcript and details views, but are not accumulated into the live long-task surface.
- Client reducers update stable message and Part identities in place. They must not rebuild assistant text from unrelated status strings or append terminal snapshots a second time.
- Permission, Skill, MCP, memory, and data-source routing remains model-selected and Core-evaluated. Clients do not infer routing from language-specific keywords.
- Conversation restoration, compaction, automation, and remote-host projection all reopen from canonical persisted state rather than UI caches.

### Platform results

| Platform | Audit and optimization result |
| --- | --- |
| macOS | Native Agent Host remains the only non-host execution path. Live reasoning and the current tool use stable projections; automation input and outcomes are canonical; the conversation repository preserves message order and cold-reopen state. Targeted presentation, streaming, repository, lazy-loading, and automation tests pass. |
| CLI | Transient status renders the active tool without accumulating completed tool scrollback. Durable journal recovery retains the canonical lifecycle. Context and long-session restore tests remain covered by the Rust suite. |
| Web | `currentTool` is transient UI state and `tools` remains canonical trace state. Stream recovery replays the journal without duplicating assistant output or losing terminal state. |
| iOS | Current-tool projection, semantic streaming reveal, reasoning parsing, canonical persistence, permission presentation, Skill state, host protocol, and FFI boundaries pass the full unit suite. Shared fixtures are bundled in the test target instead of reading host repository paths from the simulator. |
| Android | Compose projection keeps the active tool separate from canonical tool history. The JVM suite validates current-source behavior and Gradle reports all 61 tasks reusable/up-to-date on the audited run. |

### Xcode and Cargo build matrix

| Xcode platform/configuration | Cargo profile | Rationale |
| --- | --- | --- |
| macOS Debug | `dev` | Fast local linking and meaningful Rust debug symbols. |
| macOS Test | `dev` | Test and Debug exercise the same Core artifact. |
| macOS Release | `release` | Shipping optimization only. |
| iPhoneOS Debug/Release | `release` | Stable Swift/FFI artifact for device packaging. |
| iPhoneSimulator Debug/Release | `release` | Stable simulator FFI behavior and parity with device builds. |

`NolaCore.xcodeproj` supplies the profile with SDK-conditional build settings. `core/scripts/build_xcframework.sh` logs the resolved platform, Xcode configuration, architecture, and Cargo profile, uses `cargo build --locked`, and writes dependency/stamp outputs so unchanged incremental builds skip Cargo completely.

The macOS Debug Core archive includes `reqwest` proxy discovery code that references SystemConfiguration symbols. The NolaMac Debug, Test, and Release link settings therefore explicitly link `SystemConfiguration.framework`; using a Release Core artifact to hide missing Debug symbols is not an acceptable workaround.

### iOS startup and test isolation correction

The previous simulator startup hang occurred before Swift `main`: a dynamic Alamofire package product caused dyld to open an absolute `PackageFrameworks` path from the build host. `ModelDownloader` now uses native `URLSessionDownloadTask` with resume data, progress observation, cancellation, HTTP status validation, and atomic destination replacement. Alamofire was removed from the app/test package graph, so the app no longer links that dynamic product.

Five shared Core JSON fixtures previously used `#filePath` to read the host checkout from inside the iOS Simulator. Missing resources caused XCTest failure symbolication to stall in CoreSymbolication. The fixtures now live in `tests/apple/ios/Fixtures` and are resolved through the test bundle, making the tests sandbox-safe, parallel-safe, and independent of the checkout path.

### Verification evidence

- macOS Debug arm64 build: `BUILD SUCCEEDED`; the Core phase resolved `cargo_profile=dev`.
- Immediate second macOS Debug arm64 build: `BUILD SUCCEEDED`; zero Core profile/Cargo phase logs.
- macOS targeted cross-platform contract suite: 24 tests, 0 failures.
- iOS `NolaTests`: 277 tests executed, 9 explicitly skipped real-service E2E tests, 0 failures.
- CLI `cargo test -p nola_cli --locked`: 68 integration/unit tests plus the source-layout test passed.
- Web execution/stream tests: 39 pytest cases passed; current-tool and journal-recovery Node contract tests passed.
- Android `./gradlew test`: `BUILD SUCCESSFUL`; 61 tasks were up-to-date.

Build and test logs for this audit are kept under `.artifacts/` locally and are not product inputs or repository artifacts.
