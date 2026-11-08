> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Application journal producer 与提交边界

核对：2026-11-08。当前生产 producer、查询、source publication 和 replica 接收已有源码接线；不再将它们统称为尚待迁移。此前数千行中间状态、失败与通过记录已单独归档，不能继续充当当前实现章节。

## 当前所有者与流程

1. Application 接受原 owner 的输入、模型/工具响应或明确命令；reducer 提供候选 transition，不由 Host 直接改正式消息。
2. 变更收集器记录该 transition 实际改变的助手字段、Part、顺序与终态；planner 使用原始字段 reader 与提交身份构造有界操作，不用 Trace 文本重建原文。
3. producer 在正式 publication transaction 内应用变更、保留来源见证，提交 checkpoint/相关业务事实及 publication；未提交的候选不能先发布为正式 UI 事实。
4. Application 结果携带相应 publication cursor。客户端按源身份/游标补齐；传输发送或 ACK 失败不能撤销 Store 提交，也不能生成另一份消息。
5. authenticated replica 接收校验 source identity、精确记录、manifest/内容及接收窗口。本地接收记录保留源权威，不等于取得源 Run 执行权。

## 实现入口

| 源码 | 职责 |
| --- | --- |
| [assistant_journal_changes.rs](../../../../core/nola-application/src/assistant_journal_changes.rs) | 从实际语义变更收集 publication 输入 |
| [assistant_journal_planner.rs](../../../../core/nola-application/src/assistant_journal_planner.rs) | 有界读取、来源和操作计划 |
| [assistant_journal_producer.rs](../../../../core/nola-application/src/assistant_journal_producer.rs) | `publish_changed_assistant_journal` 与 task/Run 绑定发布 |
| [assistant_publication_transaction.rs](../../../../core/nola-application/src/assistant_publication_transaction.rs) | 发布的事务协调，不成为第二个业务写者 |
| [assistant_journal_read.rs](../../../../core/nola-application/src/assistant_journal_read.rs) | 快照、范围、Part/manifest 查询 |
| [Store Port](../../../../core/nola-store-port/src/assistant_journal.rs) | Store producer/reader/接收所需的受限接口 |

## 原文与投影

canonical 文本/stream、对话 Part、assistant journal 与诊断 Trace 拥有不同角色。journal 是可按精确身份消费的已提交投影，不是让客户端忽略源文档或重新创造 Run 终态的授权。相同文本 hash 不能跨 owner 自动读取；同一记录重放必须精确幂等，不能只看相同消息 ID。

新建、正文追加/替换、metadata-only、工具/思考/Task 变化与 seal 必须遵守对应生命周期。语义无变化时不因刷新强制重建全消息；完成不能把空 inline 槽当作实际正文计算摘要。跨 Run 续接保留原 source/head owner，读取关系由规范关联证明。

## 恢复与错误

恢复从已提交 Run、回执、来源记录与原始字段 reader 取得事实。客户端订阅缺口按游标补齐；遇到原文/manifest/owner 冲突明确失败，不能从 Trace 或 UI 缓冲补伪正文。关闭窗口、stream EOF、Host 进程退出都不是成功 seal。

source producer、replica receiver、原生观察器、冷恢复和性能是不同验证面；不能由一个 Store 测试推导全部跨端/真实 provider 已通过。schema、Service 与 ABI 数字取各自当前源码，不把历史 schema 30/33 等快照重新当成当前版本。

## 历史证据

完整日期化记录见[历史 producer 与验收流水](../../../历史任务/实施与审计/ASSISTANT_JOURNAL_APPLICATION_EVIDENCE_20261108.md)。本次只核对源码与文档，不重新赋予历史报告当前工作树身份。
