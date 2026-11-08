> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Runtime task / workspace ownership Port cutover

本批迁移已有后台任务命令与租约 helper 的真实调用，不改任务状态机、调度算法、权限路由或协议字段。

## 已迁生产边界

- `nola-store-port/runtime_task.rs` 拥有 `StoredRuntimeTask`、`StoredRuntimeTaskEvent` 及 `RuntimeTaskRepository`。23 个接口覆盖 Application 当前任务命令实际调用的查询、创建与事件、claim/fair claim/permission wait、heartbeat、retry、终态事件、取消、过期恢复、通知与游标，以及 progress 写回。
- `nola-application/runtime_task.rs` 的 `execute_runtime_task_command` 与 `report_runtime_task_progress` 已使用 `R: RuntimeTaskRepository + ?Sized`，没有具体 Store 参数或 DTO 引用。`ApplicationCommand::RuntimeTask` 继续调用这个生产 dispatcher；不是测试专用替代路径。
- `nola-store-port/workspace_ownership.rs` 拥有 `StoredWorkspaceCheckpoint`、`StoredSessionRunLease` 及 `WorkspaceOwnershipRepository`。Application 现有保存/列举 checkpoint、获取/查询/列举 Run lease 五个 helper 已泛型化。
- Store contracts 保留四 DTO 的兼容 reexport，真正定义归 Port。两个 SQLite adapter 委托当前同一 `NolaStore` 实例的既有方法，没有另开连接或提交外层 command receipt UoW。

公平优先级、项目队首、等待老化、全局/项目槽位、lease token、单调 progress 与预算耗尽策略均沿用原实现。创建与终态事件仍使用原 SQL 原子操作，receipt 失败回滚仍由原事务所有者完成。

## 验证

- Application runtime task unit 4/4：原 scheduling/metadata 两项；新增非 SQLite 仓库调用真实 Create/Get/ReportProgress，验证写入失败不污染仓库、错误 owner 被拒、预算超限保留明确原因、Host metadata 不丢失、计数倒退被拒。fixture 的 running state 明确预置，不把它声称为完整的非 SQLite fair scheduler。
- 新真实 Runtime 测试在隔离数据库故障注入最终 receipt UPDATE：创建的 task/event 同时回滚；正常创建后 fair claim 的 receipt 失败使任务仍 queued；移除故障后重试并关闭/重开，exact receipt 回放保持原 owner，不重复事件。
- Store `runtime_task_e2e` 8/8：真实持久化与重启、租约隔离、过期恢复、优先级/FIFO/老化、项目与全局槽位、permission wait 恢复、并发 worker 不超配、持久化 event cursor。
- Application `workspace_ownership_port_e2e` 1/1：真实 Application 命令的 checkpoint 与租约跨关闭/重开保持。

最终 `cargo check --workspace --all-targets --quiet` 通过（session 18604，exit 0）；本批测试共 13 项通过，无运行中的测试或环境阻塞。

## 剩余边界

Runtime 与统一 Application dispatcher 仍明确持有具体 Store；本批没有修改它们。其他业务模块直接使用的 task read cursor、workspace checkpoint 删除、lease release 等未归入上述五个 helper，不能据此宣称所有任务/租约访问均已去掉具体依赖。OS 进程探测、执行器以及客户端恢复策略也未移动。下一步将其他实际调用与 command UoW 合成聚合 Port 时，必须继续保持相同事务所有者，不以多连接组合替代。
