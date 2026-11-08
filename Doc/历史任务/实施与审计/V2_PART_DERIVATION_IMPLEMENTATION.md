> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Branch 逐 Part 来源记录：首条本地链路

2026-09-08，当前未发布 schema 13；仅新建隔离测试库验证，没有用户库迁移。

`conversation_branch::create_conversation_branch` 在每条目标 Message append 后、branch 最终 lineage 提交前，调用 `part_derivation::record_branch_message`。该内部接口只接受本次 operation 和真实 source/target Message ID；它从当前规范表读取 Part 列表与 owner，不能从客户端传入“原 owner”或正文摘要。fingerprint 通过已发布字段 resolver 逐段读取，每次最多 32768 UTF-8 bytes，SHA-256 绑定原始 text/JSON 字节，不解析重编码 JSON、不使用备份 `content_digest`。

`message_part_derivations` 每 target Part 一条，目标 FK `ON DELETE CASCADE`，source/root 无 FK。记录 version、operation ID，以及 target / immediate source / 可选 root source 三个 group。每个 group 有 `CanonicalPartOwner`、规范 Message run_id、Part kind/state、两个字段的可选 `PartFieldFingerprint {field,total_utf8_bytes,sha256}`。None 表示原字段不存在；零长度现存字段有空字节的 SHA。fingerprint 描述跨 inline/reference 表示的字节等价，未借用表示实际 storage 的 `SemanticSourceIdentity::InlineSha256`。

本地 branch-of-branch 先检查 source 的旧 derivation.target 与当前 source owner/run/kind/state/字段完全相同，再继承旧 root group。新的 target、immediate 和已知 root 必须保留原字段与 kind/state；target run_id 依现有 branch 行为是 None，root 原 run_id 独立保留。source 删除后，后代仍通过持久 group 核对自身字节和元数据；不需要重读被删 source。表限制记录最多 16 KiB，并有 UPDATE 拒绝触发器，来源记录不能原位改写。

旧归档/远端 branch 若仅有会话 lineage、没有逐 Part proof，root_source 记为 None，仍允许 raw branch；后续 branch 继续继承 None，不能把缺失证明的当前来源重新认作已知 root。只有普通非派生来源才初始化 Some(immediate)。Some 本身也不代表 JSON 内嵌身份已经匹配。

公开 `NolaStore::part_derivation(owner)` 仅允许读取：先检查真实 owner/字段可见性并重新计算当前指纹，再返回已匹配的记录。错误 conversation/message/part、改动后的 JSON、kind、run 或记录不匹配都会失败。没有对应外部写 RPC。

**这不是结构化语义授权。** 尚未接 archive/replica 的逐 Part 来源事实，也没有让窗口/context/语义 adapter 使用此记录跳过原 JSON 内嵌 owner 校验。root group 记录规范来源身份，不宣称原 JSON 的嵌入身份已经通过识别、映射或权限验证。

验证：`cargo test -p nola_store --lib part_derivation --quiet` 5/5；`cargo test -p nola_store --lib conversation_branch --quiet` 5/5。覆盖两代分支、根来源先删除再二次分支、直接父删除后重开、目标删除级联、最终 lineage 故障完整回滚与原 intent 重试、同长不同 JSON 字节以及 kind/run 修改后拒绝继承、外部查询 owner 不能重标记、来源表 UPDATE 拒绝，以及缺失旧 branch proof 后连续两代保留未知 root。

## 逐 Part 复制与接收方来源分类

后续同日已接通跨数据库复制，Domain 承载纯 `PartDerivation` claim；StorePort 的 `StoredPartDerivation {derivation,provenance}` 将 `LocalBranch / Replica / Archive` 留在接收端本地，wire 不携带可自报的 provenance。新远端记录固定为 Replica；从 Replica/Archive 父记录进行本地 branch 继续继承其来源类别，不升级为 LocalBranch。旧 `part_derivation` 仅返回 claim，不能当作语义授权；需区分类别的消费者使用 `part_derivation_record`。

每个 Part 一个 `part_derivation` fact，payload 最多 16 KiB，revision 固定 1，entity ID 等于目标 Part ID。facts 紧随本条 Message append，可以同页顺序接收；接收方验证完整当前 target owner/run/kind/state、两字段原始 SHA，以及 operation+直接父 Message/Part 推导出的目标稳定 ID。不要求源 Part 仍然存在，远端 root 仍只是声明，未开放任何 JSON owner 豁免。

最后的 `conversation_branch_lineage` 必填 `derivation_count`；来源端等全部逐 Part proof exact ACK 后才发送。检查逐行扫描，有界解码，不在 Rust 收集整条 branch 的 Part ID 集合。接收 final 核对目标前缀所有 Parts 的 proof 覆盖、数量、operation 和直接 source conversation。旧 wire 缺 count 明确拒绝，不默认零。ACK prune 保留有界 proof，不永久保留大 append payload。

`branch_fact_receipts` 保存 operation/entity/typed canonical SHA，无 target FK。严格解码后规范编码求摘要，因此外层 JSON 键顺序和空白变化仍是同一事实；原始正文字节的指纹不重编码。本地 enqueue 和远端成功接收均同事务记 receipt。目标后来编辑或删除后，已接受 exact replay 只推进同步、不重建 proof；changed same-operation 拒绝。若第一次收到旧 proof/lineage 时目标已存在 durable conversation tombstone，则在严格形状/派生 ID 检查后仅记 receipt；普通缺 target 且无 tombstone 仍拒绝，不把任意缺失当作删除。

本阶段验证：`cargo test -p nola_store --lib part_derivation --quiet` 10/10（5 本地+5 跨库）；`cargo test -p nola_store --lib conversation_branch --quiet` 5/5。新增跨库测试实际创建独立 SQLite，分页重开两端，覆盖源删除后来源保留、远端 origin 多代不升级、proof ACK 重开、缺 proof 的 lineage 拒绝、坏目标整页回滚、exact replay 在目标编辑/删除后保持幂等（含本地 echo 和外层 JSON 重排）、首次旧事实晚于 tombstone 不复活且 cursor 前进，以及 changed replay 拒绝。

这是 Store 同步主路径实证，尚未运行此新增 proof 协议的 Worker HTTP；尚未接 archive proof 导入导出，也未开放任何 JSON 语义消费者。

增强版真实 Worker 回归 root85255 已 exit0：`/tmp/nola-worker-part-derivation-final.log`
明确包含 `branch_derivation=true replica_provenance=true`，接收端来源 owner/root/原字段指纹
与强制 Replica 标记断言通过；15MB、branch505facts 与 archive507facts 仍通过。
本阶段所有 root 测试已终态，不再等待或重复启动。下一步按归档/导出方案继续。

## 独立归档来源条目已接通

外部 `ArchiveRestoreManifestEntry` 与内部 `ArchiveRestoreEntry` 新增 `PartDerivation {record}`，不接受额外 target 或 provenance。target 来自 record.target.owner。记录自身最多 16 KiB，外层条目沿用 128 KiB manifest 上限，不能再次把 wrapper 计入记录上限；final SQL 同样按 128 KiB 有界读取外层 entry。

`archive_restore_derivations` 对 `(restore_id,target_part_id)` 唯一，并绑定已 staging 的 Part 索引。来源条目必须晚于自己的 Message，核对完整 conversation/message/part owner；同 sequence exact replay 可用，不同 sequence 重复 target 拒绝。stage 只写隔离索引和原 entry，不写规范 proof；final 会重新验证索引与 entry 的对应关系。

最终 archive writer 先在原事务内重建真实 Message/Part/reference，再逐条调用 `part_derivation::import_archive_claim`，流式核对两个原始字段的 SHA/长度以及当前 owner/run/kind/state，强制写 Archive provenance。source/root 不需存活，任意合法 root 声明可保留，但不能授权 JSON 语义或升级本地来源。远端 archive entry 使用相同 stage/final 路径，来源仍为 Archive。任一 proof 或后续 outbox 失败均回滚整个替换和已写 proof。

最后专项：`cargo test -p nola_store --lib archive --quiet` **13/13**，`cargo check -p nola_application --quiet` 通过（session 61467）。新增 5 项包含：10 MB 来源全部删除后新库恢复与逐 range 原字节比较；错误 target/先 proof 后 Message/重复 target/自报 provenance 拒绝；最终字节冲突及 proof INSERT、最终 outbox INSERT 故障回滚与重试；跨 SQLite 分页重启，stage 零规范 proof、final 一次原子写 Archive、无 echo；固定 DTO 的最大宽 escaped 字段编码预算。最后一项仅测 record/wrapper 分离预算，不声称该极端元数据组合能成为真实规范 target，也没有放宽 ID 或 owner 准入。

集成方另报告：真实 Service 导出/新库 restore/重开 replay **4/4**，App proof/outbox/journal/receipt 四处故障注入通过；真实 Worker 15 MB archive **508 facts / 9 rounds**，新增 `archive_derivation=true archive_provenance=true archive_atomic=true` 全部通过（root session 9443）。这些由 root 运行，区别于本代理直接运行的 Store 专项。外部 JSON 正文引用及结构化语义仍受既有门禁约束。
