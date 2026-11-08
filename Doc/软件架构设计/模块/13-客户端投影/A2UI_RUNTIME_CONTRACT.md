# A2UI Runtime 与客户端投影契约

更新：2026-10-28。状态：已有实现与局部真实验收，尚未全平台结项；任务状态见[当前入口](../../../当前任务/README.md)。现行协议固定为 A2UI v0.9.1，不提供旧版本兼容；规范快照与接入过程见[阶段实施记录](../../../历史任务/2026-10-28_A2UI与跨端执行阶段归档/A2UI统一运行时与跨端渲染实施_20261021.md)。

## 唯一权威与数据归属

运行链：模型/Agent → 现有工具准入 → Application → Kernel 状态转换 → Store 原子提交 → 既有 journal/查询 → 平台渲染。客户端事件经同一 Application 命令链返回。Core 负责协议、组件图、绑定与函数求值、状态和策略；Host 执行受准入的外部效果与系统授权；Renderer 负责绘制、焦点、滚动、动画和未提交编辑。

Surface 归属必须包含 authority Host/store、Conversation、Participant/Context、来源 Run、Message/Part、wire surface ID 与 generation。可信执行上下文确定这些字段，模型不能指定其他会话或 Host。wire ID 只能在其归属命名空间解析，不是全局主键。多 Agent 同名 Surface 不覆盖彼此。业务仍属于 Conversation，不增加 Room 聚合。

提出 `UiSurfaceRef` 类型化 Part 引用：记录 Surface 身份与提交 revision；Surface 权威状态单独存储，journal 发布引用和增量。历史 Part 保留来源身份，恢复活动投影通过查询取得当前状态；需要历史版本展示时读取显式版本记录，不能将当前值伪装成历史值。具体持久类型在 P1 统一实现，禁止在客户端另存可写业务副本。

本地嵌入式会话由本地 Application 权威处理；远端会话由源 Host 权威处理，移动端镜像只缓存投影。观察远端消息不得在本地重新运行 A2UI reducer。删除会话使用现有授权事务和引用清理，不因协议不匹配删除或重建数据。

## 接入与确定性转换

UI 通过明确的工具/内容类型进入，工具名与 schema 由现有目录注册。不得扫描用户关键词、Markdown 代码围栏或普通文本来执行 UI。按传输消息边界有界缓冲，完整 JSON 才进入校验；不完整消息与有效的前向组件引用是不同情况。

Kernel 在显式能力、策略和时钟事实下执行确定性转换。单条消息验证后原子提交，不能先修改半棵树再报错。组件/数据更新遵循锁定的 v0.9.1 规范，不自行把它解释成任意 JSON Patch；updateDataModel 缺省 value 表示删除，显式 null 是数据值，两者必须保留区别。不接受 v1.0 专属消息或将其转换为当前协议。前向引用可形成待就绪 Surface，根与依赖满足后才展示；环、重复身份与越界路径按明确错误处理。

绑定求值、依赖索引、列表实例展开、校验结果与函数调用计划由 Core 产生；Renderer 收到 resolved props、子实例和动作引用，不再解析表达式。列表实例使用 Core 分配的稳定身份，重排不能只依赖数组下标。只有受影响节点重新求值，不能每次输入重算并发送完整树。

当前 Nola 可编辑模板列表要求每项具有唯一、非空字符串或数值 `id`；这是 Nola 的稳定身份准入约束，不是 A2UI 官方 schema 的新增字段要求。无稳定键、空键、重复键返回 `InvalidListIdentity`，模型应补齐数据后用未改变的 Surface revision 重试。输入绑定不能覆盖该 `id` 或其父数据路径，返回 `ImmutableListIdentity`；修改普通字段与重排保留实例和动作身份。纯只读模板列表允许无 id 和重复数据。不能以数组下标替代稳定身份。

catalog ID 是固定注册表标识，不是任意远程下载地址。功能按四端支持矩阵协商；首期 Nola catalog 只声明已实现子集。未知函数/组件返回类型化错误或明确的不可交互摘要，不能暗中执行 agent function 或切换协议。A2UI-Swift/AGenUI 的独立 store/parser 不进入生产权威路径；组件可复用性需单独验证。

## 事务、事件与桥接

沿用[公共命令信封](../../模块契约.md)的 `protocol_version/request_id/client_id/expected_revision/command`、幂等摘要和回执语义。A2UI wire version 与 Nola bridge protocol_version 分属不同层，禁止混用。Accepted 不表示业务完成，Committed 不表示外部效果完成；CommitUnknown 查询同一 request_id 的回执，不生成新动作重试。

Surface 状态、revision、命令回执及 journal/outbox 事件在同一事务提交。现有聚合 revision 用于命令并发栅栏；Surface revision 用于投影连续性，两者明确区分。禁止独立 UI 数据库双写。

当前内部投影 DTO（以 `core/nola-domain/src/generative_ui.rs` 为准，不是 A2UI 官方 wire 消息）：

- `UiSnapshot`：完整 identity（owner、generation）、revision、catalog ID/digest、phase、root instance、已解析节点和 Core summary。DTO 不重复保存 journal cursor。
- `UiChangeSet`：同一 identity、base_revision/revision、catalog_digest、phase、root instance、upserted_nodes、removed_instance_ids 和 summary。现有 journal 信封负责 cursor；DTO 不额外包含 cursor 或 changed data paths，客户端不执行绑定。
- `UiEdit` 显式区分 Set/Delete；`UiInteraction` 携带 identity、expected_surface_revision、edits 和可选 UiActionRef，置于公共命令信封中。动作解析后的 UiAction 是后继 Agent 输入，不能与客户端 action reference 混用。

初次打开、增量缺口或重连查询快照；重复事件去重，乱序不能直接应用。快照与后续 cursor 必须一致；超过响应大小限制使用一致 revision 的分段读取。显示层可以按帧合并重绘，不丢删除、动作回执或顺序。取消订阅后旧 generation 回调无效。

桥接沿用已有 C ABI/JNI/HTTP 与 journal。Web 不部署第二个 WASM Runtime；Apple 两端共用 DTO/渲染层，不能各自写协议解释器。

## 编辑、动作与执行生命周期

IME composition、输入编辑缓冲、选择区、焦点、滚动偏移留在平台端。组合输入期间不提交半成品；组合结束后的 blur/确认/有界 debounce 使用 Core 只读草稿预览，不提前持久化字段。远端更新与本地草稿冲突时保留草稿并提示，不覆盖正在输入的内容。

`PreviewUiSurface { input }` 接收完整 identity、expected_surface_revision、edits、draft_generation；返回 identity、base_revision、draft_generation 和候选 snapshot。它复用正式编辑与绑定校验，不推进 Surface revision、不写 journal、不产生 UiAction 或业务调用。checks 不通过仍返回候选节点 validation_errors；来源、版本、类型或绑定非法则明确失败。Renderer 只采用与当前完整身份、基础 revision 和草稿 generation 都一致的响应；旧响应、失败或尚未完成的校验不能用来启用提交。客户端不重新解释 checks。正式提交仍将全部草稿与 action 原子重验，预览不是授权或提交凭证。

Surface revision 推进后，客户端保留草稿且停止使用旧校验结果。用户明确选择“重新校验”时，先检查全部草稿字段仍在当前投影的 bindings 中，再更新草稿基准 revision/generation 并向 Core 重新预览；字段缺失时保留草稿并提示冲突，禁止静默丢弃。提交中、结果未知和 IME 组合输入中不得重基准。

点击提交必须把尚未提交的字段编辑和动作作为同一有序命令处理：验证来源 revision、应用编辑、求值校验、准入动作。失败保留输入，不用“先异步写值再立即发送动作”的竞态。批量编辑的路径/值/删除使用明确类型，不能混用其他协议版本的 null 语义。

客户端只提交 action reference，Core 从已提交节点解析动作定义；不能让事件夹带任意 tool 名、权限或执行代码。Core 检查归属、存活、revision、字段、能力和现有权限；可信 Host/后端执行确认和 OS 检查。生成表单内的“同意”按钮不能替代 Nola 权限面板。

纯视觉焦点/滚动留在 Renderer；具有业务语义的函数由 Core 统一求值或准入执行。渲染某节点不能自动发起网络/Agent 调用。链接、附件、文件选择也通过可信平台能力端口，结果带来源身份返回。

来源 Run 已结束后，允许的卡片动作以现有用户交互提交机制创建新执行，保留 originating Run 引用；不复活旧 lease。Run 结束、Surface 删除、流结束分别处理，不能互相代替。取消停止尚未执行的效果；已提交卡片按可用能力显示或只读。打开历史、重连、重绘不重放业务动作；删除/过期后的旧动作明确拒绝。

Surface 与公开流式正文、reasoning 投影并存。工具/Surface 发布不能把已公开正文移动进默认折叠的执行过程，也不能结束后续模型轮次的思考显示。当前 reasoning 必须按原 Part/journal 顺序判断，不能先分区再用历史正文推断已结束；工具详情与思考展开状态独立。私有/待审核文本继续服从 Core 披露规则，不把工具参数或未完整 A2UI JSON 当作正文。

## 显式重试的身份约束（设计已明确，尚未实现）

当前实现对已拒绝且未准入的队列项提供显式取消；重新点击表单属于新业务动作。不得把它描述为同一动作的重试。后续 retry-attempt 必须复用现有队列、命令回执和 source submission 事实，不另建 UI 重试数据库。

- 逻辑动作身份保留原 UiInteraction submission/queue command ID；已提交的完整 UiAction envelope 不变，重试不再执行字段编辑，不增加 Surface revision。
- 每次 dispatch attempt 使用独立的 submission/run 身份、assistant message 身份、冻结请求和 digest。现有 RemoteSubmissionIdentity 仍表示一次 dispatch；旧 receipt 永不覆盖。逻辑动作与当前 attempt 的关联由 Core 持久化，不能只存在客户端内存。
- 新 Core 命令在单一 UoW 中检查原 attempt 的权威 rejected/cancelled 且 admission 为空、队首仍存在、无有效 lease、完整 owner 和冻结动作一致，再用 expected attempt 做 CAS 创建下一次。已取消并移出队列的动作不能借此复活。
- prepared/bound、无回执、传输失败或结果未知只能查询原 attempt；admitted 只能恢复原 Run。不得以超时或模型未答复作为未执行证明。
- 同一 retry 命令 ACK 丢失时，仍查询/重放原命令 request_id，不能创建第三次 attempt。准入策略可对新 attempt 重新检查；新请求及 digest 留痕，不能改写旧请求。
- 队列唤醒、准入、取消和两类宿主传输必须同时改为读取 Core 的 active attempt，删除当前将 command_id 等同 dispatch submission_id 的假设。客户端不得拼装新的正文或自行换身份。

实现验收须覆盖并发 retry CAS、取消竞争、ACK 丢失、未知/admitted 拒绝、跨 Host/client 拒绝、旧 receipt 保留、Surface revision 不变及不重复用户消息。此节是未完成实现的约束，不能作为测试通过或功能已交付的证据。

## 数据与安全边界

Surface 数据属于其 Conversation/Context，不因展示给子 Agent 自动共享。`sendDataModel` 按 0.9.1 回传完整数据模型，不实现静默裁剪字段后仍声称发送完整模型的行为。允许进入该模型的数据来源限于所属 Agent 已发布的数据和用户在该 Surface 上提交的绑定输入；Renderer 不得自动注入宿主凭证、剪贴板、平台上下文或其他会话内容。这里是数据来源约束，尚不是字段级隐私授权机制，不能把模型设置的 `sendDataModel` 当作读取其他资源的授权。

Core 在发送前按 authority、conversation、participant、context 四项匹配目的地，仅纳入显式开启 `sendDataModel` 的活动 Surface。metadata 的 `a2uiClientDataModel` 包含 `version: "v0.9.1"` 和 `surfaces`；同一目的地中 surfaceId 歧义、非对象数据模型或超出动作总字节预算时明确拒绝，不覆盖、不截断。动作携带的是提交绑定编辑之后的权威状态；失败必须与 Surface、Part 和队列更新一起回滚。普通用户查询也须通过同一 Core 元数据构造入口，不能由各端自行拼装。引用现有上下文构建和压缩，不另建 Web UI 压缩策略。

不执行模型提供的脚本、HTML、任意 CSS、动态模块或远程 catalog。资源 URL 经 Host 资源策略，限制 scheme/来源和大小；不能用图片/链接绕过本地网络、文件和敏感读取审批。日志只记录身份、revision、大小、耗时和脱敏错误，不默认记录表单隐私值。

以下是 **当前 Core 实际执行的 Nola 资源预算**，不是官方协议限制或已验证性能阈值：单消息 256 KiB、单 Surface 状态 1 MiB、1000 节点、深度 32、单会话 8 个活动 Surface、动作输入 64 KiB；函数求值限定步数/深度，分块缓冲限定总字节和持续时间。这些数值由 Core 常量及 catalog digest 统一约束，四端不得各定标准；性能验收仍按实施任务记录实测，不把资源上限当作性能达标证明。超限保留最后已提交状态并返回局部错误。

Surface 的聊天语义摘要与 canonical Part/journal 使用同一个预算：`UI_MAX_SUMMARY_BYTES` 直接引用 `MAX_METADATA_STRING_BYTES`，当前为 4096 UTF-8 字节。Core 在字符边界截取摘要；完整组件 Text、Surface 状态和精确历史版本仍保留全文，不能因摘要缩短而改写组件正文。不得让投影允许的摘要大于 journal 编码预算，否则合法长卡片会在持久化时失败。该预算纳入 catalog digest，各端只消费 Core 的摘要，不另行截断或重建。

错误至少区分 unsupported_protocol/catalog/component/function、invalid_message/binding、revision_conflict、stale_owner、surface_closed、resource_limit、permission_denied、commit_unknown。错误作用于对应卡片/动作，不能清空整段聊天。恢复是否可重试由权威回执决定，不能默认重新执行副作用。

## 验证责任

Core 验证转换/事务/权限/恢复；共享黄金夹具验证四端语义投影一致；各端真实操作验证布局、输入法、线程和动画。历史测试清单、版本锁定与阶段门禁见[阶段实施记录](../../../历史任务/2026-10-28_A2UI与跨端执行阶段归档/A2UI统一运行时与跨端渲染实施_20261021.md)。测试未执行前不宣称共享 Runtime 或第三方适配已经完成。

## 发布、编辑、动作与重绘的四个不同事务（2026-10-28核对）

依据Application `generative_ui.rs`与Kernel `generative_ui/{interaction,projection,state}.rs`。A2UI不能只定义“Rust给客户端一棵树”：它需要分别解释模型发布、用户预览、用户提交和Renderer追赶如何并发。

### 模型发布

1. Host只能引用已提交的ExecuteTool effect、request和delivery，不能另带一份任意模型参数来渲染。
2. Application从outbox读取原payload，确认capability是UI工具，验证UiToolInput并计算输入摘要。同effect已有publication时，摘要一致返回原结果，摘要变化拒绝。
3. 未发表的新effect必须仍pending；active attempt须为ExecutionStarted，Run/request、attempt、worker generation、lease owner/token均精确匹配，lease尚有效。
4. Surface owner由可信Run/Turn/call绑定解析，不能采信LLM传来的owner。删除的Surface历史保留tombstone，但新create使用新generation，不能复活旧交互handle。
5. transition_messages验证整批协议和expected_revision；随后project_surface验证当前平台共享投影能够表达的binding/function。不能先提交再让Renderer报“不支持”。
6. Surface CAS、对应消息Part与projection事件在同一publication事务中提交。事件ID来自authority identity摘要和revision，而非临时HTTP重试ID。

如果变化后的候选除revision外与原状态完全相同，仍执行版本/合法性校验，但不新增Surface revision、Part更新或事件。非法协议整批不提交；revision冲突不能擅自改为最新值重试，应向模型返回当前revision及保持用户编辑的提示。

### 用户编辑及预览

交互必须包含精确Surface identity和expected_surface_revision，且Surface处于Ready。Kernel先从原snapshot解析所有binding；客户端只选择已准入的instance/binding名称，不能指定任意JSON pointer。

仅支持可编辑组件的value binding；disabled节点、未知instance、重叠路径拒绝。同批set先处理，数组delete按索引逆序，防止删除前行改变后续编辑的对象。编辑完再投影并检查最终全部值，因此约束和值可在一次事务共同变化，未编辑值也不能违反新约束。

preview复用同一prepare_interaction计算候选，但不写Store；snapshot revision保留原值。draft_generation必须在合法整数范围且大于0，用于客户端排除迟到preview。preview成功没有提交回执，也不是用户执行批准。

### 用户动作与下一轮输入

action只能来自编辑后投影中当前节点公开的精确action handle；disabled或handle不相等拒绝。Kernel解析得到event，这仍是用户交互数据，不是可执行tool call。

Application SubmitUiInteraction同时处理Surface编辑、消息Part、可选UiAction队列项与projection事件。event补可信timestamp、验证v0.9.1客户端消息、附准入client data model后生成完整envelope；字节预算覆盖整个envelope，超限使Surface编辑也回滚，不留下“界面变了但动作没进队列”的半成功。

UiAction使用原submission_id生成队列/Item身份，投递目标为Surface owner.context_id，delivery为NextTurn。返回queued_submission_id只证明动作已持久排队，不证明模型已消费、工具已执行或业务成功。无action的编辑不会凭空新增队列项。客户端不能把按钮文字改写成系统指令。

### Renderer追赶

| 源状态与base | Application结果 | Renderer行为 |
| --- | --- | --- |
| Surface不存在 | Missing | 按源缺失呈现，不自行重建同identity |
| base高于源revision | 错误 | 核对authority/缓存，不能倒写源状态 |
| base等于当前 | Unchanged | 保留现有渲染与焦点 |
| 无base、历史base不存在、catalog身份/摘要变化 | Snapshot | 使用源完整投影替换逻辑快照，平台瞬时编辑需按owner处理 |
| 合法历史base且catalog一致 | Changes或Snapshot | Core比较序列化大小选择较小者；客户端不能假设一定返回diff |

历史revision读取不改变Surface phase，而且不暴露交互handle。旧卡片可以查看，不因历史按钮看起来可点而取得当前执行权。Surface revision、journal cursor、draft generation和Run终态是四种不同进度，不能复用一个“最新版本”数字。
