> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Tool batch decision moved into pure Kernel

本批实际生产路径为：Application HostResponse reducer → 旧 `AgentRuntime::tool_driver` → `plan_next_tool_batch` 的事实适配 → `nola_kernel::plan_tool_batch`。工具批次选择算法已从旧 Core 删除；不是仅新增未被调用的 Kernel API。

Domain 输入只有 call ID、有序候选状态，以及 capability 的 available/is_tool/concurrent_safe 三项事实。没有 Host catalog、请求 envelope、可执行参数、数据库、时间或平台资源。`Ready` 与已终态调用映射为 Inactive，保持旧实现只新选择 Pending 的语义。

Kernel 决定全部选择规则：任何位置存在 Running/AwaitingPermission 均阻止批次，即使屏障位于第 4 个候选之后；未知、不可用、非 Tool、串行能力作为单项选择；并发只取 Pending 序列的连续安全前缀，上限 4，维持原输入顺序。该选择不构成权限授权或能力执行许可，后续现有 driver 准入仍处理具体工具。

Core 适配器只读 Host descriptor 原事实、映射生命周期类型并转换返回的 call IDs，没有第二份选择算法。Core 增加 Domain/Kernel 依赖，而 Kernel 仍仅依赖 Domain；没有反向依赖 Host Protocol 或旧 Core。原公开 Core API 与 call ID 类型不变，`tool_driver` 的实际调用点不变。

验证：Kernel 四项覆盖三事实所有 8 种组合、屏障所有位置、3125 种五元素状态/串并行组合、4+1 与 inactive 间隙。旧 Core 两项覆盖八种 lifecycle 状态、所有 unavailable 变体、unknown、非 Tool 和 serial 的真实 Host descriptor 适配。Application outbox/receipt/冷恢复实证由另一代理独立补充，不把上述纯函数测试等同于 Effect 发布验收。

本批次选择迁移不等于整套 Agent turn/permission/model/compaction reducer 已迁移。同期 root 对 Effect 时间输入、另一子任务对权限 origin 的修复见下节。

## 同期生产时间输入与权限修复

Application 普通/maintenance 创建使用 created_at_ms，Host response 使用 received_at_ms，
其他 active/idle/revert 提交使用 committed_at_ms，传入 `AgentCore::submit_at`。
Core runtime 为每次提交持有这一显式观察时间，Effect 创建与重放不再在
`runtime/effect_dispatch.rs` 内读取 SystemTime。独立 Core 调用者的 `submit` 仍是采时 facade，
它委托 submit_at；`resume_committed_host_effect_at` 提供显式恢复时间。该 facade 不是纯 Kernel，
本阶段未宣称所有旧 Core 入口均已迁移或没有其他非纯依赖。

Core 的 observed_at_ms 是单次 reducer 输入，不持久化进 checkpoint；Application 拒绝负数
观察时间。canonical context rebase 不产生时间相关 Host Effect，使用固定逻辑时间 0。
固定初始身份/状态、输入与观察时间时，独立 Application 创建得到完整相同的已提交
Host envelope；冷恢复重放时间为 1234 时 model deadline 为 601234，u64 上界安全饱和。
真实 run_effect/turn_transaction 专项 4+15 通过：`/tmp/nola-decision-clock-application.log`。

权限同批重评原来隐式使用 InteractiveUser，现保留 pending turn 的真实 input_origin。
用户直接批准的当前调用可继续；scheduled/replay 的同批其他调用仍不能继承 Conversation/Turn
授权。实际 Core 与既有 contract 19/19 通过；未新增词表或修改 profile classifier。
旧 recommended profile 工具名称分类、路径/shell scope 匹配和整套权限纯化仍待独立切换。

## 实际 Effect 发布验证

新增 `tool_batch_receipt_e2e` 3/3 通过：真实 ModelCompleted 经新 Kernel 决定 4→1 批次、
并发前缀后串行、并发前缀后权限屏障；逐个 completion 期间不能提前发布下一批。
最终 receipt 故障时 Effect 表保持原状态；冷重启时完整 request/effect/call 身份、顺序和
未解决权限均保持，没有把纯函数测试当作持久化或 Host 执行证据。

最终 workspace all-targets 编译通过（`/tmp/nola-kernel-batch-workspace.log`）；Application
全量 **260/260，0 失败、0 忽略**（`/tmp/nola-kernel-batch-application-full.log`），均 exit 0。
完整 V2 仍未完成。

真实 Service 2048-delta 长流、RSS、terminal Core 淘汰/接收端重启专项 1/1 通过，
`/tmp/nola-kernel-batch-service-stream.log`，exit 0。root 与子任务全部编译/测试句柄已终态。
