> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 通用 Effect Delivery Attempt 与执行边界

日期：2026-09-09\
目标基线：[`ARCHITECTURE_V2.md`](ARCHITECTURE_V2.md) §§3.1、4.4、20.2、20.3

## 问题与结果

此前 `RuntimeSupervisor` 只保存进程内 Core 缓存，Runtime Driver 每次直接读取 Run 的全部
pending Effect。两个 Driver 或两个进程可以同时取得同一个请求，Store 没有独立的物理投递
身份，也无法区分“已领取但尚未执行”和“外部执行已经开始”。崩溃恢复只能从 Host 事件猜测，
不能可靠地决定重试、续传或进入结果未知。

本次接通独立 `EffectDeliveryAttempt` 账本。逻辑 `effect_id/request_id` 保持稳定，每次物理投递
使用新的 Attempt UUID、递增序号、worker generation、owner、不可预测 token 和到期时间。
Application 在 Effect 所属事务提交后才允许 claim；Host 必须先提交 `ExecutionStarted` 并等待
Application ACK，随后才能跨越外部副作用边界。所有后续响应必须原样回传 delivery lease，
并在同一个 Application 事务中提交 Host event、Core checkpoint、规范 Part/outbox 状态和
Attempt mutation。

这建立的是可验证的单一活跃领取者和结果 fencing，不是外部副作用的 exactly-once 保证。

## 领域状态与不变量

Domain 定义以下状态：

```text
Prepared -> Leased -> ExecutionStarted -> Settled
                    \                  -> Interrupted(Retry | Resume)
                     \                 -> OutcomeUnknown
                      -> Settled(RejectedBeforeExecution)
```

- `Prepared`、`Leased`、`ExecutionStarted` 是活跃状态；同一 `effect_id` 最多一个活跃 Attempt。
- `Settled`、`Interrupted`、`OutcomeUnknown` 是不可变终态。
- `RejectedBeforeExecution` 只能从 `Leased` 产生，没有 execution-start 时间。
- Retry/Resume 不修改旧 Attempt；它们创建带 `predecessor_id` 的新 Attempt，并使用新 lease 身份。
- 旧 generation、owner、token、revision 在 successor 创建后仍永久失效。
- OutcomeUnknown 不进入自动 claim 候选。

Kernel 提供 prepare、claim、execution-start、heartbeat、settle、pre-start reject、interrupt、
outcome-unknown、过期未启动 reclaim 和 Interrupted successor 决策。Kernel 不访问 Store、时钟、
网络或进程；Application 提供当前时间与新身份。

## 持久化与领取

Store schema 20 新增 `effect_delivery_attempts`，保存完整 revisioned record，并物化 run、effect、
state、recovery plan 和 lease 查询字段。部分唯一索引保证每个 Effect 最多一个活跃 Attempt；
run/status keyset 索引支持有界恢复与领取，单次 limit 最大 64。

StorePort 暴露：

- pending Effect delivery candidates；
- active/指定 Attempt 查询；
- Interrupted Retry/Resume candidates；
- 原子批量 claim；
- CAS mutation commit。

Application 的 `claim_committed_host_requests` 先读取已经提交的 outbox 请求和 canonical Core
checkpoint，逐项核对 run/effect/request/replay-safety，再原子提交一批 Attempt mutations。
初次领取由 pending Effect 创建 Attempt；恢复领取只从明确标记 Retry/Resume 的 Interrupted
Attempt 创建 successor。并发 Store 连接可以同时看到候选，但只有一个 claim 能提交成功，
不会把候选读取当作所有权。

Runtime Driver 每个实例生成稳定 worker generation/owner，生产 pump 只调用 claim API，
不再以 `load_committed_requests` 作为执行入口。当前每次领取最多 64 项，租期固定 300 秒。

## Host 执行屏障与响应 fencing

Host request/response envelope 的 `delivery` 包含：

```text
attempt_id
worker_generation
lease_owner
lease_token
lease_expires_at_ms
```

Application Service protocol 已硬切到 17。对 schema 20 的 pending Effect，缺少 delivery、任一
身份字段不匹配、租约到期、旧 Attempt 或错误状态的响应都会在事务提交前拒绝。

CLI Host 在完成静态身份和 deadline 检查后提交 `ExecutionStarted`，收到持久化 ACK 才调用
模型、工具、Skill、MCP、平台或 delegated adapter。所有响应回传同一 delivery；业务序列从
execution-start 后继续。若合法领取在外部执行前已不可执行，例如 deadline 到期，Host 提交
typed `ExecutionRejected`；Application 将 Attempt 结算为 `RejectedBeforeExecution`，Core 将
逻辑 Effect 归一为失败。该路径不产生 execution-start，也不调用外部 adapter。

Application 对终态使用显式 settlement 映射。Failed、取消、delegated 终态、Tool outcome、
Task 终态和已知成功终态分别写入准确结果；新出现但未映射的 terminal Host response 会报错，
不会默认为成功。`ExecutionStarted` 在 Application 边界归一为 Core `Accepted`，但原始 typed
Host event 仍持久化。

## 冷恢复

Application `RecoverOrphanedRuns` 在现有 Run recovery transaction 内读取 Run、checkpoint、
pending outbox 和 Attempt facts。存在 live session lease 时不接管另一 owner。没有 live lease 时：

| Attempt 状态 | 恢复动作 |
|---|---|
| 未到期 `Leased` | 保留，等待原租约结束 |
| 已到期 `Leased` | `Interrupted(Retry)` |
| `ExecutionStarted` + Idempotent | `Interrupted(Retry)` |
| `ExecutionStarted` + Recoverable 且 checkpoint 有非零、身份一致 cursor | `Interrupted(Resume)` |
| `ExecutionStarted` + RequiresRecoveryDecision | `OutcomeUnknown` |
| `Prepared` | 视为损坏事实并拒绝恢复事务 |
| 已有终态 | 保持不变 |

运行中的 typed transport/worker interruption 也在同一 Host-response 事务内关闭当前 Attempt：
Idempotent 进入 Retry，Recoverable 进入 Resume，RequiresRecoveryDecision 进入 OutcomeUnknown。
Retry 可在同一 committed request 上成为候选；Resume 必须等显式 `ResumeHostEffect` 物化 fresh
request ID 后才可领取，避免 Driver 在当前 pump 尾部提前占住并丢弃 successor。

Retry/Resume 在下一 worker generation 的 claim 中创建 successor；OutcomeUnknown 不会自动重发。
恢复 mutation 与 Run/trace 计划、事件和 receipt 使用同一 Store 事务，故障不会留下半套状态。

## 回归证据

本次测试覆盖：

- Kernel 状态转换、终态不可变、旧 owner/token/generation fencing、heartbeat 和 successor；
- 两个独立 SQLite 连接竞争同一候选，只有一个 claim 成功；
- 批量 claim 与 RunEvent/receipt 故障全回滚；
- execution-start、伪造 token、第二 Driver 不可重复领取；
- Retry/Resume 可由下一 generation 领取，OutcomeUnknown 不可领取；
- 冷恢复 Resume cursor、重复恢复幂等及 recovery fault rollback；
- CLI 真实 SessionStore → SQLite claim → Runtime → Host → fake provider 的 start/terminal
  提交拒绝屏障；
- 合法 pre-start rejection 不启动外部进程；
- delegated Host 仍保留更细的 Invocation 与 PermissionDelivery 账本约束。

最终结果：Kernel 102/102、Store 275/275、Runtime Driver 5/5、Application 301/301，均为
0失败/0忽略；`cargo check --workspace --all-targets` 退出0。相关 CLI 屏障、恢复和 delegated
进程专项也通过。日志为 `/tmp/nola-effect-delivery-store-final.log`、
`/tmp/nola-application-full.log` 和 `/tmp/nola-effect-delivery-workspace-final.log`。

## 尚未完成的边界

- Runtime/Application/CLI 已接入租约续期，短TTL静默、拒绝与竞态、真实Service IPC、真实Provider
  HTTP延迟首字节和真实子进程取消均有回归；delegated/permission长静默尚未完成同等故障验收。
- Runtime 仍是调用方触发的 pump，没有常驻 Supervisor lifecycle、持续 wake、backoff、优雅
  shutdown 或跨 Run 公平调度。
- `load_committed_requests` 仍作为兼容/诊断接口存在；生产 Driver 已不通过它执行。
- OutcomeUnknown 还没有连接人工证据核对与处置命令；非幂等外部系统若不支持 delivery token 或
  查询 ACK，无法由本地账本推导 exactly-once。
- RuntimeTask 仍有独立 lease/upsert 所有权缺口，尚未统一到 V2 的 transition-specific 命令。
- 完整 Core 状态所有权、CLI/Android 领域写入、配置与 Device 绑定，以及 §20 五端投影、真实模型、
  长任务和千万字符 UI/性能验收仍未完成。

后续进展：调用方触发pump的限制已由CLI进程级常驻调度第一切片取代。Store schema21的scoped
schedule、Application Service protocol19的提交/deadline长等待、可取消Service Client、独立
Supervisor及CLI warm Host路由已接通。实现、回归证据和仍保留的跨平台限制见
[Runtime Effect Supervisor 第一切片](V2_RUNTIME_EFFECT_SUPERVISOR_2026-09-09.md)。
