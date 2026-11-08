> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# MOD-15：Observability / 验收

状态：现行模块职责；2026-11-08 核对当前实现入口及依赖。规范要求、静态接线与历史运行证据分别说明，不将未复测路径标记为通过。

## 设计、输入输出与生命周期

- 只读已提交事实与脱敏时序；不通过监控修正运行状态。
- 每个 Attempt、Effect、Run 的计量有唯一身份，缓存率以报告过的计数加权，而非平均百分比。
- 诊断可丢弃，业务可恢复；日志含错误详情不等于有 canonical blocker。
- 验收输出必须包含代码/产物身份、配置摘要、环境、样本数、原始证据引用、覆盖范围与未通过项。
- 具体测量与门禁见本模块 MEASUREMENT、TESTING_ARCHITECTURE 及 REAL_E2E_TESTING；性能报告不得把编译时间混入运行延迟。

## 对象封装与依赖协议

跨模块边界以[架构决策与所有权](../../总体设计/架构决策与所有权.md)及[流程与恢复](../../总体设计/跨模块流程与恢复.md)为准，本文细化本模块职责。

UsageLedger、Measurement、EvidenceReport 分离；观测服务只读业务事实。
所有跨边界输入输出遵循[公共信封与错误契约](../../模块契约.md)。对象命名为设计职责，不要求新增同名类型或 crate。
模块不得暴露可变内部状态；写入必须由其权威所有者处理。具体接口复用已有类型，缺少类型化边界列入整改，不用别名或额外包装宣称解耦完成。

## 当前实现入口与范围（2026-11-08）

| 源码 | 具体职责 |
| --- | --- |
| [lib.rs](../../../../core/nola-measurement/src/lib.rs) | 有界非权威测量 |
| [dependency_boundary.rs](../../../../cli/tests/dependency_boundary.rs) | Rust 声明依赖与能力边界门禁 |
| [test_mac_layers.sh](../../../../scripts/test_mac_layers.sh) | 使用 Test 配置的 Mac 分层测试入口 |

文档中的历史通过数、一次沙箱拒绝或“尚未运行”都仅属于原快照；当前测试范围以实际命令/报告为准。依赖图、静态检查和组件测试不能推出业务恢复或真机通过。 逐模块差距与证据边界见[源码核对](../../总体设计/ARCHITECTURE_SOURCE_AUDIT_20261108.md)。

## 相关设计

- [MEASUREMENT](MEASUREMENT.md)
- [HARNESS_RESOURCE_ACCOUNTING](HARNESS_RESOURCE_ACCOUNTING.md)
- [TESTING](TESTING.md)
- [TESTING_ARCHITECTURE](TESTING_ARCHITECTURE.md)
- [REAL_E2E_TESTING](REAL_E2E_TESTING.md)
- [PERFORMANCE_AND_EVALUATION](PERFORMANCE_AND_EVALUATION.md)
- [mobile-agent-test-plan](mobile-agent-test-plan.md)
- [privacy-release-checklist](privacy-release-checklist.md)

## 当前差距与验收

当前工作与验收范围见[任务入口](../../../当前任务/README.md)。此前阶段实施与验证见[阶段归档](../../../历史任务/2026-10-28_A2UI与跨端执行阶段归档/README.md)；更早记录见[历史架构整改记录](../../../历史任务/2026-10-19_当前任务归档/架构整改主线.md)。
上述验证条目是验收要求；是否已经执行、使用的代码版本和证据必须另行记录。性能指标遵循[测量契约](MEASUREMENT.md)，未测量项不得填写推测值。
