> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Committed assistant journal 与分块传输设计

状态（2026-10-06）：**Domain、Store、Application producer 已实现，主要生产提交入口
已接通同事务日志写入；固定窗口 record、精确版本快照和正文范围读取已接入
Application 命令，journal 客户端尚未接通**。Admission、Host response、
active submission、idle checkpoint 和显式完成共同调用 changed-owner producer；不通过
最新消息猜测历史 Task/Skill 的归属。准入、TransitionCommit 和 idle acceptance 现在
返回实际 changed-owner 的 `assistant_publications` 游标；buffered 未提交 delta 返回
空集合。Service 协议为 33，回执保留原提交游标，不将其解释为最新状态或执行权。
返回的 publication cursor 尚未贯通客户端通知，
因此这不是新显示链路已经替代旧 snapshot 的声明。

本次真实 Application 入口故障验收：在 `assistant_journal_records` 插入时注入失败，
冷重开确认正文、checkpoint 及全部 `assistant_journal_*` 表原样回滚，解除故障后
同一 Host delta 成功提交。测试为
`canonical_stream_transition_e2e::publication_failure_rolls_back_real_host_delta_and_cold_retry_publishes_once`。
日志单测 58 项、任务指导 9 项、流式/绑定 10 项及 idle checkpoint 2 项通过；
这些是定向验收，不代表当前全 workspace 或全客户端通过。
Store 日志专项现已重跑 49/49 通过，日志
`/tmp/nola-journal-store-production-20261006.log`；覆盖 schema 38、idle source、
精确历史 owner、多组发布、回滚、冷重开及有界读取。旧 schema 32 迁移断言已
纳入随后新增的任务表/约束；历史 handoff 的只读凭据仍不能授权旧身份写入。

以下是此前阶段验收记录：新完整publication-set、
有界多帧规划及canonical owner借用映射已通过六crate lib（1,119项）和完整
Application/Store/Domain tests（1,037项，含重叠lib）。CLI完整重跑已通过
1,408项、0失败、2忽略；目录混版本整改及未完成的Skill会话所有权见总览8.6。
schema33私有Text的2/8/32MiB显式规模验收已通过，仍不是公开传输验收。10-01富字段语义批次源码
已进一步落地，Rust六crate lib、Application完整tests、Android完整JVM及Mac/iOS
各七类原生build/test通过；CLI完整binary及短stream deadline/指定交互PTY已重验。
累计进度与准确证据见
[整体改造第8节](../../../历史任务/实施与审计/MODULAR_REDESIGN_2026-09-28.md)。本文不是现有snapshot链路已具备
线性性能的声明。

## 1. 已确认的问题与约束

当前生产链路已有独立正文日志写入，但客户端仍采用旧累计投影：

- `cli/src/session.rs::capture_committed_assistant_projection` 在有 JSON 显示订阅时，以约 50 ms 限频查询累计 assistant。提交成功与可选显示读取错误已隔离；限频只降低常数，不能消除累计复制。
- `core/nola-store/src/remote_assistant_store.rs::load_committed_assistant` 在同一 SQLite read snapshot 获取 assistant、conversation revision 和 Run `next_event_sequence - 1`。正文预算为 8 MiB，且 `presentation_input_store.rs` 的合成展示输入另有 2 MiB 预算。
- `commit_remote_assistant_projection` 每次删除该 assistant 全部 Parts、重新插入、再发布同步 replacement。发送、接收和同步均可能累计 O(n²)。
- `cli/src/web_parts/native_bridge.rs::committed_assistant_replay_event` 重放的是最新累计 snapshot。Mac `MacRemoteRunRecovery.afterTraceSequence` 只属于 Trace；它不是正文连续游标。
- `core_agent_runtime.rs` 的终态发送仍依赖完整 conversation projection；读取失败可能丢弃待发终态。增大预算不能解决大正文的恢复与完成状态交付。

现有 Run ownership、authenticated source Host、canonical terminal 和 Core→Application 提交边界保留。**禁止从 Trace、未提交 Core Part delta、模型 token 或 UI 已显示文本反推 canonical 正文。**

## 2. 权威数据模型

增加独立的 per-Run/assistant **可见内容日志**，不使用 conversation revision 作为连续序号，也不消费 `pending_sync_changes` 充当正文订阅。

建议共享 Domain 类型：

```text
AssistantJournalIdentity {
  source_store_epoch, conversation_id, run_id, assistant_message_id
}
AssistantJournalRecordV1 {
  identity, sequence, previous_record_digest, record_digest,
  operation_id, source_commit_revision, operations[]
}
AssistantProjectionOperationV1 =
  DeclarePart { part_id, ordinal, typed_metadata, generation }
  | AppendText { part_id, generation, expected_byte_length,
                 expected_content_root, chunks[], resulting_content_root }
  | ReplacePart { part_id, expected_generation, generation,
                  typed_metadata, content_manifest }
  | UpdatePartMetadata { part_id, expected_metadata_revision,
                         metadata_revision, typed_metadata }
  | RemovePart { part_id, expected_generation }
```

`sequence` 只在该 assistant 发布实际可见变化时递增，从 1 开始；一次源提交可包含多个assistant及多个有界record/batch，整体在同一事务发布。无关会话重命名、其他 assistant 或 Trace 事件不会造成正文序列空洞。`source_commit_revision` 仅供溯源，不是 CAS 连续游标。

operation ID 由 source commit identity 与稳定 operation ordinal 决定；重试不重新分配。相同 sequence/operation ID 不同 digest 是冲突，不能用“最后写入获胜”掩盖。

Part 的 `order_key`（section、position）在 Declare 后不可修改；Replace 只替换
内容和元数据，不得借换代重新排序。共享 Domain 校验同时约束生产写入和远端接收，
包含同一 batch 中 Declare→Replace 的情况。需要移动的产品行为必须另行定义
明确的排序合同，不能隐含在 Replace 中。Store schema 41 的持久目录按
`section, position, part_id` 索引读取，part_id 只作同位置的稳定决胜键和续读标识。
Declare 在同一事务登记位置，Remove 不删除目录位置；读取仍按固定快照查询每个
Part 的版本，不会把新版本混入旧页。未知续读标识拒绝，不猜位置。旧 schema
升级按提交记录逐帧重建，包括同帧 Declare→Remove 的 tombstone，不从当前 head
推断历史位置。Service protocol 40 区分新的分页顺序合同。该接口已接通生产
写入和接收，正式 UI 的事件/读取入口切换仍未完成，不能据此宣称 UI 已增量化。

`source_store_epoch` 防止备份恢复或重建数据库后复用旧序列。传输 envelope 另含 authenticated `source_host_id`；接收端必须同时绑定 Host、epoch、conversation、Run、assistant。Host 不能来自客户端任意字段。内容 digest 不包含可被 relay 改写的 Host 字符串；relay 必须通过已认证连接把 durable source epoch 绑定到正确 Host，不能沿用当前 `local` 占位值作为认证事实。

epoch不能自动识别用户在文件系统外部替换数据库。受支持的备份恢复/重建入口必须
明确轮换epoch并通知订阅重建；绕过入口的旧备份由sequence/digest冲突拒绝续传，
不能宣称仅存一个epoch字段便能透明检测一切恢复。record digest使用共享版本化
canonical编码，固定字段顺序、整数/字符串/空值与UTF-8语义；不得各客户端用默认
JSON对象序列化后自行hash。跨语言golden fixtures是切换前置门槛。

JSON外壳中的64位序号和revision使用规范十进制字符串，而不是JSON number，避免
Web/JavaScript在超过2^53时丢精度。拒绝符号、前导零、小数和超范围值；canonical
digest编码仍使用固定宽度u64大端字节。Store采用SQLite有符号整数时必须显式处理
其可表示范围，不得截断或把溢出变成旧cursor。

### 可见性不是原始 PartMutation

Domain 现有 `PartMutation::AppendText`、ownership 和 typed Part/Message 语义可以复用，但不能直接把原始 mutation 序列化给用户：review 未批准候选、推理展示策略、Task/Skill/ModelAttempt 合成状态仍须经共享 Application 可见性投影。

提取共享 `CommittedAssistantProjectionPlanner`：输入该提交的 canonical 变化与相关旧可见 Part 状态，输出上述可见操作；首次批准候选必须成为显式 reveal/replace，而非把历史隐藏 draft 当 append base。Task/Skill/ModelAttempt 合成的可见节点也要有稳定 owner、Part ID、类型和 metadata revision，不能在客户端各自重建。

Planner 不能在每个 token 上加载和比较整篇 assistant。其读取粒度应是受影响的 Part metadata、content root 和必要的局部内容。

## 3. 正文存储必须分段

仅增加网络 delta 而继续 `content_text = content_text || delta`、重读全文计算 SHA-256，仍不是端到端线性。

建议增加：

- `assistant_journal_heads`：identity、last sequence、last record digest、visible manifest root。
- `assistant_journal_records`：不可变 operation batch 与提交来源，按 identity/sequence 唯一。
- `assistant_part_generations`：Part generation、metadata revision、字节数、content root。
- `assistant_content_chunks`：不可变 chunk digest、UTF-8 bytes、长度；Part 内容由有序 chunk refs 组成。
- `assistant_snapshot_manifests` / manifest pages：已固定水位的有界恢复入口。
- 接收端的 `remote_assistant_journal_heads` 与 staging manifest/chunks；与本地会话 revision 分开。

正文 append 使用固定上限 chunk，UTF-8 边界明确；允许缓冲尚未提交的尾块，但不能改写已发布块。不要为合并尾块反复复制整个前缀。Replace 创建新 generation，旧 generation 由读引用/GC 管理；Remove 是显式 tombstone，不靠 snapshot 缺项推断。

content root 使用有版本的 chunk Merkle tree 或可验证的 append hash chain，增量更新只依赖旧 root 与新块；它不是假称“全文 SHA-256”的累加值。需要全文 SHA-256 的文件导出可以线性扫描一次，不可每次 append 扫描全文。

所有读取 API 提供 byte range/chunk pages；导出、搜索、Markdown 渲染窗口和同步消费不得隐式重新组装整篇正文作为每次刷新成本。旧整段 read API 若暂存适配器，必须明确仅供有界窗口/导出，不再接入持续发布热路径。

可见快照读取与日志操作重放读取必须分开：同一record内连续Append/Replace可能使中间
content root不再出现在最终Part head里，但接收端仍要按顺序验证这些操作。
`ReadAssistantJournalRecordChunk`只接受有效快照句柄所固定的精确record，校验已存header/batch、
owner/sequence/digest及该record确实引入的Part/content引用，再返回有界原始块。
普通快照读取继续只允许最终可见引用；不能因为摘要存在就放宽权限。将来的GC保留规则必须
覆盖仍可重放record及有效句柄涉及的中间引用，不能只追踪最终Part heads。

## 4. 源端提交与发布

同一 Application receipt UoW 中完成：

1. 验证 Run/assistant/Part ownership、旧 checkpoint 与 content root。
2. 提交 canonical 业务状态和可见 projection operations。
3. 写入 chunks、journal records 与 head；更新必要的同步 journal 引用。
4. 提交 command receipt。

任意一步失败全部回滚。`commit.applied=false`、重复 receipt 和热缓冲未提交 delta 不产生 journal record。

`CommitAssistantPublicationSet`封存一个实际Admission/Transition来源下的完整有序
owner/frame集合；source key独立于set digest，不能靠删掉帧或更换digest获取新发布槽。
初始cursor、owner顺序、frame顺序、canonical batch及验证后的supplied chunk顺序/数量/
长度/digest均进入集合凭证。各owner有独立连续sequence，但不允许分别提交半组。
整组savepoint失败移除本组所有公开head/record/version/chunk与集合凭证；调用前的
content preparations仍由外层receipt事务回滚，不能把内部savepoint当作整个用例事务。

该凭证证明的是调用者提交集合的不可变性，**不证明第一次提交已经覆盖所有Core可见
变化**。生产coordinator必须从accepted typed changes建立权威changed-owner集合；
Task/Skill/Attempt、idle/recovery来源证明及全部真实提交点完成前，不启用消费者。

参考依据（2026-10-02核对）：OpenAI
[session events](https://developers.openai.com/api/docs/guides/agents-api/sessions/events)
区分实时事件和已保存item，并以item身份衔接；
[background mode](https://developers.openai.com/api/docs/guides/background)
提供其自身的stream sequence续传。Nola据此明确事件身份、正文游标和持久水位的边界，
但不将provider序号或连接关闭视作本地commit证明。OpenCode本地
`sync/index.ts`的projector及条件事件记录在Database transaction内执行，bus effect在
提交后触发；临时PartDelta总线不是durable正文权威，也不是Nola全提交点的事务证明。

提交返回轻量 watermark/record references，不返回累计 assistant。通知只表示“该 identity 已有新水位”，消费者按独立 cursor 分页读取；通知丢失不丢正文。读取/发送失败不反向推翻已提交 Run，不取消模型执行。

`ReadAssistantJournal { identity, after_sequence, max_records, max_bytes }` 返回固定 read snapshot 内连续 records、`next_cursor` 与 `has_more`。每个批次有硬字节预算，不能只限制记录数量。journal GC 与网络订阅 ACK 独立于普通设备同步 ACK；落后到保留区间之外返回 typed `SnapshotRequired`，不能悄悄从最新状态续上。

## 5. 初次同步、断线恢复与 snapshot

首次连接或 cursor 已过期：

1. `OpenAssistantSnapshot(identity)` 固定 `{snapshot_id, at_sequence, record_digest, manifest_root}`。这是不可变逻辑快照，不要求跨多个 HTTP 请求持有 SQLite 长事务。
2. 分页读取 manifest，按 digest 拉取有界 chunks；支持逐页续传和去重。
3. 接收端 staging 验证完整 manifest、Part ownership、字节长度、digest 后，原子激活该 assistant 的 manifest/head。
4. 从 `at_sequence` 之后读取日志；快照制作期间的新提交不会丢失或重复。

大快照的 manifest 本身也必须分页；不能把成千上万 chunk refs 塞入一个无限 JSON 数组。内容可以按 Part/viewport 分段呈现，但“完整快照已激活”的 head 不能早于其一致性验证。

增量恢复请求带正文 `after_sequence + record_digest`，Trace replay 保留自己的 cursor。不得用 Trace sequence、conversation revision、接收时间或 Part 顺序猜缺失正文。当前 `expected_run_id` 的精确绑定继续使用。

## 6. 接收事务与所有权

每个 record/batch 在目标 Store 同一 write UoW 验证：

- authenticated Host、source epoch 与已 open 的 remote Run/assistant 完全一致；
- `sequence == current + 1` 且 previous record digest 一致；重复同 digest 幂等，gap 返回所需 cursor，不接受跳跃覆盖；
- append generation、expected byte length/content root、所有 chunk digest 正确；
- metadata/replace/remove 的旧版本匹配，不能抢占其他 assistant 的 Part ID；
- 先 staging/验证所有块，再一次提交 Parts/head/local revision；失败不能留下半条正文或推进 cursor。

目标本地 revision 继续表示本地可观察变更；source sequence/digest 不得写成同一个字段。转发同步只发布操作/manifest refs，不再每次 enqueue 全 assistant replacement。来自其他设备的同步不能重复制造新的 source journal sequence。

全局chunk digest去重仅是物理存储优化，不是授权。写入新manifest ref时也要校验
来源：该块必须由此次调用提供并验证完整bytes，或已有同一被授权owner/source下的
可达证明。远端不能仅凭猜中的digest将其他assistant私有块挂入自己的manifest，再
借自己的snapshot证明读取。必须分别测试引入ref和读取chunk的跨owner拒绝，而非
只测试read API有没有检查当前manifest。

## 7. 终态与大正文解耦

新增小型 `RunTerminalReceiptV2`：

```text
{ authenticated source identity, conversation_id, run_id, assistant_message_id,
  outcome, completed_at, final_assistant_sequence, final_record_digest,
  final_manifest_root }
```

该 receipt 由 source canonical terminal commit 同事务产生，不从 Trace finished 合成，也不需要读取整篇 conversation presentation。最终正文的 append/replace/close 必须位于 `final_assistant_sequence` 以内。

接收端可先持久化终态证据及 final watermark，正文尚缺时显示“执行已结束，结果同步中”，不得假装任务仍在模型执行，也不得把未完整同步的正文当完整答案。用于完整转录投影的完成边界要求满足 final watermark；执行状态与内容同步状态必须是两个 typed 字段。

终态先到、重复、断线后再查询均可恢复。缺块或 2 MiB 展示输入读取失败不能丢弃终态；最终 terminal query 只读精确 Run header + final watermark，正文独立按 journal/chunks 续传。客户端不能凭小 receipt 自行编造 canonical 文本或覆盖其他 Run。

## 8. 客户端与 adapter 边界

共享 Domain 定义 records/chunks/terminal receipt；Application 负责可见性、ownership、事务和恢复决策；Store 提供固定快照与有界页。CLI、Host bridge、FFI/Service 仅编码、鉴权、分页和背压，不独立拼接业务正文。

Mac/iOS/Android/Web 共用同一提交/读取合同。各端只持有已确认正文 cursor 与 viewport，渲染当前窗口；Markdown 不能在每个新增块上重解析整篇历史。Mac pacing/detach 属于 presentation session，不能推进 canonical cursor；reattach 从本地已提交 projection/window 恢复，无需再次运行模型。

CLI terminal scrollback 和 Web/Mac显示可有不同节奏，但都不能因隐藏窗口停掉source journal提交。无订阅时可以不通知、不读页，不能把恢复能力依赖于显示订阅是否存在。

Mac实施必须同时拆掉`refreshIfChanged → projectMessages → projectMessage/joined`
的累计正文热路径。会话窗口仅更新消息metadata；活动正文按Part/generation持有
manifest/chunk页。presentation session可保留reveal lease、detach/pending所有权，
但正文状态改为part/generation/page位置和visible byte offset，不再长期保留accepted、
visible、raw三套全文并通过hasPrefix推导delta。canonical cursor与reveal位置仍分离。

现有assembler已经复用完成节点，不能误称每个token都重解析全部历史；真实剩余
问题是未闭合表格的全pending split/map、代码尾重复扫描，以及renderGroups全节点
遍历。下一实现要以增量行状态和有界长块viewport处理这些情况，replace/remove只
失效目标generation。必须统计实际处理字节，不能仅凭小文本cache命中宣称线性。

## 9. 替换顺序与完成门槛

1. **Domain/Store 主干**：独立journal/分段正文、source/target事务、manifest分页、digest/epoch/CAS合同。新增真实SQLite tests，先不切远端。
2. **Application 可见投影统一**：抽取planner，覆盖review隐藏/批准、Part replacement/removal、Task/Skill/ModelAttempt；received短delta与回滚同批验收。新增热路径不得累计重读全文。
3. **终态切换**：同事务final watermark +小terminal receipt，精确query与完整文本同步分离；移除“完整projection读失败则丢terminal”的依赖。
4. **Host/CLI +一个真实客户端闭环**：独立正文cursor replay、snapshot staging、chunks、背压与重连；用真实Service/HTTP及Mac canonical port验收。协议换版不依赖兼容旧snapshot语义。
5. **其他客户端及同步/历史读切换**：所有adapter采用相同fixture；删除持续发布的整assistant snapshot、replacement同步热路径。完成大正文与复杂度验收后才宣称P5目标闭环。

不把第一步日志表存在、单个delta测试通过或短文本UI正常当成第5步完成。

## 10. 字节预算与验收矩阵

建议初始硬上限（协议显式返回limits，不是模型正文总长度限制）：单chunk 64 KiB、单page/frame 256 KiB、每页至多128 records、每record至多128 operations；超过的提交可分组成一个有界manifest引用的原子batch。大replace不内联正文。分页请求、staging总量、单连接未ACK队列分别有配额和可恢复错误，禁止无限内存累积。

源端发布规划同时限制canonical binary与实际JSON编码：当前batch各不超过128 KiB。
不能等客户端读页时才发现单条记录无法传输。JSON预算使用有上限的计数字节writer，
不为检测大小额外组装整份编码；超过时只在完整operation边界拆帧，保持顺序与内容。
256 KiB传输帧余量用于header、owner/content证明及一个64 KiB原始块的base64表示。
单个operation本身超过预算必须在发布前明确失败，不能写入客户端无法读取的记录。

必须测：

- 实际canonical提交→journal→HTTP/chunk→targetStore→客户端文本；未提交delta不发布、可选发送失败不推翻业务提交。
- Receipt/journal/chunk/head/同步引用全事务回滚，崩溃重开、重复命令、同序号异digest、缺页、乱序、过期cursor、snapshot中途新增提交。
- Host/epoch/Run/assistant错配；跨assistant Part碰撞；撤回review、批准候选、replace/remove、非文本Part与模型重试覆盖旧partial。
- UTF-8跨边界、空追加、长单行、代码块/表格跨chunk，最终拼接精确相等且无重复。
- 2 MiB、8 MiB、32 MiB以上正文；terminal先到、最后一页丢失、超时后恢复。小终态receipt始终可查，完整结果最终可恢复。
- 流控下固定内存上限；断线/窗口detach不泄漏订阅，不改变业务终态或触发模型重跑。
- 独立测试Trace cursor与正文cursor：丢Trace不丢正文、只读Trace不会推进正文head。
- 用instrumented repository统计实际读取/写入/序列化/发送字节，不只测耗时。固定chunk与operation大小、固定接收端数时，append N字节的持久/传输总字节应为O(N+K)，而非O(N²)；替换成本计入实际新内容总量。Merkle更新可为O(log chunks)，不能隐藏全文hash或全Parts删除重建。跨进程Service测试验证，而不只in-process helper。

预算越界必须是typed可恢复分页/容量状态；不能静默截断、退回Trace拼正文、放宽ownership或无限提高JSON阈值。

### Chunk载荷与外壳预算

64KiB原始UTF-8不能直接保证JSON text小于256KiB：控制字符转义可能膨胀到六倍。
不可变chunk发布后不能为了发送再切分并改变digest。HTTP chunk端点使用raw UTF-8
bytes（octet-stream）；JSON IPC每frame最多一个base64 chunk（64KiB编码为87384
字节，另加有界metadata），canonical digest仍对原始UTF-8的版本化chunk编码验证。
metadata/journal操作仅携带refs，不内联正文。跨chunk code/table渲染不依赖传输格式。

### 当前实施证据

Domain的header/chunk/cursor与typed operation batch已实现，含独立golden、
有界闭合metadata、受影响head局部验证及CAS计划；已扩展独立字段generation、
ReplaceContent、准确binary64 progress和Step title/state。Store已实现同UoW的
Admission/Transition提交证明、精确successor归属、chunks/manifests、heads/tombstones、
固定snapshot、分页、release及byte-range索引。Application已注册planner与真实SQLite
publisher桥接，覆盖Text/Reasoning/Tool/Source/Citation/Attachment/Step。

上述模块有分批合同、真实publication与冷重开证据；历史Domain80/专项12等记录是各自
当时源码的结果，不能当最新批次验收。10-01共享公共语义映射、严格公共值冷读及
跨端消费者已修改并重验：六crate lib、完整Application tests、Android JVM及
Mac/iOS各七类原生build/test已通过；CLI完整binary与短stream指定PTY已重验，
详见整体改造第8.2节。

2026-10-02已实现完整多owner/多frame的`CommitAssistantPublicationSet`：Store统一writer、
schema33完整集合凭证及frame成员表；Application按128 operations、128KiB实际
canonical batch、128 chunks/256KiB原始UTF-8预算分帧，保留跨帧head CAS及generation。
canonical public view只借用真实Part字段并映射初始message owner，不clone累计正文。
新SQLite末帧故障/冷重开/篡改与planner回归已进入实际r3六crate lib验收（exit0），
日志`/tmp/nola-journal-publication-set-libs-r3-20261002.log`，完整范围见整体方案第8.5节。
另有实际v32→33 additive migration及旧Run只读不授新写回归；不通过降低当前版本
号伪造旧schema。prepared旧字段的全历史正文完整性不靠Replay重扫，读取仍验证自身
manifest/chunk证明，物理GC与保留期另有合同。

生产reducer尚未调用journal publisher，Task/Skill/Attempt adapter、Idle/Recovery提交
证明、权威changed-owner coordinator及全端传输仍未实现。
这与已经启用的**私有canonical Text增量writer**是两条不同链路，不能混称未开始，也
不能据此宣称公开同步或端到端性能已经线性化。snapshot release不等于正文GC。

独立JavaScript校验可运行`node scripts/verify_assistant_journal_vectors.mjs`，使用
同一golden但独立编码实现验证UTF-8字节、header、最大u64及非法数字外壳，
以及typed Remove操作批次、空正文root和追加root；另验证同一chunk在不同epoch/Run
下不会抹掉manifest的owner绑定。该命令
已通过。它不是Web生产订阅器，也不意味着跨端正文显示已经迁移。
