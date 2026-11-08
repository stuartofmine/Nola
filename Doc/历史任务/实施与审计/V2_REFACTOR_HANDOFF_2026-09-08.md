> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# V2 重构交接：2026-09-08

最新阅读入口：先看 [当前架构](V2_CURRENT_ARCHITECTURE_2026-09-08.md) 顶部的 2026-09-09
模型完成与 Turn 决策记录，以及本文末尾的最新交接。下方早期未完成项已被后续阶段部分覆盖。

更新时间：2026-09-08 13:49，Asia/Shanghai。

## 总目标与状态

目标始终是根据 `Doc/历史任务/实施与审计/ARCHITECTURE_V2.md` 完成代码和架构的重构与优化。
**整体未完成，不能将下述局部测试通过视为 V2 已完成。**
此文记录本轮对话已执行的工作及证据，不是对当前仓库每个模块的全量验收。
新对话以实际工作区为准；不要覆盖已有修改，不要重新创建目标来缩小范围。

## 最近完成的持久化边界调整

### 会话创建与配置

- 新增 `core/nola-store-port/src/conversation_profile.rs`，持有创建会话、更新设置、更新联合配置的输入/结果以及 `ConversationProfileRepository`。
- `core/nola-store/src/contracts.rs` 重导出同一契约类型，没有新增兼容转换或改变 wire 格式。
- SQLite 实现从 Store 根模块移到 `core/nola-store/src/conversation_profile.rs`。
- `core/nola-application/src/conversation.rs` 的创建、重命名、设置更新和联合配置更新改为泛型仓储端口；Application 保留 JSON 对象、时间和标题校验。
- `operation_result.rs` 的创建结果转换不再引用具体 Store 类型。
- 新增非 SQLite 仓储测试，扩充真实 SQLite 发布失败回滚测试。

最后执行：

```sh
cargo test -p nola_application --test conversation_profile_port_tests --test conversation_profile_transaction_e2e --quiet
cargo test -p nola_store --test conversation_sync_e2e --test conversation_deletion_e2e --test remote_sync_apply_e2e --quiet
cargo check -p nola_service --quiet
```

结果：18 项测试通过，Service 编译检查通过。覆盖标题、设置、revision 和同步发布的联合事务；不是 UI 或真实 LLM 测试。

### 会话删除

- 新增 `ConversationDeletionRepository` 和独立 Store 删除模块。
- 删除事务内检查是否存在非终态 Run，拒绝运行、排队、等待权限、挂起及未知状态；终态为 completed/failed/cancelled。
- 不再通过加载助手消息判断活动 Run，维护型 Run 没有助手消息也会阻止删除。
- 检查、删除及 tombstone 发布处于同一写事务；revision 加一使用溢出检查。
- 旧同步测试原先直接删除未结束的 Run，现通过正式 `commit_transition` 取消，关闭 Part、确认 Effect 后再删除。
- 共用测试辅助文件：`core/nola-store/tests/support/conversation_deletion_fixture.rs`。它验证 Store 事务，不是 Core 取消命令的端到端测试。

注意同步语义：删除 tombstone 会替换同会话尚未发送的旧发布，因此不能断言队列总数必然加一。测试现验证 tombstone 的 revision、deleted、payload、operation identity、同步版本表，及回滚时旧发布完整恢复；已确认的发布保留。

曾执行删除、冷启动同步、跨设备同步和 ledger 四组测试，合计 29/29 通过。

### 历史追加和窗口读取（本对话此前完成）

- `TranscriptWriteRepository` 及追加 DTO 位于 Store Port；Application 历史导入不直接依赖 SQLite。
- 追加实现位于 `conversation_append_store.rs`，事务内重新检查活动 Run、已知 Run 归属和消息数量 CAS。
- `append_messages_e2e` 共 7 项通过，包括第二写入者在预检后提交 Run、外部 Run 身份冲突、发布失败后的消息/Part/revision/outbox 回滚及重试。
- `DeferredTextPart` 移到 Store Port，构造与反序列化校验身份和 UTF-8 长度。
- `bounded_message::load_parts` 一次读取同时返回 Part 预览及 deferred 元数据；已删除重复逐 Part 查询长度的 `deferred_text_parts` 路径。

## Mac 恢复：修复了破坏性顺序，但主链路未完成

`NolaMac/Services/MacCanonicalConversationRepository.swift` 的普通 `restoreBackupSnapshot` 原先先 `clear()`，再检查消息是否超出导入帧限制。超大消息被拒绝时可能已经删掉旧历史。

现在先逐会话校验 metadata 编码与所有消息批次，再清理；不同时保留所有会话的编码批次。canonical 备份入口此前已有消息帧预检。

扩充 `MacCompactionRecallEndToEndTests/testOversizedSingleMessageDoesNotCreateAnEmptyConversation`：先存原有会话，再恢复“有效会话 + 超大消息会话”的备份，确认失败后原有消息 ID 和全文保留，两个备份会话均未部分创建。

已执行 arm64 Mac `build-for-testing`，结果成功；随后用隔离状态执行原生测试，1/1 通过、0 跳过。

```sh
python3 scripts/run_mac_history_acceptance.py --timeout 180 \
  --only-testing NolaMacTests/MacCompactionRecallEndToEndTests/testOversizedSingleMessageDoesNotCreateAnEmptyConversation
```

- DerivedData：`.build/real-mac-xctest-derived-data`
- 构建日志：`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-restore-build-n3pggs_q.log`
- 测试结果目录：`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-mac-history-acceptance-mpnoq4t8`
- 该构建早于最后一次 Rust profile 模块拆分；最后一次拆分后仅执行了上述 Rust 测试与 Service check，没有再构建整个 Mac。

**这不是原子恢复。** 预检只能阻止确定性的编码/帧失败；清理后发生 IPC、磁盘或后续事务失败仍可能造成部分恢复。多会话原子发布需要 Application 级设计。

## 首要未完成项：千万字符原文发布

详细设计与验收门槛在 `Doc/历史任务/实施与审计/V2_CHUNKED_TRANSCRIPT_PUBLICATION.md`，已同步更新其中过时的 deferred 查询描述及 Mac 预检状态。

已具备：

- owner-scoped 分块上传、偏移续传、封存、原文长度校验；`LazyContentUploadStore` 延迟创建 staging。
- `CanonicalTextReference`、独立 `CanonicalTextStore`、有界复制、范围读取及字面搜索。
- `UploadedTranscriptManifest` 校验身份、归属、大小、重复绑定、完整消息和单一正文来源。
- `prepare_uploaded_transcript` 及 preparation port，准备不可变正文并核对返回引用的会话/消息/Part/长度。

缺失：

1. 将已准备引用放入规范消息写事务，同时提交 revision、发布记录和 command receipt。
2. 全部规范读取路径统一解析正文引用：窗口、deferred、range、search、context、export、branch、同步恢复。
3. 同步端先传有界正文块并确认，再发布引用，不能让远端出现悬空 Part。
4. Mac 上传、封存、发布与断点续传；现有 `result.created` 为 false 就返回不是合法的恢复完成证明。
5. 多会话恢复的原子可见性，以及明确的失败恢复流程。
6. 原生单消息千万字符恢复、切换、UI 截图与性能验收。

不要提前开放返回空内联正文的引用消息；不要缩小原生失败用例、把一个逻辑消息拆成许多消息、提高 IPC 帧预算或绕过 Mac 入口冒充修复。

此前原生套件 17 项中 16 通过、1 失败：`testNativeRecallOfTenMillionCharactersReturnsBoundedTailEvidence` 在恢复阶段遭遇帧限制，尚未进入 recall。这个失败未被解决。

## 真实 Service 进程回归证据

最近一次执行：

```sh
cargo test -p nola_service --test transcript_recall_process_e2e --quiet -- --nocapture
```

13/13 通过，约 4.95 秒；该轮在删除修改之后、最后 profile 拆分之前。

- 10M 字符 recall：27.53 ms，返回 387 bytes。
- 10M 上传：约 1.40 秒，采样峰值 RSS 21,118,976 bytes；这是 staging，不是消息发布。
- 四个各 10M 字符历史交替查询 68 次：P95 24.05 ms，采样峰值 RSS 37,109,760 bytes，预热后增长 196,608 bytes。
- 10M 历史上的新消息、设置更新和追加测试也通过；不能据此推断原生单条 10M 消息恢复成功。

这些是隔离数据库、真实 Service 子进程测试；没有调用真实 LLM，没有覆盖 Mac UI 截图。

## 此前 Web 改动摘要

- 上下文没有真实使用量时保留未知状态，不以全零记录覆盖 Core 准备结果；已知零用量与未知窗口分开。
- Web 上下文工具 schema 估算使用 Host 的初始暴露集合，不把全部 deferred 工具算入初始请求；不改变实际工具可用性或权限。
- 建议使用稳定 code + 数值参数，由 UI 本地化；移除无依据的节省 token 固定比例。
- 文档列表返回摘要而非全文；metadata-only 保存保留原文，模型内容读取走独立权限路径。
- Playwright 真实浏览器测试 `scripts/test_web_document_browser_e2e.py` 曾在中英文下通过，验证 2M 字符文档、摘要、小保存请求、保留原文、上下文展示及截图。
- 初始上下文 fixture 结果为 12,731/32,768 tokens，而非错误地统计整个工具目录。后续动态工具暴露及全链路使用量统一仍不能视为已全覆盖。
- 内置 Browser 工具当时不可用，回退使用仓库 Playwright helper + 已安装 Chrome，没有下载浏览器或调用 LLM。

## 构建环境问题

用户提供的 npm 报错来自 Node 14.17.5/npm 6.14.14 校验 lockfile，不是磁盘满。检查时磁盘可用约 190 GiB。
现有 `scripts/bundle_playwright_runtime.sh` 使用选定 Node 配套的 npm-cli.js，要求 Node >=20、npm >=9，不再直接执行 PATH 中旧 npm。上面的 Mac 测试构建已实际成功。

## 新对话的工作约束与启动建议

- 用户授权普通编译和测试；使用隔离状态、数据库和 fixtures，不删除用户数据，不改生产凭据，不轮询锁屏 iOS 真机。
- Mac 使用显式 `platform=macOS,arch=arm64`；历史测试 runner 会检测 remote-device discovery 并停止本轮测试进程组。
- 不凭关键词路由工具、Skill、记忆或权限；模型选择具体工具，Core 检查具体调用及资源。
- 不自动迁移或重建用户数据库，不用兼容层掩盖未完成的架构。
- 没有执行 commit。本次交接仅写文档，未重跑测试。
- 本文所列编译/测试进程都已收到退出结果，没有需要继续轮询的已知句柄；不要仅凭 stale 锁文件推断仍有任务运行。
- 先读 V2 主文档和分块发布文档，再选择真实主链路推进；不要重复做已经通过的细小重构来回避原生大正文恢复和架构全量验收。
- SDK 的 `nola-service-client` 当前已只依赖协议及通用库，但 `nola-application` Cargo 仍依赖具体 `nola_store` 与 `nola_core`，整个依赖切分远未完成。

建议新对话开场：

> 继续按 Doc/历史任务/实施与审计/ARCHITECTURE_V2.md 完成整体重构。先读 Doc/历史任务/实施与审计/V2_REFACTOR_HANDOFF_2026-09-08.md 和 Doc/历史任务/实施与审计/V2_CHUNKED_TRANSCRIPT_PUBLICATION.md，以当前工作区为准。重点打通规范大正文发布、读取、同步和 Mac 恢复，不要把上传成功当作恢复成功。保留已有修改，编译测试已授权，用户数据库不可删除。

## 后续架构核对补充：2026-09-08

按用户“先梳理当前架构”的建议新增
[`V2_CURRENT_ARCHITECTURE_2026-09-08.md`](V2_CURRENT_ARCHITECTURE_2026-09-08.md)，
包含当前运行关系、与目标的差距和主链路实施顺序，并修正主架构文档中过时的 CLI 直写结论。

- CLI `store::NolaStore` 实际只保存路径，持久化业务走 Service；不要凭类型名称统计 SQLite 写入者。
- CLI `nola_store`、`rusqlite` 已限定 dev-dependencies，唯一生产 Store 类型引用改用已有 Store Port 契约。
  新增直接依赖门禁与 Linux/macOS CI 检查；仍有 Application/Service 间接依赖及分支仓储 DTO，不能宣布 SDK 收敛完成。
- `execute_received_command` 已有主库外层事务；移动 FFI 直接 `execute` 写路径和跨库上传仍需分别处理。
- 本次 CLI/Agent Host `cargo check --bins` 通过；依赖门禁 1 项与真实 Service 所有权/投影查询 7 项通过。
- 未改大正文发布及 Mac 恢复实现，原生千万字符恢复失败仍未解决；没有新的 Mac/UI/真实 LLM 验收。
- 未提交 commit，保留已有工作区修改；首要后续工作仍是分块正文的规范读取、发布、同步和 Mac 恢复。

## 继续施工交接：2026-09-08 原文与恢复闭环

已有工作区全部保留，未commit、未重建用户数据库。协议当前12，schema13。
最新职责图/证据见 `V2_CURRENT_ARCHITECTURE_2026-09-08.md`；依赖精确审计见
`V2_DEPENDENCY_CUTOVER_AUDIT_2026-09-08.md`。不要继续引用最初“原文尚未publish/read”的状态。

已接正式链路：PublishUploadedTranscript、canonical主库正文引用、bounded window/range/search/context、
metadata-only branch、paged/range原字段export、staged multi-conversation archive commit及ACK依赖复制。
Mac restore已从clear再逐会话import切为stage后final一次原子提交；plain single10M原生召回通过。
200上传触发128上限的真实失败已用durable canonical证据退役duplicate staging解决，未增上限。
Mac restore/upload/export最终13/13，另真实UI长会话2/2；详见最新架构核对中的artifact路径。
Service发布/重启/原文实测、archive receipt失败注入、真实Worker507facts/9轮archive复制均通过。
CLI/AgentHost hidden Service自动启动recreate默认改false，Store拒绝旧schema前不改WAL header；
两binary×foreign/旧Nola schema保持原字节回归通过。

仍在推进：移动FFI统一typed query/admission及client pending receipt恢复；
仍未完成：Application具体Store装配/旧Core依赖、完整Kernel迁移、Android Room业务所有权、
结构化大content_json引用、native恢复作业跨进程续传、整份备份流式解码及非history领域原子性、
五端一致性、真实模型多轮与完整UI状态/长任务验收。不得把以上专项green当作整体V2完成。

### 后续端口切分与结构化正文施工

Application的replica_sync转换与5个命令用例已只依赖ReplicaSyncRepository，
records/input/result移到StorePort，Store adapter保持原SQLite连接/UoW。
非SQLite错误页/失败确认测试2/2、Store archive sync5/5、CLI bins check通过。
JSON流式验证seam已完成（详见V2_STRUCTURED_BODY_CUTOVER.md §7），尚未启用生产JSON引用。
下一步Domain/Store唯一field-aware body表示施工由body_readers继续，消费者/复制与外部绑定须一起验收后才开放。
移动端仍需最终原生实证：iOS测试已出现TEST SUCCEEDED，但需确认NolaCore条件编译实际覆盖；
Service全tests曾因旧list_conversations作为durable fixture不符合新fresh query语义失败，
body_publication负责改为真实write fixture保留原收据断言并复验。Android实际JNI/emulator测试由body_replication继续。

### 字段级引用与移动入口验证续记

`message_part_text_references` 已从core/cli源码移除，单一 `message_part_body_references`
以(part_id,field)为主键；原text写入明确content_text，搜索及text fixtures按字段过滤。
reader负责resolver/export，复制通用化仅审计，未开放外部JSON发布。
切换后root实际通过：Application lib74/74、archive receipt2/2、turn transaction14/14、CLI bins check。
CommandTransactionRepository现承接receipt路径原UoW/savepoint/revision读取；
ReceiptAdmissionError显式区分请求身份冲突和持久化失败，Runtime仍具体Store，不代表App依赖切分完成。

移动验证更新：FFI24/24；iOS已链接NolaCore的receipt测试1通过0失败0跳过，
后加缺库XCTFail分支仍需补编译确认；Android真实API35模拟器JNI1/1通过（含Rust/NDK/APK构建）。
Service process6/6通过；全套另发现SDK对已关闭Unix peer做flush timeout配置导致Darwin EINVAL，
body_publication正做最小修复和真实进程回归。不得沿用旧fresh-query fixture失败为当前状态。

### 完整会话复制的Part时间保真修复

字段表切换后CLI replica回归暴露旧HTTP fixture缺少必填Part时间。补齐并新增原值断言后，
进一步定位Store `replace_conversation_projection` 将Part两个时间都写成conversation.updated_at_ms。
已改为保留原Part.created_at_ms/updated_at_ms；HTTP往返同时断言正文、digest和独立时间。
`cargo test -p nola_cli --bin nola replica_sync::tests --quiet` 最终8通过、0失败、2 ignored；
两个真实Worker专项本命令未运行，不将此前Worker证据视为本次复跑。

### 字段读取阶段最终验证

`cargo test -p nola_store --quiet`：98 unit + 90 integration，总188通过；
Service transcript_export_process_e2e 2/2通过。新structured_field_export_e2e 3项
涵盖同一个tool_result Part两个10MB字段、冷重启后20MB逐chunk字节一致、单Part有界metadata、
Unicode边界、NULL/empty/literal-null、owner/field/完成状态/可见性/双来源冲突。
这些引用由隔离fixture插入；生产JSON发布仍关闭，window/context/branch语义消费者仍待接入。
字段复制只读审计及后续12类验收见 `V2_STRUCTURED_BODY_REPLICA_AUDIT.md`：
优先共享字段admission、接收seal JSON验证，再统一upload owner、branch身份与双字段ACK。
不得仅移除text guard开放JSON；远端seal当前绕过prepare验证，必须同时补接收事务内验证。

### 共享字段准入与JSON不可见对象复制

Kernel新增纯`validate_body_bindings`，以借用Part声明检查完整owner、完成状态、目标字段inline/ref排他、
(part_id,field)去重及对象ID不可复用。无正文拷贝或IO；支持同Part两字段，保持另一字段独立。
Store本地append、remote append、archive validate及prepared publication调用同一adapter；
adapter仍先要求text field并限制text kind/no JSON，结构化可见消息发布尚未开放。
Kernel39/39、App archive receipt2/2及CLI bins check通过。

immutable replica的chunk/seal/enqueue/require_complete已支持ContentJson准备对象，接收seal新建、
既有对象和same-operation重放均在页事务内调用bounded JSON验证；非法尾部不推进cursor。
canonical对象已存在时首chunk先校验完整descriptor与原chunk字节，防止错误field/字节污染staging。
专项11/11通过，覆盖约10MB跨库重启逐字节一致、JSON非法尾部回滚、field/重放/配额边界。
这不是JSON消息发布完成；UploadOwner字段化、双字段append/archive绑定、branch身份和语义投影仍待完成。

本阶段整合最终验证：Kernel39/39；Store完整194/194（104 unit＋90 integration，
日志 `/tmp/nola-body-binding-store-tests.log`，0失败0跳过）；Application archive receipt2/2；
Service archive_restore_process_e2e1/1与replica_sync_process_e2e5/5；CLI bins check通过。
这些正式Service用例维持既有文本发布/归档行为；JSON复制新增11项在Store双库层验证，
本轮未跑真实Worker JSON专项、未开放JSON消息、未做新UI性能验收。

结构化语义审计见 `V2_STRUCTURED_BODY_CUTOVER.md` §10：当前MessagePartView分支校验嵌入part_id
会拒绝原文保持的branch，AgentPart路径却沿用旧owner；durable context还存在不同格式恢复差异。
后续需单一owner-bound semantic projection、受限token span提取与可信branch来源证明，
同时替换window和context消费，不能只修改显示ID或把工具JSON降级成普通text。

### 语义原文字段准备实现

Domain `part_semantics.rs`新增validated CanonicalPartOwner / SemanticSourceKey / SemanticTokenLocator。
inline source指纹覆盖原UTF8字节，immutable source与完整owner/field精确绑定；
locator是JSON原token半开字节范围（含引号与转义），不是授权，也不代表可信执行参数。
SemanticValue明确Complete与Deferred，不能以null/截断值冒充缺失或完整参数。
反序列化校验nested owner、field、source和range；Domain 13unit+6integration通过。

Store `semantic_json_projection`提取固定最多64个顶层字段，已知名支持Unicode escape，
重复known key明确歧义；大value/array只返回span，输出受总预算约束。
先原validator全流语法验证，重开再提取，双SHA256一致防止source变化。
4项差分/错误源/预算测试、1项10M allocator通过，最大单次分配32768B、峰值<=36744B。

`part_semantic_source::inspect_part_json_tokens`将提取接到规范owner、visibility与expected revision，
两遍读取共享一个SQLite snapshot，读取只通过bounded original field range。
返回source key和tokens；duplicate字段禁止生成可消费locator；prepared未绑定对象不暴露，
NULL与非法空JSON不同。3项adapter测试通过。此seam未接公开Protocol或正式window/context，
仍需typed schema转换、持久投影cache、分支来源证明和所有消费者切换。

整合 `cargo test -p nola_store --quiet` 最终202/202（111unit+91integration），
日志 `/tmp/nola-semantic-source-store-tests.log`。SDK当前授权环境全套16unit+1dependency gate通过；
此前4项socket bind失败属于子任务sandbox限制，已用实际通过结果闭环。
Service SDK flush修复后process6/6、recall13/13已取得exit0。
iOS增加缺NolaCore时XCTFail版本由root补跑，当前handle92920，日志
`/tmp/nola-mobile-receipt-ios-root-final.log`，只测试IOSApplicationReceiptRecoveryTests。

语义准备变更后的CLI/AgentHost生产bins check已通过。iOS最终版补跑仍使用root session92920，尚未收到终态，不能按日志等待误判失败重启。

iOS最终补验已收exit0：`/tmp/nola-ios-receipt-root-final-20260908.xcresult`
经xcresulttool实际解析1passed/0failed/0skipped，iPhone17e iOS26.5 arm64。
当前包含缺NolaCore则XCTFail分支，验证真实pending原envelope恢复及已提交写入不重复。
日志 `/tmp/nola-mobile-receipt-ios-root-final.log`；root session92920已终态，不要再轮询/重启。

真实Worker元数据回归由root接手：子任务原编辑/测试命令在审批等待中被中断，确认未启动/未落盘，
root已实际修改cli两个ignored fixtures：7条原文Part非空digest+独立时间，chunk与archive同样，
通过正式ExportCommand对接收端metadata逐条核对。已启动
`python3 scripts/test_worker_replica_e2e.py --with-cli`，root实际live session62501，
日志 `/tmp/nola-worker-part-metadata.log`，尚待终态，不要再次启动。
本测试仍是文本复制，不能视为JSON消息发布验收；branch digest保真另有待核查点。

### 真实Worker与分支Part metadata保真

root实际Worker测试session62501已exit0。`/tmp/nola-worker-part-metadata.log`
确认7条会话Part、chunk及archive非空digest/独立时间经正式ExportCommand保持；
同时通过15MB/500chunk正文、branch504facts、archive507facts/9轮、原子可见性及noecho。
这是已完成的text链路证据，不是JSON消息开放。

代码核查发现 `conversation_branch::copy_parts` 的immutable复制及大inline外置两分支
初始化content_digest=None而未加载；已改为每Part有界读取原digest，纳入累计manifest预算，
原chunk读取SQL也在取字符串前施加32KiB长度约束。两代分支保留digest/Part独立时间
新增测试通过，conversation_branch共4/4。CLI真实Worker分支阶段增加接收端同样metadata断言。
第二次真实Worker运行root session51493，日志`/tmp/nola-worker-branch-metadata.log`，
此条记录时尚在运行，不要因无缓冲输出重启。

Worker分支metadata复验session51493已exit0：真实接收端export确认chunk-branch也保持非空digest与101/202原Part时间。
日志 `/tmp/nola-worker-branch-metadata.log`；两次Worker均已终态，不再等待或重启。

分支metadata回归最终5/5，包括两代引用复制与超限digest失败不发布目标。
Store新增 `inspect_part_semantic_draft`，与原token读取共享owner/revision/visibility固定快照，
从DB kind选择draft预期类型；识别器使用两种实际schema，不把Recognized当owner证明或可执行授权。
owned source adapter当前4/4通过，正式window/context尚未接入。
审阅发现draft第二阶段的字段解码预算仍可能被前置正文耗尽而影响后置控制字段，
已交body_readers修复并补键顺序回归；不要把当前schema draft视为最终完成。

### 两种历史JSON schema的有界识别

Store新增semantic_part_draft识别AgentPart与MessagePartView，按实际kind选固定字段规则；
识别结果为Recognized或稳定Unsupported原因，typed嵌套字段另外标记pending_nested_validation。
不做巨大JSON整体反序列化，不把识别成功当内嵌owner已获证明或历史工具可执行。
owned source adapter现有inspect_part_semantic_draft固定规范kind/owner/revision/visibility，4/4通过。

审阅发现并修复控制字段预算受正文顺序影响：extractor新增固定scalar decode mask，
草稿阶段只解码控制/需范围验证的数值字段，正文始终保留原token span/Deferred。
不增加原输出上限；两种schema的text/reasoning/tool都加入正文前后顺序回归。
公开window/context仍未切换；接下来须typed nested验证、可信逐Part branch来源绑定、
统一窗口与context消费，再开放JSON可见发布。

本阶段最终整合：Store210/210（118unit+92integration），日志
`/tmp/nola-semantic-draft-store-final.log`，root session74701已exit0；CLI bins check通过。
草稿专项4/4、两类allocator各1/1；10M draft最大单次32768B，
AgentPart峰值35347B、MessagePartView34406B。所有root/reader测试已终态。

### 身份判断、嵌套结构与逐Part分支来源

Domain SemanticIdentityAssessment + Kernel assess_semantic_identity已落并接owned draft：
匹配规范Part/Message/Run才Canonical；不匹配DerivationRequired；缺少规范Run为CanonicalTurnUnavailable，
不能由JSON填补。身份判断不授权历史工具重放。Kernel40/40，owned source adapter5/5通过。

Store semantic_nested_draft已支持origin/permission/affected_resources固定结构，
Validated/Deferred/Unsupported明确区分；大数组超过16项或原字节预算时resources为空，
不能将前缀当完整结果。4项专项、semantic lib18项与新增allocator通过，10M/20万resource
最大单次分配32768B、峰值<=40659B。root新增part_nested_source将嵌套token从已识别父Part
规范snapshot中读取，绑定原字段locator与父identity assessment；拒绝错schema、过期revision或删除的owner，
适配专项1/1通过。这不是完整typed Part projection，artifacts/evidence等仍待验证。

Store正在整合逐Part来源表message_part_derivations，branch事务从实际source/target元数据和
两原字段SHA256产生来源记录（独立fingerprint，不伪装inline storage identity）。
目标FK级联，immediate/root不依赖源存活；记录禁止原位UPDATE，读取/继承验证目标当前元数据与原字节。
known branch缺失Part proof时root_source=None，再分支继续None；raw branch可用但不能冒充身份证明。
完整archive/replica provenance保存尚未接入，未用记录豁免当前JSON身份检查/开放JSON writer。
当前root全Store整合运行中，日志`/tmp/nola-semantic-identity-store-tests.log`。

本阶段最终整合Store222/222（129unit+93integration），日志
`/tmp/nola-semantic-identity-store-tests.log`，root48689已exit0；
Application branch_transaction_e2e3/3（root54454 exit0）及CLI bins check通过。
逐Part来源实现说明见 `V2_PART_DERIVATION_IMPLEMENTATION.md`。当前所有本阶段测试已终态。
仍需把来源记录以明确可信契约接入archive/replica，完成其余typed nested字段及正式window/context共用投影。
不得把identity assessment Canonical或NestedStatus Validated当作工具执行授权。

### Part来源复制接线施工

PartDerivation/Source/Fingerprint已移Domain，Store原名reexport；
StorePort新增StoredPartDerivation与receiver-owned provenance(LocalBranch/Replica/Archive)，
wire纯claim不允许携带provenance字段。Domain14unit+6integration通过，包含外部伪造local provenance字段拒绝。

复制按每Part<=16KiB独立事实推进；append/proof可同页按序，最终branch lineage增加必填
derivation_count，发出前要求全部proof exact ACK，接收核对target prefix覆盖、派生ID、原字段与source operation。
root已接dispatcher、same-operation重新校验、outbox barrier及bounded proof ACK保留。
新replay设计使用typed canonical SHA摘要收据，不依赖target当前存活/未修改；
exact删除后重放no-op不复活proof，changed同operation拒绝。当前由body_replication完成测试，尚未声明全套通过。

归档/导出接线只读方案见 `V2_PART_DERIVATION_ARCHIVE_EXPORT_PLAN.md`；
独立单Part导出query与独立archive provenance entry避免把512份16KiB记录塞进现metadata frame。
archive origin必须接收端强制标记，外部root owner+SHA不构成历史身份授权；尚未实现此接线。

### Part 来源复制整合验收与当前架构复核

本次重新核对目标 §§3–6、20、单写者门禁与真实入口依赖，实施顺序已补入
`V2_CURRENT_ARCHITECTURE_2026-09-08.md`。目标文档末尾旧审计中“FFI 写命令绕过收据”与
“Web branch 使用 Store Port DTO”已按现实现修正；其余未满足边界继续保留。

逐 Part 来源复制已完成：每份 claim<=16KiB，接收端强制 Replica provenance；本地后代
不将 Replica/Archive 升级。lineage 等待所有 proof ACK，并核对规范目标与覆盖数。
不可变 typed canonical SHA 收据允许目标编辑/删除后的 exact replay，changed operation 拒绝；
首次旧事实晚于 durable tombstone 时只保存收据、不复活内容，无 tombstone 的缺失目标仍拒绝。
依赖读取采用 SQL 重复检测与逐行遍历，没有随整个 branch 增长的 Rust HashSet。

root 实际验证：Store227/227（134 unit+93 integration），
`/tmp/nola-part-derivation-store-final.log`；Application branch3/3，
`/tmp/nola-part-derivation-application.log`；Service replica process5/5，
`/tmp/nola-part-derivation-service.log`；CLI bins check通过。以上均已 exit0。
真实 Worker首次新增proof回归已 exit0，`/tmp/nola-worker-part-derivation.log`：
15MB/500chunks、branch505facts、archive507facts、冷重启、精确原文和无echo通过。
进一步增强CLI真实Worker fixture，直接验证接收端原Part来源、root、字段fingerprint及Replica provenance；
增强版当前root session85255，日志`/tmp/nola-worker-part-derivation-final.log`，待终态。
归档来源与独立导出query尚未接通，JSON可见消息发布仍关闭，整体V2仍未完成。

增强版真实 Worker 回归 root85255 已 exit0：`/tmp/nola-worker-part-derivation-final.log`
明确包含 `branch_derivation=true replica_provenance=true`，接收端来源 owner/root/原字段指纹
与强制 Replica 标记断言通过；15MB、branch505facts 与 archive507facts 仍通过。
本阶段所有 root 测试已终态，不再等待或重复启动。下一步按归档/导出方案继续。

### 来源导出与归档恢复接线（本轮实施）

- receiver-owned StoredPartDerivation/Provenance 已统一移Domain，StorePort reexport；Protocol维持仅协议/Domain依赖。
- ExportPart必填has_derivation；独立fresh query `read_conversation_part_derivation` 输入完整owner与expected_conversation_revision，同snapshot验证revision/visibility/当前字段SHA，返回null或<=24KiB的record。
- archive manifest/internal entry新增 `part_derivation {record}`，外部只接受claim；所属Message之后独立stage、独立sequence/digest；staging target唯一；final重新检查实际Part元数据/原文字段，在既有UoW插入强制Archive来源。
- Mac实际分页backup writer按flag独立query并写纯claim；restore每Message后stage对应记录。缺失已声明record直接失败，旧backup无该字段仍可读取。
- root追到Web GET conversation/export?format=json，原格式仅输出Web展示messages。现附加canonical对象，保存原始Part字段与纯claim，不污染共享业务ConversationPart DTO；load_canonical_archive使用有界Service分页/range/query并最终复查revision。
- Web export目前仍在客户端整体重建并作为JSON字符串响应，Mac restore也仍全文件解码；本轮没有将这些路径宣称为整体内存有界。JSON导出里的canonical与展示messages为不同表示，后者不得用于恢复来源授权。

已验证：root真实Service export2/2+archive2/2，日志`/tmp/nola-derivation-export-archive-service.log`。
包含10MB导出、query无收据、owner/revision/篡改拒绝、本地branch源删除后export→新库restore→两次冷重启stage/final原收据重放，恢复origin为Archive且claim相同。
CLI production bins check与10M两代Web branch canonical导出1/1通过；日志
`/tmp/nola-derivation-archive-cli-check.log`、`/tmp/nola-derivation-web-export.log`。
Mac build-for-testing exit0（`/tmp/nola-derivation-mac-build.log`），全MacCanonicalExportTests原生运行root15107；
Store完整整合root94830（`/tmp/nola-derivation-archive-store-final.log`）待终态；另在补Application receipt外层失败注入。

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

归档最后增量验证已终态：Store archive13/13、Application cargo check通过；
新增独立record<=16KiB、entry<=128KiB预算校验，避免把wrapper开销错误计入record预算。
最大escaped metadata fixture仅验证DTO编码预算，不声称任意历史target或root具有可信身份。
当前root与三个子任务均无待处理测试；本阶段未提交或清理工作区。

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

本轮root整合终态：Store238/238（145unit+93integration），
`/tmp/nola-semantic-local-binding-store-final.log`；Kernel41/41，
`/tmp/nola-semantic-binding-kernel-final.log`；CLI bins check通过，
`/tmp/nola-semantic-local-binding-cli.log`。全部exit0。
随后补入源会话原字段后改但target不变仍匹配历史proof的断言，
Store identity6/6最后复验exit0（`/tmp/nola-semantic-binding-source-final.log`）。

Core真实serde oracle `semantic_nested_wire_oracle`1/1通过，使用29条共享案例；
SourceEvidence/SourceLocation/CitationRange实际decoder确认namespace personal name:null
可接受、name:String拒绝，未知扩展按真实schema处理。新增Store共享案例与allocator最后结果另记。

下一阶段必须开始共同typed历史投影的实际消费接线（window、new turn、idle revert及五端），
不以继续累积准备seam替代迁移。尚未支持的nested要明确Unsupported/Deferred并保留原文，
不可猜义、伪造参数或恢复历史工具执行。

最后增量终态：Core真实oracle1/1 + Store同29案例定向1/1；nested allocator2/2。
新增5种10MB输入（evidence/location/range/namespace/supersedes）最大单次分配均32768B，
峰值分别36915/40656/40656/36920/36915B。所有root与子任务测试均已终态。


## 最新核对：历史窗口、模型上下文与回退已共同接线

2026-09-08，本节覆盖下文较早阶段的“公开消费者尚未切换”记录；整体 V2 仍未完成。

实际链路为：规范 Message/Part → Store 同快照 owner/revision/来源验证 →
`HistoricalPartProjectionRepository` → Application 共同历史 adapter →
窗口 `MessagePartView::Historical` 或有界 AgentItem 上下文。
新轮次恢复与 idle revert 均消费这条链；两个旧入口的完整 AgentPart/MessagePartView JSON 解码已删除。
活跃 Core Part 保持活跃投影语义，历史节点不会恢复权限等待或工具执行。

- Store 区分原 JSON 不存在与窗口未携带 JSON；大字段提供 Deferred 原文定位，验证失败提供 Unsupported。
  每 Part Application 预算最多 8192 bytes，选择范围共享 32768 bytes；字段编码及定位元数据计入预算。
  最小控制元数据仍装不下时明确返回错误，尚未实现自动缩小语义窗口或统一 omission counts。
- Domain 当前为固定 schema 字段目录和 Complete/Deferred 状态，尚不是完整逐 kind 的最终语义类型。
  非空 artifacts、retained IDs 等未完成验证的结构仍 Unsupported，不推测其含义。
- 上下文只恢复完整的历史工具 call/result 对；确定性 call ID 限定于目标 Part，
  不完整字段生成显式说明。最终 AgentItems、ownership 和说明合计限制为 32768 bytes，
  超预算按完整 Message 组舍弃，不拆开工具对。这不是精确模型 token 上限。
- idle revert 用元数据定位真实 boundary，并将有界上下文选择锚定到该边界；
  receipt 故障仍原子回滚。待隐藏 Message IDs 与 mutations 仍是 O(N) 元数据。
- macOS、iOS、Android、Web 和 CLI Web adapter 已接 Historical 解码/呈现；
  文本与思考可展示完整值，其余目前主要显示静态类型/状态。CLI 旧 SavedSession 终端回放没有迁移。
  Deferred locator 已保留，但没有新增公开 token-range 协议或完整交互控件。

### 本阶段验证终态

| 验证 | 结果 |
| --- | --- |
| Store 全套及新增历史 allocator | 245 项 + 1 项通过；10MB JSON 最大分配 32768 B，峰值 67960 B |
| Application lib、branch、revert、实际历史 window | 77 + 3 + 1 + 2 项通过 |
| 真实 Service transcript/export/window 与冷重启 | 3 项通过，历史查询不写收据 |
| Kernel projection / FFI Historical / CLI Historical / Web Node | 22 / 1 / 1 项及 Node 专项通过 |
| macOS 原生 MacCanonicalSessionProjectionTests | 9 项通过，0 失败、0 跳过 |
| iOS 模拟器 IOSConversationProjectionContractTests | 8 项通过，0 失败、0 跳过 |
| Android ConversationProjectionV1DecoderTests | 4 项通过，0 失败、0 跳过 |

root 最终日志：`/tmp/nola-historical-app-final.log`、`/tmp/nola-historical-window-final.log`、
`/tmp/nola-historical-service-final.log`、`/tmp/nola-historical-mac-native-final.log`、
`/tmp/nola-historical-ios.log`、`/tmp/nola-historical-android-final.log`。
本阶段已启动的 root 验证均已终态。没有提交、清理工作区或重建用户数据库。

### 后续实施顺序与未关闭的架构缺口

1. 补齐历史 source/citation/artifact/task 的语义与跨 Part 引用映射、opaque JSON 的可见原文呈现、
   canonical attachment 与 AgentPart artifact 的类型对应，以及语义预算不足时的可用分页契约。
   当前不能声称工具与引用的完整语义迁移或五端交互验收完成。
2. 收敛 Application 主库装配与事务端口：`runtime.rs` 仍持有并打开具体 NolaStore，
   Cargo 仍依赖 nola_store 和 nola_core。已有 Port 不等于 Application 全面解耦。
3. 继续将旧 Core 的 Agent 状态机、权限、上下文与长任务决策迁入纯 Kernel，
   拆分 CLI/Web 的客户端与 Host 装配，并迁出 Android Room 的业务 schedule/result。
4. 完成 ARCHITECTURE_V2.md §20 的五端事件一致性、权限恢复、真实取消、
   多窗口性能、截图与真实模型端到端验收。专项测试不能替代这些门槛。

结构化 JSON 引用的可见发布门禁继续关闭；本阶段使用已准备不可变原文验证读取。


## 最新增量：语义预算、引用身份与主库装配

本节覆盖较早阶段关于“预算不足整窗失败”“Source/Citation只能省略”及“Application仍打开主库”的记录。
保留全部工作区修改，整体V2目标继续有效，未提交或重建用户数据库。

- 窗口全部Part仍保留，语义32KiB池耗尽后明确NotLoaded，source/schema/binding均无伪造值。
  同owner/revision的独立fresh query可补读，完整窗口含metadata与占位合计受2MiB编码上限约束。
  五端已识别新状态及canonical_text描述预览；**尚未接五端点击加载UI**。
  详见[窗口预算契约](V2_HISTORICAL_WINDOW_BUDGET.md)。
- Store只按已证实的生产形状支持AgentPart artifact→canonical attachment对应，
  ArtifactReference嵌套有界验证；Opaque/Unsupported JSON可保留独立规范文本预览。
  预览计入输出预算，不参与工具执行或身份授权。
- 模型上下文两阶段建立同目标Message内的source/citation/tool映射，支持两种历史schema及两代本地branch删源重启。
  缺失、歧义、URI不符或Deferred关联明确降级；附件/任务贡献描述性数据。
  **跨Message引用尚未支持**，需要与依赖组裁剪契约一起完成。
- 修复额外发现的context item ID碰撞：所有条目与说明使用完整owner及role的独立散列身份，
  不再用裸Part ID或`p:call`后缀；重复ID/缺失ownership显式拒绝。
  真实旧tool `tool`与新Part `tool:call`复现证明按Message裁剪不会留下孤立ToolCall。
  详见[上下文引用与身份](../参考研究/V2_HISTORICAL_CONTEXT_REFERENCES.md)。
- 主库mkdir/open及显式recreate策略迁入独立`nola_application_bootstrap`，
  Service、移动FFI和CLI临时会话真实入口已切换；Application生产仅接受已装配Store，
  Service的直接生产Store依赖已删除。Application的具体Store参数/字段与旧Core依赖仍在，
  **不是全面依赖倒置完成**。详见[装配切换审计](V2_APPLICATION_BOOTSTRAP_CUTOVER_2026-09-08.md)。

本阶段验证终态：

| 范围 | 证据 |
| --- | --- |
| Store完整suite | 248/248（153unit+95integration）；`/tmp/nola-store-preview-final.log` |
| Application最终context身份、分支、回退 | 83+3+1通过；`/tmp/nola-context-identity-final.log` |
| 实际窗口预算/escaped metadata | 2项通过；详见窗口预算文档 |
| 真实Service历史/export/window | 4/4；64Part初窗、独立query、错owner/revision拒绝、冷重启无receipt；`/tmp/nola-history-budget-service.log` |
| 真实Service进程/单Part补读 | 6+1通过，包括显式开发重建入口；`/tmp/nola-history-bootstrap-service-final.log` |
| CLI两个服务入口旧schema保护 | 1/1，检查精确schema错误，排除socket失败假阳性；`/tmp/nola-bootstrap-cli-schema.log` |
| Bootstrap及workspace | bootstrap4/4；workspace all-targets check通过，`/tmp/nola-history-bootstrap-workspace-final.log` |
| macOS / iOS / Android历史decoder | 10/10、9/9、5/5；均0失败0跳过；`/tmp/nola-history-budget-{mac-native,ios,android}.log` |
| Web / CLI新共享fixture | Node通过、CLI1/1；详见窗口预算文档 |

原生decoder验证针对本轮NotLoaded/canonical_text契约；后续context item身份修复由最终Rust
83+3+1专项验证，不能将原生decoder测试当作真实模型上下文端到端证据。
所有root构建/测试句柄均已终态。下一阶段需接按需读取的实际用户交互，继续Run/Checkpoint/Effect
事务端口与DTO迁移，并最终完成Kernel、Android业务存储及ARCHITECTURE_V2 §20完整验收。


## 最新增量：实际历史详情交互与 RunEffect 端口

2026-09-08。本节覆盖前文“尚未接五端历史详情入口”及“Run/Checkpoint/Effect端口待开始”的旧阶段描述。
工作区全部保留，未提交、清理或重建用户数据库，完整V2目标仍未完成。

### 真实用户入口

macOS消息行详情sheet、iOS ChatBubble sheet、Android MessageBubble dialog、Web历史详情按钮
以及CLI `session part <conversation> <message> <part> <revision>`均已接通。
详情是独立只读状态，不替换canonical stream节点，不恢复工具/权限/Run；
owner/revision固定，失败可重试，迟到响应被丢弃，32768-byte之外的字段仍明确Deferred。
完整范围与验证见[历史详情交互](V2_HISTORICAL_DETAIL_INTERACTION.md)。

实际HTTP/CLI回归发现并修复了CLI通用startup recovery污染历史查询的问题：
显式Sessions/SessionShow/SessionPart跳过cleanup及orphan Run恢复预检；
SessionPart成功/失败读取均不增加receipt，窗口与原JSON不变，冷重启后查询相同。
Web还修复“先分页→打开旧行详情→分页返回”的竞态，应用新窗口前关闭旧详情。

**仍未完成的UI门槛**：浏览器完整点击与截图、原生视觉截图、移动真实UI点击到native库全链路，
以及超大原字段range下载交互。Browser技能初始化后可用浏览器列表为空，
没有以HTTP或Node状态测试冒充浏览器验收，也没有把原生单元测试计作完整UI验收。

### 主架构生产迁移

Run/Checkpoint/Effect的9个DTO已从具体Store迁入StorePort，9个实际Application helper改为
`RunEffectRepository`参数，Runtime两处checkpoint revision探测也走Port。
恢复投递在同一快照取得effect、canonical owner和checkpoint；transition仍使用原连接/savepoint，
不提前提交外层receipt UoW。非SQLite fixture调用真实Core与Application验证实际依赖倒置。
细节与当前Core session/canonical conversation独立身份域见
[RunEffect迁移审计](V2_RUN_EFFECT_PORT_CUTOVER_2026-09-08.md)。

### 验证终态

| 范围 | 本轮证据 |
| --- | --- |
| 最终Application整合 | lib83 + 非SQLite2 + turn transaction14全部通过；`/tmp/nola-run-effect-application-final.log` |
| Store/receipt边界 | Store adapter1、增强最终receipt失败回滚1、receipt unit3通过；见RunEffect审计 |
| 真实Service长流 | 2048 delta、RSS、terminal Core淘汰、replica接收端重启1/1通过；见RunEffect审计 |
| 全workspace编译 | all-targets check通过（子任务32980，exit0） |
| macOS | 新详情4 + 原投影10 = 14/14；`/tmp/nola-history-detail-mac-native.log` |
| iOS | 新详情2 + 原投影9 = 11/11；`/tmp/nola-history-detail-ios-final.log` |
| Android | 新详情3 + 原投影5 = 8/8；`/tmp/nola-history-detail-android.log` |
| CLI/Web真实进程 | HTTP与CLI同结果、owner/rev拒绝、无receipt/原文变化、重启1/1；`/tmp/nola-history-detail-web-cli-final.log` |
| CLI参数 / Web状态 | parser1/1；Node历史decoder、详情及实际分页函数竞态通过 |

三端上述原生/JVM测试均0失败0跳过。iOS首轮缺Combine导致编译失败已修，最终结果覆盖旧失败。
root和子任务已启动的构建/测试全部终态，不重复等待旧句柄。

下一步继续Run创建（BeginTurn、内部/maintenance Run）DTO/端口与真实调用迁移，
把Runtime收敛到同一UoW的聚合仓库。Application仍持有具体Store，旧Core纯决策、Android业务存储、
跨Message引用、五端权限/长任务/真实模型与§20性能UI门槛均未宣布完成。


## 最新增量：Run 创建、历史读取、Skill 与权限仓库端口

2026-09-08。本节覆盖下方旧阶段中相应端口“待开始”的描述。保留现有工作区修改，未提交、清理或重建用户数据库。

当前真实依赖可概括为：客户端经 Service/FFI 到 ApplicationRuntime；Runtime 仍持有具体
`NolaStore`，但普通/internal/maintenance Run 创建、Run Effect、历史读取/上下文/revert、
Skill 编辑/选择/可见性以及权限请求/规则投影的实际 Application helper 已改用 StorePort。
SQLite adapter 复用原连接，受外层 receipt Unit of Work 管理。主库打开由 bootstrap 装配。
这完成了部分业务用例的依赖倒置，并未消除 Application 对 Store 和旧 Core 的生产依赖。

| 本轮切面 | 实际变化 | 验证 |
| --- | --- | --- |
| Run 创建 | 5 DTO 与 RunCreationRepository；普通、内部、maintenance 创建真实入口泛型 | 非 SQLite 3、turn transaction 15、Store adapter 1、ledger 15、append sync 5 通过 |
| 历史与上下文 | canonical/history 与 projection 读 DTO 迁入 Port；full/page/window、restore、durable context、revert 真实入口泛型 | 非 SQLite 2、branch 3、bounded revert receipt 1、Application lib 85 通过 |
| Skill | 不可变正文版本、编辑目录、选择、内置可见性通过 SkillRepository | 非 SQLite 2、编辑 receipt 16、catalog 1、Store version 2 通过 |
| 权限 | 5 durable DTO 与 PermissionRepository；请求命令及 grant/rule helper 泛型，Core 保持规则评估责任 | 非 SQLite 2、请求/receipt 4、ledger 2 通过 |

权限新增真实 SQLite receipt 最终 UPDATE 故障测试：resolution 与 receipt 同时回滚，
请求保持 pending；同请求成功重试后，冷重启 replay outcome 相同。非 SQLite fixture
验证 typed input、干预指令去空白、错误传播、无效持久化状态拒绝和真实 Core 的有序规则评估。
这些专项不能替代 §20 的所有 scope、真实工具与 OS 权限验收。
本轮最终 `cargo check --workspace --all-targets --quiet` 通过（exit 0），日志
`/tmp/nola-port-integration-check.log`。所有本轮测试句柄已终态。
日志：`/tmp/nola-permission-repository-final.log`、`/tmp/nola-history-port-tests.log`、
`/tmp/nola-history-port-app-lib.log`；Run 创建详见 `V2_RUN_EFFECT_PORT_CUTOVER_2026-09-08.md`。

下一步按所有权收敛：先补齐 Runtime 尚直接调用的仓库能力和事务聚合接口，再移除其具体
Store 依赖；继续旧 Core 到纯 Kernel、CLI Host/客户端边界和 Android 业务存储迁移。
跨 Message 引用、五端权限/长任务/真实模型、10M 会话切换帧时间/RSS/延迟与 UI 截图仍需完成。
不可用目录中存在 Port 或局部测试通过来判定完整 V2 完成。



## 最新增量：任务、计划、Model/MCP、自动化与历史编辑

2026-09-08。本节覆盖较早端口清单；完整 V2 仍未完成。未清理、提交或重建用户数据。

### 实际生产迁移

- RuntimeTaskRepository 与 WorkspaceOwnershipRepository：4 DTO、28 方法；任务命令/进度及五个 checkpoint/lease helper 泛型。公平 claim、重试和租约规则沿用既有事务实现。
- ConversationPlanRepository 与 WorkStepTransactionRepository：3 DTO；计划和步骤所有实际 helper 泛型，步骤推进与计划关联保持一个原子存储调用。
- ModelRepository 与 McpRepository：3 DTO；模型参数/替换、MCP 编辑和目录真实 helper 泛型，同步和 receipt 仍共享原连接。
- AutomationRepository：2 DTO、12 方法；定义/Run dispatcher 泛型。非 SQLite 验证 lease 身份传递、存储拒绝、retry 状态投影和未知状态拒绝。
- TranscriptEditRepository：编辑/删除 helper 泛型；复用已有 projection DTO。SQLite 提交 edit/delete 时新增同事务活跃 Run 与替换消息已知 Run 归属复查，避免仅依赖 Application 提前检查。失败不改变 revision、消息或 operation，合法操作仍可提交/冷重启幂等重放。

### 验证终态

| 范围 | 结果 |
| --- | --- |
| 最终 Application lib | 95/95，`/tmp/nola-business-port-app-final.log` |
| 最终 workspace all-targets check | exit 0，`/tmp/nola-business-port-workspace-final.log` |
| 任务/租约 | App task 4 + Store scheduling 8 + workspace 1；详见 `V2_RUNTIME_TASK_PORT_CUTOVER_2026-09-08.md` |
| 计划/步骤 | 新非 SQLite 2；生命周期/审批/步骤/receipt 8，`/tmp/nola-plan-step-port-final.log` |
| Model/MCP | 新非 SQLite 3；编辑、持久化及同步/事务原子性 16，通过子任务验证 |
| 自动化 | 新非 SQLite 2；真实 receipt 回滚/冷重放 1，Store 定义/Run/删除 3；`/tmp/nola-automation-edit-final.log`、`/tmp/nola-automation-port-receipt.log`、`/tmp/nola-automation-store-final.log` |
| 历史编辑 | 新非 SQLite 1（包含于 lib 95）；新 Store admission 1，App Run scope/输入验证 2；`/tmp/nola-transcript-edit-port.log`、`/tmp/nola-automation-edit-integration.log` |

自动化 receipt 故障测试证明 definition、sync outbox 与 receipt 同时回滚；重试成功、冷重放不重复 outbox。
所有本轮测试句柄已终态。中途跨 agent 文件暂态编译失败已由最终整合覆盖；未将 filter 下 0-test 结果计入专项通过数。

### 下一步具体边界

代码核对仍存在的具体 `NolaStore` helper：`remote_turn.rs`、`remote_projection.rs`、
`resume_snapshot.rs`、`sop_runtime.rs`，以及 `lib.rs` 的 full session projection、remote mirror、
canonical branch 与 idle submission。`application_command.rs` 仍有权限读模型、consistent-read 窗口、
设置 CAS、trace/task 查询、lease release 等直接调用；Runtime 字段仍为 `NolaStore`。

应先把这些真实调用纳入仓库能力，并保留窗口查询的一致快照和 receipt 单一 UoW，
再设计/接入 Runtime 所需的聚合仓库，移除具体 Store 生产依赖。不要以泛型参数或 trait 目录存在
代替这一整条调用链的切换。之后仍须继续纯 Kernel、客户端所有权及 ARCHITECTURE_V2 §20 全部验收。



## 最新增量：Runtime 解除具体 Store 生产依赖

2026-09-08。本节覆盖此前“Runtime 仍持有具体 NolaStore”的旧阶段描述。

Application 的实际 Runtime、完整命令 dispatcher、receipt、projection feed/journal 和上传分发
已统一为 `ApplicationRuntime<R: ApplicationRepositories>`。`nola_store` 仅保留在
Application dev-dependencies；普通依赖树中没有 Store/SQLite。具体类型由 bootstrap 选择，
Service、移动 FFI 与 CLI 都已接该装配 alias，writer 排他和原事务连接没有拆分。

本轮同时完成远程会话、恢复快照、SOP、branch、idle checkpoint、队列、trace、settings
及 Runtime 所需 metadata 方法的 Port 接入。独立恢复候选查询也纳入一致快照。
详见 [Runtime 仓库切换审计](V2_RUNTIME_REPOSITORY_CUTOVER_2026-09-08.md)，
含真实调用范围、专项验证与全量回归结果。修复 checkpoint-only 超大呈现读取后，
最终 Application 全量 256/256（0 失败/忽略）与 workspace all-targets check 通过。
旧 Core、客户端业务存储/所有权及 §20 完整
验收仍待完成，完整 V2 目标保持未完成。



## 最新增量：工具批次 Kernel、显式时间与权限 origin

2026-09-08。本轮沿实际 Application→AgentCore→tool_driver 链路，将工具批次的选择算法
完整移至 `nola_kernel::plan_tool_batch`。Domain 仅传生命周期与 capability 事实，旧 scheduler
只做类型映射；没有新增第二份算法或按用户文本路由。

Application 的创建/提交/Host response 观察时间现显式传给 Core，Effect 创建和恢复不再在
reducer 内部读取系统时钟；standalone Core 的采时 facade 仍存在，不能据此宣称整个旧 Core 已纯化。
另修复同批权限重评隐式丢失 origin：scheduled/replay 的其他等待调用不能继承交互式
Conversation/Turn 授权，直接批准当前调用与 interactive 合法批量放行均保留。

验证：Kernel 4、Core batch adapter 2、权限 Core/contracts 19、真实 Application batch receipt 3
通过；最终 Application 全量 260/260、workspace all-targets 编译通过。
详细时间输入、持久化 Effect 身份、冷恢复和剩余权限分类边界见
[工具批次迁移审计](V2_TOOL_BATCH_KERNEL_CUTOVER_2026-09-08.md)。

下一步继续权限的纯规则/authority 决策契约及其真实调用；不能扩散旧工具名称分类器，
也不能把 scope 的路径/shell 语义顺手改掉。旧 Core 的 model/turn/compaction、客户端业务所有权
及 ARCHITECTURE_V2 §20 验收仍未完成。保留工作区所有修改，完整目标保持进行中。



## 最新增量：权限规则评估与作用域匹配迁入 Kernel

2026-09-08。六个权限事实 DTO 已归 Domain；Core RuleSet 保留薄适配，实际 tool_driver
规则评估现调用 Kernel。规则顺序、origin 过滤、匹配解释及路径/bash/wildcard 匹配没有旧 Core
算法副本。风险分类器仍在旧 Core，以显式事实传入 Kernel，未宣称整个权限系统纯化。

另修复 shell scope 两个真实边界：双引号内展开不能继承脚本前缀授权；普通字符前反斜杠
必须保留，不能把另一个脚本误认作已授权脚本。迁移契约、shell 反例及真实 once/conversation
收据与冷恢复验证见 [权限 Kernel 审计](V2_PERMISSION_KERNEL_CUTOVER_2026-09-08.md)。
最终 Application 263/263（0 失败/忽略）和 workspace all-targets 编译通过，所有句柄终态。

下一优先生产缺口已实证：Custom response 当前结束原调用，可能转模型或留下无 outbox 的
AwaitingTools，不满足 §20.2 原调用挂起与恢复。下一步处理该完整状态转换；不要用不授权/
不执行测试冒充这一验收。可信 catalog 风险契约、durable scope 执行层冷恢复、旧 Core 剩余
状态机、客户端业务所有权及全部性能/UI 验收仍待完成。工作区修改保留，完整目标未完成。



## 最新增量：Custom response 保留并恢复原调用

2026-09-08。上一轮架构总览后，本轮实施 Core 原调用挂起与恢复。CoreTool 和 delegated 均保留 call/arguments/resource，typed `permission_instructions` 有界持久化；交互旋转 request/effect ID，delegated 最终决策仍使用原 backend request/target。说明不能产生 grant，也不能经同批 conversation 放行绕过其后继明确决策。未清理、提交或重建用户数据库。

App 真实收据测试覆盖失败回滚、冷重放、原调用恢复和 sibling 独立权限；后台任务 transport 的旧回应 resolved 只表示回应可投递，新交互仍 pending。CLI SOP 保持首个 wait reference，Custom 不 Replan；新增只读 ValidatePermissionInvocation 允许同一个等待继续呈现，普通执行校验仍拒绝 Waiting。详见 [生命周期说明](V2_PERMISSION_TRANSPORT_AND_CALL.md)。

CLI Core 面板增加独立说明输入。Mac/iOS/Android/Web 已读取和显示说明，Web 修复旧卡片事件处理与迟到回复覆盖，Android 修复 permission envelope 被 trace 提前截走。**尚未给原本仅授权按钮的各端增加完整 Custom 输入入口**，不能宣称五端 Custom UI 端到端完成。

本轮已验证：Core lib 270、Host contract 66 + permission projection 2、新 Custom 专项 4；真实 App batch/permission 6；真实 CLI Host SOP 1；CLI 说明输入 1；最终 Application 全量 263、workspace all-targets 编译；Mac 新测试 2；Android 新说明 2 + 既有 SSE 5；Node 实际 Web 模板测试通过。iOS 权限旧解析与新增说明测试 2/2，xcresult summary 确认 0 失败/跳过。所有构建和测试已终态；未用历史测试冒充本轮验证。

主要日志：`/tmp/nola-custom-response-application-full.log`、`/tmp/nola-custom-response-workspace-final.log`、`/tmp/nola-custom-mac-tests.log`、`/tmp/nola-android-permission-instructions-final.log`、`/tmp/nola-custom-cli-input.log`、`/tmp/nola-custom-ios-tests.log`（结果包 `/tmp/nola-custom-ios-tests.xcresult`）。

下一步：五端 Custom 输入与明确授权交互、真实 delegated provider 恢复；随后继续可信 capability 风险事实、旧 Core turn/model/compaction 决策迁移，CLI/Android 所有权，以及全部 §20 权限/长任务/性能/UI 验收。完整目标保持进行中。


## 最新交接：Custom 输入、真实传输与后继请求（2026-09-08）

本节覆盖上一阶段“客户端只有展示、尚无输入”的状态。保留所有已有工作区修改，未 commit，未重建用户数据库；schema 14，完整 V2 仍未完成。

- Mac/iOS/Android/Web 的真实权限卡新增独立输入与 typed intervention；回调绑定 request ID，失败保留草稿，后继请求重置草稿。OS DeviceBridge 没有 Core continuation，不提供 Custom。
- 实际 native broker resolver 不再丢失 instruction history，challenge/SSE 携带有界 metadata；共用预算在消耗等待前校验。后台 transport 保存真实 Core ID 与 history，Store 在事务内复核预算，Custom 后继不会被旧 task grant 自动批准。
- Web 实际 onProjection 与 iOS 实际 AsyncStream 消费覆盖 p1 HTTP 在途时 p2 先到、旧事件重放、迟到回应，以及失败后原请求重试。Mac 新增实际远程 envelope decode→consume 链路，校验身份/序列，独立 local Core snapshot 不清 remote challenge。
- broker 的 exact response 重试返回进程内有效期 ACK，冲突回应拒绝且只投递一次；这不是 durable receipt 或执行完成证明。

已收到终态的验证：

| 范围 | 结果与日志 |
| --- | --- |
| Application 全量 | 264 通过、0 失败/忽略；`/tmp/nola-custom-input-application-full.log` |
| Workspace all-targets | exit 0；`/tmp/nola-custom-input-workspace-final.log` |
| CLI 实际 broker | 4 通过；`/tmp/nola-custom-input-broker-final.log` |
| 后台权限 | App repository/transport/Core receipt 13、Service 冷重启 1、CLI persisted return-to-composer 1 通过；CLI/Service all-targets check 通过 |
| macOS 原生 | Custom 4 + Instruction 2 + RemoteConsumption 2 = 8 通过；`/tmp/nola-custom-input-mac-final.log` |
| Android JVM | Submission 4 + ViewModel 17 = 21 通过；`/tmp/nola-custom-input-android-final.log` |
| Android Compose | emulator-5554 上真实输入、发送、p2 空草稿 1 通过；`/tmp/nola-custom-input-android-ui-verified.log`。初次空文本断言包含标签导致失败，修正为 EditableText 精确断言后通过，未放宽行为要求 |
| Web | `node web/tests/permission_instruction_test.js` 通过，包含真实模板 onProjection 的迟到 HTTP 回归 |

iOS 初次编译发现新测试 map 缺少 `_ in`，修正后最终 4/4 通过（`/tmp/nola-custom-input-ios-verified.log`、同名 xcresult）；本轮 XCTest clone 已确认不存在。Android 技术预算说明改为用户可理解的提示后，重编译与直接 instrumentation 1/1 再次通过（`/tmp/nola-custom-input-android-ui-final-capture.log`），最终输入/后继截图均已查看（`/tmp/nola-permission-instruction-editing.png`、`/tmp/nola-permission-instruction-successor.png`）。Mac 的 8/8 基线后发现远端 resolved/运行终态未清卡，正在补充实际事件消费回归，以随后补充终态为准。内置 Browser 无可用实例，因此没有本轮真实 Web 浏览器点击/截图验收。

明确下一步：

1. `native_bridge.rs::wait_for_host_permission` 仍将 300 秒 timeout、registry/trace 错误映射 Deny；必须区分用户决策与 transport 中断，设计可恢复等待及已提交响应身份。
2. live nonce/waiter/ACK 仍是进程内状态，跨进程恢复与其他客户端批准后的 canonical 终态关联未完成；不能把 exact HTTP ACK 作为此项验收。
3. 继续迁移旧 Core 的 turn/model/compaction 决策与可信风险事实，再收敛 CLI Client/Host、Android Room 领域所有权，按主文档 §20 做五端真实模型、长任务、UI 和性能验收。

全工作区 `git diff --check` 发现早已存在的两处无关空白（URLFetcherService.swift、MacMarkdownParserTests.swift），保留原修改；本轮相关路径检查通过。


### 本轮最终整合终态

在上述基线后完成两项实际入口修复：

- Mac 实际消费 `tool.permission.resolved` 和规范 run completed/cancelled/failed；以创建请求时的 generation/conversation/trace/run/turn/request/call/sequence 精确关联，旧 p1、错误运行身份和无 trace 的连接错误不清 p2。提高 replay 水位防止已终止 ask 再现，清卡不合成用户决策。最终 10/10 原生通过，`/tmp/nola-custom-input-mac-lifecycle.log`。
- Rust Host 权限协议实际为 3，但 iOS/Android 连接常量仍是 2，Android permission SSE 分支还是 1，此前输入 helper 测试未覆盖此漂移。现统一 Host 权限协议 3，硬切拒绝旧版本；DeviceBridge 自身协议 1、Core projection 协议 2 保持独立。iOS 实际 HostIdentity 解码不再吞掉协议错误，SSE 入口检查版本。Android 实际 SSE 分支使用生产 identity/transition helper，错误身份/过期/已知前驱事件不覆盖后继 UI。

最终验证全部已收到退出结果：

- iOS 4 项交互 + 3 项实际 wire/协议测试 = 7/7，`/tmp/nola-custom-input-ios-protocol.log` 与 `/tmp/nola-custom-input-ios-protocol.xcresult`；runner 已清理本轮 clone。
- Android HostPermissionEvent 2 + Submission 4 + ViewModel 17 + ConnectionWorkflow 3 = 26/26，0 失败/跳过；`/tmp/nola-custom-input-android-protocol.log`。真实 decoder→生产 transition→typed Custom 加密 payload 在回归中执行。Compose 最终截图验收仍为上述 1 项，未冒充整个真实 Host/模型会话。
- Application 264/264、workspace check、broker 4 项、Web template 回归仍为本轮已通过基线；后续只改客户端与文档，没有重新运行相同 Rust 测试。
- 本轮相关路径 `git diff --check` 通过，保留原有两处无关空白。所有测试句柄已终态，agent 无未完成写入；无须轮询旧句柄。

尚未完成：远程超时/重启恢复，完整五端 canonical 权限终态（Mac 本轮已补），所有 scope 与真实 provider/OS 验收，以及旧 Core Kernel 迁移、客户端所有权和 §20 全量目标。下一轮需处理 transport 中断的可恢复状态；仅将 timeout Deny 改 Err 会被现有 Host driver 映射为不可重试 `permission_ui_failed`，不足以修复。


## 最新交接：权限 transport 中断保留原调用

本节覆盖上一轮“超时仍映射 Deny”的状态。所有原有工作区修改保留，未 commit、未删除/重建用户数据库。完整 V2 目标仍未完成。

生产已接：

- HostProtocol `PermissionTransportInterrupted { reason }` 非终态，reason 为 Timeout/Disconnected/Unavailable；仅 RequestPermission 接受。Core 将原 effect 标为 `PermissionInterrupted` 并写 checkpoint，原 ask/call/参数/history 保持；现有 rebind 恢复 Dispatched，旧身份不能回应新 delivery。
- Application 沿 nonterminal 原事务保存 checkpoint、回应序列、outbox 和 receipt，未新增写路径；失败不消费等待。真实最终 receipt 故障与冷重放验证批准仅发布一次原调用 ExecuteTool。
- CLI permission handler 区分真实 Failed 与 typed transport interruption；后者发非终态 HostResponse，不调用 SOP wait resolution。恢复后仍可 Custom，再明确批准恢复原步骤。
- broker 超时/事件通道断开返回 typed 中断，所有退出均清理 nonce/waiter；同 Core ask 重派生成新 nonce，旧 nonce 不可批准。进程内 exact ACK 仍仅是 transport 投递确认。
- native stdin、desktop wait、交互输入分别区分 EOF、超时和不可用；明确 Deny 不变。Legacy 工具路径返回 transport error、不缓存拒绝或授权。后台输入中断不 resolve 交互。native 直接 stdin frame 与交互行有界，desktop registry 既有 `.lines()` 读取尚未改为有界，不能扩大验收声明。
- nonterminal pump 返回不再确认消费 task context。CLI 增加直接 HostProtocol 依赖，未恢复 Store/SQLite 生产依赖。

验证全部已收到终态：

| 范围 | 结果 |
| --- | --- |
| Core Host contract | 69/69，包含新中断两项（三种原因、Custom history、冷恢复重绑、明确执行、错误 effect 拒绝） |
| Core recovery | 1/1 |
| Application full | 265/265，0 fail/ignore；`/tmp/nola-permission-interruption-application.log` |
| Application tool batch | 7/7（包含于 full），真实最终 receipt 失败回滚及冷恢复原 ask |
| Runtime driver | 3/3，真实 Application 两次 pump、冷重启、不忙循环 |
| CLI broker | 6/6；`/tmp/nola-permission-interruption-broker-final.log` |
| CLI SOP | 1/1；`/tmp/nola-permission-interruption-sop.log`，中断→同 wait→Custom→后继批准 |
| CLI transport inputs | 去重 9 项通过；`/tmp/nola-permission-transport-{tools,control,interaction}-final.log` |
| Workspace all-targets | exit 0；`/tmp/nola-permission-interruption-workspace-verified.log` |

相关路径 `git diff --check` 通过。所有 agent 和测试句柄已终态。没有本轮原生 UI/真实模型/完整 Host 重启验收。

下一步必须沿实际产品入口推进：

1. `CoreAgentHost::run_turn` 在 `resume_pending_once` pump 后仍可能经 `recovered_turn_requires_interrupt` 取消 nonterminal 原 Turn；新输入不能把可恢复权限等待当 abandoned。`resume_agent_core_pending_with_cancellation` 也只依据一次性 resume 标志，持续进程的 supervisor wake-up/backoff 尚未完整接通。
2. 完整 Host 断线重连、重新绑定 lease 与 UI 等待状态；不能把 runtime-driver fixture 冷恢复等同于这些入口完成。检查 `run_turn` 返回 Ok 的上层是否误判为完成，规范 Run 本轮保持 nonterminal。
3. 后台 task + Core request ID 需要复用 pending transport，并恢复 resolved 未投递回应，避免重派创建重复交互行。
4. 继续纯 Kernel、CLI/Android 所有权和主架构 §20 验收。当前进展是安全保留原调用的事务基础，不是完整恢复目标已达成。


## 最新交接：可重复 Host 恢复与后台交互复用（2026-09-09）

保留所有工作区修改，未 commit、未操作用户库。schema 15 采用现有 RequireCurrent hard-cut；完整架构目标仍在进行中。

- `CoreRunOutcome { turn_status }` 贯通 CLI/Goal/后台调用者；nonterminal 不伪装 completed，不进入 Goal 完成/audit，不调用后台 finish，也不覆盖另一客户端刚提交的权限标记。Web `[DONE]` 保持仅 HTTP 终结语义。
- `resume_agent_core_pending_with_cancellation` 可在同进程多次无输入 pump，并刷新 resolver、执行配置及取消上下文。存在 pending permission 时，新输入返回明确拒绝，原 Run 不被 abandoned-turn Interrupt 取消。显式取消仍走 Application。
- 恢复前正式提交 ResumeHostEffect 事务，重绑 delivery 身份；Core 用新 cursor 重新发布原 Waiting Part 的权限字段（Part revision/input/call/ask 不变），使恢复也生成规范 permission trace。正常相同投影 mutation 的 no-op 行为没有被破坏；共用 finish_transition 避免复制终态提交算法。
- 原 native helper 错把 ask ID 写成 call ID，现已修复。canonical trace 暂存按 ask ID 索引，最多128项，每项32KiB；同批反序消费、旧序列/错误身份拒绝与scope/运行隔离有实际 broker 回归。
- EnsureCoreRequest 事务复用同 task+Core ask 的 pending/resolved transport，保留尚未投递的已决定回应，冲突拒绝；schema15唯一索引防重复。CLI使用返回旧ID，Ensure错误不降为其它UI fallback，旧task grant不覆盖resolved决定。

本轮最关键的真实入口验证：`cli/src/core_agent_runtime_recovery_tests.rs`，完整CoreAgentHost + HTTP provider stub + 临时SQLite进程内Application。首次模型工具调用中断→新输入拒绝→两次cached Host恢复均取得fresh canonical trace→仅清fixture cache并冷open→批准→原规范call工具完成一次。最终1/1通过，`/tmp/nola-permission-resume-host-entry-final.log`。测试起初暴露helper错绑callID及“重派无新trace”真实缺口，已修复后通过；没有缩小为只验证resolver helper。该测试不是Service IPC或真实LLM。

已收到终态：Core69/69（`/tmp/nola-permission-resume-core-final.log`）、workspace all-targets check exit0（`/tmp/nola-permission-resume-workspace-final.log`）、上层分类1+Goal audit4、multiask map2+实际broker7+desktop5、Ensure相关App15（部分包含于full）。Application最后Core重呈现修改后的全量266/266通过，0失败/忽略（`/tmp/nola-permission-resume-application-final.log`）。全部测试句柄已终态，无待轮询进程；相关路径diff检查通过。

下一步优先真实可达入口：普通CLI `--resume` 目前只进入composer，Web/Mac只有只读trace replay，没有无新输入pump端点；需要绑定固定session/run/authority到本轮恢复函数，并接持续supervisor wake-up/backoff。不得把重新提交旧prompt当恢复。现有后台非permission pending需要继续核对独立暂停/租约语义，完整Host重启/lease、五端UI/真实模型及旧Core→Kernel/客户端所有权仍未验收。

## 最新交接：用户恢复入口、冻结配置与规范终态（2026-09-09）

本节覆盖上一节“只有只读 replay”的状态。保留全部工作区修改，未 commit、未重建用户库。
本轮为实际进展；完整 V2 目标保持进行中。

- 普通 CLI `--resume` 在 composer 前无输入恢复原会话；pending 时不接受另一条输入。
  Web/mobile 新增 `/api[/mobile]/chat/agent/v2/resume`，严格只接受会话、预期规范 Run、
  Host 和 client 四个身份字段；既有执行返回 409，不替换或取消原 Run。
- 热 Host 继续使用原执行配置；冷 Host 在 Provider probe 前核对持久化
  HostInvocationBinding。绑定在实际 BeginTurn 前配置更新时保留；新轮缓存签名覆盖
  路由、fallback、生成参数和 Device 广告。凭据、query、cache key 仅存比较摘要。
  Device 广告或 resolver 缺失、配置/凭据漂移均明确拒绝，原 checkpoint/outbox 不变。
- 启动维护沿原事务保护 checkpoint/outbox 关联一致的权限等待和对应 trace，包含
  Dispatched、Accepted、中断状态及 SOP 参数上下文；坏身份/输入/history 等事实不保护。
- 恢复成功后发布原 Run 的规范会话完成投影。真实 handler 测试曾发现缺此帧，
  修复生产发布后通过，未删除断言；`[DONE]` 仍仅为 HTTP 结束。
- Mac 中断后提供恢复按钮，发送独立请求；实际执行身份通过既有 Service `active_run`
  只读查询获取并复核。原生测试发现 queued Run 不在窗口 running 活动中，现不再用
  窗口活动代替执行身份，也不从 trace 猜 Run。恢复消费按 conversation+规范 turn ID
  拒绝旧 Run 终态；同 ask 新 nonce 的完整 challenge 防止迟到旧 HTTP 成功/失败清卡。

最终验证均已收到终态：

| 范围 | 最终结果与日志 |
| --- | --- |
| CLI 实际恢复与绑定 | 3/3，`/tmp/nola-resume-entry-final.log`；完整 Host+HTTP provider stub+临时 Application，错误 Run、warm defaults 漂移、普通 composer 前恢复、冷 A/B 地址/凭据拒绝且无 B 请求/无持久化改变、原工具只完成一次；另两项覆盖绑定/Device 契约 |
| 实际成功 handler | 1/1，`/tmp/nola-resume-handler-final.log`；初始真实 broker timeout→新 nonce/更高 trace→明确批准 exact replay→原文件执行→规范完成投影 |
| 严格接口契约 | 1/1，`/tmp/nola-resume-contract-final.log`；拒绝新输入/模型字段、错误 Host、已有执行，原 cancellation 不变 |
| Application 全量 | 268/268，0 失败/忽略，`/tmp/nola-resume-application-final.log`；启动增强专项包含于全量 |
| Core | Host 69 + turn context 2 = 71/71，`/tmp/nola-resume-core-final.log` |
| Workspace | all-targets check exit 0，`/tmp/nola-resume-workspace-final.log` |
| Mac 原生 | Custom 4 + Instruction 2 + RemoteConsumption 9 = 15/15，`/tmp/nola-resume-mac-verified-final.log`；实际 Service queued Run query→恢复消费、409、旧 Run 终态、新 nonce 迟到回应 |

中途整合编译错误、handler 缺规范投影、Mac queued 身份失败均由上述最终结果覆盖。
第一次 Mac fixture 失败后的无限等待已改为有界等待；该次 Xcode 已中断终态，最终重跑通过。
本轮相关路径 diff 检查通过，无待轮询测试句柄。

验收边界：HTTP fixture 不是 Service IPC 或真实模型；Mac 是原生用例测试，没有本轮完整
真实 Host/模型 UI 截图。现有冷配置只能在冻结身份匹配时恢复，原 Web 自选配置的版本化
解析、凭据解析、Device 重新绑定仍需接通；明确拒绝不等于这些恢复能力已完成。
持续 Supervisor wake-up/backoff、完整 lease/重启、旧 Core→纯 Kernel、CLI/Android
所有权和主架构 §20 全量验收继续进行。下一步按当前架构文档继续实迁决策，不能以更多
局部 adapter 测试代替职责迁移。

## 最新交接：压缩规划和生命周期实迁 Kernel（2026-09-09）

在上一阶段恢复入口终态验证后，继续完成一段实际职责迁移，所有原工作区修改保留。

- `core/src/agent/compaction_planner.rs` 的完整轮次选择、持久指令保留、call/result 配对、
  原序分区和摘要预算迁入 Kernel。Domain 仅传无正文 facts，Core 按索引取原条目，
  原算法副本及不可达裁剪分支已删除。token 求和使用饱和运算覆盖极端预算值。
- `runtime/compaction_driver.rs` 的请求准入、自动每 Turn 两次上限、手动目标和空 archive
  后续动作由 Kernel 决定。`CompactionReason` 唯一类型归 Domain，Core 旧路径 re-export。
- `context_window.rs` 的自动触发、同 archive 三次失败、retry 重置和无进展阻断迁入
  Kernel；Core 仅采事实和应用 typed transition，provider usage 会计和 checkpoint wire 不变。

验证均已终态：Kernel 压缩 7/7（`/tmp/nola-compaction-kernel-root.log`），Core Host 69 +
压缩提示/重试 checkpoint/摘要预算 7 = 76/76（`/tmp/nola-compaction-core-final.log`），
子任务另通过 Core planner/lifecycle 专项。新真实 Application 压缩 receipt/冷重放 1/1
（`/tmp/nola-compaction-kernel-receipt.log`），最终 Application 全量 269/269、0失败/忽略
（`/tmp/nola-compaction-application-final.log`），workspace all-targets check exit0
（`/tmp/nola-compaction-workspace-final.log`）。相关 diff 检查通过。

新增 Application 测试覆盖空/超预算摘要和非法 usage 不消费等待、最终 receipt 故障回滚、
合法摘要提交后保持原 suffix/transcript、冷 receipt 重放不再次修改 checkpoint 或 pending
effects。初版 fixture 没有可压缩历史、以及冷开库会删已ACK outbox 的断言问题均已诊断；
最后严格区分 drop、startup prune 和 receipt replay，未用宽松比较掩盖副作用。

完整边界见 [压缩 Kernel 迁移](V2_COMPACTION_KERNEL_CUTOVER_2026-09-09.md)。
仍待迁移：上下文配置归一化、估算/usage 会计、摘要校验/指纹与提示契约、完整 turn/model
状态转换及事件/Effect 装配。Application/runtime-driver 仍依赖旧 Core，客户端所有权、
持续 Supervisor/配置解析/Device恢复及 §20 全量验收继续进行。
此轮是已验证进展，不是完整 V2 完成；无未完成写入或待轮询测试。

## 最新交接：上下文会计与模型响应策略实迁（2026-09-09）

上一轮为已验证进展，本轮继续实际职责迁移；保留全部工作区修改，未操作用户数据库。

ContextWindowConfig 的归一化、默认预算/阈值和 provider 窗口更新决策已迁 Kernel；Core
保留原 public/serde 结构并只映射 Domain facts。UsageOrigin、ContextUsageSnapshot 唯一归
Domain；当前估算/测量、累计 numeric ledger、normalized total 和投影决策迁 Kernel，
provider metadata 原语义保留在 Core。估算求和改饱和，常规值行为不变。

摘要空白/Unicode token budget 校验和 model_planner 的 Goal 呈现、final synthesis、文本
暂缓三个策略也已迁 Kernel。旧算法副本删除，不增加任何按用户文字路由。新真实 Host 回归
证明 Skill 完成步骤未结束时，过早 final answer 不会结束 Turn，协议工具仍可继续使用。

新 accounting 回归执行真实两次 ModelCompleted、两次冷恢复及 RebaseConversation，验证
当前上下文不等于累计 token，也不被旧 provider input 或累计缓存量覆盖。类型迁出后原测试
的跨 crate inherent impl 编译错误已修为直接核对 state 与 snapshot，断言未减弱。

最终全部终态：

- Kernel 全量65/65，`/tmp/nola-context-policy-kernel.log`。
- Core Host70 + retry4 + summary2 + 新accounting1 =77/77，`/tmp/nola-context-policy-core-final.log`。
- 子任务 Core model_planner11、配置checkpoint1、accounting1专项通过（accounting包含于上行）。
- Application全量269/269，0失败/忽略，`/tmp/nola-context-policy-application-final.log`。
- workspace all-targets check exit0，`/tmp/nola-context-policy-workspace-final.log`。

相关路径diff检查通过，无待轮询进程。没有本阶段新增的UI/真实模型验收。
详细实际调用与边界见 [上下文策略迁移](V2_CONTEXT_POLICY_KERNEL_CUTOVER_2026-09-09.md)。
下一步仍需迁移完整turn/model状态转换、AgentItem估算和provider metadata、工具曝光及
事件/Effect装配；Application/runtime-driver旧Core依赖、CLI/Android所有权、持续Supervisor、
冷配置/Device恢复和§20全量验收未完成。完整目标保持进行中。

## 最新交接：模型完成、Turn 决策及架构复核（2026-09-09）

核对当前工作区后确认 `ModelCompleted` 主分支与重试计数、Turn 输入/完成/取消选择、
AgentItem token 估算已实迁 Kernel，状态枚举唯一归 Domain。Core 仍承担完整状态所有权
与事件/Effect 装配；Application 和 runtime-driver 的旧 Core 生产依赖尚未解除。
本次格式化新增决策模块，补齐架构记录，并修正主文档中“正文引用发布尚未接通”等旧结论。

接续时已有最终日志：Kernel 74/74、Application 270/270、Core Host 70/70，均零失败/忽略；
本次另执行 workspace all-targets check，退出码 0。测试范围、日志及真实调用见
[模型与 Turn 迁移](V2_MODEL_TURN_KERNEL_CUTOVER_2026-09-09.md)。没有本次新增的 UI/真实模型验证。

下一优先项已定位到具体代码：Application receipt 回归中的后继 Core Turn 仍归属原 Store
Run，而 Mac 无输入恢复用 `trace.turnID == expectedRunID` 过滤事件。应先补真实后继 Turn
权限中断/冷恢复回归，明确 Run/Turn 关联，随后修复客户端和恢复终态投影；不要只放宽过滤。
此前首次 Turn 的恢复通过不能充当这一场景的验收。

持续 Supervisor、完整 Kernel、CLI/Android 所有权及 §20 仍未完成。保留全部工作区修改，
没有 commit，没有操作用户数据库；本次检查进程已结束。

## 最新交接：后继 Turn 权限恢复身份与提交边界（2026-09-09）

上一轮的架构梳理落实为实际修复，现有工作区修改全部保留，未提交 commit 或操作用户库。
三条并行工作分别覆盖 Application/Store 冷恢复、真实 HTTP Host 回归、Mac 恢复消费；
root 接通服务端身份、整合验证和共享测试资源。

### 已接通的生产行为

- resume SSE 顶层 `canonical_run_id` 绑定锁内校验过的 Store Run；权限 broker 新 challenge
  同样携带它。诊断 trace run ID 和当前 Core Turn ID 保持原值，最终 submission 仍属于原 Run。
- Mac 恢复不再把 Core Turn 当 Store Run；中间 Turn 终态不结束恢复。仅匹配原 Run/submission
  的完整会话投影，且本地 Application 提交成功，才清理权限等待和恢复卡片。成功提交后立即
  结束流消费，后续 HTTP 错误不能把已完成 Run 留在无法再次恢复的状态。
- 普通 Application execute 的 typed PermissionTransportInterrupted 立即持久化 checkpoint，
  非终态语义保留。修复前未到 stream byte budget 的 interruption 只在 hot state，冷恢复丢 cursor。
- 启动维护由当前 Store Run 的 checkpoint、pending Effect 和权限事实验证实际 Core Turn。
  历史 Turn completed 事件不能终结后继轮合法权限等待；只有对应真实 Core Turn 的 trace 受保护，
  旧 trace 仍按既有规则中断。这不是完整的非权限 Run/Supervisor 启动恢复迁移。

### 实证与范围

`successor_turn_permission_resume_identity_e2e.rs` 执行真实 Application 命令：首轮与后继轮、
权限运输中断、冷打开后 cursor=1、维护前后规范记录相等、新 request ID/原 effect 与 call/input、
明确批准 sequence=2 后才产生 ExecuteTool。与既有 tool_batch_receipt 合计10/10通过。
日志 `/tmp/nola-successor-permission-identity-final.log`。

root 最终 CLI entrypoint/冻结绑定3/3、真实 HTTP handler2/2，分别记录于
`/tmp/nola-successor-cli-recovery-verified.log`、`/tmp/nola-successor-handler-verified.log`。
handler 覆盖首次及后继 Turn 冷 Host 恢复，错误 Run 不改 checkpoint/outbox、不产生新 nonce
或 provider 请求；批准原调用只执行一次，最终投影携带原 submission 和实际后继 Core Turn。
严格恢复请求和 permission timeout broker 各1/1通过，日志
`/tmp/nola-successor-resume-contract.log`、`/tmp/nola-successor-permission-broker.log`。

Mac arm64 原生三套最终22/22通过，`/tmp/nola-successor-mac-verified.log`，xcodebuild exit0。
包含后继轮权限、旧/缺 Run 拒绝、中间终态、真实本地投影提交、提交失败保留等待，以及提交后
HTTP502仍正常完成。第一轮在既有协议 fixture 的 Desktop 文件打开上停滞，采样确认位于
`__open`，主动停止后退出75；现把同一共享 fixture 纳入测试 bundle，删除 `#filePath` 读取，
没有复制或缩小 fixture，最终相关9项协议测试也全部通过。

workspace all-targets 最终编译退出0，`/tmp/nola-successor-workspace-verified.log`。
最终 Application 全量271/271、零失败/忽略，`/tmp/nola-successor-application-verified.log`，
退出0；在最后的 Store 历史终态修复后重新执行。相关 diff/项目资源格式检查通过，所有本轮
测试与检查句柄均已终态，没有待轮询或待完成的后台验证。
这些原生/HTTP测试仍不是真实 LLM、完整五端 UI 或整份 V2 验收。

### 后续实际架构缺口

完整 Kernel 的运行态与事件/Effect 所有权、Application/runtime-driver 旧 Core 生产依赖、
CLI/Android 领域所有权、持续 Supervisor/lease/Attempt、冷配置与 Device 解析，以及主架构
§20 的五端、长任务、真实模型、10M UI/性能门槛仍未完成。Store 非权限启动恢复仍按历史
terminal event 推断状态，下一步需按当前规范 Run/Attempt/checkpoint 收敛恢复决策，避免继续
为每种等待类型添加例外。完整目标保持进行中。

## 最新交接：通用当前 Run 恢复规划（2026-09-09）

已完成上轮指定的下一架构切面，保留全部工作区修改，未 commit、未操作用户数据库。
旧 Store 的历史 Turn 终态推断、两次独立 Run/trace 维护和权限专例已删除；不能再引用
`recover_orphaned_canonical_runs` / `interrupt_trace_journal_runs` / `permission_recovery.rs`
作为现行入口。

现由 Application `run_recovery::recover` 在 Store Port `with_run_recovery_plan` 的同一
写事务/receipt savepoint 内验证配置、当前 checkpoint、Run/Turn 和完整 pending 请求事实，
调用纯 Kernel 决策，再原子应用 Run/trace 计划。scope Some 不绕过 live lease；旧 Turn
completed 不影响合法后继 Model 等等待；损坏事实返回错误且全回滚。保留不是派发或授权
重试，当前副作用工具结果未知时仍需独立恢复协议。只有当前 Turn 终态且无 pending Effect
可结算，hot overlay 的移除也延迟到 receipt 提交后。

新真实回归先复现后继 AwaitingModel 被错误结算；最终验证 canonical/pending 不变、真实
Host 结果继续和冷 receipt 重放。首次 Turn 的合法归属由正式 Begin 的 Run ID 证明，后继
必须匹配已提交 TurnStarted；不能要求首次 Begin 不曾写入的事件，也不能把坏事件当作缺失。
旧权限测试已切为正式 Application 入口；损坏事实改按明确拒绝且所有规范/trace行不变验收，
没有保留旧 Store API 来迁就测试。

最终：Application279/279，Kernel77/77，Store恢复5/5，非SQLite用例3/3（包含于App），
CLI入口3/3、HTTP首次/后继恢复2/2，真实Service IPC1/1，workspace all-targets check exit0。
日志、阶段失败原因和准确范围见
[当前 Run 恢复迁移](V2_RUN_RECOVERY_KERNEL_CUTOVER_2026-09-09.md)。
Service fixture全经真实IPC建Run/排NextTurn/接受Host结果/恢复与重放，两次进程重启后
完成原Effect，最终一个StoreRun、两个CoreTurn终态；没有直接seedStore或使用真实LLM。
最后 malformed TurnStarted 字段精化已由Store/非SQLite专项及全目标编译复验。
所有本轮测试和检查进程均已终态，没有待轮询任务。本阶段未重跑平台原生/UI验收。

下一主要缺口：完整Run/Attempt领域与activeAttemptID仍缺，model_request_part_owners不能
替代Attempt；持续Supervisor、租约/心跳/过期/结果未知副作用恢复、版本化配置/Device绑定
待完成。完整Core状态/事件/Effect所有权、CLI/Android领域写入与主文档§20五端、真实模型、
长任务、10M UI/性能门槛仍未完成。不要以这些专项green缩小或结束整体目标。


## 最新交接：Host 提交拒绝与重新采样隔离（2026-09-09）

架构复核确认 Attempt 仍只是 request ID 投影映射，并无独立账本。先修复实际准入缺口：
AgentHostDriver emit 返回 Result，拒绝立即向上传播且不递增序列；Runtime 优先返回
Application 错误。LLM checked 接口区分提交拒绝与 provider 错误，不允许错误分类触发
下一请求。模型 retry/fallback/协议纠正先提交 ModelStreamRestarted，再退休缓冲与换
Part ID；普通 Application execute 也立即持久化此边界。Compaction 同样接入可拒绝握手。

最终 Application279、Runtime Driver3、Host93、checked真实HTTP3、Host→进程内App2
全部通过，workspace all-targets check退出0。新增真实App测试覆盖先发布旧reasoning后
工具流截断，重试后的规范/冷状态无旧内容，以及SQLite拒绝reset后不发第二HTTP。
子任务EPERM测试已由root授权环境实际重跑通过。全部本轮进程已终态。
详细文件、日志与边界见 [Host 提交边界](V2_HOST_ACCEPTANCE_BOUNDARY_2026-09-09.md)。

保留全部原工作区修改，无commit/用户库操作。完整Attempt/activeAttemptID、持续
Supervisor/lease、Core状态所有权、CLI/Android写入及§20验收仍未完成；本阶段不冒充
真实模型或平台UI验收。下一步是在提交握手之上建立正式Attempt准入和持久化生命周期，
明确逻辑模型调用、采样Attempt、投递request/lease与CoreTurn身份关系。


## 最新交接：正式模型 Attempt 账本与采样准入（2026-09-09）

本轮从架构审计继续实际接线，原工作区修改全部保留，无commit、用户库转换/清理。
Domain/Kernel 的 Prepared/Running/终态与CAS计划，已通过 Application Begin及原
CommitTransition 接入Store run_attempts、投递关联和Run.active_attempt_id；旧
model_request_part_owners表及request别名投影移除。每次物理HTTP前checked提交实际
provider target/model，首次激活Prepared，retry/fallback/纠正/已启动后冷重采样创建新
Attempt及新Part，旧Attempt迟到结果拒绝。初始调用不清除同Turn此前逻辑调用的内容。

Service协议13、Store schema16硬切，旧Host ModelStreamRestarted wire删除，Core内部
退休命令保留。恢复校验active Attempt与当前pending关系，悬空active不能被历史终态
结算。Supervisor仅核验身份后推进热cursor，不能以旧Core请求覆盖账本started/ID。
后继CoreTurn的assistant占位在同事务物化；此前首次admission只更新不存在Message导致
QueryReturnedNoRows的问题已经真实回归修复。

最终整合记录见 [模型Attempt账本](V2_MODEL_ATTEMPT_LEDGER_2026-09-09.md)：Application281、
Kernel81、CoreHost70、CLIHost93、RuntimeDriver3，LLM真实HTTP11、Host→App retry3/fallback1、
真实Service两次重启1项通过。Store全量255及后续Attempt4/恢复5专项、非SQLite4也通过。
首轮故障包括旧fixture缺准入、后继placeholder、热overlay以及macOS非阻塞accept socket；
均据实际失败修复，未跳过业务断言。一次旧LLM delta拒绝fixture误拒首SamplingStarted，
导致server.join等待；明确定位后INT结束130，改为接受准入、只拒TextDelta，完整11项已过。

尚待：delegated invocation及内部不可观察采样的分层协议、持续Supervisor/lease/未知
副作用恢复、完整Core/CLI/Android所有权、配置/Device与§20五端、真实模型、长任务及
10M UI/性能验收。本轮HTTP是stub、Service为真实IPC，不代表真实LLM或平台原生UI验收。
完整目标保持进行中，不以专项green缩小范围。

补充准入receipt最终写入回滚测试2/2通过（1项新增，1项与Application全量重叠），
`/tmp/nola-attempt-receipt-final.log`。trigger确认事务内Running已经应用才拒绝receipt，
全规范状态回滚，同requestID重试及冷replay只保留一个revision2的Running Attempt。
最终全目标编译日志`/tmp/nola-attempt-workspace-complete.log`。本轮所有测试与检查进程
均已终态，没有待轮询工作；下轮直接从当前真实工作区继续未完成架构范围。


## 最新交接：Delegated 提交 ACK 与严格恢复（2026-09-09）

已先复核当前架构并保留全部原工作区修改，无commit、用户库操作。实际新接线见
[Delegated 提交边界](V2_DELEGATED_COMMIT_BOUNDARY_2026-09-09.md)。worker逐事件等待
Application持久化成功的ACK，首次Accepted先提交再spawn；拒绝会回收worker及进程树，
不伪造取消或完成。新持久化WorkerUnavailable状态保留冷请求身份而不自动重发原输入；
Codex/OpenCode严格恢复旧session失败不再创建新session。权限保留原request/call身份及
完整参数，Core拥有conversation grant，Host只批准provider当前请求。

关键真实回归发现：仅把worker send改ACK还不够，Application原普通热路径会先确认
未落库的小事件。最终把delegated事件全部纳入强制提交边界，真实inbox拒绝才正确阻止
执行。fixture消费stdin、正文发送真实stream delta；无放宽OS父子进程退出断言。
Store schema17、Service协议14硬切，共享fixtures已更新，未转换用户库。

Delegated专项7/7（Host事务1、Host→进程内App3、实际provider协议3）与CLIHost93、
CoreHost70通过，workspace all-targets check退出0。日志分别为
`/tmp/nola-delegated-durable-verified.log`、`/tmp/nola-delegated-host-regression.log`、
`/tmp/nola-delegated-core-regression.log`、`/tmp/nola-delegated-workspace-final.log`。
这不覆盖delegated receipt最终写入故障、真实LLM或平台UI。

下一主要架构切面仍是独立DelegatedInvocation账本、typed投影owner、permission control
投递证据；Accepted不是invocation账本，外部隐藏采样也不能冒充model Attempt。
持续Supervisor/lease、结果未知副作用恢复、Kernel完整状态与CLI/Android领域所有权及
§20的五端、长任务、真实模型、10M性能验收均未完成。完整V2不因本切面green而结束。

最终强制逐事件持久化后重新验证：Application283、HostProtocol11、Protocol27全部通过，
`/tmp/nola-delegated-durable-application.log`退出0。所有本轮测试与检查进程均已终态。


## 最新交接：独立 DelegatedInvocation 与投影归属（2026-09-09）

已按架构继续实际接线并保留全部工作区修改，无commit或用户数据库操作。
Domain/Kernel现在拥有外部invocation UUID、生命周期与CAS计划；Application在原
Begin/Commit事务中准备、接受、结算，Store同事务ledger+retained delivery+Part+
checkpoint+event+receipt。Host显式回传独立ID，拒绝缺失/混用Attempt，Core首次绑定后
不允许替换。投影owner明确区分ModelAttempt/DelegatedInvocation，跨namespace即使
字符串相同也不能串流；Protocol/Service/Mac/FFI均已接通，Store18/Service15硬切。

真实测试推动两处生产修复：SessionStarted后的冷配置绑定必须由canonical ledger证明，
冻结Run配置不改；取消意图只能写Cancelling，实际host.cancel ACK提交后才Cancelled。
已退休原目标事件在ACK前后都拒绝且不改变规范状态。冷恢复保留显式取消target关联，
按pending投递索引读账本事实。空Completed fixture会触发合法续写，现用真实最终回答。

详细范围、文件与日志见[独立Invocation账本](V2_DELEGATED_INVOCATION_LEDGER_2026-09-09.md)。
Kernel89、Protocol28、Store全量261及后续invocation7、非SQLite5、SQLite/receipt4、
真实CLI delegated8、App投影5、FFI4与Mac实际Swift Testing11通过（有重叠，不累计冒充
独立测试数量）。Mac旧#filePath Desktop读取导致实际测试阻塞，定位后改bundle资源，
最终确有11项执行；CLI两个protocol fixture依赖ambient endpoint已改显式隔离配置。

不透明外部agent内部模型采样仍不可观察；permission control明确target invocation及
决定投递证据、持续Supervisor/lease、未知副作用恢复、完整Kernel/Core/CLI/Android
所有权、配置Device和§20五端/真实LLM/长任务/10M性能门槛仍未完成。完整目标保持进行中。

最终取消状态修复后重新执行：Application全量285/285通过，
`/tmp/nola-invocation-application-verified.log`退出0；
全workspace all-targets check退出0，`/tmp/nola-invocation-workspace-verified.log`。
本轮测试与编译均已终态；后续权限控制审计只读，不改变这些验证的代码状态。

下一切面只读审计已定位：ResolveDelegatedPermission尚未绑定target invocation，Host忽略
backend_session_id，控制入队即声称permission_resolution_delivered，而实际backend写入更晚。
需独立PermissionDelivery的claim/transport证据与结果未知状态，不能只交换send/Completed顺序。
设计与真实回归场景见[权限控制投递审计](V2_DELEGATED_PERMISSION_CONTROL_AUDIT_2026-09-09.md)。

## 最新交接：Delegated 权限投递账本与外部写入边界（2026-09-09）

已按审计继续接线并保留全部工作区修改，无commit、用户数据库转换或清理。Domain/Kernel新增
独立PermissionDelivery CAS生命周期；Application与Store在原Begin/Commit/Recovery事务中写入
occurrence、授权intent、Claim及TransportWritten证据。控制请求完整绑定invocation、session、
generation、occurrence、RPC、call、原input及digest。worker必须等待Claim持久化ACK后才能返回
决定；OpenCode/Codex实际write/flush后再提交Written。Claim后中断成为OutcomeUnknown，冷恢复
不自动重发。Store19、Service16硬切，共享fixture同步。

真实多occurrence测试揭示并修复Core把任意target effect当取消的错误：权限控制不再令父
invocation进入Cancelling，只有typed Cancel/CancelTask可以启动取消。UI请求ID改用occurrence，
原backend RPC单独保存，Custom轮换不改变目标。错误invocation/session/generation/occurrence/
call/RPC/digest均在外部写入前拒绝；旧泛化Completed送达声明已拒绝。

已验证Domain/Kernel5、Store全量267、Host protocol12、Core Host71、CLI delegated11、
Application权限SQLite专项和恢复端口7。真实OpenCode/Codex进程回归证明Claim提交拒绝前零决定
字节，Written提交拒绝发生在flush之后且进程树被回收；这仍不能回滚或证明backend外部执行。
完整文件、契约和能力限制见
[权限投递账本](V2_DELEGATED_PERMISSION_DELIVERY_2026-09-09.md)。最终Application全量和workspace
all-targets均退出0；workspace日志为`/tmp/nola-permission-workspace-final.log`。首次workspace
编译发现mobile bindings测试仓库漏传`delegated_permission_mutations`，补齐端口字段后复验通过。
Application完整执行包含105项lib及全部integration target，新权限专项4/4。

外部backend尚无幂等delivery token或可查询ACK，因此不得声称exactly-once；OutcomeUnknown
仍需后续处置。持续Supervisor/lease、一般未知副作用、完整Core/CLI/Android所有权、Device配置
和§20五端/真实模型/长任务/10M UI性能验收仍未完成。完整目标保持进行中。

## 下一轮施工基线：通用 Effect Delivery Attempt（未完成接线）

只读Supervisor复核确认当前`RuntimeSupervisor`仍是热Core缓存，Runtime Driver按调用读取全部pending
Effect，两个实例可同时执行同一请求。已先落Domain/Kernel纯`EffectDeliveryAttempt`状态机和
StorePort/Store schema20基础：独立Attempt UUID/序号、worker generation/owner/token、lease、
execution-start、settlement、Interrupted/OutcomeUnknown，以及run-scoped有界候选、活跃部分唯一索引、
原子claim/CAS和recovery facts。Domain14、Kernel99与workspace all-targets check通过，最终编译日志
`/tmp/nola-effect-delivery-baseline-check.log`。

本基线尚未接Application claim命令、Runtime Driver、Host envelope/ExecutionStarted ACK或真实并发
与崩溃测试，不能视为Supervisor或普通Effect exactly-once完成。曾开始的半套Host DTO已撤回，避免
留下可绕过或全仓不可编译状态。下一轮应先完成Application/Host fencing和双SQLite连接竞争测试，
再实现持续wake/backoff循环；不要复用Host catalog lease，也不要原地替换过期Attempt owner。

## 最新交接：通用 Effect Delivery 领取、执行屏障与恢复（2026-09-09）

上一节“未完成接线”已完成到生产派发边界。保留全部原工作区修改，无commit、clean、reset或用户
数据库操作。Domain/Kernel/Store schema20 的 `EffectDeliveryAttempt` 已由 Application claim、
Host wire、Runtime Driver和CLI Host实际消费；Application Service协议硬切为17。

Application只领取已经提交并能与canonical checkpoint逐项核对的Effect。初始pending创建Attempt，
Interrupted Retry/Resume创建新successor；批量claim使用同一Store事务。Runtime Driver生产pump不再
直接加载全部pending执行。Host必须先发送`ExecutionStarted`并等待持久化ACK，才能跨越外部副作用
边界；后续每个响应精确回传Attempt/generation/owner/token/expiry。过期或伪造响应、旧worker和未
start业务响应均拒绝。外部执行前的合法拒绝使用`ExecutionRejected`，不会启动adapter或伪造started。

恢复已经并入Application Run recovery UoW：过期Leased与已启动Idempotent进入Retry；已启动
Recoverable必须有checkpoint规范cursor才进入Resume；RequiresRecoveryDecision进入OutcomeUnknown。
Retry/Resume由下一generation创建新Attempt，OutcomeUnknown不自动重发。双SQLite连接竞争、原子
rollback、两Driver竞争、冷Resume/Retry、pre-start拒绝及CLI真实进程提交屏障都有专项证据。

typed transport/worker interruption也会在同一响应事务关闭当前Attempt。Retry允许同request自动候选；
Resume必须等显式`ResumeHostEffect`物化fresh request ID，避免Driver在pump尾部提前占租后丢弃。
显式恢复保持effect/turn/payload并创建新request、新Attempt和新token。

最终Kernel102/102、Store275/275、Runtime Driver5/5、Application301/301，全部0失败/0忽略；
`cargo check --workspace --all-targets`退出0。日志分别为
`/tmp/nola-effect-delivery-store-final.log`、`/tmp/nola-application-full.log`和
`/tmp/nola-effect-delivery-workspace-final.log`。相关CLI屏障/恢复/delegated进程专项也通过。
完整实现说明见[通用 Effect Delivery](V2_EFFECT_DELIVERY_LEASE_2026-09-09.md)。

下一步应接Runtime/Application lease续期与常驻Supervisor wake/backoff/lifecycle，并给
OutcomeUnknown增加基于外部证据的人工处置命令。当前固定300秒租约没有renew接线，长时间静默执行
会导致结果被过期fence拒绝；调用方触发pump也不是持续Supervisor。RuntimeTask旧lease/upsert、完整
Core所有权、CLI/Android领域写入、配置/Device绑定与主文档§20五端、真实模型、长任务和10M UI性能
验收仍未完成。不得把单一活跃claim或本地账本表述为外部副作用exactly-once。

续租的下一实施方案已完成只读审计，见
[Effect Delivery租约续期方案](V2_EFFECT_DELIVERY_RENEWAL_PLAN_2026-09-09.md)。不能只增加依赖Host
输出的heartbeat response，因为同步dispatch可能在HTTP首字节、工具或子进程阶段完全静默。采用
Runtime主线程保有Application port、scoped Host worker通过response+ACK channel回报的结构；主线程
用独立typed Application command做CAS续租，成功后才更新本地lease，失败时关闭准入并取消外部执行。
下一轮从Kernel最大扩展约束和Application heartbeat command开始，随后必须接Runtime静默定时器与
真实HTTP/子进程故障测试，不能停在已有但未调用的Kernel heartbeat函数。

## 最新交接：Effect Delivery 执行期续租（2026-09-09）

上一段续租方案的核心链路已完成。Domain/Kernel将claim与heartbeat单次扩展限制为300秒，严格增长、
过期不可复活并安全处理整数边界。Store schema20新增typed单Attempt heartbeat CAS方法，不改表；
record/物化expiry/revision/Run event同事务，两SQLite竞争、event故障、terminal/recovery竞态已覆盖。

Application新增`HeartbeatEffectDelivery`写命令，核对active Attempt、完整lease、当前expiry、pending
outbox、checkpoint和replay safety，并进入统一receipt UoW；Service protocol硬切18。Runtime把同步
Host dispatch放到scoped worker，主线程通过response+ACK channel串行处理Application响应和timer续租；
仅start ACK后注册，terminal ACK后注销，Host响应使用最新已ACK expiry。拒绝续租会关闭准入并调用
cooperative abort。CLI已接Service heartbeat port和活跃Effect cancellation registry。

最终Application303/303、Store278/278、Protocol28/28、Runtime Driver9/9，均0失败/0忽略；真实
Service IPC heartbeat 7项suite、CLI abort专项和workspace all-targets check通过。日志为
`/tmp/nola-effect-renewal-application.log`、`/tmp/nola-effect-renewal-store.log`、
`/tmp/nola-effect-renewal-protocol.log`、`/tmp/nola-effect-renewal-runtime.log`、
`/tmp/nola-effect-renewal-workspace.log`。

后续补齐真实Host验收：CLI真实HTTP provider在收到请求后延迟headers 170ms，100ms TTL/25ms cadence
期间持续续租，terminal使用最新ACK lease；真实runtime tool handler启动独立`/bin/sh` process group，
拒绝第20次续租后目标进程树退出、完成marker不存在、无关Effect未取消，且没有terminal或伪Failed进入
Application。Runtime取消接口与准入错误改为按Effect隔离；双Effect回归证明一个续租失败时另一个仍能
续租并结算。delegated session在suspended/drain期间保留统一cancellation注册，终态后RAII清理。

最终Application303/303、Store278/278、Protocol28/28、Runtime Driver10/10、Service70/70、CLI
delivery barrier 8/8及workspace all-targets check通过。CLI完整bin基线为975通过、14失败、2忽略；
跳过新增child测试结果完全相同，代表当前工作区既有环境/fixture失败，不归因于本切片。失败样本中
external executor identity单跑通过，builtin skill与10M Web用例单跑仍失败。

尚未完成delegated/permission真实长静默的续租/拒绝故障测试；Runtime仍是调用方触发pump，不是常驻
Supervisor。RuntimeTask transition-specific lease、OutcomeUnknown人工证据处置、完整Core/CLI/Android
所有权及§20五端/真实模型/长任务/10M性能验收继续保留。

## 最新交接：Runtime Effect Supervisor 第一切片（2026-09-09）

上一段“调用方触发pump”的限制已推进到CLI进程级常驻调度。Store schema硬切21并新增显式
conversation scope的跨Run计划；Application Service protocol硬切19，提供由提交通知、租约deadline、
请求deadline或断连驱动的`runtime_effect_work_wait`，空闲时不轮询数据库。Service Client支持主动取消，
并按完整request校验返回scope；30秒服务等待另有1秒传输宽限。

新增独立`nola-runtime-supervisor` crate，负责64会话上限、dirty重建scope、单worker、每会话指数
退避、公平重新纳入、取消、drain和shutdown join，不依赖Store或Application实现。CLI只在持久会话
模式启动，Host注册/淘汰/失效同步更新scope；worker按conversation+Run路由warm Host，复用现有
CommittedRuntimeDriver与Application port，后台展示保持静默并由canonical projection供客户端读取。

全量Store279、Application306、Protocol32、Service76、Service Client25，共721项通过；Supervisor
9/9与CLI专项通过，workspace all-targets check通过。实现与限制见
[Runtime Effect Supervisor 第一切片](V2_RUNTIME_EFFECT_SUPERVISOR_2026-09-09.md)。CLI完整bin仍沿用
此前975通过/14失败/2忽略的工作区基线，不能声称全绿。

本切片仍只调度已注册且可重建Host配置的conversation；五端统一装配、durable Host配置冷解析、
delegated permission长静默、OutcomeUnknown人工证据、RuntimeTask transition-specific迁移、完整
客户端所有权及§20验收仍未完成。完整V2目标继续进行。

## 最新交接：未知 Effect 的人工“确认未执行”处置（2026-09-09）

`OutcomeUnknown` 现在有一个窄而安全的人工恢复入口。Application Service protocol 硬切20，
`AuthorizeUnknownEffectRetry`要求conversation/run/effect/request/attempt、worker generation、owner、
旧lease token、attempt revision、处置时间和1..4096字节证据全部匹配。Kernel只允许
`RequiresRecoveryDecision`的当前`OutcomeUnknown`进入`Interrupted(Retry)`；原未知原因、完成时间和
执行身份仍保留，并记录typed `ConfirmedNotApplied` resolution。

Store schema硬切22。Store在单事务内重读Attempt、执行revision CAS、重新核对当前pending outbox，
并同时写Attempt和带resolution时间的Run审计事件。Application还会核对conversation、checkpoint和
committed pending request；命令走稳定receipt UoW，最终receipt写入失败会整体回滚，完全重放返回同一
receipt。授权本身不ACK outbox、不推进Core；正常claim只产生一个fresh successor，旧lease永久失效。

Kernel专项10、Store专项13、Application专项6、Protocol全量33均通过，另有workspace all-targets check
通过。`applied`、`failed`和`abandon`没有实现：这些结论必须先有Core显式
`EffectOutcomeResolved`（或等价typed submission/event），不能通过伪造普通Host响应来关闭outbox。

## 最新交接：RuntimeTask 泛化整行写入已移除（2026-09-09）

`ReportProgress`原先先读RuntimeTask、在Application内修改整行，再调用Store upsert；并发heartbeat、
取消、状态和lease可能被旧快照覆盖，过期lease也能提交进度。现在StorePort只接受字段限定的
`RuntimeTaskProgressMutation`，Store UPDATE只改变progress summary、last activity、metadata和updated
time，并以running状态、ownership token、精确updated/lease/metadata、未过期lease及未请求取消为CAS
围栏。公开的full-row upsert已删除；原子创建只调用Store私有纯INSERT。

该切片通过Store RuntimeTask 9、Application lib 113、resume snapshot 3及相关all-target编译。下一步是
持久化task attempt identity/revision并先迁移Claim；之后再按Renew、PermissionWait、Suspend、Retry、
Complete逐个替换仅靠task token的旧状态迁移，AppendEvent也必须收窄为有所有权的typed事件入口。

## 最新交接：RuntimeTask Claim / Renew / Progress attempt 围栏（2026-09-10）

Store schema现为23，Application Service protocol现为24。`runtime_task_attempts`为每次成功Claim保存
UUID、task内递增序号、worker/generation、lease token/expiry、状态和revision；task领取、旧active
attempt supersede与fresh attempt插入同事务。Claim/ClaimFair要求显式进程generation并返回完整typed
Attempt，CLI执行上下文和Agent/Shell子进程都保留该身份，同token重用不会产生ABA身份复活。

旧RuntimeTask Heartbeat已删除。RenewLease要求task/attempt/revision/generation/token/current expiry，
拒绝过期、非增长、超过10分钟、stale/foreign/superseded；Store单事务CAS attempt revision/expiry与task
heartbeat/lease。ReportProgress使用相同active-attempt围栏，只改progress允许字段并滚动attempt revision。
CLI Renew和Progress共享同步lease state；PID progress ACK完成后才把最新Attempt序列化给child，避免子进程
必然拿旧revision。terminal/cancel清理使用remove-if-same，旧guard不能删除新attempt。

独立最终验证：Store all-target、Application all-target、Protocol33、Service all-target、Service Client
all-target、Supervisor10及workspace all-target check通过；真实Supervisor→CLI→Runtime→HTTP测试证明无需
第二次用户输入且chat POST恰好一次。测试还修正一处写死schema22的陈旧断言，现引用当前schema常量。

下一RuntimeTask切片应先做PermissionWait/Resume：挂起时原子关闭当前attempt并释放worker ownership，
权限解决后创建fresh successor；随后迁移Retry/Recover、Complete，最后删除客户端可任意kind/status的
AppendEvent并引入有序bounded typed output。Runtime cold Host factory、五端生命周期和§20完整验收仍未完成。

## 最新优先级：先覆盖核心产品能力（2026-09-10）

用户明确要求避免继续专精单一RuntimeTask细节。当前并行主线改为：压缩后原文定位与context overflow
自动压缩恢复、工具调用准确性与SOP、code memory准确性、经验/记忆自动沉淀与预算化召回选择，以及
各平台UI交互、性能和动画流畅度。每条先完成一轮真实缺陷修复和专项验证，再回到剩余架构收口。

Android聊天时间线已先做一项性能修复：删除空闲会话也永久运行的90ms滚动timer，改为由消息、流式正文、
thinking、tool事件和生成状态变化触发的有界尾随窗口；`./gradlew testDebugUnitTest --no-daemon`通过。
RuntimeTask permission新schema/DTO/SQLite骨架保留且workspace可编译，但Application/CLI尚未切生产路径，
不能声称permission suspension完成。工作区当前Store schema为24、Application Service protocol为25。

自动记忆生产接线复核显示，两端并非缺少模型 Worker：Android 的完成 Turn 已加入按会话去重的
WorkManager 作业，iOS 的 BGAppRefresh/BGProcessing 已调用 canonical Memory Worker；两端均通过
Application 的 query/claim/heartbeat/complete/fail 命令运行。此次修复了 Android 重试断点：先前模型
失败虽已提交 Core `retry_waiting`，`MemoryWorker` 仍返回 success，若没有新 Turn 就不会再次领取。
Worker 现在接收包含 completed/retryScheduled 的报告，存在 retry 时返回 WorkManager retry，首次退避
从30秒调整为与 Core `retry_after_ms` 相同的60秒，第三次失败后才终止平台链。后台 Worker 的 Core
配置只包含会话、模型和context window，不复制交互 Turn permission profile或grants。
同时修复 iOS `BGProcessing` 的执行顺序：之前未来的定时 URL 会在 Memory Worker 之前触发早退，
导致冷启动已就绪的记忆作业被跳过；现在先按 Application readiness/lease 处理记忆，再单独判断 URL 到期。

自动沉淀的去重边界也已收紧：此前 `save_memory_record_deduplicated` 虽会复用相同内容的 canonical ID，
但会继续用新候选覆盖整行，导致后续 `agent_inference` 把用户显式事实的 provenance、confidence 和来源
降级。Store 事务现在保留既有 `explicit_user` canonical 事实，自动重复项仅作为幂等确认；Application
真实 `ValidatedAutomatic` Commit 到 Store 的端到端测试覆盖了该路径。

召回的 supersession 也已补齐生产边界：此前字面查询只把文本匹配行交给 Core；如果旧事实匹配查询，
而纠错事实使用不同措辞并通过 `supersedes` 指向旧事实，Core 看不到纠错项并会把旧事实注入模型。
SQLite Memory adapter 现在从最多50个初始命中出发，补齐最多200个同 namespace/scope、未删除且未过期的
后继候选，再由 Core 应用置信度、有效期、supersession 与 token 预算。Application E2E 覆盖了
“legacy”只命中旧事实但最终仅返回不同措辞纠错事实的路径。
补充审阅发现后继链可能跨 kind；Application 现在允许跨 kind 后继参与旧事实抑制，但 typed recall 最终
只返回调用方请求的 kind。新增回归证明 preference 查询不会泄露 superseding decision，也不会复活旧 preference。

## 最新交接：五端三态权限模式统一（2026-09-10）

CLI、Web、iOS、macOS 与 Android 均使用严格 canonical `safe`、`recommended`、`allow_all`，
旧设置缺失时默认 Recommended。Safe 无宽泛 baseline；Recommended 用 ProductDefault Allow 配合
Core 风险分类器在破坏性调用前 Ask；AllowAll 用 live Turn Allow，后台、委托、重放和维护 origin 不继承。
OS 权限、资源 scope 和具体 Host/Core Deny 继续独立并后置优先，不从用户文本或多语言关键词推断模式。

CLI 使用 typed `PermissionProfile`，Host binding 包含 profile；macOS stdin、Android/iOS Web Host
请求严格传 canonical 值，非法值在 Turn 前拒绝。Web、Android DataStore、iOS/macOS conversation
均持久化选择，各设置页已提供相同三档。Android/iOS 本地 Core 通过现有 PermissionRuleSet 表达同一
baseline，未复制 Core 的风险分类器。Rust hydration/profile/Web 设置专项、Android 21 项 JVM、iOS
PermissionMode 3 项与七语种资源检查通过；Android connected tests 无设备未运行，macOS 本轮测试在
worker materialize 阶段停止，不能声明通过。

## 最新交接：单条超大上下文与 iOS Host Effect 生命周期（2026-09-10）

Provider 报 context overflow 且没有可安全归档的历史前缀时，Core 现在可为一个超大原子 item
生成仅供模型使用的有界首尾视图。canonical transcript 原文不改；Compaction checkpoint 记录
source item IDs、fingerprint 和原 token 数。ToolCall/ToolResult 按 call_id 成对投影并保留
full_output_ref 与 artifacts。重试前要求工作集 token 严格下降且不超过 prompt limit，否则拒绝恢复，
避免重复提交同一超限请求。

iOS 直接 Core Host 路径已改为通过 Application 的 claim_committed_host_requests 获取带 delivery
attempt 的请求，使用实例稳定的 worker generation/lease owner 和 Core 允许范围内的四分钟 lease。
所有已支持的 Host effect 在执行或展示权限等待前先提交 execution_started；model/compaction 在物理
采样前继续提交 model_attempt_started，response 同时回传 model attempt、delegated invocation 和
delivery identity。模型重试会生成 fresh attempt identity，并让后续 delta/final response 绑定该身份。

验证包括 Core planner 6 项、超大 overflow 集成 1 项、workspace all-target check，以及 iOS
DeviceAgentCoordinator 两项与 Memory Worker 冷恢复一项 focused XCTest。iOS 前台 Coordinator
每两分钟续租并让后续 response 使用最新 delivery identity；权限等待的跨进程长期挂起/恢复仍需后续
按 Supervisor 生命周期继续收口。

## 最新交接：Web 长流式文本的分句分配与 Reduce Motion（2026-09-10）

Web 的增量 Markdown 已保留稳定 block，但语义分句循环过去每取一段都会执行
`source.slice(queuedUntil)`，单个超大 provider delta 会反复复制完整剩余后缀。现在 tokenizer 接受
source offset，并按 Unicode code point 从共享 source 直接扫描，只为最多一个语义短句建立字符串；
既有分句边界、增量 DOM identity 和最终 Markdown 内容不变。

流式自动跟随原有 requestAnimationFrame 缓动没有读取系统 Reduce Motion。`NolaChatStream` 现在暴露
实时 media-query 结果；每次调度滚动时若用户启用 Reduce Motion，直接提交目标 scrollTop，不建立动画帧。
Markdown reveal 原有 reduced-motion 零 timer 路径保持。Playwright 专项覆盖超长 prefix 的 offset 分句、
运行时动态 Reduce Motion 检测、模板滚动接线与既有零 timer 行为。

## 最新交接：macOS 长历史尾部投影有界化（2026-09-10）

macOS 时间线虽然只渲染最新 20 条，但过去每个流式更新都会先过滤完整内存消息数组，且同一次 SwiftUI
body 求值会经多个 computed property 重复过滤；成本随历史长度线性增长。现在
`MacTranscriptWindowPolicy` 从最新消息反向执行可见性、条数与正文 byte budget 三个判断，收集够窗口后
只多检查一条来确定“加载更早消息”是否显示，然后立即停止。body 每次只建立一个窗口投影，消息顺序、
活动空 assistant 可见性、160KB 初始 byte budget 和 canonical 历史分页均保持原语义。

focused 测试使用 10,000 条可见历史断言默认窗口只检查 21 条，并覆盖活动空 assistant 与 byte budget
截断顺序。测试使用明确 `platform=macOS,arch=arm64` destination 和隔离 DerivedData，不轮询 iOS 设备。

## 最新交接：Android Host Effect admission（2026-09-10）

Android 直接 Core Host 已从无 delivery 的 committed request 读取切换到 Application claim。Runtime 使用
实例稳定的 worker generation/owner 和四分钟 lease，并在 begin、每个 Host response 与 active submission
后领取 successor。Coordinator 对支持的 effect 在实际执行或显示权限等待前提交 `execution_started`；
model/compact 在物理采样前提交 `model_attempt_started`。所有 response 回传 attempt、delegated invocation
和 delivery identity，模型 retry 使用 fresh attempt 与新 Part ID，后续 delta/final 保持该绑定。

Android 前台 Coordinator 已补两分钟 heartbeat：续租通过 Canonical runtime 的 Application 命令互斥执行，
Host journal 校验 Attempt 不变并保存最新 delivery，随后所有 response 使用新 lease；heartbeat 失败会通过
结构化协程取消正在执行的 effect，避免继续以过期 lease 提交结果。

验证：Android 全量 `testDebugUnitTest` 通过，`compileDebugAndroidTestKotlin` 通过；相关 diff check 通过。
本轮没有连接设备执行 instrumentation。iOS 的进程终止后 permission-wait recovery 仍需后续完成。

Android 的本地 permission-wait 已补进程恢复：会话加载发现 canonical active run 处于
`waiting_for_permission` 时，不再开始新 Turn，而是读取 exact run/config digest，等待 effect-work schedule
到达可接管时间，并使用新 generation/owner claim successor delivery。恢复信封要求 request attempt 与
delivery attempt 相同且 lease 字段完整；入口仅允许 `request_permission`，任何 model/tool effect 都
fail closed，避免进程崩溃后重复物理执行。Coordinator 用恢复后的 delivery 重新提交 execution-started、
展示权限并提交最终决定。新增 JVM 测试覆盖完整 fencing 与非权限 replay 拒绝。

## 最新交接：SOP 工具调用的会话所有权（2026-09-10）

Core→CLI Host 的 SOP lineage 校验此前只绑定 run、step、workspace 与运行状态，遗漏了持久化的
`owner_session_id`。因此同一工作区的另一个会话只要携带旧 run/step identity，就可能把工具或权限结果
错误归入该 SOP。`ValidateInvocation` 与 `ValidatePermissionInvocation` 现在都要求 HostRequest 的真实
session ID；Application 对有 owner 的运行执行精确匹配，owner 为空的 workspace 级运行保持兼容。
CLI 在剥离 `_nolaExecution` 业务参数、执行工具或展示权限前传入该身份，跨会话调用 fail closed 为
`invalid_sop_execution_lineage`，且不改变 SOP 状态。实现没有使用提示词或语言关键词路由。

## 最新交接：iOS 流式时间线常量成本跟随信号（2026-09-10）

iOS 时间线过去在每次 SwiftUI body 求值时重新对最新消息的完整正文和思考文本执行字符计数，并扫描当前
工具事件的显示文本来生成自动跟随信号。长回答流式增长时，滚动、动画和其他状态刷新都会重复支付与消息
长度相关的成本。现在 `ChatMessage` 在不可变快照构造时一次性生成 `ChatTimelineFollowMetrics`；时间线只读取
四个整数，保持既有 24 字符量化、显式 current tool 优先和 running/permission fallback 语义不变。

focused XCTest 使用 240,000 字符正文、2,400 字符思考和 10,000 条已完成工具历史加一个当前权限事件验证
投影边界，并复核 Reduce Motion 下的有界跟随窗口。测试在明确可用的 iPhone 17 Pro iOS 26.0 模拟器和
隔离 DerivedData 上通过，两项均成功；长历史指标测试执行耗时 0.021 秒。

## 最新交接：结构化 context overflow 与 Code Memory 工作区身份（2026-09-10）

CLI Host 的模型错误分类现优先解析 Provider 响应内的结构化 JSON error envelope。规范 code/type 可以在
中文、日文等本地化 message 下稳定映射为 `context_length_exceeded`；解析范围限制在错误字段，避免响应中
回显的用户参数恰好包含同名 code 时错误启动压缩。没有结构化错误体的兼容 Provider 保留文本回退。

Code Memory 的 dirty revision 过去只哈希 porcelain 状态，导致同一路径再次编辑时身份不变。现在身份还
流式包含变更与未跟踪文件内容，且 status 观测失败时不再返回 clean revision。专项回归覆盖相同文件两次
内容变化得到不同 revision，以及结构化错误、文本回退和请求回显误判边界。

## 最新交接：Web 压缩摘要原文定位与长 trace 增量成本（2026-09-10）

Web 的 canonical conversation projection 虽已包含 compaction Part，过去 reducer 没有向 UI 暴露它，
历史分页后也没有恢复新页的 canonical Parts，因此 `source_item_ids` 和 `source_fingerprint` 无法形成用户可用
入口。现在摘要消息显示“查看原始上下文”卡片、来源数量、摘要和 fingerprint 前缀；projection 同时建立
canonical message ID 与 part ID 到真实 message ID 的索引。点击后若目标尚未进入当前 48 条窗口，会按游标
逐页载入、恢复 canonical projection 并滚到原消息；每页要求 before 游标前进，避免异常响应造成无限循环。
系统 Reduce Motion 开启时定位使用即时滚动，否则使用平滑滚动。

Web Core Part 增量此前每次更新都在变更前后 `JSON.stringify` 完整累计 trace 两次，长输出和多工具历史下
会随已积累 Parts 线性放大。已接受且通过 cursor/去重检查的 canonical Part 必然改变公共 projection，现直接
递增版本，取消两次全量序列化。专项测试覆盖 compaction 字段保真、message/part 双重定位索引、2,000 Parts
后单次 delta 的零全量序列化，以及跨页恢复和 Reduce Motion 接线。相关两文件完整回归 44 项通过；一次
Playwright 页面初始化超时在单项复跑通过，专项 4 项再次通过。

主线复核后又移除了消息 API 的完整 projection 旁路：路由现在直接返回同一次 Application window read 的
bounded projection。浏览器不再要求预先拥有全量 source index，而是逐页合并身份索引、每页重新匹配目标；
因此 10M transcript 的原文定位和普通分页都不会先载入完整 canonical transcript。

## 最新交接：macOS 压缩摘要来源身份与当前窗口定位（2026-09-10）

macOS 的 canonical `CompactionPart` 过去把 `retained_item_ids` 当作摘要来源，既没有保留 V2 的
`source_item_ids` / `source_fingerprint`，也没有把来源身份投影到可交互 UI。现在解码优先读取规范字段，
仅为旧日志保留 `retained_item_ids` 兼容；Application projection 将摘要、source item IDs 与 fingerprint
原样写入 assistant presentation，并为每条消息保存 canonical message/part identities。摘要卡显示来源数量
与 fingerprint 前缀，但定位只使用 canonical ID 集合，绝不从摘要文字或正文猜测来源。

点击“Show original context”会在当前有界 transcript window 中精确匹配 message ID 或 part ID 并滚动到
原消息；来源尚未载入时明确提示需要加载更早历史，避免伪定位。系统 Reduce Motion 开启时使用即时滚动，
否则使用短时 ease-out 动画。两项 focused macOS XCTest 覆盖协议字段到 presentation 的端到端身份保真，
以及只按 canonical identity 定位、拒绝可见文字碰撞；使用明确 `platform=macOS,arch=arm64` destination，
结果 2/2 通过。

## 最新交接：Android 长任务投影的流式重组成本（2026-09-10）

Android `ChatTranscript` 过去在每条可见消息重组时都会重新物化完整 Skill Run 与 Task 投影，并分别
线性扫描 owner turn。长任务历史与流式正文同时增长时，相同 projection 会在每次正文 delta 中被重复
扫描。现在 Compose 按权威 projection identity 缓存 owner-turn 索引，消息仅做常量时间查找；历史 Host
tool 与 answer source metadata 的解码也按不可变 message metadata 缓存。消息归属仍只使用 Core 保存的
turn identity，不从正文或显示文本推断。`compileDebugKotlin` 已通过。

## 最新交接：Memory supersession 所有权边界（2026-09-10）

共享 Core 的最终记忆选择此前把所有候选的 `supersedes` 目标合并为全局 ID 集合。一个 ContextPlan 若同时
允许 Personal 与 Workspace namespace，Workspace 记录就可能仅通过引用 Personal 记录 ID 将其从模型上下文
抑制；不同 scope 也存在相同问题。SQLite 字面召回按单一 namespace/scope 补齐后继链，因而没有暴露这个
跨 Host、FFI 和多 namespace ContextPlan 均可触发的 Core 契约缺口。

`select_memories` 现在只承认 `(namespace, scope, record_id)` 完全一致的 supersession。跨 namespace 或
scope 的声明仍可作为自身所属域的候选参与预算选择，但不再删除另一所有者的事实；同域用户纠错与自动合并
继续正常抑制旧记录。规则只读取 typed memory ownership 和显式关系，不读取用户语言、关键词或工具名。
`cargo test -p nola_core --test memory_selection_contract` 的 10 项契约测试覆盖跨 namespace、跨 scope、同域
纠错以及 C ABI 与原生选择结果一致。

## 最新交接：自动沉淀同批幂等键（2026-09-10）

Memory Worker 输出此前只要求 record ID 唯一；相同 `(namespace, scope, kind, content)` 的事实若携带不同 ID，
Core 会同时接受。原子 checkpoint writer 为保持 Core checkpoint 身份不能在写入后按内容合并，因此这种输出
会永久产生重复记忆。Core 现在在 extraction 与 consolidation 的 job output 验证阶段按 Store 相同的规范键
拒绝同批重复，job 不会提前进入 Completed，checkpoint 与任何候选行也不会发布。规则完全基于 typed record，
不读取用户文本关键词。`memory_runtime_contract` 6/6 通过；其中旧成功夹具同步改为当前权威的
`AgentInference` 与 `nola://session/<id>` provenance，没有降低来源校验。

## 最新交接：Code Memory submodule revision 身份（2026-09-10）

Code Memory 的 dirty revision 已覆盖普通修改与未跟踪文件内容，但目录型 gitlink 过去统一写入常量
`non-file`。父仓库对脏 submodule 通常持续只报告相同的 `M vendor/module`，因此子仓库 HEAD 移动或同一路径
再次编辑时 revision 可能不变，旧 retained index 会被错误视为当前快照。CLI Host 现在遇到该目录时读取
嵌套 Git revision；该 revision 同时包含子仓库 HEAD、porcelain 与脏文件内容。非 Git 普通目录仍保守使用
`non-file`，符号链接保持独立目标身份规则。专项测试证明子仓库未提交修改和随后 commit 都会改变父工作区的
dirty revision；整个过程只观察版本控制事实，不从提示词或工具名称路由。

## 最新交接：Memory sensitivity 最终选择门禁（2026-09-10）

`MemoryRecord.sensitivity` 过去只被持久化和回传，Core `select_memories` 没有执行文档声明的 sensitivity
策略；带标签记录只要 namespace、置信度与预算通过，就会进入模型上下文。`MemoryPolicy` 现在包含精确
`allowed_sensitivities`，缺省空集合拒绝所有带标签记录，无标签事实保持原行为；空白 sensitivity 本身作为
非法 record 拒绝。选择只比较 typed metadata，不根据内容、查询、语言或 kind 猜测授权。

Core selection 11/11 与 Application Memory Port 9/9 通过。Application E2E 证明敏感事实仍可由显式 Get
管理，但默认 Recall 只返回普通事实；Core 测试证明仅完全匹配的 label 可进入选择。Apple 的非 UI Core
结果枚举已接受 `sensitivity_not_allowed`，避免严格 Codable 因新拒绝原因丢弃整个计划。更高层若要开放
某类敏感记忆，必须先取得真实产品授权再显式填入 allowlist，不能由客户端提示词路由。

## 最新交接：macOS 压缩摘要跨页原文定位（2026-09-10）

macOS 摘要来源入口现在从当前 canonical window 开始，按 Application 返回的 `before_ordinal` 逐页向前
读取，直到某页的 canonical message ID 或 part ID 与 `source_item_ids` 精确相交。搜索期间不替换屏幕上的
页面；仅在命中后一次性提交目标页，并扩大该页的可见消息窗口后滚到原消息。匹配逻辑不读取摘要或正文。

分页状态机要求游标严格向前且不得重复，并分别返回 history end、stalled cursor、read error 与 cancellation；
前三种在 UI 显示对应提示，失败时保留用户原来的页面。系统 Reduce Motion 开启时命中使用即时滚动，否则
使用短时 ease-out。focused macOS XCTest 覆盖跨两页命中及 end/stall/read-error 边界，在明确
`platform=macOS,arch=arm64` destination 上 2/2 通过。

## 最新交接：CLI 权限执行与模型上下文统一（2026-09-10）

CLI 的 Core 权限决策已经使用 canonical `safe`、`recommended`、`allow_all`，但 Host 注入模型的运行环境
仍从遗留 `auto_approve` 和环境变量单独生成 `auto_approve/configured/recommended`。运行期显式 profile
可能因此与模型看到的说明不一致。现在环境说明和 Core rule set 共用同一个 typed effective-profile
解析，并且只显示三种 canonical wire value；显式配置优先，缺省继续是 Recommended。聚焦测试同时断言
三种 profile 的实际策略和模型上下文一致，AllowAll 仍不传播到 scheduled origin。

## 最新交接：Memory usage 回执的 Session 所有权（2026-09-10）

Core 的自动上下文 usage 只从 Context Engine 最终 `used_ids` 生成，因此被预算裁剪或来源门禁拒绝的
memory 不会更新 `usage_count`/`last_used_at_epoch_ms`；失败的 Core transition 也不会提交中间状态。审计同时
发现公开的 `MemoryRuntime::RecordUsage` 回执此前只校验字段格式，没有核对回执所属 Session。一个错误复用
其他会话回执的 Host 因而可以污染当前 checkpoint 中同 ID memory 的排序反馈。

Agent runtime 现在要求 `entry.session_id` 与当前 canonical Session 完全相同，跨 Session 回执在进入
`MemoryRuntimeProjection` 前 fail closed，不占用 operation id，也不改变 ledger、stats 或 revision。聚焦
契约测试还用同一 operation id 提交修正后的本 Session 回执，证明拒绝不会毒化幂等重试，合法 usage 仍只
计数一次。门禁只比较 typed owner identity，不读取 memory 内容、用户语言或工具名称。

## 最新交接：Android Memory sensitivity 元数据保真（2026-09-10）

Android canonical Memory Store 已保存并返回 `record.sensitivity`，但旧的手写 `StructuredMemoryRecord` 没有
该字段，decode 时会丢弃；resident profile 与 `memory_search` 随后都将记录重建为 `sensitivity = null`。
这使敏感事实绕过 Core 默认空 allowlist 的拒绝规则，作为普通无标签事实进入最终选择。

Android 的 Store commit/decode、领域模型以及两个生产 recall 映射现在完整保留 sensitivity。其 Core ABI
policy 模型也显式序列化 `allowed_sensitivities`，默认仍为空且 fail closed，只有 Host 已取得授权并传入精确
typed label 时才允许。JVM wire 测试覆盖 policy snake_case allowlist、敏感记录持久化 round-trip 和旧数据
缺字段仍解析为无标签；共享 Core selection 契约继续验证空 allowlist 拒绝及精确 label 授权。所有判断只读取
typed metadata/policy，不读取提示词或内容关键词。

## 最新交接：自动记忆 usage 与模型可见输出绑定（2026-09-10）

`memory_search` Host 会为最终选择附带 typed usage metadata，但 Core 过去在生成有界模型工具结果前先取出
metadata；当结果超过 capability 的 `preview_char_limit` 并被裁成 `output_preview` 时，隐藏在尾部、实际没有
进入模型窗口的 memory 仍会更新 `usage_count`/`last_used`，进而错误影响后续 usage-ranked recall。

Core 现在把 usage 提交绑定到最终 model-visible tool envelope。完整结果继续记录逐条 usage；当 envelope 标为
`truncated: true` 且尚无逐条可见 identity 证明时，usage fail closed，不把完整 Host 候选集当成模型已使用。
Core→Host 集成测试以很小的 output budget 驱动真实 tool call/completion/后继模型请求，确认模型只收到有界
结果且 ledger/stats 保持空；单元契约同时覆盖完整输出仍保留 typed usage。规则只读取结构化裁剪状态和 usage
identity，不匹配用户文本。

## 最新交接：iOS/Android Provider overflow 协议一致性（2026-09-10）

iOS 与 Android OpenAI-compatible Host 现在只从 `error` / `errors` 顶层结构化对象的直接 `code` 或 `type`
识别上下文溢出，并遍历数组中的全部错误项。四种 provider 别名统一提交为
`context_length_exceeded`；自然语言正文、多语言 follow-up 和 request echo 均不能触发恢复。iOS 通过 typed
`providerFailure` 将 code 与 `retryable=false` 交给 Core，Android 使用相同错误码和重试语义，因此 Host 不会
先重复一次必然失败的请求，压缩和重放继续由 Core 以原 effect/submission identity 协调。iOS focused XCTest
2/2、Android provider contract test 通过。

## 最新交接：Web 会话代次隔离与原文定位交互（2026-09-10）

Web 压缩来源的跨页定位和普通历史分页现在同时捕获 conversation ID 与 `conversationLoadVersion`。任一 await
返回后，只有两者仍完全匹配才允许修改消息窗口、canonical projection、DOM 或滚动位置；同一会话 reload
产生的旧请求也会静默失效。来源按钮在查找期间暴露 disabled/aria-busy 状态，命中后聚焦真实 canonical
消息；滚动继续遵循 Reduce Motion。Web runtime 完整回归 23 项通过。

## 最新交接：CLI 长流式输出增量边界（2026-09-10）

CLI TUI 不再为每个 assistant delta 从头扫描并分配完整 newline 集合。`StreamBoundaryTracker` 只扫描新增
bytes，维护有界的换行位置，并在 durable prefix drain 后修正剩余边界；长单段输出从平方级扫描降为线性。
段落、完整行与最终提交语义保持不变。50,000 个单字节 delta 的扫描上界与跨 delta `\n\n` 边界测试均通过，
terminal UI focused tests 19/19 通过。

## 最新交接：Android 自动记忆生产 Turn 接线（2026-09-10）

Core 只在 `memory_policy.long_term_enabled` 与 `automatic_extraction_enabled` 同时开启时，才会在完成 Turn 后
创建 extraction job。Android 已有完成流后的 WorkManager 调度、独立模型 Worker、lease/heartbeat/retry 和
Application 原子提交，但普通生产 `DeviceAgentCoordinator` 构造的 Core config 没有 `memory_policy`，一直使用
Core 默认的 `automatic_extraction_enabled = false`；因此调度器通常只会唤醒一个空队列，自动沉淀并未真正
从对话完成路径启动。

Android 现在用同一个 typed policy builder 配置前台 Turn 和冷启动 Memory Worker：长期记忆与自动 extraction
开启，search limit/confidence 固定，sensitivity allowlist 默认空且 fail closed。相同 policy 同时避免 worker
恢复 checkpoint 时产生配置漂移。Android JVM 测试验证生产共享 policy 的 extraction 开关和 sensitivity 默认
门禁；Core 集成测试驱动真实 Turn 完成，确认生成一个带当前 Session source identity 的 queued extraction job。
调度与选择完全由 typed config、Turn 状态和 provenance 驱动，不读取用户文本关键词。

## 最新交接：Memory consolidation namespace 所有权（2026-09-10）

CLI 的自动 consolidation 会读取 canonical Store 中所有 global-scope memory；Core 过去只按 scope 和全局 ID
supersession 筛选，Personal、Workspace 与 Custom namespace 因而可能进入同一个 Worker job。更严重的是
consolidation materializer 把输出 namespace 固定为 Personal，所以 Workspace 候选可能被改写成 Personal 事实，
同时跨 owner supersede 原记录。

Core 现在按 `(namespace, global scope)` 分区候选，每个 job 只冻结一个 owner domain；跨 namespace 的同名
supersedes 不再删除另一域候选。Worker plan 再次验证所有 frozen records 属于同一 namespace，materializer 从
这些 typed records 继承输出 namespace，不接受混合 owner 输入。规划契约覆盖 Personal/Workspace 混合输入、
跨 namespace supersession 与单 job 单 owner；Worker 测试证明 Workspace consolidation 输出仍属于 Workspace。
专项测试共 7/7 通过，并保留 10M 字符候选选择的既有分配上界。

## 最新交接：Web Custom 权限等待身份与键盘交互（2026-09-10）

Web Custom 说明过去只用 `request_id` 判断 permission card 是否仍对应当前等待。如果 Host successor 复用该 ID
但更新 nonce、session 或 call，旧点击 handler 或迟到 HTTP 回包可能更新 successor 卡片。现在卡片绑定
protocol、nonce、request、call、Host、Client、session 与 tool 的完整 wait identity；任何字段变化都会使旧
handler/回包静默失效，提交仍只发送 typed `provide_instruction` 并恢复原 Tool future。

说明输入支持 Ctrl/Command+Enter，状态通过 `role=status` 与 `aria-live=polite` 播报。focused Node contract
覆盖同 request ID 的跨 nonce/session/call successor、迟到回包、键盘提交与原参数不变并通过。

真实 Web E2E 随后暴露了 Host 的 pending trace 仍为 thread-local：Core trace producer 与同步 permission
resolver 跨线程时，resolver 看不到 canonical ask，直接返回 `permission_transport_interrupted`。现改为有界、
按 conversation ID 分桶的进程级 registry，取出时严格验证 ask/tool/span/call；SSE envelope 分别携带原始
request ID 与 call ID。Web 以 call ID 复用 canonical tool row，待响应卡强制可见并保持 disclosure 展开，
终态只统计 Core `archivedActivityPartIds` 指定的 surface 行。真实 destructive permission E2E 已覆盖展示、
批准、恢复同一 Core Turn、批准后执行和终态无残留 waiting/running。

## 最新交接：Android/Web 权限模式读取迁移（2026-09-10）

三端审计确认 macOS、Android、Web 的新写入、运行时映射和缺省值均为 canonical
`safe/recommended/allow_all`，默认 Recommended；macOS 已兼容历史拼写并在启动 Server 时收窄非法值。
本轮修复 Android 与 Web 的读取侧迁移缺口：历史 `moderate` 映射 Recommended，`allowAll` / `allow-all`
映射 Allow All，未知/空值收窄到 Recommended。Android 后续持久化和 Web 设置提交仍只写 canonical wire value。
审计的 Mac 长会话与运行状态动画主要路径已有 Reduce Motion 或基于 Core surface 的有界刷新，本轮未再扩大动画改动。

## 最新交接：Memory age policy 的 provenance 完整性（2026-09-10）

Core 的 `maximum_age_days` 过去只拒绝可解析且确实过旧的 `observed_at`。记录缺少 observation time 时直接
放行，未来时间戳产生负 age 也被视为新鲜；Apple resident recall 和 memory search 配置了 365 天上限，因而
不可验证来源仍可能进入模型。现在启用 age policy 后，缺失、非法或未来 `observed_at` 统一以
`invalid_timestamp` 拒绝；未配置 age policy 的兼容行为不变。共享 Core 契约测试覆盖两类绕过与无策略路径。

## 最新交接：Apple Memory sensitivity 召回门禁（2026-09-10）

iOS canonical Memory Store 已持久化 `sensitivity`，但兼容 `ChatMemoryRecord` 投影过去丢弃该字段；resident
profile 与 `memory_search` 随后把记录构造成 `sensitivity=nil`，使 Core 默认空 allowlist 无法拒绝原本带标签的
敏感记忆。现在 sensitivity 在 canonical Store、profile、Core selection input 间双向保留，两条模型可见召回
路径均传递 typed 标签。iOS/macOS 的手写 `MemoryPolicy` ABI wrapper 同步加入 `allowed_sensitivities`；缺省及
旧 payload 缺字段均解码为空列表，保持 fail closed。focused 测试覆盖 canonical/profile 往返、旧 policy 解码、
wire 编码和真实 Core selection 返回 `sensitivity_not_allowed`。

## 最新交接：iOS 自动记忆跨会话冷恢复（2026-09-10）

iOS BGAppRefresh/BGProcessing 会扫描持久化会话并执行 canonical Memory Worker，但过去把
`maximumJobCount` 同时用于 `sessionIds.prefix(...)`。当前两个会话没有 ready job 时，第三个及后续会话的
claimable job 会在每次后台唤醒中被跳过；没有新 Turn 时该作业可无限期饥饿。现在调度器扫描全部有效且去重的
conversation owner，只有实际完成的作业才消耗模型 job budget；每个会话收到剩余预算，达到上限立即停止。
返回负数或超过剩余预算的 Host 结果会 fail closed。异步契约测试覆盖前两个空队列而第三个可领取，以及预算耗尽
后不再扫描后续会话。

## 最新交接：Code Memory stale 结果隔离（2026-09-10）

Codebase Memory Host 已在模型可见结果边界独立核验 repo/path/symbol、revision、内容 digest 与 source evidence，
但过去只把 freshness provenance 追加到 provider payload；即使判定为 `stale`，旧 snippet 和查询结果仍会原样
进入模型。现在 stale payload 会在外部工具输出裁剪前替换为 typed quarantine envelope，只保留刷新提示、stale
原因和可审计 provenance。当前且可验证的结果仍保留原 payload，`requires_verification` 的既有显式标记语义不变。
契约测试覆盖 digest 漂移、缺失路径和 source 冲突不会泄漏旧事实，并验证 current 结果仍可见。

## 最新交接：typed 恢复、Memory freshness 与权限说明（2026-09-10）

iOS OpenAI-compatible Provider 现在与 Android 一样扫描同一响应中的全部 `error` / `errors` envelope，
并逐个检查错误对象的直接 `code`、`type`、`error_code`。较早的网关诊断或同对象中的普通错误码不再遮蔽
后续 context overflow；对象、数组等 malformed typed field 只会被忽略。自然语言 message 与 request echo
仍不能触发恢复，真实流式和非流式入口共用同一 envelope 选择函数。

CLI Host 删除了对未曝光 tool call 的私有英文纠错与重采样。Provider 原始 call ID、capability name 和
arguments 现在原样进入 Core；Core 按冻结 catalog 返回 `tool_not_exposed_in_model_step`，不会请求权限或执行
未知工具，并把 typed recovery 交给后继模型 Step。多语言参数及 `search` / `current` 等普通文字不参与路由。

Memory age policy 启用时缺失、非法或未来 `observed_at` 均 fail closed 为 `invalid_timestamp`；无 age policy
时保留原兼容行为。Web 权限设置同时补充三态动态说明和辅助技术关联，明确 AllowAll 只作用于受系统安全
边界约束的交互式 Turn，后台与定时任务不继承。

本批专项验证：iOS Provider XCTest 13/13、Core Memory selection 12/12、CLI unknown-call 1/1、Core typed
unknown-tool recovery 1/1、Web 设置页面 6/6 与 Node 权限契约通过。完整 V2、真实模型多轮和 §20 五端
UI/性能验收仍未完成。

## 最新交接：Code Memory provenance 数量边界（2026-09-10）

Codebase Memory Host 为控制文件读取成本最多校验 64 条 code fact，但过去到达上限后静默停止并继续暴露完整
provider payload。只要前 64 条可验证，第 65 条及以后即使指向删除文件、workspace 外路径或冲突 digest，也会
随整体 `current` 结果进入模型。现在 collector 明确报告 fact overflow；结果 freshness 收窄为
`requires_verification`，加入 `provider_result_exceeds_provenance_fact_limit` 原因，并在 model-visible 边界隔离
整个 payload。契约测试用 64 条有效事实和第 65 条未验证 stale fact，确认后缀内容不会泄漏且 provenance 保留
有界的 64 条审计记录。

## 最新交接：上下文、Goal 与实际能力历史边界（2026-09-11）

本轮按用户确认的持久化原则继续收窄历史状态：会话终态只保留实际调用过的 Tool 描述；Skill catalog、SOP
目录和 Host 动态发现结果不进入历史 checkpoint。实际启用或执行的 Skill 仍由 `invoked_skills` 与 run 记录保留
引用、版本和内容摘要，实际 SOP 只保留 durable run。终态同时清除 `host_catalog:*` 幂等标记，使冷恢复后的下一
Turn 可以从当前 Host 重新发现目录。活动 Turn 仍保留完整冻结 catalog 和 schema source，以维持 typed activation、
权限等待及冷恢复的一致性；常用直接 Tool 仍保留初始可见性，其他 Tool 由 typed discovery 激活。历史裁剪只发生在
无 pending effect 的终态 checkpoint，不能提前改变当前 Turn 的能力协议。

压缩后的 checkpoint 改为带 checkpoint ID、source fingerprint、原 token 数和有界首尾 source item ID 的
user-role 不可信数据，不再作为 system 指令。`conversation_recall` 支持用 canonical message ID、part ID 和 byte
range 精确读取原文；压缩提示要求保持 canonical user-role 归属。Provider overflow 恢复补齐嵌套 typed error、
Anthropic 风格 message 和数值 413，但仍要求 typed error envelope，普通用户文本不能触发恢复。

Goal continuation、audit 与交互式 Goal Turn 会从 Core checkpoint 重新投影有界、完整且可校验 SHA 的 Goal JSON；
该投影同样是 user-role 数据，冷启动和上下文压缩不再让对话摘要成为 Goal 真相来源。Agent task wait 现在分别
处理取消和超时，只有 runtime 终态、Core 终态和 completion watermark 一致时才确认 terminal delivery。

三端 UI 初步修复了恢复期重复 permission challenge：macOS 保持正在提交的 permission card，Android 返回同一
Compose state，避免重复决策和动画。Web 普通 Markdown 完成时直接稳定现有 DOM，仅 Mermaid 需要最终重解析，
消除完成边界的第二次全量重绘。CLI 的语义流继续用较大的持久化批次，但可读短句在 100ms 边界后立即显示，
明确无 Tool 的最终合成会在完整句末落帧，不等待额外 provider callback。

权限展示协议的 replay safety 修正为 Recoverable：UI transport 中断没有执行目标 Tool，可以在同一 Core wait 上
用新 request identity 重新展示；physical delivery 同时轮换 generation 与 owner，满足 successor attempt 的隔离
约束。真实 Host/Web resume 测试证明多次中断不会额外调用模型，最终批准只执行原始 Tool 一次。

真实 Provider 验证使用 OpenRouter 上实际可用的 `deepseek/deepseek-v4.1-flash`，生产 CLI 的 queued follow-up
场景通过：第一 Turn 执行真实 bash Tool 后完成，排队输入随后生成第二条 assistant 消息，canonical 数据库中的
user/assistant 顺序与最终 run status 均正确。真实 `analyze-data` Skill 链也通过。E2E wrapper 现按显式环境、通用
模型环境、Nola V2 Store 的顺序解析配置且不输出密钥；定向模式按目标场景与最终 complete 记录审计，不再误要求
未运行的全套场景。
