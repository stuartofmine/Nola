> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Web chat transport migration contract

> 2026-10-15 整体设计复核前的历史快照，仅保留当时叙述与证据。旧 ABI、Trace 权威、schema 数字、传输别名和其他旧规则均不作为现行规范。当前入口见[软件架构总纲](../../软件架构设计/总体设计/SOFTWARE_DESIGN.md)。

Read-only audit, 2026-09-22. This records the current Rust Host contract, not a claim that the Svelte chat migration is complete. No real-model requests were made for this audit.

## Conversation and history

Implementation: `cli/src/web_parts/routes.rs`, `native_bridge.rs`, `permission_resume.rs`.

| Endpoint | Contract |
| --- | --- |
| GET `/api/chat/conversations` | `{conversations,localFirst,backend}` sorted by updatedAt descending. Conversation identity is `conversationId`. |
| POST `/api/chat/conversations` | Optional camelCase `title,selectedModelSource,selectedModelId,executionBackend,externalExecutionSessions`; returns the conversation object, not `{conversation:…}`. |
| PATCH `/api/chat/conversations/{id}` | Same editable profile fields; explicit title becomes user-owned. This is not a complete per-session model-parameter API. |
| DELETE `/api/chat/conversations/{id}` | Deletes a conversation; do not simulate deletion by clearing the UI. |
| GET `/{id}/messages?limit=48&before=<ordinal>` | Prefix is `/api/chat/conversations`. Limit clamps to 1–200. Returns `messages,conversationProjection,messageWindow,taskProjection,memoryRuntimeProjection,skillProjection,permissionProjection`. |
| GET `/{id}/traces?trace_id=…&after_sequence=…` | `{conversationId,traces:[{traceId,runId,messageId,status,lastSequence,terminalReason,startedAt,finishedAt,afterSequence,nextAfterSequence,events}]}`. An incremental sequence applies only when a particular trace is selected. |

`messageWindow` contains `totalCount,before,hasMore,limit,projectionRevision,eventSequence`. Preserve its ordinal pagination and projection watermarks; do not paginate by timestamps or use array position as identity. History messages are camelCase (`messageId,conversationId,role,content,includeInContext,meta,createdAt`), whereas nested Core projections retain their own snake_case schema.

Other existing contracts to preserve: PATCH/DELETE `/{id}/messages/{messageId}`, POST `/{id}/truncate` with `{messageId,keepMessage?}`, POST `/{id}/branch` with `{messageId?,title?}`, GET `/{id}/historical-part`, `export`, `context`, `goal`, `memory/search`; PATCH `session-memory` and `data-sources/{sourceId}`; POST `compact` and `goal`. These responsibilities must not disappear merely because basic send/history is migrated.

Global settings are GET/POST `/api/agent/settings` (sparse owned-field updates). Conversation model selection is camelCase profile selection, distinct from global settings. Sending an explicit `modelProfileId` selects the canonical remote model; do not silently replace session selection with the globally selected model.

## New turn and SSE

POST `/api/chat/agent/v2/stream` uses fetch with a POST body, not EventSource:

```json
{
  "conversationId": "conversation-id",
  "client_id": "stable-browser-client-id",
  "submission_id": "stable-submission-id",
  "user_message_id": "stable-user-message-id",
  "assistant_message_id": "stable-assistant-message-id",
  "messages": [{"role":"user","content":"…"}],
  "current_user_display_text": "…",
  "modelProfileId": "optional-canonical-model-id",
  "agentMode": true,
  "execution_backend": "nola"
}
```

The legacy client also sends `timezone,language,responseStyle,max_iterations` and, where applicable, `external_session_id`. There are legacy model/endpoint overrides; migration should prefer canonical profiles and should not introduce browser persistence of credentials. Host validates `permission_profile` and optional `expected_host_id`. Web requests may not impersonate an authenticated mobile `device_bridge`.

Persist submission and message IDs before sending. A lost response does not authorize creating a second submission. Recover the original conversation/run instead of replaying user input under new IDs.

The stream carries JSON SSE events and `[DONE]`, with heartbeat frames. Byte chunks are not event boundaries. Parse split UTF-8 and multiline SSE data; handle JSON HTTP errors before opening the stream reducer.

Canonical projection event types include:

- `core_part_updated`, `core_task_updated`, `core_skill_runtime_updated`, `core_capabilities_activated`;
- `core_permission_projection_updated`, `core_memory_runtime_projection_updated`, `core_session_projection_updated`;
- `context_usage`, `conversation_projection_committed_v1`.

Projection events do not need a Trace wrapper. Other public progress events require the canonical Trace envelope. Core text Parts are the authority for assistant body text; Trace text is not a second stream to concatenate. Retain stable message/part identity and cursor/revision ordering. Duplicate/older events must not append text again. Trace sequences can have legitimate gaps because internal events are filtered.

`run.completed`, `run.failed`, and `run.cancelled` end execution, **not durable conversation delivery**. `conversation_projection_committed_v1` with `conversation_projection` is the durability barrier. Merge its canonical assistant message by the exact `assistant_message_id`; only then release the composer after rendering settles. `[DONE]`, EOF, a terminal Trace, and HTTP permission acknowledgement are not equivalent to that barrier. Interrupted delivery must reconcile history/projection; do not fabricate a successful assistant answer.

## Permission challenge, decision, and resume

The `permission_request` event carries a challenge and Trace. Keep the complete challenge bound to the owning conversation/run; never identify it only by tool name. Core permission protocol is version **3** (independent from stream protocol version 2).

POST `/api/agent/host-permission/decision`:

```json
{
  "protocol_version": 3,
  "nonce": "exact-challenge-nonce",
  "request_id": "exact-request-id",
  "host_id": "exact-host-id",
  "client_id": "exact-client-id",
  "session_id": "exact-session-id",
  "tool_name": "exact-tool-name",
  "resolution": {"type":"authorization","decision":"allow_once"}
}
```

Authorization decisions: `allow_once`, `allow_conversation`, `deny` (optional grant belongs to the authorization variant). Intervention is a different tagged variant:

```json
{"type":"intervention","intervention":{"type":"provide_instruction","instruction":"…"}}
```

or `{"type":"intervention","intervention":{"type":"return_to_composer"}}`. Do not combine denial/grants/instruction fields or rewrite instructions as fresh user messages. Core validates version, nonce, every challenge identity field, expiry and resolution. HTTP 200 `{accepted:true,nonce}` acknowledges delivery only; identical retries are deduplicated, different decisions for the same accepted nonce are rejected. Keep the pending state until canonical permission/run projection confirms resolution. Failed requests remain retryable without inventing a new challenge.

POST `/api/chat/agent/v2/resume` is **existing-run execution recovery**, not a new turn or Trace pagination. Its strict body (unknown fields rejected) is:

```json
{"conversation_id":"…","expected_run_id":"…","expected_host_id":"…","client_id":"…"}
```

No messages, model overrides, submission IDs, context overrides or replacement flags are accepted. Fields are nonempty and at most 512 bytes. A changed Host returns 409 `host_identity_changed`; an already active delivery returns 409 `run_in_progress`. Resume returns the canonical SSE transport. A normal permission decision continues its original waiting execution; do not automatically POST resume or resend the prompt after every acknowledgement.

## Cancellation

POST `/api/chat/agent/v2/cancel` with `{conversationId}` (legacy `session_id` accepted) returns `{conversationId,cancelRequested,terminationConfirmed}`. HTTP 202 means termination is pending; HTTP 200 reports confirmed termination. Aborting fetch alone is not cancellation of Host execution. Continue reconciling durable history and ownership before allowing a conflicting send.

Separate task controls: POST `/api/chat/agent/v2/task/cancel` with `{conversationId,taskId}` and `/task/message`. These are task operations, not whole-conversation cancellation.

## Reuse boundary and migration gates

Existing `web/app/static/js/agent-trace-runtime.js` exports `NolaAgentTrace.createSseJsonDecoder`, `createTraceStore`, and `sourceItemMessageIndex`. Its Core/Trace reducers encode ordering, deduplication and projection handling worth reusing through a typed module adapter rather than replacing with a text-delta loop. It is currently a `window` IIFE, not an ESM transport module.

`chat-stream-runtime.js` contains Markdown stream settling and DOM patching; `historical-part-detail.js` and `conversation-title-policy.js` encode additional behavior. DOM mutation helpers must not concurrently own the same nodes as Svelte. Prefer separating pure protocol reducers and Markdown buffering from rendering ownership, with typed callbacks feeding Svelte keyed message/part state.

`chat.html` also owns permission cards, tool/task progress, switching conversations during delivery, scrolling/history anchors, attachments, model selection, compaction/memory/context inspectors, editing/branch/export, and resumable journals. A basic new UI is not parity with those responsibilities.

Before claiming parity, test: fragmented SSE; duplicate/reordered projections; terminal Trace before commit; loss after commit; exact permission retry/expiry/wrong identity; run resume without new input; cancel pending versus confirmed; conversation switching; historical pagination without viewport jumps; Markdown settlement without duplicating or relocating assistant text. Use isolated Host databases and local mock endpoints; no paid/real model is needed for transport-contract tests.
