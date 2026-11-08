> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# macOS Application startup recovery

Status: implementation in progress; compilation and runtime acceptance pending.

This work implements the Architecture V2 requirement that an unavailable or
incompatible canonical Store is an explicit recovery condition, not an empty
conversation list and not an implicit database reset.

## Ownership and failure contract

- Constructing the shared MacApplicationPort performs no service connection IO.
- The explicit throwing initializer remains available for callers that require
  an immediately connected port.
- Business window content is constructed only after Application Service startup
  succeeds. A failed startup shows the error and canonical database path, with
  an explicit retry action.
- Main chat, separate conversation windows, data, capability, automation,
  activity, model, extension, workspace review and other business windows use
  the same startup gate.
- Diagnostics and Settings remain outside the gate so recovery controls are not
  deliberately hidden behind the unavailable service. Their behavior when the
  service is unavailable still requires runtime acceptance.
- The gate runs connection preparation off the main actor. It does not retry
  automatically, create a replacement Store, migrate an incompatible schema,
  or submit business commands.
- Each port retains its Application Service supervisor reference for its
  lifetime. Successful startup does not create a per-window service process.
- Service stderr is drained asynchronously for the owned process lifetime and
  retains at most the last 16 KiB. Startup failure does not synchronously wait
  for pipe EOF. A final stderr callback can race the error snapshot; the
  original launch error is used if the buffer is empty.
- Failed launches are removed from the supervisor and stopped. Other application
  instances and user databases are not reset or terminated by this recovery.

## Required acceptance evidence

None of the following scenarios has been claimed as verified by these edits.

1. Compile the Mac app and its test target.
2. Launch against an isolated incompatible database: show the startup error,
   preserve the database bytes, and do not render an empty chat list.
3. Launch with an unavailable service executable: show a retryable error.
4. Repair an isolated external startup condition, then retry in the same window:
   load the original canonical conversations without restarting the app.
5. Open multiple business windows: reuse the healthy service and preserve
   independent window state.
6. Use a fixture service that fills stderr and does not become healthy: enforce
   bounded diagnostic memory and finish failure handling without waiting for
   an inherited pipe to close.
7. Repeat failed startup and close windows: leave no additional owned service
   processes or supervisor references.
8. Inspect loading, error, retry, diagnostics and settings UI at different window
   sizes and in each supported locale, including selectable long error text.
9. Kill an already healthy service: validate command failure and recovery.
   The current startup gate is not a continuous connection-status subscription.

## Remaining architecture work

The original service initialization error still needs a concrete diagnosis.
This change removes fatal startup handling; it does not establish that the
underlying database or service executable is healthy. Full V2 acceptance also
requires the separate projection, transaction, runtime ownership, cancellation,
resource-bound and cross-platform requirements in ARCHITECTURE_V2.md.

## Native Messaging role isolation

The 2026-09-06 08:23:33 crash report identifies Google Chrome as the parent
process and MacApplicationPort.shared initialization as the fatal stack. The
installed native-host launcher invokes the Mac executable with
`--nola-chrome-native-host`. Previously that branch ran after constructing the
interactive application's dependencies and scheduling startup integrations.

Native Messaging dispatch now precedes those dependencies in NolaMacApp.init.
It enters MacChromeNativeHostProcess directly, without starting the interactive
chat runtime or scheduling Chrome bridge warm-up from the bridge process itself.
This is process-role dispatch, not user-prompt routing.

Read-only inspection of the current product database found user_version 11,
schema_epoch 2 and application_id 1313819713, matching the current source
constants. This does not prove what executable or database state the earlier
crashing process observed, and does not justify a database reset.

Additional acceptance required: launch the packaged executable in Native
Messaging mode with a controlled bridge configuration and framed stdin/stdout;
confirm no interactive window, Application Service launch or recursive bridge
warm-up, then exercise extension handshake and disconnect. This has not yet
been executed for the change.

## Acceptance evidence: 2026-09-06

The implementation status above is superseded only for the scenarios listed
here; UI screenshot acceptance and full Chrome extension operation remain open.

- Mac build-for-testing passed: `/tmp/nola-mac-startup-recovery-build-20260906.log`.
- MacCanonicalStoreEndToEndTests passed 11 tests, including deferred client
  construction without filesystem/process side effects, repeated failed startup
  preserving fixture bytes and releasing the owned process, and retry on the
  same client after explicit fixture repair. Result bundle:
  `/tmp/nola-mac-startup-recovery-tests-20260906.xcresult`.
- A process-level fixture launched the built Nola executable with
  `--nola-chrome-native-host`, an explicit temporary native configuration,
  a regular file where the App support directory would be, and a service
  executable that records any invocation and fails. A little-endian framed
  `hello` received exactly `host_ready`; stdin EOF produced exit status 0 and
  no additional stdout. The service invocation marker was absent and the
  support fixture was unchanged. The run had a 15-second deadline and cleanup
  of its own child. Capabilities were empty, so this proves entrypoint isolation
  and framing, not real extension commands or HTTP polling.
- MacChromeMessagingEndToEndTests passed 3 Swift Testing cases: manifest isolation,
  disconnected-extension recovery/debounce, and broker command/result roundtrip.
  Result bundle: `/tmp/nola-chrome-startup-20260906.xcresult`.

Still required: visual loading/error/retry acceptance, real extension operations,
verbose or inherited stderr fixtures, and recovery after a healthy service dies.
