> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Mobile Agent Architecture

> 2026-10-15 整体设计复核前的历史快照，仅保留当时叙述与证据。旧 ABI、Trace 权威、schema 数字、传输别名和其他旧规则均不作为现行规范。当前入口见[软件架构总纲](../../软件架构设计/总体设计/SOFTWARE_DESIGN.md)。

## Purpose

Nola has one agent protocol and three execution surfaces. The CLI owns command-line,
Web, and host-server execution. iOS and Android reuse the Rust state machine while
keeping operating-system access inside platform adapters.

This document is the authority for mode selection, tool ownership, permission
responsibility, data movement, and user-facing execution state.
Release verification is defined in `Doc/软件架构设计/模块/15-观测与验收/mobile-agent-test-plan.md`.

## Execution modes

| Mode | Agent loop owner | Model owner | Tool owner | Device data leaves device |
| --- | --- | --- | --- | --- |
| `device_local` | Rust core on device | Mobile provider | Mobile app | No |
| `host_remote` | Paired host | Host provider | Paired host | Only user prompt and attachments |
| `hybrid` | Paired host | Host provider | Host plus approved device bridge | Only after egress consent |

An execution mode is frozen for a turn. A disconnect, trust change, or model-source
change cancels the turn instead of silently moving work to another surface.

## Non-negotiable invariants

1. Device tools and host tools have distinct stable IDs and execution targets.
2. A mobile executor never executes a `host.*` or `host_*` request.
3. Host mode never obtains an iOS or Android permission as a side effect.
4. OS authorization does not imply agent-action consent.
5. Reading device data does not imply consent to send it to a host or cloud model.
6. Grants bind principal, target, tool, scope, arguments digest, lifetime, and expiry.
7. Host identity or certificate changes invalidate all host grants.
8. Only tools with a working executor are advertised to a model.
9. Camera, microphone, clipboard read, and computer control require foreground UI.
10. Logs and conversation metadata never persist raw secrets or unrestricted tool output.

## Shared model contract

The CLI model response is canonical:

```text
text delta
reasoning delta
completed(content, reasoning, tool_calls, finish_reason, usage)
failed(error)
```

iOS and Android expose the same contract through `AgentModelClient`. Providers may
implement native function calling. Text-only providers remain chat providers and do
not receive tool schemas until they implement a reviewed structured-call adapter.

## Tool identity

Tools have an internal ID and a provider-safe function name:

```text
internal ID:   device.files.read
function name: device_read_file
target:        device
```

The internal ID is used for policy and UI. The function name is used in provider
payloads. A model cannot select a different target through arguments.

## Responsibility matrix

| Concern | Rust core | Mobile platform | Host server |
| --- | --- | --- | --- |
| Turn state and iteration | Owns local mode | Drives events | Owns host/hybrid mode |
| Tool target validation | Owns | Enforces again | Enforces again |
| Tool schema filtering | Validates | Builds device catalog | Builds host catalog |
| OS permission | Describes requirement | Owns | Never owns |
| Host policy | Carries request metadata | Displays decision | Owns enforcement |
| Tool execution | Requests | Device tools only | Host tools only |
| Data egress | Carries destination | Owns consent/redaction | Receives approved data |
| Persistence | Defines snapshot | Secure local storage | Host session store |

## Permission plan

Every tool request resolves these gates in order:

```text
availability
platform authorization
agent action consent
data egress consent
execution
result redaction
```

`allow_once` binds the host, mobile client, session, call, tool, canonical arguments
digest, nonce, protocol version, and expiry. A denial is also consumed exactly once
and returned to the model as a tool result. `allow_session` binds a specific scope.
Persistent grants are reserved for low-risk configuration and must be visible and
revocable in Settings.

## Platform boundaries

### iOS

Use document pickers and security-scoped URLs for files, EventKit for calendar and
reminders, Contacts for bounded search, HealthKit for type-scoped summaries,
PhotoPicker for selected media, and foreground-only capture/location/clipboard.
Background work is a scheduled Nola operation, not arbitrary process execution.

### Android

Use SAF content URIs for files, separate calendar read/write permissions, bounded
Contacts queries, type-scoped Health Connect access, Photo Picker for selected media,
and WorkManager for declared Nola jobs. Android has no shared Reminders permission;
Nola tasks must be labeled as app-local tasks.

## Host trust

The CLI creates a persistent random public host ID beside its Web state. Mobile
clients learn it only from the authenticated settings endpoint and pin it to the
normalized host URL. Each mobile installation also creates a persistent client ID.

Every agent request carries `expected_host_id` and `client_id`. The host rejects an
identity mismatch before starting the model. Permission challenges bind host ID,
client ID, session ID, call ID, canonical tool ID, arguments digest, a server nonce,
five-minute expiry, and protocol version. Challenges are atomically consumed before
validation to prevent concurrent replay. The transport token and mobile AES-GCM
envelope authenticate the exchange; the decision is not described as a standalone
digital signature.

The client independently checks host ID, client ID, session ID, nonce, digest,
expiry, and protocol version before presenting an approval control. Re-pairing,
certificate changes, host identity changes, or protocol changes invalidate grants
and in-flight calls.

The paired host fingerprint is visible in mobile settings and can be forgotten
without deleting conversations or OS permissions. Forgetting clears only the URL-to-
host-ID pin; the next authenticated settings exchange establishes a new trust-on-
first-use identity.
Hybrid capability selections are principal-bound: forgetting a pairing or changing
the host URL disables the bridge and clears every selected device capability.

### Mobile transport

HTTPS is the preferred transport and may be certificate-pinned. Password login is
never allowed over non-loopback HTTP. An HTTP LAN host requires a token transferred
through a separate trusted channel.

The CLI signs in a new mobile client only when `NOLA_MOBILE_PAIRING_SECRET` is set
to at least 16 characters. Missing configuration disables token issuance; arbitrary
non-empty passwords are never accepted. Failed pairing attempts are rate-limited by
source address, and the host caps the number of live mobile tokens.

Because the built-in CLI Web listener is HTTP, `nola web` creates a random,
persistent Host access token on first start and stores it beside the stable Host
identity under `~/.nola/cli/web/` (`0600` on Unix). Startup output shows the token
so the host owner can transfer it to a client through a separate trusted channel.
The token and Host ID survive process restarts; neither depends on a shell
environment variable. Modern clients do not send the token itself: it derives the
per-request HMAC and AES-GCM keys. Password pairing is reserved for loopback or an
HTTPS reverse proxy.

For non-loopback peers, unauthenticated access is limited to `/api/health`, the
Web login surface and static login assets, plus the independently authenticated
`/api/mobile/*` surface. A remote browser exchanges the persistent Host token for
a bounded in-memory Web session. Only a digest of the session identifier is kept
by the Host; the browser receives an expiring HttpOnly, SameSite cookie and a
session-bound CSRF cookie. Every state-changing remote Web request must be
same-origin and present the matching CSRF header. Logout revokes the session and
clears both cookies. The Host token is never written to browser local storage.

An authenticated Web session may use the ordinary Web, file, execution, and
native bridge endpoints. This cookie protocol does not encrypt HTTP traffic, so
LAN browser access is a local-trust feature. Use an HTTPS reverse proxy outside a
trusted private network.
The CLI accepts forwarded HTTPS only when `NOLA_TRUST_HTTPS_PROXY=1`; deployments
using this option should bind the CLI listener to loopback so clients cannot spoof
proxy headers directly.

Mobile host tokens are device secrets. iOS stores them in Keychain with a
ThisDeviceOnly accessibility class; Android encrypts them with an Android Keystore
AES-GCM key. Model records, conversation snapshots, UserDefaults, and DataStore may
store host URLs and public fingerprints but never the token. Existing plaintext
mobile records migrate on first load and are rewritten without the secret.

Protected mobile requests do not send that token as a Bearer value. They send an
HMAC-SHA256 proof over protocol version, HTTP method, request path, Unix timestamp,
a random nonce, and the SHA-256 digest of the exact HTTP body. The host accepts a
60-second clock window and consumes every
nonce once. Request JSON and sensitive responses use AES-256-GCM with a random
96-bit nonce, 128-bit tag, fixed protocol AAD, and a SHA-256-derived token key.
Agent SSE JSON is encrypted event by event; the stream terminator carries no data.

Bearer authentication remains a temporary compatibility path for older clients and
tests. Current clients use the HMAC proof and encrypted payload protocol.

## Advertised device tools

Only implemented tools appear in the mobile model catalog:

| Tool | OS gate | Agent gate | Result boundary |
| --- | --- | --- | --- |
| User-selected file read/write | Document picker / SAF grant | Per operation | Device only |
| Clipboard read/write | Foreground app | Per operation | Device only |
| Current/recent location | When-in-use location | Per operation | Device only |
| Calendar range query | Calendar read | Per operation | Device only |
| Calendar event creation | Calendar write | Per operation | Device only |

Camera, microphone, and photo-library access remain user-initiated acquisition
flows. They are not model-advertised tools until capture UI, preview, cancellation,
and egress consent are implemented on both platforms.

## User experience

The chat header always communicates the active execution surface:

| Surface | Label | Symbol | Meaning |
| --- | --- | --- | --- |
| Device | On Device | `iphone` | Model and available tools stay on this device |
| Host | Host | `desktopcomputer` | The paired host runs the agent and tools |
| Hybrid | Host + Device | `arrow.triangle.2.circlepath` | Host may request approved device capabilities |
| Cloud chat | Cloud | `cloud` | No local agent-tool claim is implied |

Permission sheets show who is asking, what will happen, the exact scope, where the
result goes, and the available lifetime. Device and host requests use the same
information hierarchy but distinct execution-location labels. Destructive actions
must name the operation rather than imply permanent access.

## Hybrid bridge contract

`hybrid` is reserved for a paired host agent requesting an explicitly advertised
device tool. It is not equivalent to host mode plus implicit access to mobile data.
The master bridge switch does not advertise every tool. Files, clipboard, location,
and calendar are separate, default-off capability selections. An unselected
capability is absent from the host model schema and cannot produce a permission
request.
The bridge must use this suspend/resume sequence:

```text
host model requests device.*
host validates advertised mobile capability and emits device_tool_request
mobile validates host/client/session/call/digest/version/expiry
mobile obtains action consent, OS authorization, and optional egress consent
mobile executes through the device executor
mobile returns one bounded device_tool_result
host consumes the result once and resumes the same agent session
```

Until this full sequence is active, connected mobile chat remains `host_remote` and
the host cannot invoke device tools.

## Delivery sequence

1. Shared execution modes, target validation, tool catalogs, and model contract.
2. Local Runtime-driven model loop with file and clipboard tools.
3. Persistent host/client identity, authenticated one-shot permission decisions,
   and disconnect handling.
4. Hybrid device bridge with explicit data-egress consent.
5. Calendar, reminders/Nola tasks, contacts, health, notification, and media tools.
6. Cross-language golden contracts, lifecycle tests, security tests, and UI tests.
