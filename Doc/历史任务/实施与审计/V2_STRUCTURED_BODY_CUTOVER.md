> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# V2 结构化 Part 大正文切换方案

状态：2026-09-08，只读审计与下一步实施契约；本文件未启用新正文表示。
当前纯文本 chunk publication、metadata-only branch、原子 archive restore 与 upload retirement 已实现并有回归。本方案补齐 `content_json` 和非 `text` Part 的大字段，不改变 Message/Part 身份，不提高 IPC frame 或 manifest 限额。

## 1. 已证实的缺口

| 入口 | 当前实现与实际限制 | 必须保留的行为 |
| --- | --- | --- |
| Mac restore | `NolaMac/Services/MacArchiveRestore.swift::appendMessage` 只上传 `kind == text && contentJSON == nil` 的正文；带 presentation JSON 的 text、tool/reasoning 等继续整体编码到 manifest | 同一 Part 可同时存在原始 text 和 JSON，不能删 JSON、拆 Message 或修改 kind |
| Application stage | `core/nola-application/src/archive_restore.rs` 限制 upload binding 对应 text Part，且 JSON 必须 None | 外部只提交 upload bindings，禁止客户端构造 canonical reference |
| Store stage | `core/nola-store/src/archive_restore.rs::validate_entry` 对所有内联字段累计限制 128 KiB，再限制编码后的单 entry 128 KiB；JSON 需通过原有 JSON 校验 | 超限应在不可见阶段失败，不能半恢复 |
| 正文对象 | `canonical_text.rs` reference 仅绑定 conversation/message/part；`message_part_text_references` 每 Part 最多一个引用，`published_part_body::reference` 要求 text kind、空内联 text、JSON NULL | owner 校验必须增加“字段”，两个正文不能串用 |
| UI/window | `bounded_message::load_parts` 对过大的非 text Part 直接报错；text JSON 放不下时返回 None，但只有 text 截断有 deferred 标记 | JSON 缺失不能被误解为原字段 NULL，不能用截断 JSON 反序列化 |
| Projection/context | `conversation_projection::project_stored_parts` 先解析 MessagePartView/AgentPart JSON；非 text 没有有效结构就失败。`lib.rs::durable_conversation_items` 从 AgentPart JSON 恢复 tool call/result、reasoning、compaction | 任意把 JSON 清空会丢工具身份、参数、状态、结果、provenance；不能降格成普通文本 |
| branch | `conversation_branch::copy_parts` 仅将无 JSON 的大 text 转 chunk；结构化 Part 和累计内联 message 受 256 KiB 限制 | 分支按字段有界复制并重绑 owner，不能 materialize 后再大帧 append |
| export | `transcript_export.rs` 已可对内联 `content_text` / `content_json` 分字段、修订固定、有界读取；只有 text 支持已发布 reference | JSON 的字节内容、NULL 与存在状态、原字符串转义与顺序必须原样往返 |
| sync/retirement | canonical text chunk/seal 与 archive final 的依赖关系已经工作；retirement 校验原 upload chunks 和 canonical chunks 的精确边界 | 加字段后不能借用别的字段的 seal、ACK、prepared 或 retirement 证据 |

本次审计发现的独立 lossless export 缺口已在后续 bounded metadata 修复中接通：`ExportPart` 增加 `content_digest` 和 Part 自身 created/updated timestamps，Store、Application ConversationPart、CLI 重建、Mac export/canonical DTO 与 restore writer 保留原值。真实 Service 混合 text/semantic Part 导出→archive restore→重启测试通过，Mac `MacCanonicalExportTests` 5/5 亦覆盖真实 export stream→backup decode→原子 writer restore；新备份准确保留原字段，只有缺少这些字段的旧 Mac 备份沿用 Message 时间 fallback。本方案的 JSON 大字段切换不再以这一问题为前置待办。

## 2. 选择：通用正文对象，加精确字段引用

采用同一 immutable UTF-8 blob/chunk 存储，引用绑定到 `(conversation_id, message_id, part_id, field)`。`field` 是封闭枚举 `ContentText | ContentJson`。这是 ARCHITECTURE_V2 中“大对象与元数据分离”的字段级实现；不引入文件路径或可被客户端任意读取的 content ID。

不把已有 Part 改成 `artifact` kind。现有 `ToolPart.full_output_ref` / `artifacts` 有独立的调用结果、provenance、保留策略语义，不能用它们替换一个完整 AgentPart JSON 并期待原 renderer/context 自动恢复。底层以后可共用 Blob 存储，但 Part 字段 owner、可见性、原始 JSON 和 Artifact 实体身份仍是不同契约。

最小 Domain 类型建议：

```rust
enum PartBodyField { ContentText, ContentJson }
struct CanonicalBodyReference {
    content_id: String,
    conversation_id: String,
    message_id: String,
    part_id: String,
    field: PartBodyField,
    total_utf8_bytes: u64,
    chunk_count: u32,
}
struct PartFieldUpload {
    upload_id: String,
    message_id: String,
    part_id: String,
    field: PartBodyField,
    expected_utf8_bytes: u64,
}
```

保留两个原字段，不把 JSON 重新编码进 text。每个字段独立遵守“inline 或 reference 二选一”：text 引用对应空 text 槽；JSON 引用对应空存储槽，但元数据明确 `present=true`，不能把 SQL NULL 槽当作规范字段不存在。无引用的 SQL NULL 仍是原始 None；零字节 text 保持内联。JSON 的字符串值 `""`、JSON null 字面值与 SQL NULL 不等价。原有非法 JSON 不因引用表示而变成合法输入。

所有字段引用使用同一名称/表示一次切换，不另留第二套可写的 text-only manifest。当前版本是否仍能纳入未发布 epoch 由集成方确认；如已经发布，必须显式 schema/Protocol 版本切换和受控迁移，旧库拒绝路径保持字节不变。不得在本子任务中自动迁移用户库。

## 3. Port、Store 与 publication

- UploadOwner 增加 field；begin/append/seal/progress/read 的身份检查覆盖 field。旧绑定不能被用于另一个 field。归档每个 Part 最多两个 field bindings，`(part_id,field)` 唯一，upload ID 也唯一。
- 将 CanonicalTextStore 的 immutable chunk 引擎推广为 body 对象；chunk 仍不超过 32768 UTF-8 bytes，保留原边界。对象 metadata 包含 field，不能只改表名不改验证。
- 引用表主键改为 `(part_id,field)`。archive prepared 和 upload retirements 持久化新 descriptor；publication/branch/restore 都通过同一 field resolver。
- ArchiveRestoreEntry、PreparedTranscriptPublication 中 references 改成通用 descriptor。外部 ArchiveRestoreManifestEntry 仍只允许 uploads，不接受 references。App 准备正文并记录 restore/import 归属，Store writer 最终验证所有字段对象完整后发布。
- JSON 上传是原 JSON 字符串的 UTF-8 bytes。准备阶段做有界流式语法验证，并保留原 bytes；不能先 `serde_json::Value` 全量加载再重编码。验证器需有固定输入缓冲和原有等价深度规则，超长字符串的跳过/解码不能分配与字符串长度等大的临时值；专门测试跨 chunk 的 escape、Unicode 和 surrogate。末尾垃圾、缺括号、错误 UTF-8 必须在 publication 前拒绝。
- 128 KiB entry budget 不变。客户端根据累计 manifest 编码预算选择把任一字段上传；不能只按单字段阈值判断，多个中等字段也会累计超限。最多 512 Parts 仍是上界，不承诺任意 metadata 数量都能塞进 128 KiB；body-independent metadata 自身超预算仍明确拒绝。不要提高引用上限而跳过最终编码预算检查。
- final 输入继续只有 restore ID、目录 CAS、entry count/digest 和提交时间。bodyfield 不进入全量 final metadata。保留当前同 ID 更新、清理终态 Run/从属记录、排除会话 tombstone、远端原子可见性和 completed replay。

## 4. 统一 reader 与语义投影

原始字段与 UI/模型工作集必须分开。新增/推广 `published_part_body` 为字段 resolver，提供 `metadata(owner,field)`, `range(owner,field,offset,limit)`, `full_for_explicit_consumer` 和预算内读取。所有操作先验证已发布 owner 与可见性，再读 immutable 对象。

- `TranscriptExportRepository` 沿用现有 field 参数与 revision pin，给两个字段都返回规范长度。保留已接通的 digest 和 Part 时间，不改导出编码格式的原字段值。
- 一般正文查询可复用 export 的 field-aware 原文 range 契约，避免暴露 upload ID。现有 `read_conversation_text_range` 和全文文字搜索保持“text 内容”的语义；JSON 源码查询需显式 field，不能默认把巨大 JSON 搜索命中冒充聊天正文。
- `DeferredTextPart` 推广为有字段和总长的 `DeferredPartField`。窗口必须显式区分 `Absent / InlineComplete / Deferred`；绝不能将 deferred JSON 交给现有 parser，或把 None 当作规范 None。有界预览可以展示原 text，但不取代 JSON 原文。
- 为 structured Part 增加可重建的 bounded semantic projection。Application 在准备流式 JSON 时提取已知结构的 kind、身份、tool call ID/名称/状态/provenance、可见摘要等有界字段；每个超过预算的 input/result/detail 字段有明确 deferred 来源。projection 必须绑定 source reference/digest、版本和 owner，不能由客户端提供，也不能变成第二份可编辑规范 JSON。未知 JSON 原文保留，不能强行解读为 AgentPart。
- `project_stored_parts` 接收该有界语义投影并显示按需详情；用户请求详情时通过原始字段 range 获取。不得简单放宽非 text Part 限制并反序列化 10M JSON。
- `restore_canonical_context` / `durable_conversation_items` 只消费预算内完整语义组件。小型完整 JSON 维持现有行为；大型 tool call/result 必须作为对应组合选择或整体省略，保留 call ID、provenance 和显式 omitted 原字段证据。不能伪造 `{}` arguments、空 tool result 或把工具结果改成普通 user text。若引入 Core 的 deferred tool payload，应作为明确 ContextPlan 类型评审，不偷偷复用 `output_preview` 充当完整 output。
- 文本 tail 的字符计数语义继续保留；字段 range 使用 UTF-8 字节。JSON range 返回原字符串片段，不宣称每片段都是独立合法 JSON。

## 5. sync、branch 与 retirement

| 部件 | 精确修改 |
| --- | --- |
| `canonical_text_replica.rs` | chunk/seal 带 field-aware reference；identity/replay 校验含 field；使用原持久化 chunk，禁止重分块保留旧 chunk_count |
| `archive_restore_sync.rs` | 对每个 entry 的两个字段都 enqueue body facts；final 等待所有字段 seal 与 stage/target ACK；接收 final 对全部字段 require_complete |
| `conversation_branch.rs` | 遍历两个字段；已引用字段逐 chunk 复制并绑定目标 conversation/message/part/field；大内联字段走同一 prepare 路径；原 JSON 不解析重写，semantic projection单独重建 |
| `content_upload_lifecycle.rs` | 独立只读主库确认已提交 prepared/publication owner，包括 field；回收前逐 chunk exactbytes/geometry比对；退休读取校验 field；主库未提交或回滚不得删 upload |
| ACK/pruning | 现有 final 依赖保留策略扩展为全部字段；不能收到 text seal 就允许含 JSON ref 的 entry/final 发布 |

Branch 的目标 Message/Part ID仍按 operation 稳定派生；archive restore 则保持备份原 ID。二者不能混用。既有 JSON 内嵌旧 AgentPart identity 的处理须明确：规范 JSON bytes 不改写；派生 semantic projection 对显示 owner 使用新 branch Part owner，保留来源 provenance。现 `MessagePartView` parser 对内嵌 ID 必须等于 Part ID 的规则会与 exact JSON clone 冲突，这也是 branch 消费侧必须一起改的接口，不能仅复制对象便宣称 branch 完成。

## 6. 可合并的实施切分与验收

1. **Domain/Port 与 Store 表示**：field identity、统一 resolver、JSON 流式验证、两个字段 prepared/final 校验、稳定 schema fixture；新表示尚不由生产入口写入。
2. **所有消费者与复制**：export/原文 range、显式 deferred 字段、bounded semantic projection/context、branch 双字段复制、sync seal/final、retirement field ownership。先对直接 Store fixtures 验证；不要只新增孤立 helper。
3. **Protocol/Application/Mac 同步切换**：唯一 manifest binding 加 field，上传有界原始 JSON；接 archive writer 和普通 publication；维持已接通的 export Part metadata。该合并点才启用新引用写入。
4. **正式通路验证**：Service subprocess 与真实 Mac restore→window→structured detail→first turn→export；Worker 实际 HTTP stage/final 同步及重启。使用同一套固定原始 fixture，不能拿纯文本成功替代富文本验证。

最低 fixture 集：单 Part 同时含 10M text 与 10M JSON；JSON 大字符串包含引号、反斜线、中文、emoji，完整原字节往返；非 text ToolPart 含大 input/result 且小型 call ID/state/provenance 不丢；两个会话各 100 条带双字段引用；多中等字段累计超过 manifest budget；同 upload 跨 field/跨 restore 被拒；正文缺 chunk、JSON 无效、目录 CAS/活动 Run、sync/journal 注入失败均保持旧目录；每批双方重启、最后 final 独立接受前旧历史可见；退休后按原 chunk offset 重读并精确重放。

资源断言保持现有 frame/entry/Chunk 上限，外加 JSON validator、投影和 context 峰值工作集不随单字段总长增长。对未知/超过语义预算的 JSON 保留原文且显式 deferred；不能静默 drop 字段后让测试通过。

## 7. 已实现但未启用的 JSON 验证准备接口

`nola-store::structured_json_validation::validate_json_chunks` 接受逐条 `(offset, String)`、每块最多 32768 UTF-8 bytes 的不可变来源与声明总长，返回总长、chunk count 和最大来源 chunk。它不写库、不生成规范 JSON、不接现有 publication/restore；field identity 完成后才作为准备门禁接入。

本地 `Cargo.lock` 固定 serde_json 1.0.151。源码审计确认：`read.rs::IoRead::ignore_str` 不积累大字符串，但 `ignore_escape` 明确不验证 Unicode codepoint；`de.rs::ignore_value` 用 scratch 栈替代默认 recursion guard，数字忽略也不做有限值规则。直接 `IgnoredAny` 会接受当前 Value validator 拒绝的 lone surrogate、超限数字和过深 JSON，不能用作门禁。

实现只提供一个词法验证视图：字符串普通 UTF-8 bytes 不保留，control byte 拒绝，单 escape/完整 surrogate pair 用最多 14 bytes 的带引号片段交原 serde String parser 校验；合法字符串在解析视图里变为空字符串。原始来源从未重写。其余 bytes 原样送入 serde Deserializer，由无 collection 的 Visitor 遍历 seq/map，保留与旧 Value validator 相同的数字、语法、末尾和 128 默认深度行为。错误位置映射回原始字节偏移，不使用缩短后的行列作为原字段位置。

差分测试覆盖 1/2/3/4/7/12/31/32768 bytes 分块、所有 ASCII control/escape、跨块 surrogate、字符串 key、重复 key、正负数/指数/下溢/溢出、126–129 层边界及源长度/offset 错误。独立 allocator 测试从懒迭代器生成 10M JSON 字符串、10M 对象 key、超过 10M 的 surrogate escape 字符串和 10M 数字，断言最大单次分配不超过 32768、总活跃 heap 峰值不超过 32768+4096；因此不是只用“每次读 32 KiB”推断没有内部全文缓存。

## 8. Phase 1 底层已实现，JSON 发布继续关闭

Domain 现有唯一 `CanonicalBodyReference` 和 `PartBodyField`。descriptor 的 `field` 必填；`CanonicalTextReference` 只是临时 Rust type alias，六参数 `new` 明确构造 ContentText，`for_field` 显式指定字段。`ExportBodyField` 同样 alias 唯一字段枚举。缺少 field 或未知 field 的 wire 均拒绝，不靠 serde default 猜测。已有构造器 fixtures 自动输出 field；CLI 两处手写 replica fixture 显式补 `content_text`。

`nola-store-port::canonical_body::CanonicalBodyRepository` 提供 `prepare_body` 与 `read_prepared_body_range`。Store 复用当前主库 `canonical_text_objects` / `canonical_text_chunks`，旧表名只是内部名称；没有新增数据库、对象复制体系或外部可写 manifest。字段与 conversation/message/part 一起参与 persisted reference 精确相等校验。JSON prepare 要求 owned transaction，共用 chunk writer 在 commit 前调用 §7 验证器，失败回滚对象/chunks/配额。只读 prepared resolver 是内部准备能力，不能作为客户端持有 descriptor 就访问任意原文的授权。

`message_part_text_references` 仍是本阶段仅文本可见的引用表，尚未改成双字段可见表。`canonical_body::require_text_reference` 显式围住旧路径：text publication/append 与 archive prepare/stage、replica chunk/seal/enqueue/append、published Part resolver、text search、旧 standalone text adapter、upload adoption 和 retirement（包括退休 seal/EOF read）。即使 JSON 对象已经完整 prepared，或测试直接植入旧引用/退役证据，它也不能进入这些文本路径。

底层验收包括：同一 Part 分别准备原 text/JSON、跨 field 与跨 owner 读取/重放拒绝、保留原 JSON 空白/重复 key/转义、10M JSON 单对象冷重启范围读取、无可见 Message 或 sync facts、无效 JSON/中断/外层 rollback 保留原对象，以及前述文本入口围栏。下一步仍需消费者、复制和 manifest 一起完成后，才能开放 JSON field publication；本阶段没有完成这一启用条件。

Phase 1 实测回归：Domain 11 个 unit + 6 个集成测试通过；Store lib 98/98；正式 Service archive 1/1、export 2/2（含 metadata 原样恢复）、uploaded publication 1/1（含 10M 正文）通过。集成方同轮 Application command receipt 3/3、archive receipt 2/2 通过。旧纯文本生产路径保持可用，JSON 只在内部准备层验证/读取。

## 9. Phase 2 原字段消费者已完成，语义消费者仍待接入

当前已将 §8 当时保留的 text-only 物理引用表替换为唯一 `message_part_body_references(part_id, field, reference_json)`，主键 `(part_id,field)`，field 非空且只允许两个枚举值；不存在旧表或双写 view。既有纯文本 publication、archive、replica 与测试写入明确指定 `field='content_text'`，文本搜索/复制查询只 JOIN 对应字段，避免一 Part 双字段造成重复行。

`published_part_body::resolve_field` 验证 conversation/message/part 的完整归属及 canonical visibility，返回 `Absent | Inline{total_utf8_bytes} | Referenced(CanonicalBodyReference)`。引用字段必须已完成，descriptor owner/field 必须与绑定行和 immutable object 精确一致，且只允许本字段空存储槽（text 为空字符串；JSON 为 SQL NULL）。两个不同字段互不排斥；JSON 引用存在时，物理 NULL 槽不是逻辑 absent。非引用的旧 inline 字段保持原有状态读取语义。

`field_range` 统一读取任意 Part kind 的两个原字段；固定 UTF-8 byte offset 与 4..32768 range budget，保留 Unicode 边界和 EOF。`transcript_export` page/range 全部走该逻辑，metadata 同时报告两个字段的逻辑长度，保留原 digest/Part 时间。旧 text `reference` wrapper、prefix/tail 继续显式拒绝 JSON；window/context/branch 的语义恢复尚未开放，不能把这一原文字段能力当作完整结构化呈现已实现。

新增 `structured_field_export_e2e` 隔离 Store fixtures 3/3：同一个 tool_result Part 同时引用两个 10,000,000-byte 字段，冷重启后完整 20MB 逐 chunk 对比且 metadata 不重复、仍小于 4KiB；Unicode 字节边界；SQL NULL、存在的零字节 JSON 字符串、内联/引用 JSON literal `null` 四种状态；错误 owner/field、双来源冲突、未完成引用、projection 隐藏全部拒绝。JSON 绑定只由隔离 fixture 直接插入，生产 JSON writer 依然关闭。

本阶段最后回归：完整 `nola_store` 测试 188/188（98 unit + 90 integration）通过；正式 Service export 2/2 通过。集成方同轮 Application lib 74/74、archive receipt 2/2、turn transaction 14/14 通过，CLI replica 8 passed / 0 failed / 2 ignored。后续门禁保持不变：semantic projection/window/context/branch 与完整 field-aware sync/retirement 尚未整体切换，外部 JSON upload/publication 不开放。

## 10. Window / context / branch 的唯一语义投影（只读审计与下一切分）

本节是待实施契约，不表示 JSON publication 或结构化消费者已开放。以下路径核对于本次 Store 双字段 resolver/export 落地后。

### 10.1 已证实的消费分歧

- `nola-application/src/conversation_projection.rs::project_stored_parts` 先尝试完整 `MessagePartView`，再尝试完整 `AgentPart`。前者要求 JSON 的 `part_id/kind` 等于 DB Part；后者直接调用 `project_core_part`，使用 JSON `id`，没有同等 owner 校验。`AgentPart` 的 ownership 另含 `message_id/turn_id`（`core/src/agent/parts.rs`）。相同原文在两种格式下不是相同身份行为。
- `nola-store/src/conversation_branch.rs::copy_parts` 为目标 Message/Part 派生新 ID，但小结构化 JSON 原样复制。因而 `MessagePartView` 分支会拒绝新 owner，`AgentPart` 分支则可能把旧 ID 带回新窗口。大结构化 JSON 当前被预算或 text-only resolver 拒绝；这是真实门禁，不应通过删除校验放开。
- `nola-application/src/lib.rs::durable_conversation_items` 只解析 `AgentPart`，从 JSON 内嵌 ID 建 AgentItem；已有 `MessagePartView::Tool` 并不会恢复为 ToolCall/ToolResult，而可能降为拼接 `content_text`。其中工具状态、call ID、input、structured_output/full_output_ref 决定上下文语义，不能只修 renderer。
- `nola-store/src/bounded_message.rs::load_parts` 只允许 text 预览，非 text 超出剩余预算立即失败；text JSON 有时被整体省略。`canonical_branch.rs::restore_canonical_context` 用最多 32 条消息、`(prompt_token_limit / 4).clamp(128,32768)` 字节预算恢复，省略信息目前只有 `DeferredTextPart`。这不能表达“工具 input 尚未完整载入”。
- `MessagePartView::Tool.input/output` 和 `StructuredValue` 目前要求完整值，Mac `MacApplicationPort.swift::ToolPart` 同样解码完整值。因此不能把大 input 截成 JSON 字符串或塞一个伪造 `{omitted:...}` 对象冒充原参数。需要统一 DTO 增加显式延迟状态，所有客户端一起迁移。

### 10.2 唯一数据流与所有权

采用一份 **CanonicalPartSemanticProjection**（Domain 中不含 IO 的规范派生类型；以下名称为提案），同一份投影供 Application 的窗口、上下文选择与分支恢复消费。停止从 `StoredPart.content_json` 在不同消费者内分别 `from_str::<AgentPart/MessagePartView>`。原始字段及其 NULL 状态仍由现 `CanonicalBodyReference` / field resolver 管理；投影不是新正文、可写 manifest 或客户端输入。

Store 负责从原始流生成有界中立语义数据，Domain 定义版本化语义契约，Application 映射到 AgentItem / MessagePartView；Store 不依赖 Kernel 的 `AgentPart` 实现类型。支持的两种历史 wire shape（AgentPart 与 MessagePartView）各用一个明确版本适配器，输出同一类型；对小对象用现 serde DTO 作差分 oracle。schema 判定不依赖键顺序，也不从文本内容推断种类。

投影必须绑定：`schema_version`、规范 `conversation/message/part` owner、DB Part kind/state、两个字段各自的 source identity（reference 全值或 inline 精确字节摘要/长度）、规范修订。`content_digest` 是原备份业务字段，不可假设它就是 source 校验哈希。拟用 `message_part_semantic_projections(part_id PK, source_key, schema_version, projection_json)` 作为**本地可重建缓存**；不增加 sync 可写事实。缓存命中需 source/owner/version 全等，未命中在有界准备阶段生成，发布前要求可消费状态已确定；远端 final 也从同一原文字节生成或验证，不信任来源端提供的投影。

规范 DB owner 永远是显示、window mutation、activity action 与 AgentItem ownership 的主体。原文的 `id/part_id/message_id/turn_id` 只作已验证的来源身份记录。普通 append/import 在首次建立来源绑定时验证已识别 schema 的内嵌 owner 与当前身份；无法证明的历史格式保存 raw 且标记 Unsupported，不能因 AgentPart fallback 而绕过校验。

Branch 要新增受 Store branch 事务控制的 Part 来源绑定（拟表 `message_part_derivations`，每目标 Part 一条）：目标 owner、operation ID、源 owner、源字段 identities、源内嵌身份。只允许从当前可见源 Part 派生，不能让客户端任填“原 owner”豁免校验。branch-of-branch 保留根来源身份并记录直接父来源；目标投影从已验证源投影重绑定生成，raw JSON 两字段逐字节不改。备份必须携带这个有界来源证明，或者在恢复前根据归档 lineage 与可验证源绑定重建；仅复制现 conversation-level lineage 不足以可靠恢复已经失去源会话的 branch 备份。缺失证明的旧备份仍可 raw 恢复和导出，但不能冒充已验证结构化语义。

### 10.3 最少字段与大值表达

投影使用 `SemanticValue = Complete(typed bounded value) | Deferred(locator, value_type, total_bytes, optional bounded preview)`；Absent 与 JSON literal null 分开。locator 指向规范 owner/field/source identity 和原始 JSON token 的 `[start,end)` 字节区间，不复制大值，也不制造第二个 content object。String token 的范围包含引号与原转义；预览是明确标记的 decoded excerpt，不能当原始 range。完整字段仍通过已有 original-field range 获取；子值读取也必须验证当前可见 owner/source，不能按任意 content_id 绕过权限。

| kind | 必须精确保留的有界语义 | 可延迟的正文/集合 |
| --- | --- | --- |
| text | 规范 owner、完成状态、原 text 来源 | markdown/text；若 JSON 也含 text，明确 schema 主来源，不拼接或覆盖原 `content_text` |
| reasoning | 状态、title 来源、provider continuation 是否存在 | detail/summary；历史恢复不伪造 provider continuation |
| tool / tool_result 的已识别结构 | call ID、capability ID、状态、origin、开始结束时间、只读permission-display；保留output仅公开message/Part/call owner | input、structured_output、output_preview、summary、permission rationale、artifacts；不公开raw_input、私有full_output_ref或它们的locator |
| source / citation | source ID、origin call ID、页/行/range、mime type | title、uri、evidence；外部 resource identity 不改写，超限 URI 以 locator 延迟 |
| artifact / attachment | attachment 身份、资源类型 | name、uri、artifact detail，不能转成普通 text |
| step / task / skill | task/run/skill 身份、状态、计数、取消标记 | title、progress detail、结果引用和 failure |
| compaction | token counts、summary 来源、retained item identity 来源 | summary、retained IDs 集合；原摘要不是自动生效的 Core checkpoint |
| 未知或 schema 不兼容 | DB kind/state、raw fields 来源、明确 Unsupported 原因 | 完整原 JSON 仍可查看/导出，不猜测工具行为 |

控制身份与 discriminator 有硬上限，超限不能截断后继续匹配；应成为“原文保留、语义不可用”的明确状态，并在需要模型恢复时拒绝或省略该语义单元。长 title/URI/数组不是控制身份，可延迟。投影自身上限建议 8 KiB/Part，但整个 window 仍按现有响应预算计费；512 Parts 不能各自无条件占满 8 KiB。使用剩余窗口预算选择 Parts/消息并返回 continuation/deferred 元数据，不提高 frame 限额。

私有执行字段仍保留在原始canonical备份中，但普通历史/public DTO既不展示其值，也
不制造可点击locator。显式原文审计/export须经过独立owner/source、可见性及review
披露校验；保存原文不授予普通工具卡读取私有路径或恢复历史授权的能力。

### 10.4 有界提取算法与接受集合

现 `structured_json_validation` 的 StringView 会清空所有字符串，仅能验证语法，不能直接用作语义解析器。下一步在同一 32 KiB immutable chunk cursor 上新增 streaming token/span 提取器，复用已验证 escape/surrogate/深度/有限数值规则，记录**原字节** offset。不构造完整 Value，也不调用会积累巨大 key/string 的 `deserialize_string`。

推荐两次顺序扫描：第一遍验证并取得顶层 discriminator、字段出现信息；第二遍按所选 schema 提取固定已知字段、token spans、受预算限制的 scalar/小集合。这样不依赖 kind 位于 input 前面。未知或超大 key 使用固定长度匹配状态流式跳过；已知字段名即使采用 `\u` 转义也须按解码后名字识别。深度维持既有 128 规则；临时内存由一个 chunk、固定深度栈、受限 key/control scalar 和固定总投影预算组成，不能为一个元素百万数组生成百万 locator。仅存 schema 有关固定字段与有限集合前缀及明确 deferred 集合 span。

JSON 语法接受集合与现 validator 完全一致，不能因语义提取额外拒绝 raw JSON。已知 schema 的重复控制键、矛盾 shape、错误字段类型可标记 Unsupported；未知 JSON 合法仍然保存。需显式区分这与 serde Value 的重复键 last-wins 语法接受行为：语法验证通过不等于安全取得工具身份。支持 schema 应对完整小样本做差分，重复键/歧义不得在两个 adapter 中选出不同工具行为。所有省略都携带 type 和 span，不把 `null` 当缺失、不把空对象当超预算 input。

### 10.5 Window、context 与 branch 的具体消费规则

Window 只读投影，返回规范新 owner，工具状态/input/output 保持 typed Complete 或显式 Deferred。Mac/Web/mobile/Android 不再从 raw JSON/output_preview 反解析工具参数，不把 Deferred 当可执行请求。历史 waiting_for_permission/pending 只显示历史状态；没有当前 live Run + receipt 的真实授权请求不可出现可执行批准按钮。

Context 从同一投影选择语义单元：完整工具输入且 call/result 在本轮预算内时生成成对 AgentItem；input 不完整时不生成截断 arguments 的 ToolCall。工具结果过大时可以生成明确“结果省略且有原文定位”的独立上下文说明，或整体省略 call/result 单元，不能把摘要伪装成原结构化 output。Call/result 的选择和省略信息计入同一个 prompt 预算；单个语义单元超限不导致加载全量。未知结构化 Part 必须在 context selection 中显式报告省略，不能静默退为 content_text 并宣称完整历史。Compaction 只恢复历史摘要内容，不能直接用来源 retained IDs 发起新的截断/rebase。

Branch 的 AgentItem ID 全部从目标规范 Part + item role 派生（例如 call/result），不能沿用 JSON 中旧 Part ID。call ID 在目标会话内由已验证来源 call identity 稳定映射，ToolCall/ToolResult/Citation.origin_tool_call_id 共用映射；不得按出现顺序临时编号。来源 source/artifact 的外部身份保持原值，显示 attachment/task 身份按其实体是否派生分别处理，不把 URI 重写成新 ID。Compaction retained IDs 只有在 branch 可见前缀映射中找到对应项时才关联；缺失项保留来源 provenance 并标记不可解析，不能误指向源会话的当前项目。Branch first turn 继续使用 bounded selection + RebaseConversation，不复制旧 checkpoint、不恢复历史运行中的工具。

### 10.6 可独立实施的切分与开放门槛

1. Domain 定义唯一投影/Complete-Deferred/locator/来源绑定；StorePort 仅暴露读取与有界准备接口。Store streaming extractor 用 10M key/value/array、跨 chunk escape、重复键和双格式差分测试证明内存上限。
2. Store 引入可重建投影缓存及 branch Part 来源绑定；修改 bounded_message 不再塞截断 JSON。prepared、archive final、remote final 在各自事务中校验来源与可消费状态；失败保留旧可见状态。
3. Application 同时替换 `project_stored_parts` 和 `durable_conversation_items`，context 明确选择完整工具对/省略；branch exact 双字段 clone + 新 owner 绑定。不要仅修 UI 的 `part_id`。
4. Protocol/FFI 与所有客户端一次切换 Deferred typed rendering、原字段/子值读取和历史工具非执行态；归档 DTO 增加可信来源证明的保存/恢复契约。
5. 验收包括同 Part 双 10M、各种非 text kind、MessagePartView/AgentPart 两种格式、branch-of-branch、源会话删除后备份恢复、第一次新轮工具对、未知 JSON/raw fidelity、跨 DB 同步 final 后投影等价、完整 IPC frame/allocator bounds。所有消费者完成前保持 JSON 可见 publication gate；先支持 JSON chunk/seal 准备不意味着已完成语义开放。

### 10.7 下一实现包的文件与接口边界

以下签名是下一步可直接施工的接口草案，不是已存在 API。先实现 A+B 的纯准备/读取测试，不增加外部 JSON writer。

| 实现包 | 文件与输入 | 输出与事务责任 |
| --- | --- | --- |
| A：纯契约 | 新 `nola-domain/src/part_semantics.rs`：`CanonicalPartOwner`、`SemanticSourceKey`、`SemanticSchemaVersion`、`SemanticTokenLocator`、`SemanticValue<T>`、`CanonicalPartSemanticProjection`；`SemanticProjectionState::{Supported,Unsupported}` | 不依赖 serde Value 的无界集合。完整 scalar/结构只在显式 byte/item budget 内；Unsupported 带稳定 reason code 与 raw sources，不携带不可控错误原文 |
| B：解析与准备 | 新 `nola-store/src/semantic_json_projection.rs`：`extract(chunks: Iterator<Result<(u64,String)>>, owner, part_kind, part_state, source_key, budget: SemanticProjectionBudget) -> SemanticProjectionDraft`；内部分两遍时接可重新打开的 ChunkSource 工厂，不 collect iterator | draft 包含固定数目 raw token spans、来源身份、语义值、schema/status、实际 resident bytes 统计。输入 chunks 仍最大 32768 bytes；语法失败是错误，schema 不支持是明确结果 |
| B：持久化 | 新 `nola-store/src/part_semantics.rs`：`prepare_on_connection(&Connection, owner, expected_sources, budget)`；`load_on_connection(&Connection, owner, expected_revision, budget)` | prepare 只读取 owner-bound prepared/inline source，写本地缓存；引用尚未发布时不能经 load 返回。load 校验可见性、sources、版本后返回投影；prepare 的调用者拥有 publication/restore/sync final 事务 |
| B：Port | 新 `nola-store-port/src/part_semantics.rs`：`PartSemanticRepository::load_part_semantics(owner, expected_revision, budget) -> CanonicalPartSemanticProjection`；`read_semantic_value(locator, expected_revision, offset, limit) -> bounded range` | Port 不返回原 AgentPart，不允许客户端写投影；子值范围限额沿用原 body range，过期 source/revision 拒绝，String decoded preview 与 raw bytes 用不同字段 |
| C：Application | 新 `nola-application/src/part_semantics.rs`：`project_window_part(&projection, window_budget) -> MessagePartView`；`select_context_unit(&projection, role, context_budget, identity_map) -> Selected/Deferred` | selected 返回 bounded AgentItems + ownership；deferred 返回精确遗漏类型/locator。旧 `project_stored_parts` 与 `durable_conversation_items` 改为薄调用，不再各自解析 JSON |
| C：分支 | `conversation_branch.rs` 增 `derive_part_semantics_on_connection(source_owner,target_owner,operation_id,source_keys)`，同事务写 `message_part_derivations` 与目标 cache | 输入必须从现存源读得，不接受外部覆盖身份。返回新投影/来源绑定和 dual-field refs；每 Part 有界，无全会话映射 Vec。关联 call/item 可用按目标 operation 的 SQLite 索引查询 |
| D：客户端 | Domain `projection.rs`、Protocol DTO/FFI、Mac `MacApplicationPort.swift`、其他正式 projection consumers | 统一 typed Complete/Deferred 后再启用 C 的外部输出；不提供长期新旧两套可写协议。窗口/详情均按 locator 调读取请求；禁止把原文读取变成隐式执行 |

A/B 的首个可验收提交应只增加类型、提取器、缓存准备 seam 与分配器/差分测试，保持现 publication fences。C/D 与 branch/archive provenance 的 wire 保存完成后才是实际消费者切换；“准备能解析 10M JSON”不能替代这个切换验收。

### 10.8 已实施的 token/span 准备 seam

`nola-store/src/semantic_json_projection.rs` 已提供 `extract_top_level(open,total_bytes,known_fields,maximum_output_bytes)`；`open` 可重新打开最大 32 KiB chunks。第一遍使用既有严格 JSON validator，第二遍提取原始 token 半开区间，并要求两遍 SHA-256 相等。扫描不依赖 `kind` 的位置；顶层固定字段上限 64、字段名 256 bytes、输出预算最高 32768 bytes，元数据与完整 scalar 共用预算。巨大未知 key 不累积；对象/数组只输出 Deferred span。小 scalar 仅在原 token 能放入临时预算时解码，保守 Deferred 不改变 raw 数据。

输出 `TokenProjection{root_kind,fields,source_sha256}`，每个 `FieldToken` 包含字段索引、原 span/kind、`Complete(ScalarValue)|Deferred` 和 `duplicate_known_key`。转义后同名重复键只保留第一 span 作诊断，强制 Deferred 并标记歧义；adapter 必须拒绝把它变成唯一可消费 locator，不能采用 first-wins。非 object 的合法 JSON 返回 root kind 与空字段，不伪造语义 schema。

真实验证：4 个 unit tests 通过，覆盖与 serde Value 的语法接受集合差分、全部 ASCII control/escape、surrogate、深度边界、跨 1/2/3/7/31/32768 bytes chunks、Unicode/escaped keys、精确原 token 与 scalar、重复键、预算边界、重开 source 变化/截断/错误/非法 offset。独立 allocator integration 1/1 通过：10M string/key/array/escaped-surrogate 输入均生成于惰性 iterator，最大单次分配 32768 bytes，峰值 live heap 分别 36744 / 32922 / 32926 / 36744 bytes（output budget 4096）。全部模式在巨大字段之后仍正确取得 `kind=tool`。该 seam 不写数据库，不开放协议入口，也未替代 window/context/branch 消费者。

### 10.9 已实施的两种历史 schema draft（尚非可执行投影）

`nola-store/src/semantic_part_draft.rs::extract_draft(open,total_bytes,expected_part_kind,maximum_output_bytes)` 已落。首先以固定 identity 列表识别 `kind/id/part_id/message_id/turn_id`，再用对应 schema/kind 固定字段表生成 draft；两个阶段各自重开校验，并要求跨阶段 source SHA-256 也一致。AgentPart 的 8 种原 kind 与 MessagePartView 的 10 种原 kind 均有明确规则，不把 `artifact/step` 偷换成原 JSON 的 `attachment/task`，不对失败 MessagePartView 任选 AgentPart fallback。

输出 `SemanticPartDraft{schema,kind,status,field_names,tokens,pending_nested_validation}`。`DraftStatus::Recognized` 只表示顶层 schema/控制字段已识别，不代表内嵌 owner 可信、typed nested payload 已验证或工具可执行。`Unsupported` 携带稳定 reason 与字段名，区分缺失、重复、身份格式歧义、未知 kind、DB kind 不符、类型错误、控制值超预算/非法；raw token/source 证据仍保留。Typed nested `artifacts/affected_resources/permission/origin/evidence/location/range/artifact/retained_item_ids` 仅检验顶层种类，进入 `pending_nested_validation`，后续消费前必须完成 bounded nested adapter。

提取器新增固定 scalar decode mask；draft 第二阶段只为身份、状态、bool 控制及需要范围校验的数值解码，正文始终 span/Deferred。因此靠前的中等 markdown/title/summary/input 不会挤占靠后的 kind/owner/call ID，预算未扩大。可延迟正文不是空串/null/伪造对象，下一消费层须通过 owner-bound locator 选择真正需要的片段。

真实验证：draft unit 4/4（18 种实际字段形状，MessagePartView 以现 Domain serde decoder 交叉检查；控制状态/default/type/u32 边界、重复 escaped keys、双身份歧义、变源、invalid JSON、超大 ID、DB kind mismatch）；其中包含两 schema × text/reasoning/tool × 正文前后共 12 个逼近预算回归。独立 draft allocator 1/1：两种 Tool JSON 的前置 10M input、后置 kind/identity 均 Recognized，input Deferred，最大单次分配 32768 bytes，峰值 AgentPart 35347 / MessagePartView 34406 bytes（预算 8192）。原 token extractor allocator 同时复验 1/1。该模块没有依赖 Kernel AgentPart 实现或新增公开 writer，owner/lineage 信任与窗口/上下文实际切换仍由后续 adapter 负责。

### 10.10 已实施的首批 typed nested draft

新 `nola-store/src/semantic_nested_draft.rs::validate_nested(open,total_bytes,kind,maximum_output_bytes)` 接受由上层绑定好的原 JSON token 子流，chunk offsets 从该 token 的 0 开始，返回相对 token 的原始半开 spans；转换回 field locator 与 owner/lineage 信任仍由上层负责。支持 `NestedKind::{Origin,Permission,AffectedResources}`，不声称其他 pending nested schema 已完成。

`NestedDraft` 返回 `Validated | Deferred | Unsupported(NestedIssue)`、子流 `source_sha256`、origin/permission 的有界 `object_tokens`、完整验证后才提供的 `resources`、`total_items` 与 `effective_origin_kind`。Origin 按实际 HostCapabilityOrigin 识别 unknown/builtin/skill/mcp/device/extension/internal，缺省 kind 为 unknown，其余可空 string；Permission 按实际 ToolPermissionPart 验证 request_id/permission_domain/rationale 与可空 decision。两者允许原类型允许的未知扩展，但已知重复键（包括 escape 后同名）明确 Unsupported。Validated 不提供批准或恢复执行权限。

AffectedResources 用新增 `semantic_json_projection::extract_array_items` 顺序扫描原数组，最多保留 16 个 item；整个 item raw 暂存与元数据共享原预算。只有已经完整放进硬预算的小 item 才交给与 Domain 同形、deny_unknown_fields 的三 string DTO 校验；不会对巨大 item 或完整大数组反序列化。每个 item 保留精确原 span，所有 item 通过才返回资源列表。任一超预算或数组超过 16 项，返回 Deferred 且 resources 为空，不能将首批元素误当完整集合；无效 item 返回 Unsupported，同样没有部分成功资源列表。两遍 source SHA-256 一致性与原 JSON 严格 validator 继续生效。

真实验证：nested unit 4/4，覆盖 origin 枚举/default/nullable/重复键，permission 原 rationale/转义/缺失/类型，resource 数组逐 item 原 spans 与真实 Domain decoder 对比、deny_unknown_fields/重复字段、空数组、17 项上限、巨大单 item、非法语法与重开变源。相关 semantic 模块专项整合 18/18。独立 allocator 1/1：10M provider_title、10M rationale、10M resource URI、10.8MB/200000 个合法 resource 项，最大单次分配均 32768 bytes，峰值分别 40659 / 40658 / 39112 / 39112 bytes（预算 8192）；四种输入均明确 Deferred。artifact/evidence/location/range/retained IDs 等 typed nested 仍待后续实现，window/context/branch 与 JSON writer gate 未在本阶段切换。

### 本地分支身份绑定与后续消费者切换审计

Kernel新增纯`assess_semantic_identity_with_derivation`，Domain新增
`VerifiedLocalDerivation`。当前owner直接匹配仍Canonical；否则仅接受Store重验过的
LocalBranch记录，并核对当前target owner/run、root owner及AgentPart root.run_id。
只匹配immediate、不匹配root不能通过；root未知、Replica/Archive均不升级。
这是历史描述身份绑定，不授权工具、权限或Run恢复，也尚未映射跨Part call/result/citation引用。

Store owned draft现在在同一读snapshot取得原JSON draft与实际target两原字段指纹/来源记录，
另外明确比较draft SHA与proof.target JSON SHA，防止未来拆读引入TOCTOU。
两种历史schema、两代本地branch、源删除、原JSON不变、外部origin不升级已通过；
目标原字段改变会拒绝。Kernel41/41、初版Store identity6/6。

嵌套source adapter已按实际AgentPart schema接Source.evidence、Citation.location/range，
与工具origin/permission/resource按不同parent kind校验；citation range/location locator专项
2/2通过，涵盖Unicode原escape保真、过期revision与错parent拒绝。
底层nested扩展与真实Core serde oracle由body_readers整合中；不得把尚未验证的类型当已完成。

实际消费者审计见`V2_SEMANTIC_CONSUMER_CUTOVER_PLAN.md`：除window/new turn，
idle revert无checkpoint分支也调用旧durable decoder并full load，需要共同切换。
公开window/context仍未使用新typed语义投影，JSON可见发布仍关闭。
root全Store56734（`/tmp/nola-semantic-local-binding-store-final.log`）与CLIcheck47429
（`/tmp/nola-semantic-local-binding-cli.log`）此条记录时待终态。

### 10.11 Evidence / Location / Range typed nested 增量

本节保留当批实现与验收历史，后续合同更新见下方2026-10-01补注。

当批新增 `NestedKind::Evidence/Location/Range`，仅对应 AgentPart Source.evidence 与 Citation.location/range；当时MessagePartView citation 使用已平铺的数字字段，不伪造这些子对象。内部 `semantic_nested_draft/evidence.rs` 按实际 `SourceEvidence::Memory{usage:MemoryUsage}` 校验 required/default/nullable 字段、MemoryScope/MemoryProvenanceKind、adjacent-tag MemoryNamespace 与 supersedes string array。Location 的 page/line_start/line_end 为 optional u64，section 为 optional string；Range 要求 start/end 为 u64，不另加当时原 DTO 没有的 start<=end 规则。

每次子 token 读取都顺序耗尽整个 parent immutable source，并检查 parent SHA（包含子 token 后缀）；子流移动/截断已拥有的 chunk，不额外保留大源缓冲。usage scalar 与深层 namespace/supersedes 复用一份剩余预算；supersedes 超过 16 项或任何已知大字段无法完整取得时明确 Deferred。未知字段、已知重复键按实际 serde DTO 规则处理，没有 whole-Value 大对象解析。

共享 `core/fixtures/semantic_nested_wire_cases.json` 的 29 个案例同时进入真实 Core 类型 oracle（`core/tests/semantic_nested_wire_oracle.rs`）与 Store bounded validator，各 1/1 通过，包括 personal namespace 的 name:null 可接受、name:string 拒绝、未知扩展允许、整型边界与 duplicate escaped key。新增 nested 单元前三项先通过，之后 shared-case 定向补验通过；root 对合入前版本完整 Store 238/238、Kernel 41/41、CLI bins check 已通过。

最终 allocator 2/2（原模式与新增模式）：新增 10MB usage.title / location.section / range decimal token / namespace.custom.name / supersedes 数组全部明确 Deferred；最大单次分配均 32768 bytes，峰值依次 36915 / 40656 / 40656 / 36920 / 36915 bytes（8192 budget）。root 已接 owner-bound adapter；窗口、上下文与历史工具消费尚未由本增量开放。

2026-10-01合同更新：公开MessagePartView也有typed Source.evidence与Citation.location，
Store以对应public schema验证，不再是“public仅flat”的合同。Shared Domain严格
namespace unit variant拒绝任何name字段（含null），custom要求String；未知public
字段拒绝，不能清洗后升级为合法public值。配对byte range须同时存在且有序；location
的page/line端点独立，可保留0、仅一端或原始观察顺序，URI为opaque描述值而非授权。
本批Store/Domain/Core实际oracle与Application严格冷读证据见
[整体改造第8.2节](MODULAR_REDESIGN_2026-09-28.md)。上面的29case/allocator是历史
源码结果，不据此宣称新合法域已验收或改写其原通过记录。

## 11. 共同历史读取与实际 Store window 切换

`nola-store/src/historical_part_projection.rs` 已实现 Domain/StorePort 的 `read_historical_part(owner, expected_revision, maximum_output_bytes)`。一个 read snapshot 覆盖 canonical owner、revision、可见性、原字段 source、identity assessment、nested validation 和 token 读取。无规范 JSON 返回 None（即便余量为 0）；有 JSON 返回 `HistoricalPartProjection`，不再让窗口/上下文各自解析 AgentPart / MessagePartView。未识别 schema、不可信内嵌身份或未支持/非法 nested 以明确 Unsupported 返回，原 source 仍可查询，不能产生可执行历史字段。

已验证字段以原 token 读取后解码为 `StructuredValue`，包括 JSON String 的真实转义/Unicode 语义；超预算字段返回 `SemanticValue::Deferred` 的原 JSON locator，不放伪造 null/空对象/预览。控制字段先于正文，再按 schema 固定字段顺序分配预算，因此原 JSON 的 input/markdown 位于前后不会改变 owner/call 完整性。最终实际 JSON 编码必须 <= caller budget（最高32768），过小连元数据都容不下时明确错误。额外 256 JSON value node 预检限制小数组语法的内存放大，超限同样 Deferred。已支持 nested 使用统一 adapter；未实现 typed collection 只接受确切完整空数组（artifacts/retained_item_ids），非空仍 Unsupported。

Store `bounded_message.rs` 与 `conversation_read_repository.rs` 的 BeforeOrdinal 实际路径已同时切换：选页按 owner-bound 两字段原 UTF-8 长度计费，不再经过旧 text-only message_bytes；所有选中消息统一走有界窗口读取，不只 oversized 第一条。content_text 对任意 Part kind 保留 UTF-8 安全预览与 `DeferredTextPart` 原长度；content_json 不进入 window 的 StoredPart 载荷。此处 None 是“未 hydrate”，不是原字段不存在，Application 必须对每个规范 Part 调共同读取器，以其 None 判断原 JSON 缺失。All/Offset 的旧完整 canonical 读取语义未在本步骤隐式改成预览。旧无调用的 text-only prefix/message_bytes 已删除，其他 publication/search/full-text 门禁保留。

本轮验证：共同 reader unit5/5，包括真实 typed Tool 字段、identity/revision、Unsupported、256-node、前后 input 同预算、10MB immutable JSON original locator；append_messages7/7 与双10M字段 export3/3 改为验证新窗口行为且精确原文仍可查询。最终 Store 全套151unit+94integration=245通过；另新增完整 Store window+reader allocator1/1（目标在全套启动后创建），10MB原JSON最大Rust单次分配32768、峰值live heap67960bytes，projection编码<=8192。共246个唯一tests通过。Root 正在完成实际Application window/context与各端消费验收；本节不表示外部JSON publication或所有客户端已开放。

### 11.1 规范附件、未知 JSON 描述与 typed budget

实际 Application 用户附件写入 canonical `attachment`，其 JSON 是 `AgentPart::Artifact`；模型产物写入 canonical `artifact`。共同 draft 只为 AgentPart 增加这一确切 `artifact -> attachment` 对应，仍要求 canonical/verified derivation 身份，不对其他 kind 猜测或降级。`NestedKind::Artifact` 按真实 `ArtifactReference` 校验 uri/name 必填字符串、mime_type/operation 可缺失或 null 字符串；未知扩展遵循原 DTO，已知重复键拒绝，巨大字符串明确 Deferred。

`HistoricalPartProjection.canonical_text` 为可选 `{text,total_utf8_bytes}`，独立承载规范 content_text 的 UTF-8 安全描述性预览；Unsupported 也能保留原文本，不将未知 JSON 转成可信字段。预览最多1024原UTF-8字节，使用剩余编码预算并预留最坏转义开销，最终实际序列化不超过调用预算；不会挤出语义字段或掩盖超预算。全量原字段仍通过既有 owner-bound 原文查询。

StorePort 新 typed `read_historical_part_with_budget` 返回 `Absent | Available(Box<HistoricalPartProjection>) | BudgetDeferred{canonical_kind}`。先核对 owner、revision、可见性与 JSON 是否存在，再判断预算；预算0不能绕过鉴权或把无 JSON 误报未加载。旧 Option reader 为兼容包装，BudgetDeferred 映回明确错误。Application/各端可把 typed 预算结果映射 NotLoaded/SelectionBudget，不匹配错误字符串。

增量 Store reader6/6，包括实际 append API 写入用户附件形状、错误内嵌 owner 拒绝、opaque JSON 原文预览、预算/版本/归属/Absent；Artifact nested 专项1/1。10MB共同reader allocator1/1通过；新增Artifact字段 allocator验证最大单次32768字节、峰值40665字节（完整nested allocator终态见后续整合记录）。公开 JSON writer 门禁不由本步骤放开。
