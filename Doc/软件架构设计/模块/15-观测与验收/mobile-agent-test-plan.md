> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Mobile Agent Verification Plan

## Release claim

Nola may claim mobile Agent support only when the same release proves all three
execution modes, platform permission boundaries, host trust, encrypted transport,
Trace presentation, and lifecycle recovery. Passing one platform, one happy path,
or a final-string-only assertion is insufficient.

## Required mode matrix

| Scenario | iOS | Android | CLI/Web evidence |
| --- | --- | --- | --- |
| Local model, no tools | Required | Required | N/A |
| Local model with device read tool | Required | Required | N/A |
| Local model with device write tool | Required | Required | N/A |
| Host chat with host tools disabled | Required | Required | Required |
| Host tool allow once | Required | Required | Required |
| Host tool deny | Required | Required | Required |
| Hybrid with bridge disabled | Required | Required | No device schema received |
| Hybrid with one selected capability | Required | Required | Only selected schemas received |
| Hybrid device result success | Required | Required | Result consumed once |
| Hybrid device result failure/deny | Required | Required | Model receives bounded error |

## Security invariants

1. A model-supplied `_nola_approved_once` or `_nola_denied_once` is ignored.
2. Permission decisions bind host, client, session, request, tool, arguments digest,
   nonce, protocol version, and expiry.
3. A challenge, decision, or device result cannot be consumed twice or by another
   mobile client using the same conversation ID.
4. Changing one tool argument invalidates the decision and result.
5. Missing or changed host identity blocks the approval control.
6. Unselected device capabilities are absent from the host model schema.
7. Mobile HMAC binds method, path, timestamp, nonce, and exact body digest.
8. Replayed auth nonces, stale timestamps, changed bodies, changed paths, incorrect
   tags, and incorrect tokens are rejected before Agent execution.
9. Mobile Agent SSE, settings, attachment text, and device results are encrypted.
10. Web Host tokens never persist in model JSON, conversation metadata,
    UserDefaults, or Android DataStore.
11. The CLI Host access token and Host ID survive restart, and their host-side
    credential files use private permissions (`0600` on Unix).
12. An unauthenticated non-loopback peer can reach only health discovery, Web
    login assets/session establishment, and independently authenticated
    `/api/mobile/*` routes. Other pages redirect to login and APIs return 401.
13. A valid Host token creates a bounded, expiring Web session; the server stores
    only session/CSRF digests, the session cookie is HttpOnly + SameSite, and the
    Host token never enters browser storage.
14. Remote state-changing Web requests reject missing/mismatched CSRF tokens and
    cross-site origins. Logout revokes the server-side session.

## Platform permission cases

| Capability | Initial | Denied | Granted | Revoked while app runs |
| --- | --- | --- | --- | --- |
| Selected file | Picker cancel | No result egress | Scoped read/write | Next call reprompts |
| Clipboard | Foreground only | No access | One action | Background call blocked |
| Location | Not determined | Bounded error | Approximate accepted | Next call blocked |
| Calendar read | Not determined | No query | Bounded time range | Next call blocked |
| Calendar create | Read only | No write | One event | Next write blocked |

iOS tests must cover limited/restricted authorization where the framework exposes
it. Android tests must cover partial runtime grants, Activity recreation while a
launcher is open, and content URI grants that disappear after process death.

## Lifecycle and recovery

- Background the mobile app while a permission card is visible.
- Rotate/recreate the Android Activity during runtime permission and SAF flows.
- Kill and relaunch after a host challenge; expired cards must not reappear.
- Disconnect before the host receives a decision or device result.
- Disconnect after the host consumes a result but before the client receives SSE.
- Restart the CLI host; host ID stays stable and in-flight challenges expire.
- Restart the CLI host; the access token remains valid without an environment
  variable and existing paired clients reconnect without reconfiguration.
- Change host ID or certificate; the client requires explicit re-pairing.
- Send multiple tool calls in one model response; no call after the first suspended
  request may create a side effect.
- Close the conversation during a live long task, reopen it, and recover the
  assistant bubble, reasoning, tools, parameters, resources, bounded results, and
  Markdown from the Trace Journal without duplication or replay animation.
- Deliver duplicate and out-of-order Trace events. Duplicates are idempotent and a
  sequence gap buffers later events until Journal replay supplies the missing range.

## Shared Trace and presentation contract

All mobile execution modes normalize into the same versioned `AgentTraceEvent`
contract used by Core, macOS, Web, and CLI. Platform adapters may map events to
localized display models, but they must not define a parallel event vocabulary or
infer lifecycle state from user-visible strings.

The canonical long-task fixture in `core/tests/fixtures/` is a required compatibility
gate. iOS and Android tests must replay that fixture through their production
adapter, reducer, presentation model, and mounted UI and assert:

1. The assistant bubble is created immediately and keeps one stable message ID.
2. Pending, reasoning, long-task phases, tools, and final Markdown remain inside
   that bubble.
3. Running tools expose sanitized parameters and affected resources when expanded.
4. Completion updates the same tool row and preserves the real truncated result or
   error instead of replacing it with a generic status.
5. Stable committed Markdown blocks keep their identity while only the mutable tail
   changes.
6. Duplicate events are idempotent and cross-conversation/run/tool identities are
   isolated.
7. An accepted semantic no-op advances the sequence and Journal cursor but publishes
   no Compose/SwiftUI presentation update.
8. A sequence gap does not leak later state; Journal replay fills the gap before
   projection continues.
9. Reopen and reconnect replay reconstructs the same completed projection without
   duplicate text, duplicate tools, or repeated reveal animation.

## Automated suites

```text
cd core && cargo test --quiet
cd cli && cargo test
cd android && JAVA_HOME="$(/usr/libexec/java_home -v 17)" ./gradlew test
cd android && JAVA_HOME="$(/usr/libexec/java_home -v 17)" \
  ./gradlew :app:compileDebugAndroidTestKotlin
cd android && JAVA_HOME="$(/usr/libexec/java_home -v 17)" \
  ./gradlew connectedDebugAndroidTest
xcrun simctl list devices available
xcodebuild test -project Nola.xcodeproj -scheme Nola \
  -destination 'platform=iOS Simulator,name=<available simulator name>'
xcodebuild test -project Nola.xcodeproj -scheme NolaMac \
  -destination 'platform=macOS'
python -m pytest -q \
  web/tests/test_agent_host_full_e2e.py \
  web/tests/test_chat_stream_runtime.py
```

Targeted transport integration tests should start CLI Web with temporary state,
configured pairing credentials, and a loopback port. They must use real encrypted
request bytes and parse real SSE frames rather than calling handler helpers only.
Use the evidence levels and streaming gates in `Doc/软件架构设计/模块/15-观测与验收/TESTING.md`. Compiling an
instrumentation target is not evidence that it ran on a device.

Trace tests must consume the canonical fixture rather than hand-building a
platform-specific approximation. UI tests must keep one mounted hosting instance
through pending, reasoning, tool running, assistant deltas, tool completion,
terminal state, Journal recovery, and sequence-gap repair. Reducer-only and
final-string assertions are supporting evidence, not E2E evidence.

## UI acceptance

- The header always says `On Device`, `Cloud`, `Host`, or `Host + Device`.
- Enabling the bridge without selecting a capability still displays `Host`.
- Permission cards show execution location, data boundary, one-operation lifetime,
  redacted arguments, and an action-specific primary button.
- A long task immediately shows one assistant bubble with a stable localized
  processing state; reasoning, current phase, and tools update inside it.
- Expanding a running tool shows sanitized parameters and resources. Expanding the
  completed row shows the real bounded result or error and any safe artifact links.
- Tool or phase events with unchanged visible meaning do not flash, resize, or
  republish the bubble.
- Incremental Markdown is structured before completion, preserves committed nodes,
  and does not switch from raw text to a different completed renderer.
- Device bridge cards state that execution is local and the result returns to the
  current paired host session.
- Host fingerprint and forget-pairing action are visible in settings.
- Dynamic Type/font scaling, VoiceOver/TalkBack, high contrast, reduced motion,
  RTL layout, keyboard navigation, and narrow-screen layouts remain usable.

## Exit gates

- All suites pass on a clean checkout.
- The canonical `AgentTraceEvent` fixture passes on Core, iOS, Android, macOS, Web,
  and CLI adapters with the same ordered projection.
- Semantic no-op tests prove the cursor advances with zero UI publication; sequence
  gap tests prove later state remains buffered until Journal recovery.
- Reopen/reconnect E2E proves Trace Journal recovery produces one assistant bubble,
  one copy of each tool, and one copy of the answer.
- No test is skipped because a permission or crypto path is difficult to simulate.
- At least one real-device smoke test passes on a current iPhone and a current
  Android device.
- Security logs contain identifiers and digests only, never tokens, raw clipboard,
  calendar contents, file contents, or decrypted transport payloads.
- Documentation and system permission text match actual data movement.
- Apple export-compliance answers and Google Play Data Safety declarations are
  reviewed against the custom AES-GCM/HMAC transport and optional host egress;
  existing manifest flags are not accepted without that review.
