# 架构历史实施记录

2026-11-08 从现行架构中分离。下文保留原日期、失败/通过与暂停范围，不构成当前实施状态、操作限制或兼容要求；相对链接按归档位置调整。

> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 工具 transcript 来源所有权、冷恢复与归档现状

日期：2026-10-17。状态：原整改仍暂停；用户随后明确要求编译并修复 Mac，本轮仅执行该范围。跨端同步仍排除；未启动 App、设备发现或 Apple 原生测试。

最新修复与验证：缺失的归档正文 validator/installer 及来源枚举消费已补齐。原文完整性、精确 staged entry、目标 Tool Part、call/result 身份在同一恢复事务校验；归档只形成 Archive 来源，不赋予执行权。未实现的 Replica 接收路径明确拒绝。Mac Debug build 已成功（`/tmp/nola-mac-compile.6SjlUt/build-fixed.log`，exit 0）；归档定向 4 项测试通过（同目录 `archive-tests.log`），覆盖恢复冷读、错误摘要拒绝、安装故障整笔回滚及归档提交的正文 seal 依赖。以下“暂停时”表格保留历史交接状态，不代表这些编译缺口仍存在；CLI/Mac 备份 UI 全流程未实测，跨端同步未完成。

## 1. 身份与权威边界

原始 `AgentItem::ToolCall`、`AgentItem::ToolResult` 和可见 canonical Tool Part 是不同对象。Tool Part 的 preview、structured output、模型摘要和 UI call 映射都不能代替原始 transcript，也不能自行建立工具执行或权限事实。

来源绑定保存原始 item ID、kind、call ID、name、精确 conversation/Run/Turn/message/Part owner、原文 SHA-256 及独立 canonical body reference。Application 只从实际接受的 Core transcript 和 `item_ownership` 取来源，在对应变更的同一持久事务内，以正式 `AssistantSourceCommitStamp` 和 `AssistantPartOwner` 交给 Store。普通 Host 工具及 Core 控制工具遵守同一边界；不得因为控制调用没有普通展示输出就删掉其原始来源。

调用可以先于结果持久化，结果到达后追加其独立绑定；不能改绑既有 item、覆盖既有原文或用展示 JSON 补造缺失调用。ToolResult 绑定需要原始 ToolCall 来源。正文使用既有不可变 UTF-8 canonical 分块，独立于 Tool Part 的 `content_json`，单块不超过 32 KiB、单个来源上限 256 MiB；正文摘要核对实际字节。

实现定位：`core/nola-application/src/tool_source.rs`、`core/nola-store-port/src/tool_source.rs`、`core/nola-store/src/tool_source.rs`。这些路径说明职责，不代表最新暂停工作树已编译通过。

## 2. 冷读取、引用与压缩恢复

读取以正式 conversation revision 为快照，重新核验来源绑定及 canonical Part/Turn owner；不能依赖最新 checkpoint、可裁剪 outbox、item ID 前缀或内存缓存推断来源。

历史水合只接收真实来源中完整、唯一且身份一致的 ToolCall/ToolResult 对。`HistoricalReferenceIndex` 只索引实际水合成功的 pair；缺绑定、重复 call、跨 owner、摘要不符或未形成完整 pair 的展示 Part 不获得引用信用。历史 call ID 可映射为历史命名空间，但不重新执行、不恢复批准，也不授予当前 Run 权限。

历史上下文有预算限制。超出有界水合预算的原文不能被截成看似完整的 ToolCall/ToolResult；压缩来源查询另走同一原文端口，有界片段明确标记截断，缺失来源保留为缺失。不得通过删掉控制工具引用、只保留当前 checkpoint 或摘要最新用户指令来掩盖冷来源缺口。

实现定位：Application `historical_context_references.rs`、`tool_source.rs`、`transcript_export.rs`。历史引用组件测试与真实模型恢复证据属于不同验证层，不互相替代。

## 3. 本地分支不是外来信任

已测本地路径区分 `AcceptedRun` 与 `VerifiedLocalDerivation`。本地 branch 在记录正式 `PartDerivation` 的同一事务调用 `copy_branch_sources`：保留原 item ID、原 accepted owner 快照及摘要，并复制为目标分支自有 canonical 正文。命名空间按 conversation 区分，不能因保留原 item ID 而跨会话误取来源。

读取分支来源时核对当前目标 Part、直接来源、operation 和本地派生事实。两代分支及冷重开仍走该核验；原会话删除后，分支拥有的正文不依赖被删除源会话存活。分支是历史派生，不是新的 accepted Run、工具重放或权限批准。

最新工作树在 StorePort `ToolSourceProvenance` 中又增加了 `Replica`、`Archive` 等消费尚未齐的变体。新增枚举本身不表示对应来源已被验证或安装，也不能据此外推前述本地路径在最新工作树仍可编译。外来 `ToolSourceClaim` 只是一份待核声明，不允许调用方自报本地可信 provenance。

## 4. 来源导出与归档：已落接口，不是完成链路

当前暂停代码新增 Domain `ToolSourceClaim` / `ToolSourceExportRange`。claim 包含 schema、原始 item/kind/call/name、目标 owner、原始 accepted owner、body reference、body digest 和必填 nullable `derivation`；不包含可由发送方自行授予的可信 provenance。

Application/Protocol 导出端口为：

- `ReadConversationToolSources { owner, expected_conversation_revision }`：按 Part 返回至多两个独立来源 claim。
- `ReadConversationToolSourceRange { owner, source_item_id, expected_conversation_revision, offset_bytes, limit_bytes }`：范围上限 1..32768 字节，响应包含精确 owner、source ID、摘要、revision、offset/next offset、总字节数、文本和 EOF。

使用方必须在同一 revision 下核对 owner、来源身份、游标连续性、UTF-8 字节数及最终摘要；不能把 Part 字段 reader 当来源原文 reader，也不能将 archive content ID 直接当接收库中的本地可信引用。

| 暂停时的代码位置 | 已落内容 | 未完成或未证边界 |
| --- | --- | --- |
| Domain / Protocol / Application 导出 | claim/range DTO、metadata/range 命令及分发 | 新接口及所有调用者未做本轮集中编译/测试 |
| Domain / Application 归档 | manifest `ToolSource { claim, upload_id }`、Store entry；sealed upload 的字节/摘要检查和本地 body ref 准备 | 共享来源 validator/installer 未实现；不能完成可信来源的事务安装 |
| StorePort / Store | 新 provenance 形态已加入 StorePort | 现有消费未齐；不可宣称当前工作树可编译 |
| CLI `store/session.rs` | canonical archive 增加独立 `tool_sources` 载体；按范围读取并核对摘要 | 新调用链与正负回归未完成集中验证 |
| Mac 备份/恢复服务 | 来源 Swift DTO、独立范围流、`tool_sources` 数组、每来源独立上传及 manifest 接线；备份格式代码改为当前 version 3 | 半实现未测；新增正负用例及最终接口审查未齐，Apple 验证暂停 |

Mac 涉及 `MacToolSourceArchive.swift`、`MacApplicationPort.swift`、`MacDataManagementService.swift`、`MacArchiveRestore.swift`、`MacTranscriptBodyUploader.swift` 和 `MacCanonicalConversationRepository.swift`；现有 `MacCanonicalExportTests.swift` 调用适配不是新链路的通过证据。此处 version 3 是暂停代码中的当前格式选择，不表示该格式已验证可用；不补旧格式默认字段、兼容别名或自动转换。

安装契约要求完整来源正文、owner、原始身份和摘要被核验后，才可在同一事务形成接收方拥有的 Archive 来源绑定。失败不得只留下能被引用的半个 pair，不得把归档 claim 当作 `AcceptedRun` 或 `VerifiedLocalDerivation`。本次补齐安装与拒绝路径并通过定向测试，仍不将它等同于客户端完整恢复体验验收。

跨端同步本轮明确排除，未完成，也没有本轮同步验收。只传 Tool Part 或正文不能补齐关系；缺少可信来源时显式不可用，不回退到展示 JSON。不以本地分支证明替代远端来源核验。

## 5. 已有证据及其快照边界

归档扩展前的第三轮工具来源专项证据目录为 `/tmp/nola-tool-source-validation-third.DDKkXRoX`，session 37969，exit 0：

- Engine lib：513 通过。
- Store 来源 7、有界历史 1、上传发布 5：均通过。
- Application lib：328 通过、0 失败、1 忽略。
- 7 个 integration targets：合计 26 通过。
- 总计 880 通过、0 失败、1 忽略；正常四项二进制已重建并记录 SHA。

可复查该目录的 `results.ndjson`、`engine-lib.log`、`store-tool-source.log`、`store-bounded-history.log`、`application-lib.log`、`application-affected.log` 和 `normal-binaries.log`。其中历史引用、两代本地分支、冷重开及来源删除隔离的测试证据，不应表述为跨端或归档导入验收。

真实 192 模型恢复证据目录为 `/tmp/nola-context-tool-source-recovery.FXxSSjnG`，session 67662，exit 0。场景耗时 48.447 秒，独立冷来源查询 0.05 秒；HTTP 序列 200/400/200/200 中的 400 是明确注入的上下文错误。产生 1 个压缩记录，accepted 用户输入逐字保留，正式 Answered seal、精确答案及 `missing=[]` 均通过。已核对四项产物 SHA，源配置未修改，`cleanup_errors=[]`。可复查 `status.json`、`stages.json`、`resilience.stdout.log`。

该真实场景证明的是所测快照在一次受控错误后的同会话恢复及冷来源查找，不证明自然服务端溢出、所有工具来源组合、跨端同步或原生 UI。880 项结果不能覆盖之后的归档改动；新修复证据限于本文开头的 Mac 编译与 4 项定向测试，不能宣称目标全部完成。Darwin 链接告警仍保留，不宣称零告警。

集中历史与结果边界见[权限与会话整改集中验收记录](../2026-10-19_当前任务归档/权限与会话整改集中验收记录_20261016.md)。恢复实施及验证需要新的明确任务；本文不启动任何构建、测试、Apple 操作或用户数据转换。
