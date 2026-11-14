> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Client Projection Boundary

## Status

This document is a normative cross-platform contract. It applies to macOS,
iOS, Android, Web, and CLI renderers. Platform code may choose a native UI
implementation, but it may not introduce another conversation state machine.

## Dependency direction

```text
Domain types
    <- Core reducer and Part lifecycle
    <- Application transaction and ConversationProjection
    <- Host protocol adapter
    <- Client presentation projector
    <- Native view
```

Dependencies only point downward. A View cannot call Store, a Host cannot
mutate a View model, and a Trace reducer cannot create conversation messages.

## The three client state layers

### 1. Canonical cache

The canonical cache is a read-only client mirror of an Application
`ConversationProjection`. It accepts either an atomic versioned snapshot or
Application-issued bounded patches with owner, base/new cursor and stable Part
identity. Clients apply those patches exactly; they cannot author message
mutations, infer a missing suffix or merge conflicting versions themselves.
Revision advancement must not require rebuilding every message or view.

Only an Application command may change durable conversation state. A Host
terminal envelope identifies the committed projection but does not grant the
client permission to recreate it.

### 2. Pending submission overlay

The pending overlay is created before Host acceptance so the user message and
empty assistant shell appear immediately. Its identities must be the exact
submission-owned user and assistant Message IDs.

When canonical projection contains those identities, the overlay may remain
temporarily as the visual owner while a reveal animation drains. It is then
discarded, never archived or imported into the canonical cache.

### 3. Transient presentation override

Native interaction state such as Markdown reveal progress, disclosure state,
scroll anchoring, and unconfirmed artifact presentation belongs to an
in-memory presentation override keyed by authority/store, conversation, window
and message/Part identity. Live progress also binds Run/Attempt generation.

It may shadow a canonical Message for rendering. It cannot change role,
identity, durable text, context inclusion, or history ordering, and it is
discarded after terminal reconciliation or conversation teardown.

## Event ownership

| Event | Owns product content | May mutate durable state | Purpose |
|---|---:|---:|---|
| Core Part lifecycle | Yes | Through Application only | Text, reasoning, tools, sources, artifacts |
| ConversationProjection | Yes | Already committed | Canonical history and context usage |
| Core terminal | No | No | Turn status and terminal reason |
| Trace | No | No | Diagnostics, timing, protocol audit |
| UI animation tick | No | No | Presentation pacing only |

Application projection is a bounded fold, not transcript reconstruction. It
may collapse a canonical entity ledger into a small UI-neutral Part containing
stable identities and user-presentable fields. For example, failed Model
Attempts are folded from `run_attempts` into `model_attempt` Parts with the
failed provider/model, typed failure, and linked fallback identity. Raw
provider bodies remain in diagnostics and are never copied into chat content.

`Completed(String)`-style client events are forbidden. Terminal state cannot
carry a second copy of assistant text. The final body is the ordered Text Parts
in the canonical projection.

## Required invariants

1. A submission owns stable user, assistant, Turn, and Part identities.
2. A Part cursor is monotonic and idempotent for all Part kinds.
3. Text is append-only within a Text Part revision; replacement requires a new
   explicit canonical revision, never suffix guessing in a View.
4. Reasoning, tool calls, tool parameters, sources, and artifacts are Parts,
   not strings inferred from Trace labels.
5. Permission suspension preserves the same Turn and pending Host effect.
6. Cancellation produces one terminal transition and never synthesizes an
   assistant message in the client.
7. Context and usage use their own committed watermarks and owner binding;
   the client may update usage without rebuilding the transcript. A combined
   read declares its consistency snapshot rather than fabricating equal revisions.
8. Reopening a conversation renders only the canonical projection. Trace
   journal replay cannot insert or persist a message.
9. A failed Model Attempt remains visible after refresh. A successor is shown
   as an explicit fallback link; the client must not infer it from retry text.
10. Projection folding may omit or summarize canonical detail to stay bounded,
    but it cannot manufacture a second source of truth or reconstruct bodies
    from Trace.

Reference metadata may be presented in a collapsed message footer. The native
client may merge repeated source/citation links while retaining canonical ID
coverage and distinct page/line/range citations; this does not reorder prose or
interactive surfaces or rewrite stored Parts. A terminal reply with an open
Skill snapshot is shown as incomplete/cancelled, never as still executing or as
verified success. Technical counters belong to disclosure content.

## Forbidden interfaces

- Client-side `appendAssistantMessage`, `persistAssistantMessage`, or a local
  fallback `push` after a Host-owned Turn.
- A generic message mutation helper that falls through into canonical cache.
- Rebuilding product text from `assistant.delta`, `run.completed`, stdout, or
  a terminal snapshot after Core Parts have been emitted.
- Language-specific routing, permission, or tool-selection keyword lists.
- Converting a protocol error into a plausible assistant answer.

## Routing regression matrix

Client routing tests must submit the original user input unchanged through the
same Core input port. Coverage must include:

- a direct request, a natural follow-up, and an elliptical follow-up that
  depends on conversation history;
- equivalent requests in Simplified Chinese, English, Japanese, and a mixed
  language form;
- ambiguous words such as `search`, `current`, `remember`, `file`, `task`, and
  capability names used in an architecture discussion rather than a request;
- attachments whose filenames contain tool-like words;
- an unknown tool call followed by Core-mediated recoverable model guidance;
- a concrete permission denial and a conversation-scoped grant replay.

Assertions are made against the concrete model tool call and Core decision,
not a desired tool guessed from the prompt text. A mock-model unit test cannot
prove semantic routing reliability; release evidence requires real-model
multi-turn sessions for every supported Host class.

## Acceptance evidence

Unit tests prove reducer and identity invariants but do not prove presentation.
Every product-path E2E must additionally:

1. run a real multi-turn conversation using a real model and Host;
2. capture checkpoints before first response, during reasoning, during tool
   execution, during incremental Markdown, at terminal, and after reopening;
3. open and semantically inspect every screenshot;
4. watch the complete recording to inspect flicker, reflow, scrolling, reveal
   pacing, and overlay-to-canonical handoff;
5. correlate each checkpoint with submission, Turn, Message, Part cursor, and
   projection revision;
6. record an explicit observation result. File existence, hashes, dimensions,
   logs, and green tests are not substitutes for visual observation.
