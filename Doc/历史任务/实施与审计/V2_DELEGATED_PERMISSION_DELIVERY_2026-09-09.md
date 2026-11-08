> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Delegated 权限投递账本与提交边界（2026-09-09）

本阶段保留原工作区修改，没有 commit、转换用户数据库或清理历史文件。实现承接
[权限控制投递审计](V2_DELEGATED_PERMISSION_CONTROL_AUDIT_2026-09-09.md)，目标是将
“授权决定已提交”“worker 已领取”“回复已写入协议运输”分成可验证事实。

## 已接通的规范事实

- Domain 和 Kernel 新增 `DelegatedPermissionDeliveryRecord` 及 CAS 迁移。状态为
  `AwaitingDecision`、`Prepared`、`Claimed`、`TransportWritten`、`OutcomeUnknown` 和
  `Interrupted`。`TransportWritten` 只证明协议回复已经 write/flush，不代表 backend 已执行。
- 每个权限 occurrence 绑定 Run、独立 DelegatedInvocation、target effect、backend、规范
  backend session、worker generation、occurrence、RPC request、call、完整结构化 input 及其摘要。
  决定使用 typed `AllowOnce`、`AllowConversation` 或 `Deny`，不能用提示词或字符串推导授权。
- Store schema 19 持久化独立账本、按 occurrence/control/target 的有界索引和逐 revision
  `delegated_permission_changed` 事件。Begin、Commit 和 recovery plan 在同一事务写入账本、
  outbox、checkpoint、事件与 command receipt；任一最终写入失败会一起回滚。
- Application 在 `PermissionRequested` 时创建 `AwaitingDecision`；权限结果与控制 outbox 同事务
  变成 `Prepared` 并绑定 delivery。只有 `Prepared` 可冷加载发送。Host 的 Claim 必须先经
  Application 提交并收到 ACK，worker 才可把决定交给外部协议；write/flush 后再提交
  `TransportWritten`。
- Claim 后进程或提交边界中断时，恢复写 `OutcomeUnknown` 并禁止自动重发。Claim 前的开放记录
  恢复为 `Interrupted`。已有 `TransportWritten`、`OutcomeUnknown` 或 `Interrupted` 证据不降级。
- Host protocol 显式携带 invocation、generation、occurrence、call、RPC 与 input digest。
  缺失或任一字段不匹配都在外部协议收到决定前拒绝；旧的泛化
  `permission_resolution_delivered` 完成回应不再表示权限已送达。
- UI 权限请求 ID 使用 occurrence ID，原 backend RPC ID 单独保存在 continuation。即使 backend
  在同一 session 重用 RPC/call，新 occurrence 也不会消费上一轮授权；Custom 指导轮换保持原
  backend 目标不变。
- 权限控制 effect 保留 target 关联但不触发取消。只有真正的 Cancel/CancelTask 请求可令父
  invocation 进入 Cancelling；权限控制采用 Recoverable 投递语义。

## 已验证的边界

- Domain/Kernel 权限迁移 5/5；Store 完整 267/267，权限账本专项及内存端口通过。
- Host protocol 12/12、Core Host 71/71、CLI delegated 11/11。真实 OpenCode/Codex 子进程
  回归覆盖 Claim 提交拒绝前零决定字节，以及 write/flush 后 Written 提交拒绝并回收进程树。
- Application SQLite 专项覆盖完整四个 revision、伪造 Claim、未 Claim 的 Written、receipt
  最终写入回滚、冷 replay、Claim 后恢复为 OutcomeUnknown，以及 DelegatedInvocation 与
  ModelAttempt 的 owner 隔离。恢复端口 7/7 覆盖所有状态、损坏身份的整批拒绝和活 lease 跳过。
- Store schema 为 19，Application Service protocol 为 16；共享协议 fixture 已同步硬切，
  不隐式迁移不兼容用户库。

## 能力边界和后续工作

外部 backend 当前没有幂等 delivery token 或可查询的应用确认，所以本实现不能声称
exactly-once，也不能把 `TransportWritten` 称作 `BackendAcknowledged`。`OutcomeUnknown`
需要后续产品级观察、人工处置或 backend 提供的真实证据；自动重发会重复授权副作用。

本阶段没有完成持续 Supervisor/lease、一般副作用的结果未知恢复、完整 Core 状态所有权、
CLI/Android 领域写入迁移、Device 配置绑定，以及主架构 §20 的五端、真实模型、长任务和
10M UI/性能验收。完整 V2 继续进行。
