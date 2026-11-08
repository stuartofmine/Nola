> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 逐 Part 来源记录：有界导出与归档恢复契约方案

2026-09-08。只读审计后的实现方案，本文件不表示归档/导出功能已经接通。

后续实施更新：来源 DTO 已搬到 Domain，Store 保留 reexport；StorePort 已有
`StoredPartDerivation` 与 `LocalBranch / Replica / Archive` provenance。逐 Part
复制已实现，归档与独立导出查询仍待接入。下面的接口命名为原方案，实施时沿用已落地类型，
不要再次定义 local_verified/archive_claim 平行枚举。

## 现状与真实调用链

- `core/nola-store/src/part_derivation.rs` 定义来源 DTO，并提供 `NolaStore::part_derivation(owner)`；该读取会重新计算 target 原始字段指纹。当前 DTO 仍在 Store，不能让 Protocol/客户端直接依赖它。
- `core/nola-domain/src/transcript_export.rs` 的 `ExportPart` 没有来源记录；`core/nola-store/src/transcript_export.rs` 每页最多一条 Message、512 个 Part、256 KiB 编码元数据。正文另外通过 revision-pinned range 读取。
- `NolaMac/Services/MacDataManagementService.swift::MacCanonicalBackupStream.appendPagedConversations` 是实际备份写入点：逐页元数据、逐 Part 原始 text/JSON range，最后复查 conversation revision，再发布临时文件。仅改旧 `appendConversations` helper 不会修复生产路径。
- 备份解码为 `MacCanonicalConversation`；`MacCanonicalConversationRepository.restoreCanonicalBackupSnapshotAsync` 使用 `MacCanonicalArchiveWriter`。`MacArchiveRestore.swift::messageInput` 目前仅复制原始 Part 字段，来源记录会丢失。
- Domain `ArchiveRestoreManifestEntry` 仅 Profile/Message，内部 `ArchiveRestoreEntry` 对应以已准备 reference 替代外部 upload。Application 先完成正文准备；Store stage 校验并累积逐条 digest。最终 Store `archive_restore.rs` 在统一事务校验 CAS/active Run/digest，替换 Message/Part、插入正文绑定、发布 sync/receipt/journal。
- `archive_restore_sync.rs` 复制每条内部 entry；新增 variant 必须覆盖其验证、传输、最终 apply，不能只让本机 restore 成功。

## 选择：单独的导出查询与单独的归档条目

不要把完整来源记录塞入 `ExportPart` 页或 `ArchiveMessage.parts`。记录上限 16 KiB，512 个 Part 理论上约 8 MiB，会突破现有导出 256 KiB 与 archive entry 128 KiB 限制；降低页面允许 Part 数又会破坏当前一 Message 一页协议。备份文件可在每个 Part 内保存可选 `derivation`，因为实际 writer 是流式输出，文件布局与 IPC 页布局不必相同。

建议契约：

1. 将 `PartFieldFingerprint`、`PartDerivationSource`、`PartDerivation` 搬到 Domain；Store 保留旧路径 reexport。另定义 `PartDerivationEvidence { record, origin }`，其中 origin 由 Core 写入；本地产生为 `local_verified`，外部归档输入强制为 `archive_claim`。origin 不是客户端可升级的权限声明。
2. `ExportPart` 添加 default-compatible `has_derivation: bool`。新增 fresh query `read_conversation_part_derivation { owner, expected_conversation_revision }`，返回 `Option<PartDerivationEvidence>`。Store 在同一个读 snapshot 检查实际 owner、visibility、revision，并重新核对 target 指纹；单条 record 编码最多 16384 bytes，外层响应另设明确小预算（例如 24576 bytes）。不在查询中加载整条 Message 正文。
3. Mac 实际 writer 在有记录的 Part 上调用此查询，在输出 content 字段后追加 `derivation` JSON。最终 conversation revision 复查仍保留。即便 immediate/root 源已删除，仍导出记录内持久 group，不尝试从源表恢复。
4. 新增 manifest variant `PartDerivation { target: CanonicalPartOwner, record: PartDerivation }`；内部 entry 同名并由 Application/Store确定 `origin=archive_claim`。每 Part 一条 entry，跟随所属 Message entry，计入原有 sequence、manifest_digest、expected_entry_count。外部不接受 `local_verified` 字段；旧归档没有条目继续允许。
5. 增加 staging 索引 `(restore_id,target_part_id)` 唯一，确保目标已在本次归档的 staged Message 中且 owner 完全相符。同 sequence 精确重放沿用现有规则；不同 sequence 重复同 target 即拒绝。stage 只保存隔离条目，不写规范 derivation 表。
6. final 在插入目标 Message/Part/reference 后逐条核对真实 target 的 owner/run/kind/state/原始两字段 fingerprint，再插入不可变来源记录。全部处于现有 replacement+outbox+receipt+journal 事务；任一记录不匹配整体回滚。不得要求 source/root 仍存在。

## 信任边界：字节匹配不能证明历史 owner

外部可任意制造相同字节 SHA 及伪造 root owner；无签名的 JSON 无法证明这些历史身份由 Core 生成。因此不能将导入记录无标记地写成当前本地 branch 的已验证事实。

`archive_claim` 完整保留原声明以便再导出，但仅证明当前 target 与声明字节相符。它不允许绕过 JSON 内嵌 owner、live owner、permission 或语义适配器验证。后续 branch 对该记录不得重新初始化已知 root：有效可信 root 保持未知；如果产品要显示原归档根身份，必须明确读 claim，而非把它当授权依据。`origin` 要保存在不可变规范记录中；再次导出/导入不能把 claim 升级。即便备份自称 `local_verified`，外部入口仍强制降为 claim。

若未来要求跨安装保留可验证历史身份，需要另立签名和可信设备密钥契约；本方案不假设文件来源可信。原文导出/恢复不应因缺此能力而被禁用。

## 下一步按文件落地

- Domain 新 `part_derivation.rs` 共享 DTO；`transcript_export.rs` 新查询返回 DTO/has 标志；`archive_restore.rs` 新 manifest/internal variant。
- StorePort 新 provenance read 契约，Store `part_derivation.rs` 拆出可借用 Connection 的 bounded validate/read/import helper；schema 加 origin 与 staging唯一索引、immutable 校验。
- Store `transcript_export.rs` 仅返回存在性，不聚合记录；Application/Protocol/AppCommand/Runtime/Service fresh分类接独立查询。
- Application `archive_restore.rs` 校验新条目上限并固定信任级别；Store `archive_restore.rs` 扩展 stage、final、计数/digest索引与事务写入。
- Store `archive_restore_sync.rs` 与 replica 接收路径支持新条目；同步不得把 archive claim 升级为 local_verified。
- Mac `MacApplicationPort.swift` 增加 DTO/query；`MacDataManagementService.swift` 修改实际分页 writer；canonical Part 备份 DTO 增可选 derivation；`MacArchiveRestore.swift` 在每条 Message 后 stage对应来源条目。不要把 derivation混入普通 MessagePartInput 写API。
- CLI `cli/src/store/session.rs` 的实际分页导出重建也需读取记录。若CLI通用内存模型不保留该可选字段，应增加导出专用DTO，避免悄悄丢失。

## 必须验收

- 10 MB 单 Part、512 Part Message（各16 KiB记录）：Service 每次返回均有界，备份保留原text/JSON及来源；不能以preview替代。
- local branch → 导出 → 删除全部源 → 新库restore → 再导出；记录声明保留，origin明确为archive_claim。
- 伪造 root owner、同长不同JSON、target重标记、kind/run变更、重复target、过大record：允许保留不可信历史声明的边界明确，target不匹配必须final失败；任何root声明均不能授权结构化语义。
- final来源插入/复制outbox/receipt/journal逐点失败注入：旧目录、消息、原文绑定与来源记录全部不变；相同restore identity重试稳定。
- 真实Mac backup→restore与Service冷重启；sync接收端缺条目/乱序/改digest不能发布半个归档；旧归档缺记录后继续branch保持未知root。

本轮补验终态：Store root94830 exit0，232/232（29个suite，无失败/忽略）；
Mac root15107 exit0，MacCanonicalExportTests 7/7、0失败/0跳过，真实本机arm64。
日志`/tmp/nola-derivation-mac-native.log`，artifact目录
`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-mac-history-acceptance-d3r02hix`。
后续record与archive wrapper预算对齐及新增故障注入的最后增量测试另记，不重复启动已终态任务。

来源归档最终跨进程/跨端证据：root Worker9443已exit0，
`/tmp/nola-derivation-archive-worker-final.log`包含15MB/500chunks、branch505facts、
archive508facts/9轮，`archive_atomic=true archive_derivation=true archive_provenance=true`。
接收端每个staging轮次都断言来源不可见，final才与目标原文同现；正式独立export query
验证Archive origin和完整claim，原文range仍逐字节相同且无echo。

Application新增`archive_derivation_receipt_e2e`1/1通过：proof INSERT、archive final outbox、
projection journal、receipt committed UPDATE四处故障均回滚旧目录、原文、原proof和事件/既有收据；
冷重启后成功final仅一个Archive proof，原envelope replay返回相同protocol receipt。
用户可读格式说明见`../../软件架构设计/模块/04-存储与索引/CANONICAL_BACKUP_FORMAT.md`。

此阶段接通了来源归档/导出，不等于完整typed semantic projection或JSON可见引用发布。
后续继续验证其余嵌套字段，完成window/context共同消费与本地可信branch身份重绑定；
全Application解耦、Android业务存储迁移与五端/真实模型/UI门禁仍在完整V2范围内。
