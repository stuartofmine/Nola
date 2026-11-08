> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# MOD-02：Core / Kernel

状态：现行模块职责；2026-11-08 核对当前实现入口及依赖。规范要求、静态接线与历史运行证据分别说明，不将未复测路径标记为通过。

## 设计、输入输出与生命周期

- 所有者：`nola_agent_engine` 实现 Agent reducer/恢复/trace，`nola_kernel` 提供纯决策；`nola_core` 是复用它们的 facade，当前物理依赖见总图。
- 输入：`CommittedState + TypedInput + DecisionFacts(time, capability/config revisions)`。
- 输出：`Transition(new_state, domain_events, effects, outcome?)`；无直接 I/O。
- 前置：相关身份、顺序与执行权有效；拒绝不能部分修改输入状态。
- 持久化：只提出 transition，由 MOD-03 提交；同输入的决定必须可重放。
- 错误：invariant/admission；不能用解析 tool 输出或回答关键词进行权限/完成判断。
- 验证：LC 全状态可达性、非法组合、任意事件序列后不变量、取消/权限/结果竞态。

## 对象封装与依赖协议

跨模块边界以[架构决策与所有权](../../总体设计/架构决策与所有权.md)及[流程与恢复](../../总体设计/跨模块流程与恢复.md)为准，本文细化本模块职责。

运行状态聚合封装合法迁移；Kernel 仅接受显式事实，不能读取全局配置。
所有跨边界输入输出遵循[公共信封与错误契约](../../模块契约.md)。对象命名为设计职责，不要求新增同名类型或 crate。
模块不得暴露可变内部状态；写入必须由其权威所有者处理。具体接口复用已有类型，缺少类型化边界列入整改，不用别名或额外包装宣称解耦完成。

## 当前实现入口与范围（2026-11-08）

| 源码 | 具体职责 |
| --- | --- |
| [reducer_port.rs](../../../../core/nola-agent-engine/src/reducer_port.rs) | 消费式候选 reduction 与恢复端口 |
| [submission.rs](../../../../core/nola-agent-engine/src/agent/runtime/submission.rs) | 运行状态转换实现 |
| [lib.rs](../../../../core/src/lib.rs) | rlib facade 与纯辅助 ABI 重导出 |

状态机实现已在 agent engine，不能再把 core/src/agent 说成待抽取的另一份实现。facade 仍有纯辅助函数，但最终原生产物属于 mobile_bindings；整个包装 crate 的存在不证明所有路径无 I/O。 逐模块差距与证据边界见[源码核对](../../总体设计/ARCHITECTURE_SOURCE_AUDIT_20261108.md)。

## 相关设计

- [LIFECYCLE](LIFECYCLE.md)
- [COMPLETION](COMPLETION.md)
- [REQUIREMENT_ADMISSION](REQUIREMENT_ADMISSION.md)

## 当前差距与验收

当前工作与验收范围见[任务入口](../../../当前任务/README.md)。此前阶段实施与验证见[阶段归档](../../../历史任务/2026-10-28_A2UI与跨端执行阶段归档/README.md)；更早记录见[历史架构整改记录](../../../历史任务/2026-10-19_当前任务归档/架构整改主线.md)。
上述验证条目是验收要求；是否已经执行、使用的代码版本和证据必须另行记录。性能指标遵循[测量契约](../15-观测与验收/MEASUREMENT.md)，未测量项不得填写推测值。
