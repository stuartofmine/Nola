> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Application Runtime 仓库聚合切换：2026-09-08

本记录对应 ARCHITECTURE_V2 的业务事务所有权与依赖倒置，保留全部未提交工作区修改。
完整 V2 尚未完成，本记录不替代 §20 验收。

## 最终生产边界

`nola_application::ApplicationRuntime<R: ApplicationRepositories>` 不设具体 Store 默认参数。
完整 `execute_application_command`、receipt admission/commit/rollback、projection journal/feed、
content upload 分发均使用同一泛型仓库实例。具体 SQLite 类型由
`nola_application_bootstrap::ApplicationRuntime` alias 选择，Service、mobile-bindings 与 CLI
通过该 composition 接线；没有修改既有 writer 锁或数据库路径。

Application Cargo 的 `nola_store` 已移至 dev-dependencies，源码中的 Store 引用只用于测试。
实际 `cargo tree -p nola_application --edges normal` 中无 `nola_store`、`rusqlite` 或 `libsqlite3`。
旧 `nola_core` 仍是正常依赖，纯 Kernel 替换并未因此完成。

聚合接口覆盖完整命令面；各业务 helper 保留更窄的 repository bounds。
聚合接口没有拆分数据库连接：receipt 原子事务、恢复一致读、窗口一致读都作用于同一 R。
Snapshot trait 保留原 FnOnce(&Self) 读取作用域，SQLite adapter 委托现有连接/savepoint。
独立 FindResumableGoalSession 查询也加入同一 consistent read，避免候选与来源选择跨版本。

## 本轮补齐的业务切面

- Remote turn、projection、mirror 和 full session 查询：5 DTO；来源判定与记录读取同快照，远程 projection/grants 同 receipt 原子提交。
- Resume：候选 metadata + 聚合恢复 facts 同快照，mailbox ack 保留 revision/CAS，不再通过全文加载寻找候选。
- SOP：Core 保留状态转换，Port 执行 pinned definition/revision 提交；branch 直接使用既有 Domain intent/result。
- Idle checkpoint：真实 Core helper 通过 IdleCheckpointRepository，将记忆输入版本条件、输出和 checkpoint 一次提交。
- Queue、trace、settings：6 DTO 归 Port，dispatcher 显式通过仓库调用；checked setting helper 保留 admission 与 revision 检查。
- Conversation query、Store maintenance、archive projection metadata：补齐完整 Runtime 原有方法需求，均复用原连接。

## 验证范围与剩余项

已通过的专项包含远程 owner/cursor/grants/receipt、恢复跨 writer 一致快照、SOP、队列、trace、
settings、idle memory；每个具体结果随最终整合补录。
Bootstrap/Service/mobile all-targets 编译通过，bootstrap 4、mobile 25 通过，包括跨进程 writer 排他与恢复。
CLI all-targets 编译通过，Application Port 5、MCP catalog 2 通过。
真实 Service 2048-delta 长流、RSS、terminal Core 淘汰和接收端重启专项 1/1 通过：
`/tmp/nola-runtime-generic-service-stream-final.log`（预算修复后再次通过）。

最终 workspace all-targets check 已通过：`/tmp/nola-runtime-generic-workspace-check.log`。
Application 首轮完整 no-fail-fast 审计 254 通过、1 失败、0 忽略，日志
`/tmp/nola-runtime-generic-application-audit.log`。唯一失败是旧 local_projection_window fixture
的 checkpoint-only 10M 呈现：规范 Part 为空，旧读取先将超大字段交给 App 后才预算拒绝。
读取边界已新增 SQL admission：超限时不构造并返回巨大的聚合 JSON，明确报错而非回退空正文。
10M checkpoint-only 拒绝路径专项已通过，Rust 最大单次分配 1346 bytes、峰值 2520 bytes，
原 checkpoint 字节未变；SQLite 内部解析/扫描工作并未证明常量。
日志 `/tmp/nola-local-window-checkpoint-budget.log`。本修复 local window 2、projection lib 9、投影/remote/window/resume integration 14 均通过，
其中包含控制字符、Unicode、quote/backslash 和重复 key 在 order/map 中的编码预算验证。
修复后 workspace all-targets check 已再次通过：`/tmp/nola-runtime-generic-workspace-final.log`。
最终完整 Application 复验 **256/256 通过，0 失败、0 忽略**（含 lib 97）：
`/tmp/nola-runtime-generic-application-final.log`，exit 0。该结果覆盖前述 254+1 的旧失败阶段。

下一架构重点是纯 Kernel 与旧 Core 的实际执行所有权、客户端业务状态/存储迁移，以及
五端权限、长任务、真实模型和 10M 会话 UI/性能验收。不能仅以删除 Store 依赖判断完整 V2 完成。

## 下一生产切面：工具批次规划

只读调用链审计：Application 创建/恢复 AgentCore 后仍经 `AgentCore::submit` 委托旧
`AgentRuntime`；工具批次由 `core/src/agent/runtime/tool_driver.rs` 调用旧
`core/src/agent/tool_scheduler.rs` 决定，权限、模型规划和压缩也尚在旧 Core。现纯 Kernel 已包含消息/Part
准入、conversation transition、projection diff/reducer 与 semantic binding，不能等同完整 Agent Kernel。

下一步把工具批次选择完整算法移入 Kernel，Domain 仅携带有序 call ID、生命周期状态、
已解析的 capability availability/route/concurrent-safe 属性。生产 tool driver 实际调用新算法，
删除旧选择实现；验证 running/permission-wait 屏障、未知能力串行、混合串并行、模型顺序与
4+1 分批，并以 HostResponse→durable outbox→冷恢复证明 Effect 身份和发布时序不变。

随后处理权限与 tool lifecycle 的纯决策映射，以及把 effect_dispatch 内系统时间读取改为
显式输入。旧 reducer 所查路径主要返回 Effect，不能笼统描述为它直接执行网络或工具；
问题是时间和 Host 传输契约仍混入决策层。完整 turn/model/compaction 状态机之后继续迁移。

本轮 root 与子任务启动的编译/测试全部已终态，无待续进程。完整 V2 目标仍未完成。
