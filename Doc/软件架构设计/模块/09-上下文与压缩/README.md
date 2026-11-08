> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# MOD-09：Context / Compaction / 原文召回

状态：现行模块职责；2026-11-08 核对当前实现入口及依赖。规范要求、静态接线与历史运行证据分别说明，不将未复测路径标记为通过。

## 设计、输入输出与生命周期

实体与所有权统一见[会话、Run、Goal 与子任务边界](../10-Goal任务与子代理/AGENT_THREAD_ARCHITECTURE.md)。

- 目标上下文归属 ConversationRef + participant；协调 Context 独立。Agent 配置不是历史容器，共享公开时间线不授予跨成员私有证据访问权；当前 Room participant 已通过独立 context Conversation 与 ContextOwner 持久映射；公开时间线仍不是跨成员私有证据授权。
- Core 选择可压缩范围并计算预算；Host 执行物理请求；Store 保存原文、摘要 lineage 和派生索引。
- 输入：canonical 原文引用、summary blocks、原始 tail、Goal/Task 引用、有效窗口、物理快照及 usage。
- 输出：ContextPlan 或完整 CompactionCandidate(source fingerprint, text, lineage, usage)，原文绝不被覆盖。
- 热分叉保持原物理前缀，末尾追加指令；没有有效快照走有界冷计划，不伪造缓存命中。
- 输出截断且有正文：事务内保留片段并续写；空/工具调用/取消不能提交半摘要。
- 摘要、Goal、权限、Skill 的内容是分层数据，不能因放入 context 而改变其来源权威或信任等级。
- 原文 recall 根据会话授权与引用读取；压缩摘要引用外部文字不提升为用户指令。
- 验证：热/冷、超大历史、未知快照原因、真实截断、原文 hash 不变、BM25 可重建、恢复前后权限不变。

## 对象封装与依赖协议

跨模块边界以[架构决策与所有权](../../总体设计/架构决策与所有权.md)及[流程与恢复](../../总体设计/跨模块流程与恢复.md)为准，本文细化本模块职责。

ContextPlan、SummaryBlock、OriginalReference 分离；摘要不是原文或权限的所有者。
所有跨边界输入输出遵循[公共信封与错误契约](../../模块契约.md)。对象命名为设计职责，不要求新增同名类型或 crate。
模块不得暴露可变内部状态；写入必须由其权威所有者处理。具体接口复用已有类型，缺少类型化边界列入整改，不用别名或额外包装宣称解耦完成。

## 当前实现入口与范围（2026-11-08）

| 源码 | 具体职责 |
| --- | --- |
| [context.rs](../../../../core/nola-application/src/context.rs) | Context checkpoint、组件来源与原子维护提交 |
| [compaction_driver.rs](../../../../core/nola-agent-engine/src/agent/runtime/compaction_driver.rs) | 压缩推进及模型请求边界 |
| [request_snapshot.rs](../../../../cli/src/llm_parts/request_snapshot.rs) | 已编译物理请求的热分叉候选快照 |

canonical 原文、逻辑工作集、压缩摘要、物理请求快照和计量不能合并为同一历史。热路径资格来自实际请求见证；缓存命中由实际 usage 证明，未知不能显示为零。 逐模块差距与证据边界见[源码核对](../../总体设计/ARCHITECTURE_SOURCE_AUDIT_20261108.md)。

## 相关设计

- [CONTEXT_ACCOUNTING_V2](CONTEXT_ACCOUNTING_V2.md)
- [INCREMENTAL_CONTEXT_DESIGN](INCREMENTAL_CONTEXT_DESIGN.md)
- [CONTEXT_AND_ACCOUNTING](CONTEXT_AND_ACCOUNTING.md)
- [HARNESS_CONTEXT_PROJECTION](HARNESS_CONTEXT_PROJECTION.md)

## 当前差距与验收

当前工作与验收范围见[任务入口](../../../当前任务/README.md)。此前阶段实施与验证见[阶段归档](../../../历史任务/2026-10-28_A2UI与跨端执行阶段归档/README.md)；更早记录见[历史架构整改记录](../../../历史任务/2026-10-19_当前任务归档/架构整改主线.md)。
上述验证条目是验收要求；是否已经执行、使用的代码版本和证据必须另行记录。性能指标遵循[测量契约](../15-观测与验收/MEASUREMENT.md)，未测量项不得填写推测值。
