> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# MOD-04：Store / 内容存储 / 索引

状态：现行模块职责；2026-11-08 核对当前实现入口及依赖。规范要求、静态接线与历史运行证据分别说明，不将未复测路径标记为通过。

## 设计、输入输出与生命周期

- 所有者：`nola-store-port` 定义事务/查询接口，`nola-store` 实现 canonical schema。
- 输入：Application 的 Unit of Work、按授权范围的读取。输出：commit receipt、分页 snapshot、内容范围及 digest。
- 原子性：SQL 中 canonical events、views、checkpoint、inbox/outbox 同提交；blob 先安全写入校验后
  才提交引用，失败留下的未引用 blob 可由独立 GC 回收，不能先引用未完成文件。
- 索引/BM25、物化 view、缓存快照均可重建；transcript/权限/Goal/Task 原始事实不可从 Trace 推测重建。
- 存储三类：权威记录、可重建派生、临时诊断。工作文件快照是导出/恢复材料，不是绕过事务的第二写入口。
- 版本：schema、wire、checkpoint 独立标识且兼容性显式校验；不做无声 default/旧别名转换。
- 验证：提交去重、引用完整性、恢复等价、备份/导入、删除与保留策略、索引缺失可恢复。

## 对象封装与依赖协议

跨模块边界以[架构决策与所有权](../../总体设计/架构决策与所有权.md)及[流程与恢复](../../总体设计/跨模块流程与恢复.md)为准，本文细化本模块职责。

Repository/UnitOfWork 封装数据访问；存储适配器不按客户端名称决定业务规则。
所有跨边界输入输出遵循[公共信封与错误契约](../../模块契约.md)。对象命名为设计职责，不要求新增同名类型或 crate。
模块不得暴露可变内部状态；写入必须由其权威所有者处理。具体接口复用已有类型，缺少类型化边界列入整改，不用别名或额外包装宣称解耦完成。

## 当前实现入口与范围（2026-11-08）

| 源码 | 具体职责 |
| --- | --- |
| [schema_store.rs](../../../../core/nola-store/src/schema_store.rs) | 当前 schema 的组合初始化与严格三项版本/身份校验 |
| [canonical_text_store.rs](../../../../core/nola-store/src/canonical_text_store.rs) | 规范原文与内容引用 |
| [assistant_journal.rs](../../../../core/nola-store-port/src/assistant_journal.rs) | journal 读取/发布/接收的存储端口 |

同 schema 不等于所有平台共享同一个文件路径；大文本已含 SQLite canonical 对象、分块、stream head/manifest，不能统称 ~/.nola/blobs。schema.sql 是组合 DDL 的一个入口，不是唯一文件。 逐模块差距与证据边界见[源码核对](../../总体设计/ARCHITECTURE_SOURCE_AUDIT_20261108.md)。

## 相关设计

- [CANONICAL_BACKUP_FORMAT](CANONICAL_BACKUP_FORMAT.md)
- [local-sqlite-store](local-sqlite-store.md)
- [CORE_CHECKPOINT_CONTENT_CONTRACT](CORE_CHECKPOINT_CONTENT_CONTRACT.md)
- [ORIGINAL_TEXT_READER_CONTRACT](ORIGINAL_TEXT_READER_CONTRACT.md)
- [COMMITTED_ASSISTANT_JOURNAL_DESIGN](COMMITTED_ASSISTANT_JOURNAL_DESIGN.md)
- [COMMITTED_ASSISTANT_JOURNAL_APPLICATION_ADAPTER](COMMITTED_ASSISTANT_JOURNAL_APPLICATION_ADAPTER.md)
- [COMMITTED_ASSISTANT_JOURNAL_STORE_PLAN](COMMITTED_ASSISTANT_JOURNAL_STORE_PLAN.md)
- [STORE_RESULT_BINDING](STORE_RESULT_BINDING.md)
- [CONVERSATION_AGGREGATE_CONTRACT_20261015](CONVERSATION_AGGREGATE_CONTRACT_20261015.md)

## 当前差距与验收

当前工作与验收范围见[任务入口](../../../当前任务/README.md)。此前阶段实施与验证见[阶段归档](../../../历史任务/2026-10-28_A2UI与跨端执行阶段归档/README.md)；更早记录见[历史架构整改记录](../../../历史任务/2026-10-19_当前任务归档/架构整改主线.md)。
上述验证条目是验收要求；是否已经执行、使用的代码版本和证据必须另行记录。性能指标遵循[测量契约](../15-观测与验收/MEASUREMENT.md)，未测量项不得填写推测值。

## 当前版本与复制不变量（2026-10-28）

Core revision与Store checkpoint CAS revision是独立计数。权限settlement校验前后Core witness区间及原owner/cursor；记录已提交后迟到chunk只做精确幂等确认，不重建stage。

具体字段、返回值、错误和源码索引见[当前 Core 与 Host 接口](../01-领域与协议/CURRENT_RUNTIME_INTERFACES.md)。本次仅核对文档，不新增运行验收结论。
