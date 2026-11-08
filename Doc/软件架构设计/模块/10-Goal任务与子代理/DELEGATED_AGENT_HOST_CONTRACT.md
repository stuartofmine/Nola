> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Delegated Agent Host Contract

Status: active migration contract. The legacy external-executor persistence
command and DTO have been removed from Host Protocol, Application, and CLI
Session boundaries. Remaining clients must converge on this contract without
reintroducing adapter-authored terminal projections.

## 与范围化子任务的关系（2026-11-02）

本文的 delegated executor 是物理 Host 协议，不等同于 `spawn_agent` 创建的领域 Task。统一身份与所有权见[会话、Run、Goal 与子任务边界](AGENT_THREAD_ARCHITECTURE.md)。Task 有父 owner、独立 child Conversation/Run、冻结 profile 和终态回执；外部 executor 的 backend session 只负责执行效果，不能另写 Task/Goal 完成。

Android child executor 复用父模型 route；iOS 已接同一授权连接且生成策略相同的独立 HTTP 子模型客户端，其他独立模型配置在准备持久 Task 前明确拒绝，不静默降级。CLI/macOS Host 可按既有冻结 route 准备独立模型。Task max_turns 的有效值准入冻结，原生与 CLI 恢复都使用该值；上下文引用与任何 executor session 都不是权限授予。

## 1. Ownership

There is one authoritative mutation path:

```text
User submission
  -> Agent Core command
  -> checkpointed Turn
  -> versioned Host request
  -> correlated Host events
  -> Agent Core reducer
  -> canonical ConversationProjection
  -> Store transaction
  -> client projection
```

The boundaries are strict:

- Agent Core owns Turn state, tool Parts, reasoning/text Parts, permission
  policy, cancellation state, terminal state, and projection revisions.
- A delegated Agent Host adapter owns process discovery, process lifetime,
  protocol parsing, stdout/stderr transport, and executor-specific resume IDs.
- Store owns atomic persistence only. It does not interpret executor events.
- Clients render the canonical projection only. They do not merge raw deltas,
  infer terminal messages, or repair ordering.

A Host adapter must never construct Message, Part, Run, transcript, or Store
rows. It emits execution facts only.

## 2. Frozen request identity

`DelegatedAgentRequestV1` is created by Core from the Turn's frozen session
configuration. It carries:

- `contract_version`
- stable `backend_id`, never a localized display name
- optional executor-owned `backend_session_id`
- optional selected `model_id`
- Core-owned text and reasoning Part IDs
- exact model-visible messages and tool catalog
- typed settings snapshot

Clients may update the execution backend only while a conversation is idle.
The active Turn never reads a mutable toolbar selection.

## 3. Resumable Host session

A delegated process is not a synchronous function call. The Host maintains a
session keyed by the Core effect ID and exposes this state machine:

```text
created -> starting -> streaming -> awaiting_permission -> streaming
                                    |                     |
                                    +-> cancelled         +-> terminal
```

The Host request handler must return control while the process is waiting for
permission. It must not keep the Core driver blocked and must not open native
permission UI directly.

Process state is Host-owned and recoverable by effect ID plus backend session
ID. Conversation state remains Core-owned and recoverable from a checkpoint.

## 4. Event stream

Every event is correlated with the Host response envelope's request ID, effect
ID, session ID, Turn ID, and monotonic sequence. The delegated payload contains
only executor facts:

- session started
- text delta
- reasoning delta
- tool started/progress/completed
- permission requested
- token usage
- completed/failed/cancellation confirmed

Core validates the event family against the original Host request before
reducing it. Duplicate or replayed sequence numbers are acknowledged but never
applied twice. A terminal event closes the effect exactly once. A terminal
snapshot is not appended as another text delta.

## 5. Permission handshake

Permission is a Core policy decision over a concrete capability and resource.
It is never inferred from user-language keywords and never delegated to an
executor's default prompt policy.

```text
Host: permission_requested(request_id, call_id, capability, resource, input)
Core: evaluate frozen product/user/conversation/Turn rules
  allow -> resolve_delegated_permission(allow, structured grant)
  deny  -> resolve_delegated_permission(deny)
  ask   -> checkpoint PendingPermissionRequest and emit RequestPermission
User response -> Core records scope -> resolve_delegated_permission(...)
Host: resume the same suspended executor request
```

The Host must not execute the operation before the resolution. Conversation
grants are checkpointed once and reused by later concrete calls that match the
same structured rule. UI sheets are projections of Core pending permission
records, not independent modal state.

## 6. Tool projection

Delegated tools are already executed by the delegated Agent. Their lifecycle
events therefore reduce directly to `RuntimeEvent::ToolStarted`,
`ToolCallProgress`, and `ToolCompleted`. They must not become model tool calls,
because that would ask Core to execute the same tool a second time.

File artifacts carry URI, name, MIME type, and operation. The client renders
those typed artifacts and can open them through its platform adapter. It does
not parse filenames from prose.

## 7. Cancellation and failure

Escape/cancel creates a Core cancellation effect for the active delegated
effect. The Host cancels the exact process/session and confirms cancellation.
Core alone marks the Turn cancelled. Process exit without a terminal event is
a typed Host failure, not an inferred successful completion.

## 8. Persistence invariant

For one `(session_id, submission_id)`:

- exactly one Core Turn exists;
- one assistant message identity is allocated before the first Host delta;
- deltas append to Core-owned Parts in sequence order;
- terminal materialization closes those Parts without replacing prior messages;
- Store commits the resulting projection and cursor atomically;
- reconnect/replay produces the same projection byte-for-byte.

`CommitExternalExecutionProjection`, adapter-authored
`ExternalExecutionPartProjection`, and client-side terminal merging violate
this invariant and are deletion targets.

## 9. Visual acceptance is contract evidence

Final-state data assertions are insufficient. A real multi-turn run must
produce and an engineer must actually inspect:

- screenshots immediately after submit, before first token, during reasoning,
  during a tool call, during incremental Markdown, and after completion;
- a screen recording covering the entire stream, expansion/collapse, smooth
  following, cancellation, a second Turn, conversation switching, and restart;
- correlated Host event, Core projection, Store cursor, and UI revision logs.

The evidence receipt records the artifact path, observation time, inspected
frame/time range, and findings. Merely creating a screenshot or asserting that
the file exists is not visual verification. Screenshots prove spatial state;
recordings prove flicker, replacement, reflow, latency, and ordering behavior.

## 10. Migration gate

The migration is complete only when:

1. CLI, Web, and macOS delegated executors implement the same Host contract.
2. No external executor calls Store or a client Emitter directly.
3. No client selects an execution path outside Core.
4. Permission requests suspend and resume through Core.
5. Contract replay proves idempotence and terminal exactly-once behavior.
6. Real-model multi-turn runs are reconciled against Store and projection.
7. Screenshots are opened and inspected; recordings are watched and sampled.
8. The legacy external execution commands and DTOs are deleted.

## 11. Client live-projection boundary

The live client has exactly two render inputs:

1. Core's monotonic typed Part projection for accepted Turn state.
2. A reversible submission overlay shown before Core accepts the submission.

The overlay may contain the just-submitted user content and an empty assistant
shell. It is identified by the same submission, user Message, and assistant
Message identities sent to Core. It is never archived as conversation truth.
When the canonical projection contains those identities, the overlay is
removed rather than merged.

Reasoning, tool, source, citation, and text updates share one Part cursor and
one client projection reducer. Diagnostic trace events may update developer
diagnostics, but may not render a second copy of any product content. A client
that handles text Parts in one path and restores reasoning/tools from a
terminal snapshot is non-conforming because it necessarily changes layout at
the terminal boundary.

本批 iOS 独立 HTTP 子模型使用已有连接的相同 source 与冻结生成策略，只改变 model 身份；独立客户端仍由原生精确恢复器重建。来源/策略不匹配在准备 Task 前拒绝。运行中 guidance 走已有 FIFO 与原 Task owner/Run，工具 call_id 作为稳定提交身份；终态 task_id 续接不复用旧 Run 终态。详情见统一关系文档的后续接线范围。
