> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Nola Shared Agent Core

> 2026-10-15 整体设计复核前的历史快照，仅保留当时叙述与证据。旧 ABI、Trace 权威、schema 数字、传输别名和其他旧规则均不作为现行规范。当前入口见[软件架构总纲](../../软件架构设计/总体设计/SOFTWARE_DESIGN.md)。

> This integration contract is subordinate to
> [ARCHITECTURE.md](../实施与审计/ARCHITECTURE.md). Core owns deterministic conversation
> semantics, while Nola Store owns durable complete messages. A Core checkpoint
> is execution recovery state, not a platform conversation database.

## Purpose

`core` is the platform-neutral functional core used by the Rust CLI, Web,
iOS, Android, and macOS Agent Host. The native macOS app is not a CLI wrapper: it
talks to a local or remote Agent Host through the same protocol and consumes
Application/Store projections. The Rust core owns deterministic agent state transitions, tool capability
classification, permission sequencing, context planning, and the JSON command/event
contract.

Platform clients remain responsible for UI, model transport, operating-system
authorization, sandboxed resource access, and lifecycle integration. They must not
copy or independently reimplement shared policy.

## Linking model

| Client | Rust artifact | Host bridge |
| --- | --- | --- |
| CLI | Rust `rlib` dependency | Direct Rust calls |
| iOS | `NolaCore.framework` subproject product | C ABI imported as `NolaCore` |
| Android | `libnola_core.a` for each ABI | Minimal `libnola_core_jni.so` |
| macOS | `NolaCore.framework` + `NolaRuntime` subproject products | Versioned Host protocol + Store/Application ABI |

Android cannot load a static archive directly from the JVM. The JNI shared object is
only an ABI adapter and contains no agent policy. It links the same Rust static archive
used to produce the iOS library.

Generated archives, frameworks, and runtime executables are build outputs and must not be committed. `NolaCore/NolaCore.xcodeproj` is the only Apple build owner for the Rust framework and native runtime products; application targets must not invoke Cargo directly.

## Runtime contract

The host creates or restores one `AgentCore` per conversation, not one kernel per
turn. The thread survives multiple turns and owns an append-only transcript, a
compactable model-context view, context usage, conversation-scoped permission grants,
pending Host effects, immutable Turn contexts, and UI Parts.

The only public mutation API is an UTF-8 JSON `AgentSubmission`:

- `user_input`
- `permission_response`
- `host_response`
- `update_host_catalog`
- `host_disconnected`
- `resume_host_effect`
- `interrupt`
- `compact`
- `reset`

Each submission synchronously returns one `AgentTransition` containing ordered event
envelopes, new Host request envelopes, and a snapshot. A platform executes Host
requests and submits normalized Host responses; it never dispatches internal model or
tool commands.

The mobile coordinator renders only Core events:

- `turn_started`
- `item_started` / `item_delta` / `item_completed`
- `permission_requested`
- `tool_started` / `tool_completed`
- `host_progress`
- `context_updated`
- `compaction_requested` / `compaction_completed`
- `turn_completed` / `turn_cancelled` / `turn_failed`

Provider chunks must first be submitted as a correlated `model_delta` Host response.
They must not be appended directly to SwiftUI or Compose state.

## Memory ownership

- `nola_core_agent_v2_new` and `nola_core_agent_v2_restore` return an opaque thread
  pointer owned by the host.
- The pointer must be released exactly once with `nola_core_agent_v2_free`.
- Every returned JSON `char*` is owned by the host and must be released with
  `nola_core_free_c_string`.
- Calls for one thread are serialized by the Swift actor or Kotlin `Mutex`.
- JNI accepts UTF-8 `ByteArray`; it intentionally does not use Modified UTF-8
  `jstring`, which can corrupt non-BMP characters.

The ABI version is exposed by `nola_core_agent_v2_abi_version` and must match the host
wrapper before a thread is created or restored. The v1 `agent_runtime` ABI has been
removed and must not be recreated as a fallback.

## Permission model

There are two independent permission layers:

1. Agent approval: whether the user permits this model-requested action.
2. Platform authorization: whether the operating system grants the required resource.

Agent approval never substitutes for an OS permission. OS authorization is requested
only after the user approves the agent action.

### Capability matrix

| Capability | CLI | iOS | Android |
| --- | --- | --- | --- |
| File read/write | Allowed roots + agent rule | `fileImporter` + security-scoped bookmark | SAF `content://` URI grant |
| Network | CLI network policy | App capability/ATS + agent policy | Manifest `INTERNET` + agent policy |
| Process/bash | Agent approval | Unsupported | Unsupported |
| Computer control | Agent approval | Unsupported | Unsupported |
| Camera | Unsupported | AVFoundation authorization | Runtime permission + Activity Result |
| Microphone | Unsupported | AVFoundation authorization | Runtime permission + Activity Result |
| Photos | Unsupported | PhotoKit authorization | Android Photo Picker |
| Location | Unsupported | Core Location authorization | Runtime permission |
| Contacts/calendar | Unsupported | Existing iOS data-source permission manager | Runtime permission + ContentResolver host |
| Health | Unsupported | HealthKit authorization | Health Connect contract |
| Notifications | Unsupported | UserNotifications authorization | Runtime permission on API 33+ |
| Background work | Process lifecycle | BGTaskScheduler/background refresh | WorkManager/application lifecycle |
| Clipboard | Agent approval | UIPasteboard host | ClipboardManager host |

Remote Host tools use the same `request_permission` Host effect as native tools. A
remote Host `bash` request does not grant the mobile application permission to start a
local process.

## Scoped resource grants

Raw desktop paths are never accepted as mobile file authorization.

- iOS returns an opaque security-scoped bookmark plus the selected file URL.
- Android returns a persistable `content://` URI grant.
- Rust carries the opaque grant through the permission response and later
  `execute_tool` request without interpreting it.
- The platform executor resolves and validates the grant immediately before access.
- iOS accepts only file URLs backed by a valid scope or the application container.
- Android accepts only `content://` URIs from `ContentResolver`.

Mobile host executors currently cap file payloads at 1 MB and HTTP(S) responses at
2 MB. Unsupported capabilities return a structured tool error rather than succeeding
silently.

## Context management

`ContextPlan` is the single compaction-decision implementation for all clients. Its
inputs are context window, estimated tokens, configured threshold, message count,
reserved output tokens, and the number of recent messages to preserve. Its output
contains:

- compaction decision and reason
- soft trigger and hard prompt limit
- target token count
- recent messages to retain
- messages to archive
- context usage percentage

Compaction is driven by the active Turn's model token window, reserved output budget,
tool schema cost, and configured threshold. It is not triggered by a platform-specific
message-count heuristic. CLI, iOS, and Android submit the same context policy and render
the same Core compaction events.

## Build

### iOS

Prerequisites: Xcode, Rust, and the Apple Rust targets. The Xcode project runs the
builder before Swift compilation and statically links the result without embedding it.

```bash
core/scripts/build_xcframework.sh
```

The generated module is imported from Swift as `NolaCore`. Xcode builds it through
the `NolaCore.xcodeproj` framework target, whose public header and module map wrap
the shared C ABI. The macOS-only `NolaRuntime` target builds `nola-agent-host` and
`nola-service` from the same source revision before the app embeds them.

### Android

Prerequisites: Android SDK, Android NDK, Rust, and `cargo-ndk`. Dependency installation
is explicit; build scripts do not install tools automatically.

```bash
cargo install cargo-ndk
core/scripts/build_android.sh
```

The script discovers `cargo` from `PATH` or standard rustup locations and discovers the
NDK from `ANDROID_NDK_HOME`, `ANDROID_NDK_ROOT`, Android SDK environment variables, or
`android/local.properties`. It uses the cargo-ndk platform flag (`-P`) and produces archives for
`arm64-v8a`, `armeabi-v7a`, `x86`, and `x86_64`.

## Adding a host tool

1. Publish a `HostCapabilityDescriptor` with stable ID, input schema, permission
   domain, parallelism, cancellation, output, and replay policy.
2. Add platform permission declarations only where the capability exists.
3. Implement the Host executor using scoped platform APIs; never pass raw mobile paths.
4. Return `tool_completed` or `failed` through the matching Host response envelope.
5. Add cross-platform contract coverage for Submission/Event JSON and denial.
6. Verify cancellation while waiting for permission and while executing the tool.
7. Verify checkpoint recovery does not repeat unsafe effects.

## Verification checklist

Run only on hosts with the corresponding toolchains installed:

```bash
cd core && cargo test --quiet
cd cli && cargo check
cd android && ./gradlew test
xcrun simctl list devices available
xcodebuild test -project Nola.xcodeproj -scheme Nola \
  -destination 'platform=iOS Simulator,name=<available simulator name>'
xcodebuild test -project Nola.xcodeproj -scheme NolaMac \
  -destination 'platform=macOS'
cd android && ./gradlew connectedDebugAndroidTest
```

`connectedDebugAndroidTest` requires a booted emulator or attached device. JVM tests and
`assembleDebugAndroidTest` prove compilation but do not replace device-level Compose and JNI validation.

Additionally verify:

- ABI version mismatch fails before runtime creation.
- Chinese and emoji survive Swift and JNI round trips.
- Permission requests are serialized one at a time.
- Denied permissions produce tool results and do not stall a turn.
- Cancelling a turn clears pending permission UI.
- iOS bookmarks and Android URI grants cannot be replaced by arbitrary paths.
- ContextPlan returns equivalent results for identical input on all clients.
- Generated static artifacts are reproducible and excluded from source control.
Mobile tool executors receive typed calls from the v2 coordinator. The removed
`host_call_requested` UI-event shape must not be accepted by iOS or Android;
Core Trace is presentation output, never an execution input.
