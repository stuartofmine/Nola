# 架构历史实施记录

2026-11-08 从现行架构中分离。下文保留原日期、失败/通过与暂停范围，不构成当前实施状态、操作限制或兼容要求；相对链接按归档位置调整。

> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Cross-platform automation contract

Status: target design, not a claim that the shared executor is implemented.
Audit date: 2026-09-22.

## Observed mismatch

The canonical database is shared, but the product schema and executor are not.
`AutomationDefinition.metadata_json` is still an opaque platform payload:

- `MacAutomationStore` writes `nola.mac.automation.v1` with prompt, workspace,
  hourly/daily/weekly timing, context sources, skills, retries and timeouts.
- `cli/src/store/schedule.rs` filters a separate list of source formats. Web
  actions use `enabled`, `time`, `url`, `userPrompt` and script metadata.
- `cli/src/background_scheduler.rs` is a Mac LaunchAgent due-work probe. Its
  matcher expects `isEnabled`, `hour` and `minute`; it launches the Mac app.
  Its existence does not prove that Web source actions are scheduled.
- Web `run-now` executes synchronously and responds with the actual result;
  a successful HTTP receipt is not an asynchronous queue acknowledgment.

Replacing HTML with Svelte does not repair these differences. The interim page
must state its source-task scope and its Host's actual execution capabilities.
An estimated date must not be labeled as an active scheduler's next run.

## Required shared product model

Application, not a client, owns a versioned typed definition with these fields:

| Field | Required semantics |
| --- | --- |
| Identity | Stable ID, revision, creation/update times and deletion tombstone |
| Work | Discriminated `agent` or `source` payload; never infer kind from prompt text |
| Agent payload | Prompt, workspace resource, optional skill ID, typed context dependencies |
| Source payload | Typed source resource or explicit script body and working directory |
| Schedule | Hourly/daily/weekly recurrence, validated local time, explicit timezone policy |
| Execution policy | Timeout, bounded retries, data freshness and change trigger |
| Availability | User-enabled state separate from executor support and worker health |

UI labels do not encode authorization. A workspace, context dependency or script
is a concrete resource evaluated by the normal Core permission boundary. Saved
configuration, toggling enabled, and approving one script are not blanket grants
for future edited work.

## Commands and projections

1. List returns typed definitions, revisions and per-Host capabilities, including
   supported work kinds, schedules, triggers and whether a scheduler is active.
   Unsupported definitions remain visible; they cannot become a different kind.
2. Create uses a client-generated operation identity. Update/delete require the
   expected revision. A one-row mutation never reconciles a stale full collection.
3. Script approval binds to the reviewed definition revision/content. Changing
   the reviewed resource or script invalidates approval. A truncated preview is
   insufficient for approval; the complete review payload must be available.
4. Run requests return a durable run ID and state. Clients poll/subscribe to the
   same run projection used by Mac history; transport failure is not proof that
   the run did not start. Retrying an operation must not execute it twice.
5. Scheduler and manual runs both enqueue through Application. Workers claim
   existing canonical occurrence/lease records; there is no separate Web-only
   queue. Cancellation, retries and terminal state are owned by that lifecycle.
6. Public projections omit worker lease tokens and secrets. User content remains
   verbatim, while protocol failures use stable codes translated by each client.

## Scheduling semantics to implement centrally

Timezone policy is not a display option: named-zone recurrence and Host-local
recurrence must be distinguishable. DST gaps/folds, missed occurrences and retry
timing need shared deterministic tests. Do not store `UTC` while calculating in
Host local time. No client may promise a timer merely because it can estimate a
date. Mobile clients may manage definitions while the designated Host executes;
they must show that dependency rather than imply unrestricted background work.

## Delivery gates (still open)

- Typed shared definition and revision-checked command tests, including two
  clients editing different rows and conflicting edits to the same row.
- Convert Mac and Web projections without metadata field loss; iOS/Android must
  use the same wire contract. No per-platform interpretation of user prompts.
- Connect agent and source execution to a real scheduler/worker lifecycle;
  verify cold start, restart, duplicate occurrence, lease expiry and cancellation.
- Real isolated Host tests from creation through a mock execution and history,
  including timezone and script-review revision changes. Never use personal jobs
  or real credentials as acceptance fixtures.
- Svelte and native visual/interaction checks: equivalent controls, localized
  errors, responsive layout, focus restoration and truthful progress states.

The Svelte source-task migration is a step toward these gates, not a substitute
for them. This document intentionally keeps the missing shared executor and
cross-platform task model in scope.

## Implemented foundation in the current migration

`AutomationDefinitionCommand::UpdateIfRevision` adds a shared conditional update
without changing the existing unconditional `Upsert` contract. The Store checks
the expected revision, active row and unchanged format inside the same write
transaction as the row update and sync outbox entry. Conflicts return no committed
projection; they are not successful updates. The Web script-approval and enabled
state endpoints use this command and return HTTP 409 for a stale revision.

The source-task list now exposes `executionCapabilities` explicitly. The current
CLI Host advertises manual execution, no scheduled dispatch, Host-local timing
and no general prompt automations. The Svelte client treats absent capabilities
as unavailable rather than guessing from a task name or format.

Remaining gaps include revision-bound deletion and run requests, idempotent
creation/manual execution, shared typed payloads, and shared dispatch registration. Occurrence calculation is now shared through `QueryAutomationOccurrences`; OS wake registration remains Host-specific.
The Mac full-collection reconciliation path is not converted to conditional
editing by adding this command. These limits remain open even when the Web
approval tests pass.

## 统一发生时间查询（2026-11-02）
发生身份为 automation_id 与 scheduled UTC milliseconds；现有持久 occurrence key 不重写。查询显式提供原计划 IANA timezone、frequency/time/ISO weekday 与有界时间区间；系统调度查询严格未来，错过补跑查询 (last observed, now]，不得将漏跑默认为新任务。DST 重叠选较早 instant，空洞按 offset gap 前移并保持分钟，实际发生时间由同一个 Rust 查询返回；平台仅注册/唤醒。已有计划的 timezone/通知策略不被查询修改；iOS 原有当前设备时区语义在调用时明确传入，不能静默写成 UTC。
