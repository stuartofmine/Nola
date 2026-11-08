> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# MOD-03：Application / Service

状态：现行模块职责；2026-11-08 核对当前实现入口及依赖。规范要求、静态接线与历史运行证据分别说明，不将未复测路径标记为通过。

## 设计、输入输出与生命周期

- 所有者：`nola-application` 用例 + service 接入，是唯一业务写入协调者。
- 输入：命令、查询、订阅和 Host result。输出：已提交 receipt、不可变投影/游标或结构化拒绝。
- 前置：command 幂等键、会话归属、配置 revision；重复相同命令返回同一结果，不同载荷同键返回冲突。
- 提交：inbox 去重、Core transition、业务物化、checkpoint/replay facts、outbox、usage 同事务。
- 网络通知仅传已提交游标；发送失败可重订阅，不能撤销已提交业务事实。
- 错误：未确定提交不发成功回执；COMMIT 结果不明或响应丢失必须查询原 request_id，不能假定回滚并生成第二个 Run。
- 验证：每个提交边界 crash injection、重发/乱序、跨会话拒绝、所有客户端通过统一入口。

## 对象封装与依赖协议

跨模块边界以[架构决策与所有权](../../总体设计/架构决策与所有权.md)及[流程与恢复](../../总体设计/跨模块流程与恢复.md)为准，本文细化本模块职责。

用例服务封装事务与命令幂等；客户端只依赖命令、查询、订阅端口。
所有跨边界输入输出遵循[公共信封与错误契约](../../模块契约.md)。对象命名为设计职责，不要求新增同名类型或 crate。
模块不得暴露可变内部状态；写入必须由其权威所有者处理。具体接口复用已有类型，缺少类型化边界列入整改，不用别名或额外包装宣称解耦完成。

## 当前实现入口与范围（2026-11-08）

| 源码 | 具体职责 |
| --- | --- |
| [application_command.rs](../../../../core/nola-application/src/application_command.rs) | 当前 typed 用例与 wire tag 映射 |
| [lib.rs](../../../../core/nola-application/src/lib.rs) | 泛型 Application runtime 与提交协调 |
| [runtime_owner.rs](../../../../core/nola-application-bootstrap/src/runtime_owner.rs) | 具体持久 runtime 的独占取得与交还 |

Service 是本地进程接入，embedded 是移动部署；两者复用 Application 用例。bootstrap 负责具体 Store 装配，不是可绕过命令边界的客户端 SDK。查询不应为读取制造业务回执。 逐模块差距与证据边界见[源码核对](../../总体设计/ARCHITECTURE_SOURCE_AUDIT_20261108.md)。

## 相关设计

- [APPLICATION_PORT_CONTRACT](APPLICATION_PORT_CONTRACT.md)
- [CANONICAL_CONVERSATION_TRANSACTIONS](CANONICAL_CONVERSATION_TRANSACTIONS.md)
- [CONVERSATION_MUTATION_CONTRACT](CONVERSATION_MUTATION_CONTRACT.md)

## 当前差距与验收

当前工作与验收范围见[任务入口](../../../当前任务/README.md)。此前阶段实施与验证见[阶段归档](../../../历史任务/2026-10-28_A2UI与跨端执行阶段归档/README.md)；更早记录见[历史架构整改记录](../../../历史任务/2026-10-19_当前任务归档/架构整改主线.md)。
上述验证条目是验收要求；是否已经执行、使用的代码版本和证据必须另行记录。性能指标遵循[测量契约](../15-观测与验收/MEASUREMENT.md)，未测量项不得填写推测值。
