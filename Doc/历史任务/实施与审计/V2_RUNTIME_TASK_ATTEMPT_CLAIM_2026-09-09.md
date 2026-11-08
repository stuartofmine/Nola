> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# RuntimeTask Attempt Claim Cutover

The first RuntimeTask ownership slice now persists a distinct attempt for every
successful claim. `runtime_task_attempts` records an opaque UUID, a monotonically
increasing per-task attempt number, worker identity/generation, lease token and
expiry, state, revision, and claim time. The task-row update and attempt insert
share one SQLite transaction. Losing concurrent claims create no attempt.

`RuntimeTaskRepository::claim_runtime_task` and `claim_runtime_task_fair` return
`Option<RuntimeTaskAttempt>` instead of a boolean. The serialized Application
response is therefore the full lease identity or `null`. Claim requests require
an explicit bounded `worker_generation` and name the opaque secret
`lease_token`. The CLI claim API returns the typed attempt and each production
caller supplies a process-scoped generation. External and MCP lease guards retain the
returned attempt, while shell and agent subprocess requests carry it across the
process boundary. The Application rejects missing, non-canonical, controlled,
or overlong generation and lease identities.

Reclaiming after a retry atomically marks the prior leased attempt `superseded`
and increments its revision before inserting the successor. Reusing the same
lease token cannot recreate the old attempt identity. This prepares renew,
progress, permission, retry, and completion commands to require exact
`attempt_id + revision + worker_generation + lease_token + expiry` matching.

The canonical SQLite schema is version 23. This repository deliberately does
not migrate older canonical databases; opening an older schema continues to
return the existing recreate-required invariant. The Application Service wire
is version 24. Version 22 introduced the typed Claim response; version 23
replaced the legacy task/token heartbeat with exact-attempt `renew_lease`, and
version 24 replaced task/token progress with exact-attempt `report_progress`.

Lease renewal now carries `task_id`, `attempt_id`, expected attempt revision,
worker generation, lease token, current expiry, observation time, and requested
expiry. Application validation rejects expired, non-growing, non-canonical,
foreign, stale, or superseded identities and caps one lease horizon at ten
minutes. Store renewal uses one transaction: it CAS-updates the attempt expiry
and revision, CAS-updates the task heartbeat and lease projection, reads the
exact resulting attempt inside that transaction, and commits both together.
CLI heartbeat loops retain the returned attempt and replace their local
revision/expiry after every ACK; a failed renewal cancels or terminates the
owned execution.

Progress now carries the same attempt identity, expected revision, generation,
token, and current expiry. The Store first advances the attempt revision with
an exact live-lease CAS, then updates only progress, activity, and metadata on
the running task projection in the same transaction. Cancellation, expiry,
renewal, or another progress update makes the whole transition lose its CAS;
receipt persistence failure rolls both rows back. CLI workers install each
successful Claim attempt in a process-local synchronized lease state. Renew
and progress hold the same per-task mutex through the Service ACK and roll the
shared attempt forward, so concurrent heartbeat and progress calls cannot copy
and submit one stale revision. Terminal/cancel cleanup removes the entry only
when its attempt id still matches, preventing an old worker from deleting a
newer ABA successor.

Remaining transition order:

1. Require the same identity and revision for permission suspend.
2. Settle or interrupt the attempt atomically with retry and completion.
3. Thread the retained identity into every remaining downstream transition CAS.
4. Move output to ordered chunks and typed artifact references.

Focused evidence:

- `cargo test -p nola_store --test runtime_task_e2e --quiet`
- `cargo test -p nola_application --lib runtime_task::tests --quiet`
- `cargo test -p nola_protocol --lib --quiet`
- `cargo test -p nola_service --test runtime_task_worker_recovery_e2e --quiet`
- `cargo test -p nola_service --test permission_request_cold_restart_e2e --quiet`
- `cargo test -p nola_cli --bin nola store::task::tests --quiet`
