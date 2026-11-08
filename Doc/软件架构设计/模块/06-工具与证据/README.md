> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# MOD-06：Tool Host / Evidence

状态：现行模块职责；2026-11-08 核对当前实现入口及依赖。规范要求、静态接线与历史运行证据分别说明，不将未复测路径标记为通过。

## 设计、输入输出与生命周期

2026-10-16 细化：执行回执、ModelEvidence、UIControl 分离；预览令牌不得进入模型、压缩或历史召回。浏览器动作与观察分开，owner/page 丢失失败关闭；结构化快照按范围读取，语义进展不摘要随机输出包装。详见[根因边界](../../总体设计/执行权限与证据边界_20261016.md)。

- 输入：已准入 ToolCall（schema revision、资源、执行权限、超时、取消 scope）。输出：类型化 ToolResult。
- 现有回执包含 `state, effects[], checks[]`；必须区分调用状态、业务数据、展示摘要、完整结果引用。
- 每个 effect：真实规范化资源、操作、观察到的 before/after version；缺失版本要有证据限制原因。
- 每个 check：资源/版本、predicate、passed、观察来源。声明范围不等于观察结果，exit=0 不证明任务完成。
- 组合操作能证明时报告完整顺序；不能证明时如实输出，不能由标签获得验证信用。
- Core 消费已核对身份的 Host observation，提出证据及义务变化；Application 将原 Host
  输入、Core 事件、checkpoint 与后继 effect 放在同一事务提交。不是先提交回执、再单独
  修改义务的两次业务写入；Host 不直接清除验证状态。
- 完整结果存储受会话/数据库边界控制；大输出提供有界预览与范围 reader，不将截断后的文本当完整证据。
- 验证：顺序、并行、委托、重启返回同一契约；范围未知/版本未知/部分失败/用户拒绝分别有出口。

执行回执类型的唯一代码所有者为 Domain `execution_evidence`；Host 协议使用同一类型，
不维护第二份 receipt schema。`ExecutionEvidenceObservation` 保留裁剪前 receipt 与原 Host
outcome，绑定 call、request、effect、host、attempt 和 response sequence。Core 事件 envelope
提供 session/turn/revision/cursor；payload 的相关 ID 只是相关性，不是授权或真实性证明。
缺 typed receipt 保留 None，缺写后版本保留 None，不用成功返回值补成已验证。
生产封装：ToolResult.execution 现为必填 nullable 字段，null 表示没有 Host 回执，
对象中的 state=unknown 表示显式报告未知。缺字段拒绝，不设兼容默认；CLI 保留
原 Option，Core 投影缺回执时移除业务数据伪造的保留字段而不补造回执。
HostToolOutcome 成功、无回执/ExecutionState 未知、Task 未完成可以合法并存。
Deferred 必须有相应回执；明确 NotStarted/Partial 或失败检查不能伪装成功。
跨模块校验使用同一契约，不能封装允许 Unknown 却在 Core 准入将其一律拒绝。
证据索引与消费它的 Core transition 同事务形成，并核验所引用的原 Host 输入；并行乱序缓冲
可能引用此前已提交的 Host inbox，不能全部绑定到本次触发刷新的一条回执。不能从 UI Part、
模型摘要或最新 checkpoint 反推来源。索引保存来源及摘要，不复制完整工具输出，
不自动产生 requirement acceptance。
逐项 Obligation 与结果封存仍需单独贯通；证据事件落库本身不代表全部验证要求已满足。
工程验证诊断从 Core 已有 scope/version 事实派生；未知范围或缺 Host 版本输出
`evidence_insufficient`，不建议重复验证。它不是工具失败、等待授权或 Run 终态，
不能触发自动重试或覆盖运行状态；各客户端须将其作为独立诊断消费。

## 对象封装与依赖协议

跨模块边界以[架构决策与所有权](../../总体设计/架构决策与所有权.md)及[流程与恢复](../../总体设计/跨模块流程与恢复.md)为准，本文细化本模块职责。

ToolCall、ToolResult、ExecutionEvidence 各司其职；执行适配器不能直接封存任务。
所有跨边界输入输出遵循[公共信封与错误契约](../../模块契约.md)。对象命名为设计职责，不要求新增同名类型或 crate。
模块不得暴露可变内部状态；写入必须由其权威所有者处理。具体接口复用已有类型，缺少类型化边界列入整改，不用别名或额外包装宣称解耦完成。

## 当前实现入口与范围（2026-11-08）

| 源码 | 具体职责 |
| --- | --- |
| [tool_source.rs](../../../../core/nola-application/src/tool_source.rs) | 从原始 ToolCall/Result 建立来源提交 |
| [tool_source.rs](../../../../core/nola-store/src/tool_source.rs) | 来源冷读、分支复制、归档正文 validator/installer |
| [execution_evidence.rs](../../../../core/nola-domain/src/execution_evidence.rs) | 唯一执行回执类型，未知不伪装成功 |

归档 validator/installer 和当前来源端口已有实现，旧“未实现、暂停 Apple 编译”不是本次工作限制；是否可跨 replica 导入工具来源须读当前 installer，不能从普通 journal 复制推断。 逐模块差距与证据边界见[源码核对](../../总体设计/ARCHITECTURE_SOURCE_AUDIT_20261108.md)。

## 相关设计

- [HARNESS_RESULT_AND_ADMISSION_CONTRACT](HARNESS_RESULT_AND_ADMISSION_CONTRACT.md)
- [CLI_SOP_CAPABILITY_LONG_TASK_DESIGN](CLI_SOP_CAPABILITY_LONG_TASK_DESIGN.md)
- [工具 transcript 来源所有权、冷恢复与归档现状](TOOL_TRANSCRIPT_SOURCE_ARCHITECTURE_20261017.md)

## 当前差距与验收

当前工作与验收范围见[任务入口](../../../当前任务/README.md)。此前阶段实施与验证见[阶段归档](../../../历史任务/2026-10-28_A2UI与跨端执行阶段归档/README.md)；更早记录见[历史架构整改记录](../../../历史任务/2026-10-19_当前任务归档/架构整改主线.md)。
上述验证条目是验收要求；是否已经执行、使用的代码版本和证据必须另行记录。性能指标遵循[测量契约](../15-观测与验收/MEASUREMENT.md)，未测量项不得填写推测值。

历史暂停与当时验证见来源专题记录；本轮不沿用历史操作禁令，也不以历史通过数证明当前归档/跨端同步全部验收。
