> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 模型完成与 Turn 决策迁入 Kernel

日期：2026-09-09。以未提交工作区为准，保留此前全部修改。此记录是一段职责迁移，
不是 `ARCHITECTURE_V2.md` 的整体完成报告。

## 实际调用与所有权

`core/src/agent/runtime/submission.rs` 的 `ModelCompleted` 主分支现在调用
`nola_kernel::plan_model_completion`。Domain 提供显式 facts、动作和状态更新；Kernel
决定重复输出、截断恢复、拒绝后解释、审核、仅推理/空响应、未完成工程或 Skill 义务、
最终综合及普通完成的优先级与重试计数。Core 解析模型内容、应用更新并装配事件和 Effect。

`runtime/turn_lifecycle.rs` 调用 Kernel 的输入准入、完成及取消计划：当前轮输入先继续
原 Turn，下一轮输入只在原 Turn 完成后启动；取消只选择属于当前 Turn 的可取消 Effect
和既有规则允许的工具状态。没有改变输入来源权限、上下文捕获或取消状态集合。
`AgentTurnStatus` 与 `ToolCallStatus` 的唯一类型定义迁入 Domain，Core 保留重导出路径。

`AgentItem::estimated_tokens` 改为借用 facts 的适配。Kernel 使用 `serde_json::to_writer`
向字符计数器序列化，保留原 JSON 转义及 Unicode 字符口径，消除完整 JSON 字符串副本。
使用标准内存 writer 接口，不访问文件或其他外部资源。序列化递归栈仍随嵌套深度变化。

## 验证证据

接续时核对到以下最终日志；它们不是本次架构阅读时重新运行的测试：

| 范围 | 通过数 | 日志 |
| --- | ---: | --- |
| Kernel 全量，含估算分配测试 | 74 | `/tmp/nola-model-transition-kernel-final.log` |
| Application 全量 | 270 | `/tmp/nola-model-transition-application-final.log` |
| Core Host contract | 70 | `/tmp/nola-model-transition-core-final.log` |

均记录零失败、零忽略。本次复核格式化新增决策模块，并重新执行
`cargo check --workspace --all-targets --quiet`，退出码 0。

关键测试入口：

- `core/nola-kernel/src/model_completion.rs`：重叠条件的优先级、重试阈值和状态标记。
- `core/nola-kernel/src/turn_transition.rs`：输入来源/状态、当前轮和下一轮顺序、取消对象。
- `core/nola-kernel/src/item_token_estimate.rs`：转义、Unicode、数字及嵌套 JSON 的原公式 oracle。
- `core/nola-kernel/tests/item_token_estimate_allocation.rs`：大 JSON 估算时的分配约束。
- `core/nola-application/tests/model_completion_kernel_receipt_e2e.rs`：真实 Application 提交
  当前轮和下一轮输入，连续三次模型完成，逐次冷 receipt 重放；比较 checkpoint、消息、Part
  和剩余 outbox，保证不重复模型 Effect。

没有新增真实 LLM 或平台 UI 验收，不能由这些测试推断五端一致性已完成。

## 下一处需要闭环的身份边界

上述 Application 测试证实：下一轮输入会创建不同 Core Turn，但 Store 中仍只有原 Run。
因此 Store Run ID、Core Turn ID、trace run ID 不能相互替代。

`NolaMac/UI/MacChatWorkspaceView.swift` 的无输入恢复路径当前比较
`trace.turnID == expectedRunID`，并对终态使用同样条件。它的“Core turnID 就是 canonical
Run ID”注释不适用于后继 Turn。现有首次 Turn 恢复专项没有覆盖这一情况。

下一步应先用真实 Application→Host 链复现后继 Turn 的权限等待与冷恢复，明确恢复响应
中的规范 Run 与 Core Turn 关联，再调整投影消费和终态判断。不能直接放宽身份过滤，
也不能未经领域契约核对就为每个 Core Turn 创建新 Store Run。

## 尚未迁出的职责

旧 Core 仍持有完整运行态、命令处理、事件/Effect 装配、模型文本分析、重复工具批次恢复、
provider metadata 和工具曝光等逻辑。Application 与 runtime-driver 仍生产依赖旧 Core。
持续 Supervisor、冷配置/Device 恢复、CLI/Android 领域所有权及主架构 §20 验收继续未完成。
