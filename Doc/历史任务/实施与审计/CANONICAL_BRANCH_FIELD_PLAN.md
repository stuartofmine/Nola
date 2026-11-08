> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Canonical branch 双字段迁移计划

状态：共享 Rust、Mac 与 Service 定向验收通过；不代表 P5 整阶段完成。2026-09-28。
隶属 [Canonical 流式正文合同](../../软件架构设计/模块/13-客户端投影/CANONICAL_STREAMING_TEXT_CONTRACT.md) 和整体改造 P5。
本文件不授权提前开放 JSON publication，也不表示 Chunked writer 已接入。

## 迁移前边界

`core/nola-store/src/conversation_branch.rs::copy_parts` 当前调用
`published_part_body::reference`。该 text-only 接口遇到 `content_json` 引用会明确
拒绝，而不是将空 inline 槽误当作空原文。这个 fence 是必要保护，不能仅删除。

当前 branch 复制小型 Inline text/JSON；大 Inline Text 和 Sealed Text 复制成
目标所有者的不可变内容。`copy_parts` 返回的引用合同及下游 publication、replica
仍有 text-only 约束。即使底层 `CanonicalBodyReference` 已能表示字段，也不证明
所有正文消费者能正确 hydrate JSON。

## 前置依赖与不可拆开的发布边界

### 一、统一 source 字段读取，暂不开放新 publication

- `conversation_branch.rs::copy_parts` 分别通过
  `CanonicalFieldReader::open` 读取 `ContentText` 和 `ContentJson`，不得使用仅服务
  展示正文的 `open_text_projection` 代替原始字段读取。
- 由 reader 区分 Absent、Inline、Sealed，不以物理列长度推断逻辑正文是否存在。
- 小 Inline 原样复制；超过既有内联预算的字段通过 bounded range 复制。禁止为
  巨型 JSON 提高内联预算，或先全文 hydrate 再切块。
- 目标 content ID 的确定性生成必须包含 field。当前单一 `body + source part`
  身份不足以同时容纳 text 和 JSON。
- 保留 completed message/part、可见前缀、每 message manifest 预算及现有 owner
  检查。不得为了兼容新 reader 放宽 Sealed 的完成态约束。

这一阶段可以收敛读取实现，但 sealed JSON branch 仍应保持明确拒绝，直到下面
所有消费者和发布通路同时准备好。

### 二、JSON hydration 与 publication/replica 一起验收后开放

需要同时核查或修改（replica的chunk/seal层原本已支持typed JSON，不能误称它仍
全部text-only；本次主要接通append manifest/绑定和archive入口）：

| 边界 | 具体入口 | 必须满足的合同 |
| --- | --- | --- |
| 原始字段解析 | `published_part_body::{resolve_field,field_range}`、`CanonicalFieldReader` | 同快照、完整 owner、可见性；空 inline 不代表缺正文 |
| 正文消费者 | `conversation_read_repository`、`historical_part_projection`、`bounded_message` | 识别 deferred JSON，不能默认为空 payload |
| 语义消费者 | `part_semantic_source`、`part_nested_source`、context 分类 | 原始 JSON 的语义读取不能使用展示裁剪结果 |
| 目标写入 | `conversation_append_store::append_in_transaction` | 两字段分别验证、绑定且无冲突 inline 真源 |
| 内容校验 | `canonical_body::validate_json_object` | JSON 完整结构校验，不只检查 UTF-8/chunk 完整性 |
| 同步 | `canonical_text_replica::{enqueue,apply,require_complete}` | typed field 身份、依赖完整后发布；缺依赖不推进 cursor |
| 原始导出 | `transcript_export` 及 archive 导入/导出消费者 | 两字段均可重建，不只导出展示文本 |

`canonical_body::require_text_reference` 和 `published_part_body::reference` 的旧
text-only fence 只能在对应消费者已改为 field-aware 后逐入口替换。不能全局删除
fence，不能用 unchecked reference 转换让 JSON 冒充 Text。

一个 Part 可有两份独立引用；每份必须精确绑定目标 conversation/message/part/
field。任何一个字段缺 chunk、格式损坏或身份冲突，目标 Part 均不可半发布。

### 三、保持 branch、derivation 与 archive 的原子关系

仍由 `conversation_branch` 外层写事务拥有整个操作：

1. 验证 source 可见且完成的前缀与目标身份。
2. 准备目标 owner 的字段内容。
3. 调用 `conversation_append_store::append_in_transaction`。
4. `part_derivation::record_branch_message` 记录两字段来源事实。
5. `part_derivation_sync::enqueue_message` 和 branch lineage 与内容同事务提交。

不得让内容复制自行开启独立提交。后续 lineage 或第二字段失败必须回滚此前内容、
messages、引用、sync、derivation。重放沿用既有 operation receipt，不新增可见历史。

`part_derivation` 已有按原始字段读取的基础，但验收必须比较双字段 fingerprint，
不能仅验证 content_text。archive 必须保留目标自有内容和 provenance：删除 source
后目标与其 archive 恢复仍可读取，不能依赖 source message/reference 永久存在。

## 安全与所有权

- source message 由 source conversation 可见前缀查询选中，reader 再检查完整归属。
- target 使用独立 owner 和 field 身份，不把 source reference 原样挂在目标 Part。
- 全局 chunk 去重只是物理优化，digest 不是跨 owner 的读取或复制权限。
- 私有 canonical 原始 JSON 不得从公开 assistant journal 的 ToolInput/Output 引用
  推导；两者不是同一授权合同。
- 这一计划仅针对现有 Inline/Sealed 双字段；未来 Chunked 接入须再次覆盖同一矩阵。

## 验证矩阵

| 场景 | 必须验证 |
| --- | --- |
| 两字段 Inline/Sealed/Absent 合法组合 | 原始字节和语义一致，null 与空字符串不混淆 |
| 双 Sealed | 两个独立 content ID，完整 target owner/field 绑定 |
| Unicode、跨 chunk、大 JSON | 完整内容可重建；读取有界，不全文 hydrate 后切块 |
| hidden、错 owner、未完成 message/part、活跃 source Run | 拒绝且目标不可见，无复制副作用 |
| 第二字段损坏/缺尾 chunk/无效 JSON | 整事务回滚，无孤立引用或 sync revision 推进 |
| 最后 lineage 失败与相同 operation 重试 | 前次零半提交，重试正确且幂等 |
| source → replica 乱序与缺依赖 | 不发布空 Part、不提前推进 cursor，补齐后完整发布 |
| replica 重放及冷重启 | 两字段与来源一致，无重复历史 |
| 删除 source 后读取 target | 目标正文及 derivation 独立存在 |
| archive 导出、导入、冷重启 | 两字段和 provenance 均保留，不依赖 source 存活 |

保留并扩展现有 `conversation_branch.rs` 测试：

- `final_lineage_failure_rolls_back_copied_content_messages_and_sync_then_identical_retry_succeeds`
- `unfinished_part_and_active_source_run_refuse_branch_without_target_visibility`
- `bounded_structured_payload_survives_branch_without_losing_its_json`
- `inline_and_referenced_branch_generations_preserve_part_metadata`

同时复用 `canonical_text_replica` 的缺 chunk、changed replay、foreign reference
回滚测试，以及 `archive_derivation_tests`、Application `branch_transaction_e2e`。
新增 helper 测试不能替代真实 branch → replica → archive → 冷重启链路。

## 交付条件

只有生产 reader、target publication、replica 和 derivation/archive 均通过上述
矩阵后，才能开放 sealed JSON branch。仅增加类型、移除 fence 或接一个 writer
不构成整阶段完成；以下记录共享 Rust 的实际验收，不能代替客户端与 Service 验收。

## 当前接线范围

- branch独立解析两字段，大Inline或Sealed经有界range复制到新owner；目标content ID
  包含字段，两个字段不共享对象身份。来源无需继续存在。
- Kernel字段绑定准入由append与replica manifest复用，物理chunks/seal仍由已有
  canonical_text_replica验证。引用绑定不再写死content_text；缺任一依赖不发布。
- archive Store准备/提交、Application字段上传manifest、上传retirement及Mac备份
  写入器一起迁移。ArchivePartUpload要求显式field；同Part两份上传身份独立。
- text range/search只读取自己的原始ContentText字段，不因独立JSON引用而拒绝。
  专门的普通transcript文本上传接口仍维持文本合同，不让JSON冒充文本。
- 新增branch→replica→冷重启、删除来源后branch→archive→derivation恢复、第二字段
  损坏/缺chunk/错误fingerprint原子拒绝测试；Mac备份增加超128KiB原始JSON往返。

## 已核实的共享验收

- 最终 Store lib **229/229** 通过，0 failed/ignored，日志
  `/tmp/nola-dual-field-store-20260928-final.log`。该结果包含最后新增的
  `sealed_geometry_corruption_cannot_be_republished_as_a_valid_branch`：Text/JSON
  分别覆盖声明 chunk_count 错误及声明末尾之外多余 chunk；即使原始范围仍能读出，
  再 branch 也必须拒绝，目标不可见，原子操作相关计数保持不变。
- Kernel **117/117** 通过，日志 `/tmp/nola-dual-field-kernel-20260928.log`。
- Application archive_restore_staging、archive_restore_receipt、archive_restore_runtime、
  archive_derivation_receipt、branch_transaction 五组共 **12/12** 通过，日志
  `/tmp/nola-dual-field-application-20260928.log`。这是此前定向结果，不称作最后
  geometry 测试新增之后的 Application 重跑。

- Mac 定向 **10/10** 通过（session1607 exit0）：MacCanonicalExportTests 7 项与
  MacTranscriptBodyUploaderTests 3 项。日志 `/tmp/nola-dual-field-mac-20260928.log`
  为 `TEST SUCCEEDED`；`/tmp/nola-dual-field-mac-20260928.xcresult` summary 核实
  10 passed、0 failed、0 skipped。覆盖该批真实备份往返与 uploader，不称作全部 Mac 测试。

- Service `archive_restore_process_e2e` **2/2** 通过（session67806 exit0），日志
  `/tmp/nola-dual-field-service-20260928.log`。这是实际进程边界定向验收，不以
  Store in-process 或 Mac 测试替代。

- Store 五组 integration **17/17** 通过（session32931 exit0），分别为
  append_messages_e2e、structured_field_export_e2e、structured_json_validation_bounded、
  sync_replica_e2e、turn_append_sync_e2e，计数7+3+1+1+5。日志
  `/tmp/nola-dual-field-integration-20260928.log`。该组不同于此前 hydration 的33项，
  不混用计数。使用 `CARGO_INCREMENTAL=0 cargo test -p nola_store --test append_messages_e2e --test turn_append_sync_e2e --test sync_replica_e2e --test structured_field_export_e2e --test structured_json_validation_bounded --quiet`。
Chunked 可追加 writer、增量 streaming 与全端最终验收仍未完成。
