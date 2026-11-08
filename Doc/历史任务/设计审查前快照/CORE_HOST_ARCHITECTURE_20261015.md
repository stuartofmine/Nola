> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Core / Host 分层架构

> 2026-10-15 整体设计复核前的历史快照，仅保留当时叙述与证据。旧 ABI、Trace 权威、schema 数字、传输别名和其他旧规则均不作为现行规范。当前入口见[软件架构总纲](../../软件架构设计/总体设计/SOFTWARE_DESIGN.md)。

本文档定义 Nola Agent Runtime 的规范边界。实现与本文冲突时，以本文的
`Submission -> Event + HostRequest` 契约为准。

## 1. 设计目标

Nola 的 iOS、Android、macOS、CLI 和 Web 不能各自维护一套 Agent 循环。平台差异
应体现在能力和展示层，而不是决策语义。统一运行时需要满足：

- 同一输入、状态和 Host 响应产生确定性的状态转换。
- 模型、工具、文件、网络、权限 UI、系统 API 和存储都由 Host 执行。
- Core 不直接访问文件系统、网络、时钟、进程或平台框架。
- UI 只消费 Core 事件投影，不直接解释供应商流式分片。
- 会话可以在进程退出、Host 断开或上下文压缩后恢复。
- 本地 Host 与远程 Host 使用同一关联请求协议。

## 2. 分层

### 2.1 Agent Core

Core 负责：

- Session、Turn、Item、Part 和 ToolCall 状态机。
- 用户输入排队、当前 Turn 追加和中断语义。
- 模型请求与工具调用的编排。
- 权限规则匹配、会话级授权和审批生命周期。
- 上下文窗口、压缩阈值、压缩事务和失败熔断。
- 有序 Event、ConversationProjection、checkpoint 和恢复。
- Host 能力目录校验、effect 归属、响应去重和过期响应拒绝。

Core 的公开会话对象是 `AgentRuntime`。内部 `AgentRuntime` 按启动校验、提交路由、
Turn 驱动、Host 响应、effect 恢复和 UI 投影拆分；这些内部层不属于 ABI。一个
checkpoint 同时保存 `AgentSessionState`、确定性的 `AgentSessionServices` 和创建时
捕获的模型/工具/上下文配置。Host 重连可以更新能力目录，不能静默改变同一 checkpoint
的模型、工具 schema 或上下文窗口契约。

Core 不负责：

- 发起 LLM HTTP 请求。
- 执行 shell、文件、浏览器、MCP 或平台工具。
- 展示权限面板。
- 读取平台数据源。
- 决定 SwiftUI、Compose、终端或 Web 的视觉表现。

### 2.2 Host

Host 负责：

- 发布带版本号的 `HostCapabilityCatalog`。
- 执行 `HostRequestEnvelope`。
- 将供应商流转换为规范 `HostResponseEnvelope`。
- 执行平台权限交互，并返回结构化决定。
- 持久化 checkpoint、完整工具输出和 artifact。
- 实现取消、超时、进度与远程连接生命周期。

Host 不得：

- 自己运行第二套 Agent/tool loop。
- 根据展示文案反推工具参数。
- 绕过 Core 直接追加最终消息或授予权限。
- 修改 Core 生成的 session、turn、effect 或 call 标识。

### 2.3 Application / Nola Store

Application 是 Core 与持久层之间唯一允许的事务边界。Core 保持纯 reducer，不打开
数据库；Host 执行 I/O，但不能自行决定哪些消息成为会话事实。Application 在一个提交中
保存 Core Event、ConversationProjection、checkpoint、outbox 与 Host receipt。

桌面端通过版本化的本地 Application Service 协议提交这些事务。`nola-service` 是 Store
唯一 writer；Mac、CLI 和 Web 是服务客户端，`nola-agent-host` 仍只执行 effect。服务不可用
时客户端必须明确失败，禁止静默打开 Store 形成第二 writer。

Nola Store 是以下数据的唯一持久事实源：

- Conversation、Message、Part 的稳定 ID、顺序、状态和完整内容；
- Run、Turn、Core Event、checkpoint、pending effect 与重放 cursor；
- context snapshot、压缩摘要及其覆盖范围；
- 会话级设置、权限规则引用、长期记忆引用、任务和 artifact 引用。

完整 transcript 只追加。上下文压缩新增 snapshot，不删除、替换或重写原消息。平台的
Core Data、Room、JSON 文件、终端缓冲区和 SwiftUI/Compose/Web 状态只能是缓存或投影，
不能成为第二事实源。备份、恢复、数据统计、后台任务、右侧检查器和聊天列表必须查询同一
Store；禁止某个页面读 Store、另一个页面读旧平台 Archive。

远程 Host 也遵守同一规则。客户端可以在 Turn 开始时保存用户输入信封，但 assistant
正文、推理、工具、来源和终态只能按 Host 返回的稳定 Core Part ID 与单调 cursor 复制。
客户端不得在流结束后把整个 UI 气泡写回 Store。重复 cursor 是幂等 no-op，缺失 cursor
触发重放，不能用当前 UI 文本猜测缺失内容。

远程执行不会把 Host Snapshot 伪装成可恢复的本地 Core checkpoint。Nola Store 单独保存
`remote_session_mirrors`：每个会话一份、带已认证 `source_host_id`、Core revision、单调
event cursor 和完整 `AgentCoreSnapshot`。相同 cursor 且内容相同是幂等重放；相同 cursor
内容不同是协议冲突；旧 cursor 被忽略。Application 在本地 checkpoint 与远程镜像之间按
更新时间选择事实来源，并统一返回带 `source` 的 `SessionProjection`。这样 UI 无需知道
执行位置，同时远程只读镜像也绝不能被用于恢复或推进本地 reducer。

备份中的旧平台会话结构只允许作为版本化导入 DTO。导入完成后立即物化到 Nola Store，
之后不再参与运行时读取或写入。

Core checkpoint 使用明确的 `conversation`、`execution`、`host`、`journal` 命名域。
禁止把多个可演进结构同时 `flatten` 到同一 JSON 对象，否则字段所有权不明确且无法可靠
反序列化。Task、Goal、Memory、Skill 和 Permission projection 即使内容已清空也必须保存
revision；清空本身是重连后必须可观察的状态转换。

Application 从该 checkpoint 生成平台无关的 `SessionProjection` 查询模型。所有客户端
直接从 Nola Store 查询它；`session show`、Agent Host trace 和平台 Archive 仅是展示、
诊断或一次性迁移适配器，不能作为持久会话查询 API。

### 2.4 Presenter

Presenter 将 `CoreEventEnvelope<AgentEvent>` 投影为平台 UI：

- 文本、推理、工具调用和结果使用稳定 Part ID 原位更新。
- 流式 Markdown 更新同一 Part，不先显示 raw 文本再整体替换。
- 工具参数从首个 argument delta 起创建稳定 Tool Part；最终 JSON 只收敛 input，
  不替换整张工具卡。
- 工具过程默认可见，详细参数与完整输出按需展开。
- 权限、失败、取消和压缩状态来自 Core Event，不读取 Host 私有状态。
- 本地化只改变展示文本，不改变协议中的稳定枚举值。

## 3. 唯一变更入口

产品表面只能提交 `AgentSubmission`：

- `user_input`
- `permission_response`
- `host_response`
- `update_host_catalog`
- `host_disconnected`
- `resume_host_effect`
- `interrupt`
- `compact`
- `reset`

`AgentCommand` 和 `AgentEffect` 是 Core 内部实现，不属于跨平台 ABI。平台不得构造
模型完成、工具完成或内部进度命令；这些状态只能由带关联信息的 Host 响应驱动。

一次 Core 调用返回：

```text
AgentSubmission
  -> Core reducer
  -> CoreEventEnvelope[]
  -> HostRequestEnvelope[]
  -> AgentRuntimeSnapshot
```

Host 执行请求后，将 `HostResponseEnvelope` 作为新的 `host_response` submission
提交。`effect_id` 表示逻辑操作，`request_id` 表示本次投递尝试；`host_id` 与可选
`lease_id` 表示具体执行器生命周期。它们与 `session_id`、`turn_id`、
`catalog_revision`、`availability_epoch` 和单调 `sequence` 共同建立归属与去重边界。
任一字段不匹配都必须 fail closed，不能按“同名工具”或“同一会话”猜测归属。

### Host 生命周期

Codex 风格 Thread 是进程内的长生命周期对象，不是“每次 HTTP/终端输入从磁盘恢复一次”
的函数。CLI/Web Host 维护按 `session_id` 隔离的 Host registry；registry 只负责实例
生命周期和会话级互斥。Store/Application 持有唯一状态机写入权；`CoreAgentHost`
只缓存冻结配置和从已提交 checkpoint 重建的只读投影，并通过
`CommittedRuntimeDriver` 排空已提交 Outbox。一次 Turn 结束后实例可以继续复用驱动器，
但任何输入和 Host response 都必须先提交 Application 事务。权限或设备工具等待期间，
响应通过原 effect correlation 返回，严禁用第二次聊天请求模拟 resume。

Registry 中存放的是会话槽位而不是裸 Host。全局锁只用于查找或创建槽位；首次初始化、
配置切换和替换只持有对应 `session_id` 的槽位锁。同一会话的并发首次访问因此只能发布
一个 Host，不同会话不会因为模型探测、checkpoint I/O 或 Turn 执行而互相阻塞。

### Trace 与 wire envelope

Core Trace 是文本、思考、工具、用量、阶段和终态的唯一语义来源。Core 在
`AgentTransition.trace_events` 中直接发布由已排序 `event_envelopes` 投影出的 Trace；
每个 Turn 独占 trace/run identity 和单调 sequence。Host 可以在同一个 wire envelope
中附加权限 nonce、参数摘要、设备 capability 和过期时间，但这些字段只
用于验证并完成对应 HostRequest，不能再次驱动 UI 状态。移动端遇到缺少 Core Trace 的
Agent 事件必须报告协议错误；不得回退到 `text`、`tool_result`、`done` 等旧 wire 类型
重建第二套状态机。
恢复请求沿用原 `effect_id`，并携带 `resume_after_sequence`；Host 不得从 0 重新发送
已经确认的流。

## 4. Turn 与流式输出

`UserInput.items` 与 `turn_context_items` 是两个独立协议字段。前者进入 transcript，
后者冻结在 `AgentTurnContext`。Core 以“Turn Context + 压缩后的 Session Context”
构造每次 `ModelRequest.items`，并以这份真实模型可见集合计算 token 占用。Presenter、
Host 和平台不得把 Turn Context 再写成 system chat message。

平台通过 `turn_context_platform` 与 `turn_context_components` 提交来源事实。Core
在 Turn 创建时统一执行授权/时效清洗、组件排序、预算截断和 digest 计算，并把原始
组件、metrics 与物化 Items 一起冻结进 checkpoint。Context Engine 不在这里启动
第二套历史摘要流程；Session transcript 的 LLM 压缩仍由 Agent Runtime 的 compaction
effect 负责。

Web completion 与 SSE Agent Turn 不得在调用 Core 前执行
`prepare_context_with_llm_summary`。两者提交原始 typed history、当前输入和组件，
由 Core 统一物化并在阈值前触发 compaction effect。显式 compact API 只是用户命令
入口，也转换为 Core submission，不维护独立的会话摘要状态。无活动 Turn 的手动压缩
只提交 Session checkpoint 和上下文指标，不创建伪 Turn，也不会触发模型继续采样；
自动压缩完成后才恢复当前 Turn。

Host 上下文在入口处归一化为类型化 `ContextComponent`。运行配置不得保留
`host_context_messages` 或其他 role/content 注入旁路。

Canonical data-source definitions are shared product facts, not a promise that every Host can
execute every source kind. Each Host projects only the kinds it owns into its runtime reader:

- CLI/Web Agent Host owns `http` and `localFile` readers.
- macOS owns `platform` sources such as HealthKit, Calendar, Contacts and Reminders, and resolves
  them into typed turn context before submitting to Core.
- Native Web/browser sources remain owned by the platform connector that created their validation
  contract.

A Host must skip foreign source kinds without decoding them as a similar local kind, deleting them,
or rewriting their canonical record. In particular, `platform:health` is not a local file path.
Malformed records for a Host-owned kind still fail closed. Regression coverage must open one real
canonical Store containing mixed Host kinds, run the Agent Host process, and prove the foreign
records survive byte-for-byte semantically unchanged.

Core 在关键 reducer transition 后产生 `journal.checkpoint` Host effect。Core 不选择
文件、数据库或云存储；CLI/Web Host 在装配 `CliAgentHostDriver` 时提供原子
checkpoint writer。Web 即使不启用 CLI transcript 存储，也必须按 conversation ID
绑定独立 checkpoint 路径，进程重启后以 checkpoint 恢复压缩状态和未完成 effect。

Session 是长期容器，Turn 是一次用户意图的执行事务。

平台工具执行器接收已经由 Core Catalog 与 tool-plan receipt 绑定的类型化请求，只负责
验证当前 receipt、执行 OS effect，并返回带原 request/effect/sequence 关联的
HostResponse。iOS 不得把 `ResolvedTool` 重新编码为旧 `host_call_requested` UI 事件
再解码执行；Android 的旧事件 parser 同样不属于 v2 主运行路径。

macOS 远端模式发送当前用户消息与独立 `context_components`；不得把会话组件拼成
system history。Web Host 只反序列化组件并交给 Core Context Engine，授权清洗、时效、
排序、预算和 digest 仍由 Core 完成。

1. 用户输入创建或追加到 Turn。
2. Core 立即创建 Assistant Part，并发出可展示事件。
3. Core 产生模型 HostRequest。
4. Host 将文本和推理分片转换为 ModelDelta 响应。
5. Core 按稳定 Part ID 更新投影。
6. 模型工具调用进入权限、执行和结果生命周期。
7. 无更多工具调用时 Turn 完成。

用户在运行中继续输入时，delivery 明确区分：

- `current_turn`：作为当前 Turn 的追加输入。
- `next_turn`：进入 Session mailbox，当前 Turn 完成后自动开始。

中断是 submission，不是 UI 私自丢弃流。Core 产生 Cancel HostRequest，Host 必须确认。
取消请求自身也是 pending Host effect，并指向被取消的 target effect。模型流、权限面板、
工具、上下文读取和存储 effect 使用同一取消路径；平台不得调用私有 cancellation token
绕过协议。存在待取消 effect 时，Turn 先进入非终态 `cancelling`；只有关联的
`CancellationConfirmed` 清除 target 与 cancel effect 后，Core 才原子地产生
`TurnCancelled` 并进入 `cancelled`。Presenter 的终态不能反向终止 Host 进程。

## 5. 权限

权限分为两层但只有一个状态机：

- Host Catalog 声明静态环境事实：能力是否可用、权限域、取消与重放语义。
- 对 shell 等参数相关能力，Host 使用实际 sandbox/允许目录做风险预检。
- Core 消费可信风险事实和 Session/Turn 规则，唯一决定 allow/ask/deny。
- 模型提供的权限域、理由或“已经批准”标记不具备任何授权效力。
- 受 Host 允许目录约束的本地只读文件能力不弹通用权限；网络、写入、进程、
  浏览器和系统隐私能力仍保留独立权限域。

权限策略采用最后匹配规则生效：

```text
tool/capability + resource pattern + source -> allow | ask | deny
```

规则来源可包括内置安全策略、用户配置、会话授权和平台限制。Core 负责最终合并；
Host 只负责展示审批并返回决定。

权限请求必须携带原始结构化工具输入。命令、路径、URL 和 MCP 参数不能由 Host 从
标题或资源摘要反向解析。Presenter 可以脱敏展示，但执行器收到的仍是原始结构化输入。

`allow_once` 只对当前请求有效，`allow_session` 写入当前 Session 状态。内部低风险记忆
写入等能力可由 Core 内置规则允许，不应反复弹窗。

## 6. 上下文与压缩

历史导入与当前轮输入必须使用 typed `AgentItem`。禁止把 system/developer/user/
assistant/tool-call/tool-result 历史拼成一条用户消息；已有进程 Session 只接收当前轮
增量，历史导入只用于首次建立 Session。这样压缩、重放、权限恢复与模型可见上下文
共享同一语义边界。

以下数值必须分开：

- `context_window_tokens`：模型总窗口。
- `reserved_output_tokens`：为本轮输出保留的空间。
- `prompt_token_limit`：本轮 Prompt 实际可用上限。
- `auto_compact_tokens`：自动压缩触发阈值。
- `compaction_target_tokens`：压缩后的目标大小。

大窗口模型不意味着应无限扩大活跃上下文。Host 可以探测模型窗口，但 Core 决定压缩
事务。压缩必须由代码和 LLM 共同完成：

1. Core 按完整 API round 选择归档边界和必须保留的最近 Items；不能拆散
   tool call / tool result 事务。
2. Host 调用压缩模型生成结构化 continuation summary。
3. Core 校验摘要身份、保留尾部和 token 收益。
4. 成功后原子替换；失败则回滚原上下文。
5. 连续失败触发自动压缩熔断，避免无限重试。

系统/开发指令始终保留。完整 transcript 是 Nola Store 中只追加的 Message/Part 会话
事实。Core Runtime 在活动 Run 内持有确定性投影，Host 只执行 effect；Application 原子
提交 checkpoint、Event、Part mutation 和 outbox。摘要只替换 `model_context`，不是原始
记录的替代品。

## 7. Checkpoint 与恢复

checkpoint 保存活动 Run 的可恢复 `AgentSessionState`，包括：

- Store transcript 的活动投影、可压缩 model context 与 ConversationProjection；完整正文
  仍从 Store Message/Part 重建。
- 活跃 Turn、排队输入和工具状态。
- 每个活跃 Turn 冻结的 `TurnContext`。
- 会话授权与权限规则。
- 上下文状态和压缩代次。
- Host 能力目录版本。
- pending Host effects 及可重放请求信封。
- event cursor 和 revision。
- Session 单调 ID 序列；Reducer 不调用随机数生成内部 Turn/Item/Effect ID。

当前 checkpoint schema 为 `8`。除 pending effect 外，checkpoint 还持久化
`AgentTraceContext`，保证恢复后 trace/run identity 与下一 sequence 不变。pending
effect 至少持久化：

- effect/request identity；
- Host/lease/session/Turn owner；
- Catalog revision 与 availability epoch；
- capability、call 和 target effect correlation；
- replay safety；
- `dispatched / accepted / streaming / cancelling` 状态；
- 最后接受的 response sequence；
- 可恢复的完整 HostRequest envelope。

事务顺序是：

```text
reduce -> register pending effects -> persist checkpoint -> execute Host effects
```

checkpoint 采用原子替换。恢复时必须校验 schema、session identity、Host identity、
lease identity 以及 Catalog revision/availability epoch。只要存在 pending effect，
restore 时提供的 Host owner 必须与 checkpoint 完全一致。Catalog 更新不会静默重绑定
旧 effect；恢复必须通过 `resume_host_effect` 明确发起，保留原 `effect_id`、生成新的
`request_id`，并记录新的 Host/lease/catalog owner。

Host 响应通过 `effect_id + request_id + host_id + lease_id + session_id + turn_id +
catalog_revision + availability_epoch + sequence` 验证 ownership 与顺序。工具实现应
尽量支持幂等键；无法保证幂等的副作用需要在 artifact/journal 中记录提交状态。

pending effect 的恢复由能力目录中的 replay safety 决定：

- `idempotent` / `recoverable`：Core 可自动返回给 Host 恢复。
- `requires_recovery_decision`：只出现在恢复队列，收到显式 `resume_host_effect`
  submission 后才能再次下发。
- 不支持流断点续传的模型 Host 必须返回 `resumption_unsupported`，禁止从头重放并
  造成重复正文。

每个 submission 是原子事务。Reducer 返回错误时必须恢复 submission 前的完整
Session 状态，不能遗留半更新的 Turn、权限或 ID 计数。

用户输入可携带稳定 `submission_id`。Core 在开始 reducer 事务前完成幂等检查，
并将已接受 ID 写入 checkpoint；客户端重试、网络重发或重复点击只能得到当前
snapshot，不能创建第二个 Turn。Host 响应则以上述完整 owner tuple 去重并拒绝
旧连接、旧投递和乱序增量。

## 8. 能力目录

Core 只编排当前 Host 明确发布的能力。每项能力至少包含：

- 稳定 capability ID。
- 输入 schema。
- 风险与权限域。
- 展示策略：可见、延迟加载、内部或隐藏。
- 是否支持流式输出、工具参数流、进度、artifact 和后台执行。
- 并发语义：串行或明确声明为并发安全。
- 取消语义：不支持、尽力取消或协作式取消。
- 崩溃恢复语义：幂等、可恢复或必须由恢复策略决定。
- 数据出境与本地执行属性。

Core 始终保留完整模型工具 schema，只在创建 Turn 时按当前 Catalog 投影可用工具，
并把模型、工具、窗口、Host id、Catalog revision 和 availability epoch 冻结为
`TurnContext`。
初始化时不能永久删除暂时离线或尚未完成设置的能力。Catalog 更新必须使用单调
revision/availability epoch；断连会把能力置为 `requires_connection`。

模型工具投影以 Host Catalog 为唯一能力边界。只有同时满足
`availability == available`、`route == tool` 和 `exposure == visible` 的能力可以进入
`ModelRequest.tools`。`deferred` 必须先经过显式发现或提升，`internal` 与 `hidden`
可以由 Core/Host 内部编排，但不得因配置中存在同名 schema 而暴露给模型。Host
注册了执行器不等于模型有权调用它；模型规格与运行时执行器必须分离。

跨进程或远端 Host 的 Catalog 还携带 lease。Core 校验 lease id、heartbeat 和 epoch
的单调关系，但不读取系统时钟；Host 负责计时，并在 lease 过期时提交
`host_disconnected`。进程内 Host 可显式使用无 lease 模式。

模型配置与模型运行实例也遵守 Catalog 边界。Store 的会话设置只保存稳定选择引用，Host
本地配置只保存非敏感端点/参数和安全凭据引用，安装目录只描述可验证 artifact；三者都不能
证明模型已经运行。只有 Host Catalog 中带 instance id、实际 context window、availability
epoch 和 heartbeat 的模型能力可以进入新 Turn。选择模型、打开模型中心或恢复会话不得隐式
启动运行时，也不得把旧运行快照恢复成“可用”。

### 8.1 Effect 状态机

```text
dispatched -> accepted -> streaming -> terminal(remove)
     |            |           |
     +------------+-----------+-> cancelling
                                      |
                                      v
                         cancellation acknowledged
                         (remove target + cancel, Turn -> cancelled)
```

- `Accepted` 只确认 Host 接管，不代表操作完成。
- delta/progress 将 effect 推进到 `streaming`，但不创建新的 effect。
- terminal response 在归属与 sequence 校验后移除 effect，再驱动 Turn 状态转换。
- cancelling target 的迟到 delta 不得重新写入 Part；迟到 terminal 只负责回收。
- Cancel effect 必须返回 target effect id，Core 同时清理 cancel 与 target。
- `TurnCancelled` 是取消握手完成后的事实，不得在仍有 cancel effect 时提前投影。
- `AgentHostDriver` 只有 `dispatch/dispatch_batch`；取消不再是 trait 旁路。

macOS 可以拥有 shell、文件、Chrome 扩展、辅助功能和文档技能；iOS 可以拥有
HealthKit、日历和提醒事项；Android 可以拥有 Compose/系统数据源。能力集合不同，
Agent 语义不应分叉。

### 8.2 工具调度

工具调度是 Core 的纯逻辑，不属于 CLI、macOS 或远程 Host：

1. 保留模型给出的工具顺序。
2. 连续且由 Host 明确声明为 `concurrent_safe` 的工具组成一个并发批次。
3. 写操作、未知能力及没有并发声明的能力一次只调度一个。
4. 运行中的工具和未决权限构成调度屏障。
5. Host 未发布或已不可用的工具不会下发 I/O，而是生成可见的失败 Tool Part。
6. 一个批次结束后，Core 再规划下一批；Host 不得自行重排工具调用。
7. 会改变后续规划契约的能力（例如加载 Skill 指令）是事务屏障。同一模型批次只先
   激活该能力，待指令进入上下文后重新规划，不执行同批次中基于旧契约生成的调用。

Core 将批次整体交给 `AgentHostDriver.dispatch_batch`。默认 Host 可以串行执行；
只有能力明确声明为 `concurrent_safe` 且平台执行器具备隔离条件时才能真正并发。

### 8.3 工具结果与 artifact

工具结果分为三个层次：

1. `output_preview`：按 Catalog 字符预算生成，供 Presenter 直接展示。
2. `full_output_ref`：Host 持久化完整输出后的稳定引用。
3. `artifacts`：文件、截图、报告等带 URI、名称、MIME 和操作类型的产物。

模型历史保留结构化小结果；超过预算时只注入截断预览，不把整份大文件或搜索正文
塞回上下文。Presenter 从 Tool Part 读取预览和 artifact，不读取 Host 私有日志。

完整输出续读能力按需激活，而不是常驻在首轮模型工具目录中。只有 Host 实际持久化
大输出并返回非空 `full_output_ref` 时，工具结果才携带 typed capability activation，
让 Core 在下一 Step 暴露 list/read/search/find 等续读 schema。模型必须原样使用该
引用；网页 URL、来源链接、工具名或推测路径都不是合法引用。普通内联结果不会激活
续读能力，避免内部控制工具干扰正常检索与回答。

这吸收了 Claude Code 的“只并发安全只读工具”机制，同时避免把具体工具名称或
平台规则写进 Core。CLI 的只读判断只用于构建自己的能力目录。

## 9. 从参考项目吸收的机制

### Codex CLI

- 稳定 Submission/Event 协议。
- Session 与 Turn 分离。
- 输入 mailbox、取消和工具生命周期。
- 工具能力声明并发、取消等待、参数差量和暴露方式，运行时服务不进入 reducer。
- 总窗口、Prompt 上限与自动压缩阈值分离。

### OpenCode

- Message Part 作为 UI 与存储的共同投影。
- 工具状态原位更新。
- 最后匹配权限规则和动态工具目录。
- 工具输入先建立 pending Part，再进入 running/completed/error；参数与正文都按
  稳定 Part ID 增量更新。

### Claude Code

- Abort 生命周期。
- 保留最近尾部的压缩边界。
- 压缩失败回滚与恢复。
- 连续只读且明确并发安全的工具可以批量执行，写操作保持串行。

### Open WebUI

- 远程 Host 的关联信封。
- 心跳、请求/响应分离和断线恢复。
- 分布式连接维护反向索引和清理边界，不能依赖进程内“仍然在线”的隐式假设。

这些机制只作为架构参考；Nola 保留自己的协议、数据模型和原生平台实现。

## 10. 当前实现状态

已完成：

- 单一 `AgentRuntime` 取代公开 Thread wrapper 与内部 reducer 双层状态机。
- Session-owned `AgentInputQueue` 明确 current-turn 与 next-turn 边界。
- `PartUpdated` 是文本、推理和工具生命周期唯一公开增量事件。
- private `RuntimeEvent` 隔离内部 reducer signal 与跨平台协议。
- Host effect 使用显式状态、完整 owner tuple、lease 与单调 response sequence。
- 所有取消通过 `HostRequest::Cancel` / `CancellationAcknowledged`，无 driver 旁路。
- replay 保留 effect identity、创建新 request identity，禁止 catalog 静默改绑。
- 公共 FFI mutation 输入改为 `AgentSubmission`。
- CLI 与 `nola-agent-host` 共用 Core Host loop。
- CLI Host 的模型、工具、权限、checkpoint 与取消适配。
- CLI 与 Web Host 公开 Core v2 稳定 Part；客户端按 `part_id` 原位投影，不再把
  非规范 delta 当作唯一正文来源，也不重复持久化 Host 已提交的 assistant 消息。
- Skill 激活建立重新规划屏障，避免模型在读取 Skill 指令之前并行执行后续工具。
- 确定性工具预检发生在权限展示之前；应改用专用工具的 Bash 调用直接返回可恢复
  协议反馈，不弹出无意义的权限面板。
- 验证与失败恢复没有固定轮次上限；运行由预算、取消、超时和明确终态约束。
- 手动压缩在没有可摘要历史时返回成功的 no-op 结果，保留完整 transcript 与当前
  checkpoint，而不是伪造摘要或报错。
- pending Host 请求进入完整 checkpoint，可在 CLI 重放。
- 工具调用与完整工具结果写入只追加 transcript；模型可见预览写入 model context。
- 工具参数 delta、输出预览、完整输出引用和 artifact 进入统一 Tool Part。
- Host Catalog 支持运行时更新、lease/heartbeat、断连 epoch 和按 replay safety 恢复。
- FFI 提供 checkpoint restore 与 pending Host request 查询。
- iOS 与 Android 已具备 v2 原生桥和 DeviceAgentCoordinator，可提交 Submission、
  读取 Snapshot、恢复 checkpoint 并查询 pending Host requests。
- Core 规划并发安全工具批次，Host trait 接收完整批次。
- CLI 持久 Session 以及 Permission、Skill、MCP、Memory、DataSource、Automation、Runtime
  Task 适配器通过本地 Application Service 提交类型化命令；客户端不再直接打开 canonical
  Store。Runtime Task 的租约、心跳、重试、完成、过期恢复和事件游标属于 Application
  事务，而不是 Host 进程内状态。

后续实现工作：

- 删除 iOS、Android presenter 中残留的 provider 直写与重复 Part 状态。
- 收束 macOS 本地/远端 adapter，确保都只映射 Agent Host 协议，不维护平台自有 Agent loop。
- iOS/Android Presenter 消费 v2 ConversationProjection，删除本地重复 Part 状态。
- Host 持久层实际落地 `full_output_ref` 与 artifact URI 的生命周期管理。
- 持续扩展 Core 契约、Host 适配和跨平台真实端到端恢复场景；full 覆盖审计必须
  对 Core、CLI、Web、macOS、iOS、Android 与 Backend 均无缺失模块。
## Capability and permission ownership

`HostCapabilityDescriptor` is the authoritative runtime capability definition.
It owns availability, exposure, route, input schema, normalization, output
policy, parallelism, cancellation, replay safety, and permission domain. Core
projects the model-visible function schema from that descriptor, so the model
and executor cannot silently drift onto different argument contracts.

Core also owns permission matching. A domain such as `filesystem.write` is
only the explanation shown to a user; authorization rules are evaluated and
checkpointed against the concrete capability and resource, such as a target
path, command, URL, or query. Hosts collect the user's decision and execute an
approved effect, but must not downgrade conversation approval or maintain a
second competing authorization policy.

## Host-declared per-Turn capability budgets

A Host can attach a `turn_budget` contract to capability normalization metadata. Related capabilities share a semantic budget group, so Core counts the accepted calls rather than inferring limits from tool names or user wording. The contract supports an explicit argument-based expansion and an optional `finalize_on_exhaustion` boundary. When that boundary is reached and the admitted tool batch finishes, Core enters its tool-free evidence-synthesis step; it does not merely hide schemas and leave the model in an executable planning loop.

The built-in external retrieval group currently shares one budget across `web_search` and `web_fetch`: ordinary research permits five calls, while an explicit `research_mode: academic` selection expands the group to 24. The ordinary limit leaves room inside the product E2E ceiling for time acquisition and completion/recovery control calls. The expansion is evaluated from validated tool arguments and frozen Host metadata, never from language-specific prompt matching.

## Post-tool final-answer boundary

A model step that emits one or more tool calls is a private execution step. Any assistant prose emitted in that same step is retained only as a provisional draft for recovery and must not become the public assistant body. After the last tool result, Core requests a tool-free, read-only synthesis from the original request and flattened evidence. The Host and Presenter may render reasoning and tool Parts, but they must not manufacture a second answer or promote a private draft.

Final synthesis must enforce the original request's inclusion, exclusion, eligibility, and output-format constraints. An item the user asked to omit cannot reappear as an explanation, rejected alternative, source link, or internal evidence summary. Pseudo tool-call markup is not a valid final answer. Repeated empty output, private-draft replay, or pseudo-protocol replay is treated as no progress; after three identical no-progress attempts Core terminates the Turn with an explicit failure instead of looping indefinitely.
## Remote transcript transaction boundary

Remote transcript replication and Core recovery are deliberately separate:

- `begin_remote_turn` creates stable User/Assistant Message, Part and Run facts without a checkpoint;
- `commit_remote_transition` applies ordered Run events and Part mutations without touching local
  Core recovery state or an effect outbox;
- `save_remote_session_mirror` persists the authenticated Host's complete Core projection by
  monotonic cursor;
- only a locally owned Core transition may write `core_checkpoints`.

Using placeholder JSON such as `{"state":"running"}` as a checkpoint or using checkpoint revision
as a remote streaming CAS is forbidden. Store schema 11 keeps remote transition idempotency in a
separate ledger and also owns canonical memory, permission, Skill, MCP, data-source, task and
automation records.

User-input acceptance, rather than the first provider token or provider completion, is the
persistence boundary. Every external execution backend uses `begin_remote_turn` before launching
the executor. Desktop clients provide stable `submission_id`, `user_message_id`, and
`assistant_message_id` values; Agent Host preserves those identities. Completion, failure, and
cancellation update the original Assistant Part and Run through `commit_remote_transition`; they
must not append a reconstructed User/Assistant pair after execution.

The presentation layer may pace semantic stream segments, render partial Markdown, and attach
transient activity state, but it does not own the transcript. At terminal state it performs one
canonical reconciliation that preserves presentation-only attachment state. Multiple terminal
reloads or competing reconciliation strategies are prohibited because they can erase a
just-accepted Turn or replace streamed content with an older snapshot.

Regression coverage must prove that provider failure before the first token retains the stable
User and failed Assistant messages, successful completion creates no duplicate identities,
process reopen restores the same terminal content, and earlier messages retain their order. Real
external-executor E2E additionally verifies the Mac-to-Agent-Host identity boundary.

The scheduler, native UI and Agent Host must open the same platform database identity. On macOS
that identity is `MacAppEnvironment.databaseURL` (`State/nola-v2.db` in V2 production). LaunchAgent
plists, receipts and logs are runtime artifacts; automation and data-source product facts exist
only in the Store. Definition edits flow
through Application commands, and schedulers read the resulting Store projection.

## Automation occurrence ownership

Store schema 12 separates an automation definition from each concrete occurrence. The canonical
`automation_runs` row owns the occurrence key, state, attempt, lease owner, lease expiry, retry
eligibility, heartbeat, terminal timestamps and structured result. Core/Application are the only
layers allowed to apply these transitions.

- BGTask, WorkManager, launchd and HTTP endpoints are wake sources, not execution ledgers.
- A Host may execute an occurrence only after an atomic Application claim.
- A live Host heartbeats the same lease and commits one terminal result or an explicit retry wait.
- A recovered process claims the same occurrence after lease expiry; it does not create a new run.
- `automation_results`, Room rows, JSON history, activity rows, receipts and logs are projections,
  diagnostics. They never decide whether work should run.
- UI reads the canonical run projection and may link diagnostics as attachments; it does not merge
  independently inferred platform status into product state.

This boundary makes duplicate platform wakeups harmless and preserves occurrence history across
process death, client restart and scheduler replacement.

## Desktop persistence boundary update (2026-08-20)

macOS and CLI use `nola-service` as the only product-fact writer. Archive/metadata fallback stores and importers do not exist. CLI background processes are canonical Runtime Tasks. Host resource files such as model weights, Chrome native-messaging manifests, security-scoped bookmarks and automation runner artifacts remain Host-owned resources, not duplicate product databases.
