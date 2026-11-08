> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 当前 Run 恢复决策迁移

2026-09-09。保留未提交工作区；本记录覆盖启动恢复职责迁移，不代表完整 V2 完成。

## 修复的真实行为

旧 Store 从一个 Run 的任意最后 Turn 终态事件推断整个 Run 终态，只对权限等待添加例外。
真实 Application 回归证明：前轮完成后，后继 Turn 仍为 AwaitingModel，且有已提交 ModelStream，
执行恢复却会把原 Run 标为 completed。会话级恢复还绕过现存 lease；普通 execute 中 Run 与
trace 的两次维护分别提交，存在部分成功的边界。

现在 `RecoverOrphanedRuns` 调用 Application 的 `run_recovery::recover`。Store Port 的
`with_run_recovery_plan` 在同一个写事务/外层 receipt savepoint 内逐会话读取事实、调用规划器
并应用计划。旧 `recover_orphaned_canonical_runs`、`interrupt_trace_journal_runs` 和 Store
`permission_recovery.rs` 算法删除，没有保留旧实现作 fallback。

## 当前边界

- Store 提供当前 Run、checkpoint、未 ACK Effect、租约、最近 TurnStarted 身份和未结束 trace。
  使用 conversation keyset，不先加载所有会话 checkpoint；计划失败回滚整个操作，已有 receipt
  事务不会被内部提交。Store 不解析 Core 状态，也不决定权限或恢复优先级。
- Application 校验配置摘要、checkpoint owner/revision/cursor、当前 Turn 归属、journal/outbox
  完整集合、请求身份/Host/lease/catalog/payload，以及权限待办的原参数和指令。首次 Turn 由
  Begin 创建时的 Run ID 证明归属；后继 Turn 必须匹配已提交的 TurnStarted。存储中有坏事件
  不等同于“没有首轮事件”，缺失或 malformed Turn ID 会被明确拒绝。
- Kernel 接收 Domain facts，决定 Preserve、Finalize 或 Reject。有 lease（含 scoped 恢复）时
  不改 owner 状态；无本地 invocation/outbox 的 remote Run 不由本地维护结算。有效当前状态仍
  有 pending Effect 时保留；只有当前 Turn 已终态且无 pending Effect 才可结算。
- 无效 checkpoint 或不一致请求返回错误并回滚，不借旧 Turn 的终态伪造成功。有效本地状态
  无 pending Effect 且未终态时同样明确拒绝，需要补齐其生命周期契约，不能自动清空等待。
- trace 通过已验证当前 Core Turn 关联，诊断 run ID 不参与规范 Run 身份判断；有 live owner 时
  所有 trace 保留。已结算 Run 的 hot overlay 只在事务提交后移除，receipt 失败不提前丢缓存。

保留不代表授权重执行。此操作不启动 Provider/Tool、不创建 Attempt、不抢占 lease、不重绑
Device，也不保证已接受且结果未知的副作用工具可自动重试。

## 验证

| 范围 | 结果 | 日志 |
| --- | --- | --- |
| Application 全量 | 279/279 | `/tmp/nola-run-recovery-application-verified.log` |
| Kernel 全量（含 allocation integration） | 77/77 | `/tmp/nola-run-recovery-kernel-verified.log` |
| Store 恢复契约 | 5/5 | `/tmp/nola-run-recovery-store-verified.log` |
| 非 SQLite Application 恢复用例 | 3/3 | `/tmp/nola-run-recovery-port-verified.log` |
| CLI 冻结绑定/恢复入口 | 3/3 | `/tmp/nola-run-recovery-cli-verified.log` |
| HTTP 首次与后继 Turn 恢复 | 2/2 | `/tmp/nola-run-recovery-handler-verified.log` |
| 真实 Service 进程 IPC | 1/1 | `/tmp/nola-run-recovery-service-verified.log` |
| workspace all-targets check | exit 0 | `/tmp/nola-run-recovery-workspace-verified.log` |

无失败/忽略。非 SQLite 测试包含于 Application 全量；最后的 malformed TurnStarted 字段
区分另由 Store 5 项和非 SQLite 3 项复验，最终全目标检查也已覆盖。

新 Application 场景覆盖后继 Model、scoped lease、损坏 checkpoint、trace UPDATE 故障、
最终 receipt 故障和冷重放。Store 测试验证第二连接在规划事务中不能改 checkpoint、跨会话
回调失败全回滚、savepoint 不提交外层事务，并证明历史终态不决定 Store 的写入计划。

Service 测试所有建 Run、排队 NextTurn、接受 Host 结果、恢复和 receipt 重放均走真实子进程
IPC；两次重启后继续原模型 Effect，最终只有一个 Store Run 和两条 Turn completed。只读 SQL
用于最终断言，没有直接预置 Application/Store 状态。它使用构造的 Host 模型结果，不是真实 LLM。
本阶段没有重新执行平台 UI/截图验收，不能引用上一阶段 Mac 22 项作为本轮原生验证。

## 剩余目标

目前 schema 仍缺完整 V2 Attempt/activeAttemptID；请求级 Part owner 不等同 Attempt 隔离。
持续 Supervisor 的 wake-up、lease/heartbeat/expiry、结果未知副作用的恢复，以及版本化配置/
Device 绑定仍未完成。完整 Core 状态、命令与事件/Effect 所有权、CLI/Android 领域写入边界和
主架构 §20 的五端、真实模型、长任务、10M UI/性能验收继续推进。
