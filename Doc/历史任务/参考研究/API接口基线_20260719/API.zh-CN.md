> 历史接口基线：2026-07-19。2026-10-28归档，仅供追溯，不作为当前路由或鉴权规范。

> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Nola 接口文档

版本：2026-07-19
适用后端：Cloudflare Worker (`backend/worker`) 与 D1

> English version: [`API.md`](API.md)

## 1. 基本约定

### 1.1 Base URL

- 生产：`https://<your-domain>`
- 本地后端：`http://127.0.0.1:8000`
- 本地 Web API：`http://127.0.0.1:5001/api`

### 1.2 鉴权

1. **Bearer Token（业务接口）**
   - Header：`Authorization: Bearer <access_token>`
2. **Client UUID（部分接口）**
   - Header：`X-Client-UUID: <uuid>`
3. **Debug/Admin Token（调试接口）**
   - 按部署配置要求传递（仅运维用途，不面向普通客户端）

### 1.3 内容类型

- 请求：`Content-Type: application/json`
- 流式接口：`text/event-stream`（SSE）

## 2. 后端 API（核心）

### 2.1 健康与运维

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/healthz` | 健康检查 |
| GET | `/admin/outbox/dlq/count` | DLQ 数量 |
| POST | `/admin/outbox/dlq/replay` | DLQ 重放 |

### 2.2 OAuth / 账号

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/oauth/login` | OAuth Relay 登录页 |
| GET | `/oauth/start/{provider}` | 启动 provider 登录 |
| GET/POST | `/oauth/callback/{provider}` | provider 回调 |
| POST | `/oauth/relay/exchange` | relay code 换取 access token |
| POST | `/v1/auth/apple` | Apple 登录/注册 |
| POST | `/v1/auth/google` | Google 登录/注册 |
| POST | `/v1/auth/github` | GitHub 登录/注册 |
| GET | `/v1/auth/me` | 当前用户信息 |

说明：`/v1/auth/register` 与 `/v1/auth/login` 返回 `410`（官方后端禁用邮箱密码鉴权）。

### 2.3 模型与对话

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/v1/models` | 模型列表 |
| POST | `/v1/chat/completions` | 对话补全（可 `stream=true`） |
| GET | `/v1/chat/conversations` | 会话列表 |
| POST | `/v1/chat/conversations` | 创建会话 |
| PATCH | `/v1/chat/conversations/{conversation_id}` | 更新会话 |
| DELETE | `/v1/chat/conversations/{conversation_id}` | 删除会话 |
| GET | `/v1/chat/conversations/{conversation_id}/messages` | 消息列表 |
| POST | `/v1/chat/conversations/{conversation_id}/messages` | 新增消息 |

### 2.4 Agent 设置

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/v1/agent/settings` | 获取 Agent 设置 |
| POST | `/v1/agent/settings` | 更新 Agent 设置 |
| GET | `/v1/agent/permission_rules` | 权限规则列表 |
| POST | `/v1/agent/permission_rules` | 新建权限规则 |
| DELETE | `/v1/agent/permission_rules/{rule_id}` | 删除权限规则 |
| GET | `/v1/agent/tools` | 工具元数据 |

### 2.5 设备、定时任务、结果、通知

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | `/v1/devices/register` | 注册推送设备 |
| POST | `/v1/sources/sync` | 创建/同步定时任务 |
| GET | `/v1/sources` | 定时任务列表 |
| DELETE | `/v1/sources/{source_id}` | 删除任务 |
| POST | `/v1/sources/{source_id}/run_now` | 立即执行 |
| GET | `/v1/results` | 执行结果列表 |
| GET | `/v1/results/{result_id}` | 执行结果详情 |
| POST | `/v1/results/{result_id}/ack` | 结果确认 |
| GET | `/v1/notifications` | 通知列表 |
| GET | `/v1/notifications/unread_count` | 未读数 |
| POST | `/v1/notifications/{notification_id}/read` | 单条已读 |
| POST | `/v1/notifications/read_all` | 全部已读 |

### 2.6 搜索增强

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/search/serper` | Google/Serper 搜索 |

### 2.7 移动桥接接口

| 方法 | 路径 |
|---|---|
| POST | `/api/mobile/login` |
| GET | `/api/mobile/me` |
| GET | `/api/mobile/agent/settings` |
| POST | `/api/mobile/attachments/parse` |

官方 Worker 只是 OpenAI-compatible 模型网关，不是 Agent Host。Agent 流只由 `nola web` 的 `/api/chat/agent/v2/stream` 与 `/api/mobile/chat/agent/v2/stream` 提供，并且只输出 canonical Core Part/Trace 事件。

## 3. Web API（`/api/*`，本地优先）

Web API 由 `nola web` 本地 Web Host 提供，默认使用 `/api` 前缀，会优先
操作本地数据，再按能力同步后端。

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/health` | Web 服务健康状态 |
| GET | `/api/tasks` | 仪表板聚合数据 |
| GET | `/api/scheduled-actions` | 本地任务列表 |
| POST | `/api/scheduled-actions` | 创建任务（html/search/script/file/folder） |
| DELETE | `/api/scheduled-actions/{source_id}` | 删除任务 |
| POST | `/api/scheduled-actions/{source_id}/run-now` | 立即执行 |
| POST | `/api/scheduled-actions/{source_id}/approve-script` | 审批脚本任务 |
| GET | `/api/results` | 执行记录列表 |
| GET | `/api/results/{result_id}` | 执行记录详情 |
| GET | `/api/models` | 可用模型 |
| POST | `/api/chat/agent/v2/stream` | 权威 Agent Host 聊天 SSE，并负责用户/助手消息持久化 |
| GET | `/api/chat/conversations/{conversation_id}/traces` | 按会话回放 Agent Trace |
| GET | `/api/agent/settings` | Agent 设置 |
| POST | `/api/agent/settings` | 保存 Agent 设置 |

### 3.1 Agent Trace 回放

```http
GET /api/chat/conversations/{conversation_id}/traces
    ?trace_id=<trace_id>
    &after_sequence=<sequence>
```

该本地 Web Host 接口回放 Trace Journal 中的诊断活动，不恢复执行权、正式正文、
授权决定或 Run 结果。业务恢复须读取 Application 投影/回执并调用精确 Run 恢复端口。

| 参数 | 位置 | 必填 | 语义 |
| --- | --- | --- | --- |
| `conversation_id` | path | 是 | 权威的会话隔离范围 |
| `trace_id` | query | 否 | 仅回放属于该会话的指定 Trace |
| `after_sequence` | query | 否 | Trace 内的排他游标；省略或传 `0` 表示从头回放 |

事件按 `sequence` 升序返回，只进入诊断 reducer，以 Trace 游标和 event_id 去重。
经过过滤的 Trace 序号不保证连续；正式投影使用自己的提交水位，不能靠诊断流补正文。

隔离规则：

- 即使传入 `trace_id`，服务端仍必须应用 `conversation_id` 范围；
- 不属于当前会话的 Trace 不得返回；
- 不得用 provider `call_id` 或工具名作为回放隔离键；
- 不同 Trace、Run 或会话中相同的调用 ID 必须相互独立；
- 接口沿用已认证的本地 Web 会话，不能暴露其他会话的 Journal。

示例：

```bash
curl -H "Authorization: Bearer <local-web-token>" \
  "http://127.0.0.1:5001/api/chat/conversations/<conversation-id>/traces?trace_id=<trace-id>&after_sequence=42"
```

客户端应从最后一个连续序号继续轮询或重连，而不是从“已收到的最大
序号”继续。空回放结果不能用于跳过已知缺口；缺少终态事件时也不能把
执行判定为成功。

## 4. 错误码与返回

- `2xx`：成功
- `400` 或 `422`：参数错误
- `401` 或 `403`：未授权
- `404`：资源不存在
- `429`：请求过于频繁
- `5xx`：服务端错误

## 5. 示例

### 5.1 获取模型列表

```bash
curl -H "Authorization: Bearer <token>" \
  http://127.0.0.1:8000/v1/models
```

### 5.2 创建定时任务

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

### 5.3 模型 SSE

```bash
curl -N -X POST http://127.0.0.1:8000/v1/chat/completions \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-4o-mini","stream":true,"messages":[{"role":"user","content":"你好"}]}'
```
