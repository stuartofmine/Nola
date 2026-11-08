> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Conversation Mutation Contract

## Purpose

This contract defines the only supported path for changing a durable conversation. It prevents a
client projection, a streaming buffer, a trace journal, or a terminal renderer from becoming a
second source of message truth.

## Ownership

- Core owns the semantic transition: turn, submission, message, Part, tool call, permission wait,
  completion, cancellation, and conversation rewind.
- Application owns use-case validation and the atomic transaction that commits the Core checkpoint
  together with the canonical conversation projection.
- Store owns persistence, revision checks, idempotency, and projection materialization.
- Hosts execute requested effects and return typed responses. They do not synthesize durable
  messages or projection mutations.
- Clients render a read model plus explicitly transient UI state. They do not submit a replacement
  transcript.

## Allowed Commands

Interactive history can change only through typed product intents:

- `BeginAssistantTurn`
- typed Host responses for the active Run
- `RevertConversation`
- `EditConversationMessage`
- `DeleteConversationMessage`
- `CreateConversationBranch`
- `ImportConversationTranscript` for a completed external archive

There is no generic append-message or apply-projection Application command.

## Revert Invariant

The caller supplies only:

- conversation identity
- boundary message identity
- whether the boundary remains visible
- an idempotent operation identity

Application must load the current canonical projection, resolve the boundary, derive every hidden
message, submit the corresponding Core transition, and atomically commit the resulting checkpoint
and projection. A caller must never supply `expectedRevision`, `parentMessageId`, visibility
mutations, replacement messages, or a complete `ApplyConversationProjectionInput` for rewind.

If the boundary is absent from the current visible projection, the command fails. If an active Run
exists, the command fails. Core checkpoint advancement without the matching projection, or
projection advancement without the matching checkpoint, is invalid.

## Streaming Invariant

Streaming changes only Parts belonging to the assistant message shell created at turn submission.
The shell is visible immediately. Deltas update stable Part identities; they never append a second
assistant message. Terminal completion changes Part and message state without replacing the
message collection.

Transient animation buffers may lag or group deltas for presentation, but they are disposable and
must reconcile by Part identity and projection revision. They cannot be restored as conversation
history.

## Acceptance Evidence

Contract tests must prove revision conflicts, idempotent replay, boundary validation, active-Run
rejection, checkpoint/projection atomicity, and stable Part identity. Real multi-turn acceptance
must then observe screenshots and recordings at submission, thinking, tool use, streaming,
completion, conversation switching, and relaunch. Artifact creation alone is not visual evidence;
every screenshot must be opened and every recording watched.
