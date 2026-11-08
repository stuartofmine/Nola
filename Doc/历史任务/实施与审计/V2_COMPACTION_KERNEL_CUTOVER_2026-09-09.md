> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 压缩决策迁入 Kernel（2026-09-09）

本轮沿实际 `Application → AgentCore → request_compaction` 链路迁移决策。
保留已有工作区修改和 checkpoint JSON；未增加数据库或 Host 写入路径。
这不是整个 Agent turn/model/context 状态机已经纯化的声明。

## 实际边界

- Domain 的 `compaction_request` 持有唯一 `CompactionReason` 与准入事实/决策类型；
  Core 旧公开路径 re-export。Kernel 决定重复请求拒绝、每 Turn 自动压缩两次上限、
  手动目标的 60% 上限与无可压缩历史时的继续/空完成/拒绝动作。
- Domain 的 `compaction_plan` 只携带条目种类、call ID 与估算 token 数，Kernel 不接收正文。
  Kernel 完整选择保留的 system/developer 指令、最近完整轮次，清理不完整 call/result 对，
  并按原序返回保留/待摘要索引。Core 仅映射事实、按索引取原条目。
  摘要预算由 Kernel 统一从硬 prompt 上限扣除保留条目与 framing，软目标不足不能改写最新轮。
- Domain 的 `compaction_lifecycle` 持有唯一重试记录与标量状态；Kernel 决定同 archive
  三次失败上限、改变 archive 后重试重置、自动触发阈值、失败计数与无进展阻断。
  Core 仅应用 typed transition，保持原 provider usage 会计与条目 token 估算。

Core 原选段、摘要预算、准入和生命周期决策副本已删除；不可达的旧尾部裁剪分支一并删除。
Kernel 对 token 求和使用饱和运算，极端数值输入不再溢出 panic；正常历史预算策略保持。
没有从用户文本关键词推导工具路由或授权。

## 单个超大原子项的 Provider 恢复（2026-09-10）

Provider 明确返回 `context_length_exceeded` 时，如果最新完整轮只有一个超预算条目，普通
规划会因保护当前轮而没有可归档前缀。Core 现在只在这个恢复分支建立有界 model-context
投影并立即重试，不再原样重复必然溢出的请求。该投影带规范 source item ID、SHA-256
指纹与原 token 估算；完整正文仍逐字保存在 canonical transcript，可由 ID 定位，不删除或
覆盖规范条目。正文摘录保持原 Message 角色，避免把用户文本提升成 system 指令。

若超大项属于工具事务，Core 以 `call_id` 同时选择 ToolCall 和 ToolResult，二者保持原身份、
顺序和配对；结果继续保留 typed `full_output_ref` 与 Artifact 引用。恢复视图使用有界首尾
预览，checkpoint 只记录来源元数据。该兜底不改变正常阈值压缩和手动压缩，也不从文本
关键词判断是否启用。Core 在写入 checkpoint 和重试前强制验证工作集估算 token 严格下降且
不超过 prompt 上限；无法取得该进展时直接拒绝恢复，避免形成相同请求的 overflow 循环。

## 仍留在旧 Core 的职责

上下文配置归一化、token 估算/provider usage 会计、压缩摘要内容校验与指纹、压缩提示契约、
AgentItem 和完整 turn/model 状态机，以及事件/Host Effect 的装配仍在旧 Core。
Application 与 runtime-driver 仍依赖旧 Core。后续需继续迁移这些真实边界，不能以新增
纯函数模块代替完整纯 Kernel 与单写者验收。

## 验证

Kernel 压缩决策 7 项通过，包含约 5.6 万组短历史组合的分区/指令/当前用户/配对不变量，
具体预算边界、自动次数及重试/无进展状态转换。Core 原 Host 合约 69 项及压缩提示、
重试 checkpoint、摘要预算 7 项通过；另 Core planner/生命周期专项由子任务验证。
Application 新增真实压缩事务测试通过：空/超预算摘要与非法 usage 拒绝后原 checkpoint、
消息/Part 和 outbox 不变；最终 receipt 故障完整回滚；合法摘要提交保留原 suffix/transcript；
冷开库只清理已 ACK 的 Compact effect，随后 receipt 重放不改变任何规范行或 pending Effect。
初版 fixture 的历史短于软保留目标，因此正确地没有生成 Compact；调为真实超限历史后验收。
测试另外区分启动清理与收据重放，未放宽重放幂等断言。

最终 Application 269/269、workspace all-targets check 通过。
日志：`/tmp/nola-compaction-kernel-root.log`、`/tmp/nola-compaction-core-final.log`、
`/tmp/nola-compaction-kernel-receipt.log`、`/tmp/nola-compaction-application-final.log`、
`/tmp/nola-compaction-workspace-final.log`。这是 Rust 决策与事务迁移验证，没有本阶段新增
原生 UI 或真实模型/完整跨进程恢复验收。
