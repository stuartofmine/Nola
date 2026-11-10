> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 工具原始来源、冷读取与归档

核对：2026-11-08。当前实现已有来源提交、原文冷读、本地分支复制及归档 validator/installer。旧“provenance 消费未齐、validator 未实现、Apple 必须暂停”不是现行说明；运行证据仍需绑定其原工作树。

## 身份与权威

原始 ToolCall/ToolResult、canonical Tool Part、展示 preview 与模型摘要是不同对象。来源绑定保留 item/kind/call/name、精确 conversation/Run/Turn/message/Part owner、body reference、摘要及提交 revision。Application 从实际接纳的 transcript/ownership 建立来源，不能让 Host 或外来 claim 自报已被接受。

当前 `ToolSourceProvenance` 有 AcceptedRun、VerifiedLocalDerivation、Replica 和 Archive 形态；枚举存在不证明每条导入路径都可用。调用与结果分别绑定，结果须引用真实调用，不覆盖已有原始正文或重新解释 ID。

## 读取与分支

`ReadConversationToolSources` 和 `ReadConversationToolSourceRange` 通过原 Part owner、source item 和 expected conversation revision 查找/范围读取。逐段核对 offset、UTF-8 字节数、摘要与完整性。Tool Part 字段 reader 不是原始来源 reader，展示 JSON 不能补造缺失来源。

冷上下文水合保留历史调用身份；超出预算时明确截断或通过原文端口继续读取，不将截取片段伪装成完整调用。来源缺失是可识别错误，不借删除控制工具引用、最新用户指令或只读当前 checkpoint 掩盖。

本地 branch 在正式 PartDerivation 事务中复制目标自有正文与来源关系，保留原 accepted owner 快照。它是历史派生，不是新执行、权限批准或工具重放；源会话删除不应要求丢弃分支自有正文。

## 归档与接收

- Domain claim 是待核声明，不是可信 provenance。归档 manifest 以 sealed upload 的字节/摘要和当前恢复 entry 证明输入。
- 当前 Store 已实现 `validate_imported_tool_source_body`：检查完整 canonical body、长度/SHA、原始结构及 item/call/name 一致性。
- `install_imported_tool_source` 要求正在进行的归档事务，并核对精确 staged ToolSource entry、目标已完成助手 Tool Part 和调用/结果关系；不允许在 autocommit 外挂一个半完成来源。
- 当前内部 `ToolSourceImport` installer 只接纳 Archive。Replica provenance 形态存在，不等于可走同一可信安装；不能从普通 assistant journal 复制推断工具 transcript 来源接收已经全链路完成。
- 当前格式备份/归档恢复与旧格式迁移不同。不自动删除、重建或转换用户数据，不恢复历史工具的执行权。

## 当前实现入口

[Application source](../../../../core/nola-application/src/tool_source.rs)、[Store Port](../../../../core/nola-store-port/src/tool_source.rs)、[Store validator/installer](../../../../core/nola-store/src/tool_source.rs)、[archive admission](../../../../core/nola-application/src/archive_restore.rs)。

验收分别覆盖原文冷读、跨 owner/摘要拒绝、分支、归档安装回滚、CLI/Mac 导出 UI、replica 与真实模型恢复；不能彼此替代。

旧阶段测试、失败和暂停记录见历史来源记录（本地工作记录，不随仓库分发）；它们不定义当前操作限制或当前工作树已通过范围。
