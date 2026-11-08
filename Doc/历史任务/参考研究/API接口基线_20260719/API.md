> 历史接口基线：2026-07-19。2026-10-28归档，仅供追溯，不作为当前路由或鉴权规范。

> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Nola API Documentation

Version: 2026-07-19
Backend: Cloudflare Worker (`backend/worker`) with D1

> 中文版本：[`API.zh-CN.md`](API.zh-CN.md)

## 1. Conventions

### 1.1 Base URL

- Production: `https://<your-domain>`
- Local backend: `http://127.0.0.1:8000`
- Local Web API: `http://127.0.0.1:5001/api`

### 1.2 Authentication

1. **Bearer token (business APIs)**
   - Header: `Authorization: Bearer <access_token>`
2. **Client UUID (required by some APIs)**
   - Header: `X-Client-UUID: <uuid>`
3. **Debug/Admin token (ops only)**
   - Provided per deployment config

### 1.3 Content Type

- Request: `Content-Type: application/json`
- Streaming: `text/event-stream` (SSE)

## 2. Backend APIs

### 2.1 Health & Admin

| Method | Path | Description |
|---|---|---|
| GET | `/healthz` | Health check |
| GET | `/admin/outbox/dlq/count` | Get DLQ count |
| POST | `/admin/outbox/dlq/replay` | Replay DLQ messages |

### 2.2 OAuth & Identity

| Method | Path | Description |
|---|---|---|
| GET | `/oauth/login` | OAuth relay login page |
| GET | `/oauth/start/{provider}` | Start provider OAuth flow |
| GET/POST | `/oauth/callback/{provider}` | Provider callback |
| POST | `/oauth/relay/exchange` | Exchange relay code for token |
| POST | `/v1/auth/apple` | Apple sign-in/up |
| POST | `/v1/auth/google` | Google sign-in/up |
| POST | `/v1/auth/github` | GitHub sign-in/up |
| GET | `/v1/auth/me` | Current user profile |

Note: `/v1/auth/register` and `/v1/auth/login` return `410` (email/password auth disabled on official backend).

### 2.3 Models & Chat

| Method | Path | Description |
|---|---|---|
| GET | `/v1/models` | List models |
| POST | `/v1/chat/completions` | Chat completion (supports streaming) |
| GET | `/v1/chat/conversations` | List conversations |
| POST | `/v1/chat/conversations` | Create conversation |
| PATCH | `/v1/chat/conversations/{conversation_id}` | Update conversation |
| DELETE | `/v1/chat/conversations/{conversation_id}` | Delete conversation |
| GET | `/v1/chat/conversations/{conversation_id}/messages` | List messages |
| POST | `/v1/chat/conversations/{conversation_id}/messages` | Add message |

### 2.4 Agent Settings

| Method | Path | Description |
|---|---|---|
| GET | `/v1/agent/settings` | Get Agent settings |
| POST | `/v1/agent/settings` | Update Agent settings |
| GET | `/v1/agent/permission_rules` | List permission rules |
| POST | `/v1/agent/permission_rules` | Create permission rule |
| DELETE | `/v1/agent/permission_rules/{rule_id}` | Delete permission rule |
| GET | `/v1/agent/tools` | List Agent tools |

### 2.5 Devices, Sources, Results, Notifications

| Method | Path | Description |
|---|---|---|
| POST | `/v1/devices/register` | Register push device |
| POST | `/v1/sources/sync` | Create/sync scheduled source |
| GET | `/v1/sources` | List scheduled sources |
| DELETE | `/v1/sources/{source_id}` | Delete source |
| POST | `/v1/sources/{source_id}/run_now` | Run source now |
| GET | `/v1/results` | List execution results |
| GET | `/v1/results/{result_id}` | Get execution result |
| POST | `/v1/results/{result_id}/ack` | Acknowledge result |
| GET | `/v1/notifications` | List notifications |
| GET | `/v1/notifications/unread_count` | Get unread notification count |
| POST | `/v1/notifications/{notification_id}/read` | Mark one notification as read |
| POST | `/v1/notifications/read_all` | Mark all notifications as read |

### 2.6 Search

| Method | Path | Description |
|---|---|---|
| GET | `/search/serper` | Serper search |

### 2.7 Mobile Bridge

| Method | Path | Description |
|---|---|---|
| POST | `/api/mobile/login` | Mobile login bridge |
| GET | `/api/mobile/me` | Mobile profile bridge |
| GET | `/api/mobile/agent/settings` | Mobile Agent settings bridge |
| POST | `/api/mobile/attachments/parse` | Attachment parsing bridge |

The official Worker is an OpenAI-compatible model gateway, not an Agent Host. Agent streams are served only by `nola web` at `/api/chat/agent/v2/stream` and `/api/mobile/chat/agent/v2/stream` using canonical Core Part/Trace events.

## 3. Web APIs (`/api/*`)

Web APIs are local-first and may sync to backend when available.

| Method | Path | Description |
|---|---|---|
| GET | `/api/health` | Web health status |
| GET | `/api/tasks` | Dashboard aggregate |
| GET | `/api/scheduled-actions` | List local scheduled actions |
| POST | `/api/scheduled-actions` | Create scheduled action |
| DELETE | `/api/scheduled-actions/{source_id}` | Delete action |
| POST | `/api/scheduled-actions/{source_id}/run-now` | Run action now |
| POST | `/api/scheduled-actions/{source_id}/approve-script` | Approve script action |
| GET | `/api/results` | List results |
| GET | `/api/results/{result_id}` | Result details |
| GET | `/api/models` | List models |
| POST | `/api/chat/agent/v2/stream` | Canonical Agent Host chat stream; owns User/Assistant persistence |
| GET | `/api/chat/conversations/{conversation_id}/traces` | Replay conversation-scoped Agent Trace events |
| GET | `/api/agent/settings` | Get Agent settings |
| POST | `/api/agent/settings` | Save Agent settings |

### 3.1 Agent Trace replay

```http
GET /api/chat/conversations/{conversation_id}/traces
    ?trace_id=<trace_id>
    &after_sequence=<sequence>
```

This local Web Host endpoint replays diagnostic activity from the Trace journal.
It cannot restore execution, canonical text, permission decisions or a Run result;
those require Application projections, receipts and the exact Run recovery port.

| Parameter | Location | Required | Semantics |
| --- | --- | --- | --- |
| `conversation_id` | path | yes | Authoritative conversation/session scope |
| `trace_id` | query | no | Restricts replay to one trace belonging to the conversation |
| `after_sequence` | query | no | Exclusive per-trace cursor; omit or use `0` to replay from the beginning |

Replay events are returned in ascending sequence order to a diagnostic reducer.
Its cursor and event identity deduplicate diagnostic overlap only. Filtered Trace
sequences need not be contiguous; canonical projection watermarks are independent.

Isolation rules are normative:

- `conversation_id` is always applied, even when `trace_id` is supplied;
- a trace that does not belong to the requested conversation is not returned;
- provider `call_id` or tool names are never used as replay scope;
- identical call IDs in different traces, runs, or conversations remain
  independent;
- the endpoint uses the authenticated local Web session and does not expose
  another session's journal.

Example:

```bash
curl -H "Authorization: Bearer <local-web-token>" \
  "http://127.0.0.1:5001/api/chat/conversations/<conversation-id>/traces?trace_id=<trace-id>&after_sequence=42"
```

Clients should poll or reconnect from the last contiguous sequence, not the
largest sequence merely received. An empty replay page does not authorize
skipping a known gap, and the absence of a terminal event must not be
interpreted as successful completion.

## 4. Error Codes

- `2xx`: success
- `400` or `422`: invalid request
- `401` or `403`: unauthorized
- `404`: not found
- `429`: rate limited
- `5xx`: server-side error

## 5. Examples

### 5.1 List models

```bash
curl -H "Authorization: Bearer <token>" \
  http://127.0.0.1:8000/v1/models
```

### 5.2 Create source

```bash
curl -X POST http://127.0.0.1:8000/v1/sources/sync \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com",
    "frequency": "daily",
    "scheduled_time_iso": "09:00:00",
    "timezone": "Asia/Shanghai"
  }'
```

### 5.3 Model SSE

```bash
curl -N -X POST http://127.0.0.1:8000/v1/chat/completions \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-4o-mini","stream":true,"messages":[{"role":"user","content":"Hello"}]}'
```
