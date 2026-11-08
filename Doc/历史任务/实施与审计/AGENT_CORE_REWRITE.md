> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Agent Core Rewrite

## Decision

Agent Core is the product's only conversation state machine. Platform clients
must not maintain a second Agent loop or infer state from provider callbacks.
They submit inputs, execute Host effects, and render Core events.

The rewrite borrows boundaries from Codex CLI without copying its Tokio and
process-specific implementation. Nola must also run through a synchronous
Rust FFI boundary on mobile, so the Core remains a deterministic reducer.

## Runtime hierarchy

```text
Host process
  └─ Thread registry                 Host-owned lifecycle and persistence
      └─ AgentCore                   The only public mutable Core boundary
          └─ private AgentRuntime    Deterministic reducer implementation
              └─ AgentThreadState
                  ├─ conversation    Transcript, model context, context window, Parts
                  ├─ execution       Active Turn, input queue, permission rules
                  ├─ host            Capability snapshot and pending effects
                  └─ journal         Trace, cursors, revision, idempotency
```

`AgentCore` is analogous to a Codex thread: it is the only public mutable
conversation entry point. `AgentRuntime` is now private reducer machinery. Its
command, runtime-event, scheduling, and projection layers may change without
becoming a platform protocol.

## Public protocol

The stable v2 protocol remains:

```text
AgentSubmission -> AgentCore -> AgentTransition
                               ├─ ordered AgentEvent envelopes
                               ├─ HostRequest effects, including journal.checkpoint
                               ├─ Core Trace
                               └─ snapshot
```

The public state types are `AgentCoreConfig` and `AgentCoreSnapshot`. The
private reducer has no Host-facing configuration or snapshot aliases.

Only `AgentSubmission` may mutate a thread. Provider deltas, tool completions,
permission decisions, compaction results, Host reconnection, cancellation, and
steering all return as submissions. A platform must never call an internal
reducer command.

`AgentSubmission::UserInput` has two deliberately separate channels:

- `items` are appended to the immutable transcript;
- `turn_context_items/components` are prepared by the Core Context Engine and
  frozen into the active Turn only.

Core applies authorization, freshness, platform ranking, budget, and digest
rules to typed context. Hosts must not turn context into fake system messages.

## Internal layers

The former monolithic reducer is split by responsibility:

| Layer | Responsibility |
| --- | --- |
| `kernel` | expose the only public Core boundary and hide reducer internals |
| `bootstrap` | validate configuration/history and restore checkpoints |
| `submission` | map stable submissions to internal commands |
| `turn_lifecycle` | start, steer, complete, fail, and cancel Turns |
| `model_planner` | build model effects from immutable Turn context |
| `tool_driver` | reduce permissions/tool lifecycles and plan tool effects |
| `compaction_driver` | plan and resume proactive context compaction |
| `host_response` | validate and reduce normalized Host responses |
| `effect` | represent Core-planned I/O without transport identity |
| `effect_dispatch` | bind, correlate, deliver, replay, and cancel Host effects |
| `projection` | turn events into stable UI Parts and conversation state |

The persistent `HostEffectJournal` stores effect ownership and accepted
response sequence. `effect_dispatch` creates a delivery attempt from that
journal; it does not own Turn policy or effect meaning. Model, tool,
compaction, and Turn-lifecycle reducers plan `AgentEffect` values only; none of
them may construct transport envelopes directly.

The split is behavioral, not merely directory organization: public code cannot
reach these layers directly.

## Ownership rules

Core owns:

- append-only transcript, compactable model context, and stable Item/Part/Call IDs;
- current Turn and next-Turn mailbox;
- immutable per-Turn `TurnContext` captured from session configuration and Host catalog;
- permission evaluation and session grants;
- tool lifecycle and safe scheduling barriers;
- context accounting and compaction decisions;
- ordered events, UI projection, checkpoints, and recovery;
- duplicate submission, duplicate Host response, Host lease validation, and doom-loop protection.

Every pending Host effect uses a Core-owned lifecycle:

```text
dispatched -> accepted -> streaming -> terminal
     \---------------------------> cancelling -> acknowledged
```

The effect stores both logical `effect_id` and delivery-attempt `request_id`.
Responses additionally bind Host lease, Session, Turn, Catalog revision,
availability epoch, and monotonic sequence. Reconnection never rewrites pending
ownership implicitly; explicit replay preserves effect identity and creates a
new request identity.

Host owns:

- model transport and stream decoding;
- filesystem, shell, browser, device, data-source, MCP, and skill I/O;
- OS permission prompts and sandbox enforcement;
- durable checkpoint storage and full tool-output blobs;
- process lifecycle, networking, logging, and background execution.

UI owns:

- rendering Parts and events;
- collecting user input and approval decisions;
- presentation-only animation and navigation state.

UI must not reconstruct tool state, merge provider streams, calculate context
usage, or decide whether a Turn is complete.

## Codex mechanisms adopted

- A thread is the public bidirectional conversation conduit.
- Session state outlives a Turn; Turn state is discarded at its terminal event.
- Turn configuration is captured rather than inferred repeatedly by clients.
- Submitted operations are the only mutation path.
- Steering and next-turn input are separate delivery semantics.
- Session services are separated from durable conversational state.
- Tool calls are tasks with explicit start, progress, terminal, and cancellation
  events, rather than opaque text embedded in an assistant answer.
- Cancellation is an ordinary correlated Host request; there is no direct
  driver cancellation side channel.
- Persistence and compaction are lifecycle boundaries, not UI conveniences.

Active-Turn automatic compaction resumes that Turn after the compaction Host
response. Idle manual compaction updates model context and checkpoint without
creating a Turn or issuing a normal model sample. Core emits the
`journal.checkpoint` effect; Host selects the file, database, or remote store
and guarantees atomic persistence.

## Deliberate differences from Codex

- The Core performs no asynchronous I/O and has no Tokio dependency.
- Platform capabilities are negotiated through a serializable Host catalog.
- Every Host effect is replay-aware because mobile processes can be suspended.
- UI Parts are first-class Core state so SwiftUI, Compose, CLI, and Web render
  the same lifecycle without parsing logs.

## v1 removal invariant

The old `agent_runtime` ABI has been removed. Active clients must use:

1. `AgentCore` through the v2 FFI;
2. `AgentSubmission` for every mutation;
3. `AgentEvent`/Part projection for display;
4. `HostRequest`/`HostResponse` for every external action;
5. Core checkpoints for process recovery.

No compatibility implementation or platform-private fallback may be reintroduced.

## Current implementation state

- CLI and Web use Application-owned Core reductions plus a stateless
  `CommittedRuntimeDriver`; no production Host owns an in-memory reducer loop;
- the CLI presenter is a projection-only adapter and does not rebuild or persist a
  provider-specific shadow transcript from streamed events;
- durable CLI session resume and append logic does not retain a mutable message cache; it reads
  the current Store-owned projection through Application Service;
- iOS and Android v2 coordinators submit typed Host responses through FFI;
- macOS local and remote modes use Agent Host as their only interaction path;
- conversation creation is a strict `create_conversation` command that cannot
  carry transcript messages; current messages are committed separately through
  Run transitions or the monotonic append contract;
- the removed `import_conversation` command and its message-bearing creation
  shape are rejected by protocol tests rather than decoded as aliases;
- Web no longer reads or writes the pre-release JSON conversation state and no
  longer replaces complete message arrays; branch appends to a new conversation,
  revert hides explicit message identities, and compaction leaves transcript
  content immutable;
- Web ordinary Turns and explicit compaction are Core-owned;
- Web binds a Core checkpoint even when CLI transcript storage is disabled;
- external adapters must emit current typed Host envelopes; removed event aliases are rejected.

## Completion and verification contract

The rewrite is complete only when every row below has both implementation and
verification evidence. A source search alone is not sufficient.

| Requirement | Authoritative implementation | Required verification |
| --- | --- | --- |
| one public Core entry point | `agent/kernel.rs`; private `AgentRuntime` | Rust visibility/contract test and committed Runtime build |
| Thread owns many Turns | `agent/session.rs`, `agent/turn.rs` | restore across multiple Turns and one-active-Turn test |
| transcript/context separation | `AgentSubmission::UserInput`, conversation state | round-trip checkpoint test proving context components never enter transcript |
| immutable Turn context | `AgentTurnContext` | catalog/config change during a Turn does not alter its request |
| stable Item/Part lifecycle | `parts`, `projection` | streamed text, reasoning, tool input, terminal Part contract tests |
| catalog-authoritative model tools | `host`, `runtime/model_planner` | visible available tool is projected; deferred/internal/hidden/unavailable/non-tool capabilities are excluded |
| effect-only Host I/O | `effect`, `effect_dispatch`, Host envelopes | fake Host driver E2E with model, permission, tool, checkpoint, cancel |
| correlated recovery | `HostEffectJournal` | stale lease/catalog, duplicate sequence, replay and cancellation tests |
| proactive/manual compaction | `compaction_driver`, context window | active-Turn resume and idle-Thread no-sampling tests |
| canonical Trace UI input | Core Trace plus platform presenters | iOS, Android, macOS, CLI, Web fixture parity |
| typed platform context | Core ContextComponent and Host payloads | local/remote macOS plus Web/mobile contract tests |

Until these gates run successfully, documentation must describe the migration
as implemented but unverified, not complete.

### Current verification evidence

- `cargo test --workspace --quiet`: shared Store, Application service, Core,
  CLI/Web runtime, recovery, protocol and PTY suites pass with the current
  schema.
- `./gradlew test`: Android JVM suites pass against the current native command
  contract.
- iOS and macOS `build-for-testing` pass after rebuilding the Rust bindings.
- `scripts/run_real_mac_xctests.sh`: an isolated Swift -> Agent Host -> Core ->
  real LLM run passes and verifies durable cross-conversation memory recall.
- `NOLA_REAL_WEB_ONLY=1 scripts/test_real_cli_web.sh`: token login -> production
  Web Host -> `nola-service` -> Core -> real LLM passes with ordered durable
  assistant messages, data sources, permissions, memory, Skill execution,
  compaction, and continuation.

This evidence covers the implemented slice. It does not by itself satisfy every
cross-platform row in the completion table or every architecture gate in
`ARCHITECTURE.md`.
