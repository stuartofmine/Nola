> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# MOD-12：Memory / DataSource / Automation

状态：现行模块职责；2026-11-08 核对当前实现入口及依赖。规范要求、静态接线与历史运行证据分别说明，不将未复测路径标记为通过。

## 设计、输入输出与生命周期

2026-10-16 细化：自动化保留原 execution_spec 及提交摘要；claimed/preparing 与 admitted 分界明确。绑定前确定未准入异常必须持久 launch_failed；准入未知先按原 invocation 对账，不因失联重跑。详见[根因边界](../../总体设计/执行权限与证据边界_20261016.md)。

- Memory：Core 决定写入语义，Store 保存 provenance、scope、revision；Host 不私自维护长期记忆。
- DataSource：Host connector 返回读取证据与 freshness；Application 提交 snapshot；缺权限/过期/空结果不同。
- Automation：持久触发事实经相同 Application admission 创建 Run，复用授权/执行/回执链，不自建 agent loop。
- 输入必须包含所属用户/会话/设备与来源；原始敏感数据只按明确策略保存，不自动进入全部会话。
- 验证：关闭记忆、撤权、过期数据、重复调度、后台取消、断网重试，均使用同一 typed failure/等待契约。

## 对象封装与依赖协议

跨模块边界以[架构决策与所有权](../../总体设计/架构决策与所有权.md)及[流程与恢复](../../总体设计/跨模块流程与恢复.md)为准，本文细化本模块职责。

Memory、DataSource、Automation 为不同业务对象；不能用通用 JSON 对象混合写入规则。
所有跨边界输入输出遵循[公共信封与错误契约](../../模块契约.md)。对象命名为设计职责，不要求新增同名类型或 crate。
模块不得暴露可变内部状态；写入必须由其权威所有者处理。具体接口复用已有类型，缺少类型化边界列入整改，不用别名或额外包装宣称解耦完成。

## 当前实现入口与范围（2026-11-08）

| 源码 | 具体职责 |
| --- | --- |
| [memory.rs](../../../../core/nola-application/src/memory.rs) | 语义记忆命令及当前 recall 查询结构 |
| [data_source.rs](../../../../core/nola-application/src/data_source.rs) | 数据源记录、快照与读取用例 |
| [automation_execution.rs](../../../../core/nola-application/src/automation_execution.rs) | 发生实例、定义 revision 与实际执行绑定 |

AutomationDefinition 已有 typed execution_spec（agent/user_task）；MemoryRecord.owner、RecallQuery.binding 与 Store owner 过滤/去重已接线，移动端 profile 搜索复用 Recall。原生 workspace/turn 选择界面与真实并发模型验收不由这些接口自动推定。 逐模块差距与证据边界见[源码核对](../../总体设计/ARCHITECTURE_SOURCE_AUDIT_20261108.md)。

## 相关设计

- [MEMORY_EXECUTION_POLICY_CONTRACT](MEMORY_EXECUTION_POLICY_CONTRACT.md)
- [MEMORY_SCOPE_V2](MEMORY_SCOPE_V2.md)
- [MEMORY_RECALL_ARCHITECTURE](MEMORY_RECALL_ARCHITECTURE.md)
- [CODE_MEMORY_NATIVE_ARCHITECTURE](CODE_MEMORY_NATIVE_ARCHITECTURE.md)
- [AUTOMATION_CROSS_PLATFORM_CONTRACT](AUTOMATION_CROSS_PLATFORM_CONTRACT.md)
- [data-source-centered-architecture](data-source-centered-architecture.md)
- [BACKGROUND_DATA](BACKGROUND_DATA.md)

## 当前差距与验收

当前工作与验收范围见任务入口（本地工作记录，不随仓库分发）。此前阶段实施与验证见阶段归档（本地工作记录，不随仓库分发）；更早记录见历史架构整改记录（本地工作记录，不随仓库分发）。
上述验证条目是验收要求；是否已经执行、使用的代码版本和证据必须另行记录。性能指标遵循[测量契约](../15-观测与验收/MEASUREMENT.md)，未测量项不得填写推测值。
