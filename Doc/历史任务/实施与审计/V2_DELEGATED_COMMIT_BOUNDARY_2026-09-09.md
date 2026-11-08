> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Delegated 执行提交与恢复边界（2026-09-09）

本阶段保留原工作区修改，未 commit、未转换用户数据库。目标是补齐外部 agent invocation
账本之前的实际执行边界，不将不透明外部调用冒充内部模型采样 Attempt。

## 当前实际接线

- CLI Host 首次 spawn 前提交 Accepted；Application 普通热执行也立即落库。
- worker 使用有界事件通道及每事件独立 Result ACK。Application 对所有 delegated 事件
  强制持久化，不再以低于流式字节预算的热缓冲状态确认；拒绝不能推进 worker。
- Host 保留 JoinHandle，提交拒绝、通道关闭时回收进程树与 worker，并保留原提交错误。
  这不是用户取消，不额外制造 Completed、Failed 或 CancellationConfirmed。
- 冷请求已有 cursor 却没有可核验原 worker 时，记录 typed WorkerUnavailable，保留
  原 Effect/请求而不重发原输入。backend session ID 本身不是安全重执行的证据。
- Codex/OpenCode 恢复失败不再 fallback 新建会话。Codex 恢复回执还需匹配原 thread ID。
- 删除 OpenCode 权限按 profile/kind 自动批准的路径；RPC requestID、原 callID 与原参数
  显式传至 Core。true 表示批准一次，false 为明确拒绝，None/Err 是运输不可用。
  Core 的 conversation grant 仍规范持久化；Host 不给 provider 额外 session grant。
- Store schema17、Application Service 协议14硬切；共享协议 fixture 同步。投影 wire2、
  delegated contract1未改变。

## 回归与定位

真实 Host→进程内 Application 用隔离 SQLite trigger 拒绝 Accepted/SessionStarted/TextDelta。
首次拒绝不启动业务进程，后两项通过真实 OS PID 检查确认父子进程均退出，且拒绝内容不进
canonical inbox、checkpoint、Part，也没有伪完成。另三项实际子进程协议测试覆盖严格
恢复、三种 provider 的 observer 拒绝和 OpenCode 无答复不自动批准。

最初子环境进程采样 EPERM，根环境实际运行后揭示了独立问题：ACK 返回时 delegated
小事件仍仅在 Application 热缓冲，数据库拒绝未被触发。通过进程 sample、真实 inbox 与
协议解析定位，最终修为每 delegated 事件持久化后确认。fixture 同时消费真实 stdin 输入，
正文改发 stream delta（assistant snapshot 被解析器作为重复内容忽略）。没有减弱拒绝或
进程清理断言。临时诊断输出已删除。

最终 delegated 专项7/7通过，包含 Host 事务1、Host→App3、外部协议3，
日志 `/tmp/nola-delegated-durable-verified.log`。Application 冷恢复专项验证 Accepted 与
WorkerUnavailable 持久化，重启维护保留原请求/Effect/cursor，且不创建 model Attempt。
本测试不覆盖 delegated receipt 最终写入故障；数据库注入点是 inbox 事务写入。

## 剩余边界

独立 DelegatedInvocation 账本与 typed projection owner 尚未建立；permission control
的已提交决定投递证据、结果未知副作用恢复、持续 Supervisor/lease 仍需接入。
逐事件持久化有事务开销，后续任何合并必须保持“实际 ACK 前已提交”的契约。
进程回收不能撤销拒绝前外部进程已产生的副作用。以上使用协议 fixture，不代表真实 LLM、
完整五端 UI、长任务或10M性能验收。完整 V2 状态所有权迁移仍继续。

最终强制逐事件持久化后重新验证：Application283、HostProtocol11、Protocol27全部通过，
`/tmp/nola-delegated-durable-application.log`退出0。所有本轮测试与检查进程均已终态。
