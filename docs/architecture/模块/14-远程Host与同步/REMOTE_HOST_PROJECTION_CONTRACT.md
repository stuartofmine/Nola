> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Remote Host Projection Contract

Status: target authority/import contract, reviewed 2026-10-15. Current caller
coverage remains an implementation task; these rules do not assert migration complete.

## Decision

A remote Host response is committed as a Core projection, never as client-authored
assistant text. Transport deltas and traces are diagnostic/live-presentation inputs;
they are not durable conversation facts.

The local replica acceptance boundary is:

`CommitRemoteConversationProjection -> Application -> one Store transaction`

The execution authority remains the authenticated source bound at admission.
The receiving Application accepts source identity, epoch, source cursor,
original submission/Run binding and content references, then commits a local
replica receipt and read projection. A local snapshot may represent the replica
but cannot restore or advance a new local reducer for the remote Run.
Use the existing journal/projection protocols; the command name above denotes
this responsibility, not an alternative client-authored transcript API.

Incremental source-committed patches may be imported with their commit witness;
uncommitted transport deltas remain transient. A terminal notification triggers
verification and seal import, not reconstruction from the displayed bubble.

That transaction advances together:

- the authenticated Host mirror and its monotonic Core cursor;
- the imported Run terminal state, when a validated seal is present;
- the assistant message terminal state, under that same seal;
- conversation revision;
- source-scoped permission observations, without changing local authorization;
- the idempotent remote transition receipt.

## Invariants

1. `source_host_id` is mandatory and comes from an authenticated Host session.
2. `conversation_id` comes from `AgentCoreSnapshot.session_id`, not UI state.
3. The terminal Envelope carries the exact, case-sensitive `submission_id`
   allocated by `OpenRemoteTurn`. Application resolves the assistant identity
   from that durable Run. A client must not supply a second identity, consult an
   in-memory active-turn map, derive an assistant ID from a naming convention,
   normalize case, or allocate another ID.
4. A completed response must contain at least one canonical Part owned by the
   assistant message.
5. Cursor rollback and same-cursor/different-content commits are rejected.
6. Clients render `ConversationProjectionV1`; they do not merge raw assistant
   text, traces, Store Parts, and provider deltas into a second transcript.
7. Permission rules in the snapshot describe the source authority only. They
   never replace local grants, OS permission facts, or a different Host's rules.
   A remote decision is acted on only by its original authority with exact
   request/source binding; replication is not an authorization decision.
8. Source cursor/epoch determine ordered acceptance, never wall-clock updatedAt.
   A new epoch requires explicit lineage/binding validation, not acceptance
   because its timestamp is newer.
9. The same imported Attempt cannot be charged once as a source aggregate and
   again through its replica events.

## Migration rule

The former `FinishRemoteConversationTurn(assistant_text, metadata)` port is an
invalid abstraction because it lets every client manufacture a competing
assistant record. Platform adapters must migrate to the projection commit port;
the old port and raw Store remote-transition primitives are removed after all
callers use the contracted port.

External agent executors are Hosts, not alternate Application layers. Codex,
Claude Code, and OpenCode events must enter the same Host response/Core reducer
path as native model and tool effects. An executor adapter may retain its own
opaque session ID, but it cannot write Store-shaped Parts, infer assistant
identity, or persist a separate transcript.

The temporary `CommitExternalExecutionProjection` command violates this rule:
it accepts adapter-authored Store Parts and calls `commit_remote_transition`
without a Core checkpoint or correlated ModelStream effect. It is a deletion
target, not a supported compatibility contract. Migration is complete only
when external executors translate their stream into correlated `ModelDelta`,
`ToolCallProgress`, `ModelCompleted`, `Failed`, or cancellation Host responses,
and this command is no longer decodable by Application.

## Visual acceptance

Runtime correctness is not established by logs or generated artifacts alone.
Acceptance must open every lifecycle screenshot and watch every referenced
recording, then compare the observed pixels with the exact canonical projection
revision. Repetition, omission, flicker, delayed reflow, and message replacement
are boundary failures even when unit tests pass.
