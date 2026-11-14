> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# MOD-10：Goal / Task / Subagent

状态：现行模块职责；2026-11-08 核对当前实现入口及依赖。规范要求、静态接线与历史运行证据分别说明，不将未复测路径标记为通过。

## 设计、输入输出与生命周期

- Core 决定分解、进度、验收与取消树；Application/Store 保存唯一记录；Supervisor 只执行已 claim 的工作。
- 输入：明确目标、约束、验收条件、依赖、父子关系、权限上限、执行预算；输出：带 revision 的计划/进度/证据。
- 子 agent 收到的是作用域化任务，不是父会话所有权限与可写状态。子结果不能自行完成父 Goal。
- 进度更新引用已发生事件；文字“完成了”不代替产物/检查证据。阻塞保存 blocker 与解除条件。
- 压缩/重启从同一记录恢复；可读文件导出必须携带 canonical revision，编辑回写走命令/CAS，禁止双写。
- 验证：压缩后约束完整、子任务部分失败、父取消、重复结果、同资源冲突、无进展识别与恢复。

## 对象封装与依赖协议

跨模块边界以[架构决策与所有权](../../总体设计/架构决策与所有权.md)及[流程与恢复](../../总体设计/跨模块流程与恢复.md)为准，本文细化本模块职责。

Goal、Task、AgentRun 分离聚合与生命周期；子代理持有受限任务引用而非父级可写对象。
所有跨边界输入输出遵循[公共信封与错误契约](../../模块契约.md)。对象命名为设计职责，不要求新增同名类型或 crate。
模块不得暴露可变内部状态；写入必须由其权威所有者处理。具体接口复用已有类型，缺少类型化边界列入整改，不用别名或额外包装宣称解耦完成。

## 当前实现入口与范围（2026-11-08）

| 源码 | 具体职责 |
| --- | --- |
| [goal_execution.rs](../../../../core/nola-application/src/goal_execution.rs) | Goal 执行模板与准入绑定 |
| [submission.rs](../../../../core/nola-application/src/agent_room/submission.rs) | Room 接纳、结构化选择与原消息关联 |
| [child_pump.rs](../../../../core/nola-mobile-bindings/src/child_pump.rs) | 移动端 child 的实际执行/恢复泵 |

当前源码仍有 AgentRoom、Room submission/dispatch 与独立成员 Context；统一 Conversation 是产品方向，不能文档化为底层 Room 已删除。主控结构化路由函数已有实现，也不能再一概标成“未实现”。 逐模块差距与证据边界见[源码核对](../../总体设计/ARCHITECTURE_SOURCE_AUDIT_20261108.md)。

## 相关设计

- [GOAL_LONG_HORIZON_ARCHITECTURE](GOAL_LONG_HORIZON_ARCHITECTURE.md)
- [AGENT_THREAD_ARCHITECTURE](AGENT_THREAD_ARCHITECTURE.md)
- [GOALS_AND_TASKS](GOALS_AND_TASKS.md)
- [DELEGATED_AGENT_HOST_CONTRACT](DELEGATED_AGENT_HOST_CONTRACT.md)
- [专家与子代理统一执行契约](EXPERT_SUBAGENT_EXECUTION.md)

## 当前差距与验收

当前工作与验收范围见任务入口（本地工作记录，不随仓库分发）。此前阶段实施与验证见阶段归档（本地工作记录，不随仓库分发）；更早记录见历史架构整改记录（本地工作记录，不随仓库分发）。
上述验证条目是验收要求；是否已经执行、使用的代码版本和证据必须另行记录。性能指标遵循[测量契约](../15-观测与验收/MEASUREMENT.md)，未测量项不得填写推测值。

## 当前Goal与本地child接口（2026-10-28）

Goal的can_pause/can_resume/can_cancel由Core控制观测发布，客户端不从stream状态猜测。移动child准入携带current_tools并核对冻结权限；唤醒计划只发现本Store真实准入任务。Task journal sealed属于终态，实际结果仍从canonical seal读取；只读child消费Core execution_ownership。

具体字段、返回值、错误和源码索引见[当前 Core 与 Host 接口](../01-领域与协议/CURRENT_RUNTIME_INTERFACES.md)。本次仅核对文档，不新增运行验收结论。
