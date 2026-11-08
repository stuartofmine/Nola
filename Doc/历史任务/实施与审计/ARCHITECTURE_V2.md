> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Nola Architecture V2

状态：Accepted，作为 V2 重构与验收基线\
适用范围：macOS、iOS、Android、CLI、Web、Agent Host、Application Service、Core、Store\
兼容策略：产品尚未发布，采用硬切换，不保留旧协议、旧事件或旧数据库兼容代码\

当前工作区的入口、依赖、事务与实施顺序核对见
[`V2_CURRENT_ARCHITECTURE_2026-09-08.md`](V2_CURRENT_ARCHITECTURE_2026-09-08.md)。
本次全局依赖、封装和平台职责审计见
[`ARCHITECTURE_REVIEW_2026-09-12.md`](ARCHITECTURE_REVIEW_2026-09-12.md)。
可直接约束文件布局、模块输入输出和端口定义的规范见
[`ARCHITECTURE_V2_MODULE_CONTRACTS.md`](ARCHITECTURE_V2_MODULE_CONTRACTS.md)；Rust、Swift、Kotlin
实现与评审规则见 [`../../软件架构设计/工程规范/CODING_STANDARDS_RUST_SWIFT_KOTLIN.md`](../../软件架构设计/工程规范/CODING_STANDARDS_RUST_SWIFT_KOTLIN.md)。
目标约束与历史问题描述不等于当前实现状态；整体 V2 验收仍未完成。

## 1. 决策摘要

Nola V2 采用一个明确的运行链路：

```text
Platform Client
    -> Typed Client SDK
    -> Nola Application Service
    -> Pure Agent Kernel
    -> Runtime Supervisor
    -> Provider / Tool / Skill / MCP / Platform Adapter
    -> SQLite WAL Store
    -> Revisioned Projection Stream
    -> Platform Client
```

核心决策：

- Application Service 是设备内唯一业务事务入口和唯一持久化写入者。
- Agent Kernel 是纯状态转换内核，不访问数据库、网络、文件、进程、钥匙串或 UI。
- Runtime Supervisor 管理 Run、Attempt、取消、恢复、租约和 Effect 调度，但不能直接写 Store。
- Agent Host 只执行 Application Service 已提交的 Effect，并返回带身份和序列的结果。
- 客户端只提交 Command、订阅 Projection，不直接拼接或持久化规范消息。
- 完整会话、模型上下文、长期记忆是三个不同的数据模型。
- 流式 delta 是可丢弃的实时预览，完整 Part 是唯一规范内容。
- Tool、Skill、MCP 和平台能力统一进入同一个 Capability 与 Permission 协议。
- 不按用户文本关键词决定工具路由或权限，模型选择具体工具，Core 评估具体调用。

## 2. 为什么需要 V2

当前代码已经存在 Core、Application、Store、Service 和 Host 等命名上的层次，但真实所有权仍然分散：

- Mac UI 同时连接 Application Port、本地 Agent Host、Remote Host 和本地仓库。
- iOS 与 Android 仍保留各自的 Prompt、Tool Catalog、权限和流事件归并逻辑。
- CLI 既是客户端，又包含 Web、Agent Host、工具执行、会话、权限、任务和部分 Store 访问。
- Application Service 主要是同步 execute RPC，没有持续 Projection 订阅。
- 长会话以完整 JSON 快照跨进程传输，产生高延迟、大量内存复制和帧大小限制。
- delta、完成事件、持久化消息和 UI 缓冲之间缺少严格的身份与替换语义。

这些结构性问题会重复产生以下症状：

- 助手回复重复、遗漏或覆盖旧消息。
- 思考过程、工具调用和正文顺序不稳定。
- Markdown 在完成时整块替换，导致闪烁和位置跳动。
- 权限已批准后仍反复弹窗。
- 每轮显示正在启动环境，首 token 延迟高。
- 长任务失去进度、取消或恢复能力。
- 长会话和多窗口切换造成 Service 与客户端内存膨胀。
- 各平台功能、本地化和行为逐渐分叉。

V2 不再逐项修补这些表面症状，而是消除多写入者、多投影器和多运行时所有权。

## 3. 架构不变量

以下约束必须由类型、依赖规则和测试共同保证。

### 3.1 单写者

- 只有 Application Service 可以打开 Store 的读写连接。
- Client、Agent Host、Provider、Tool、Skill、MCP 和 Platform Adapter 不得写 Store。
- 同一个 Run 在任何时刻只有一个有效 sequencer。
- 同一个 Effect 在任何时刻只有一个有效 lease owner。
- 所有业务写入必须经过 Application Command。

### 3.2 单一消息真相

- Message 与 ContentPart 是规范会话内容。
- UI 数组、Markdown AST、AttributedString、SSE 缓冲和 provider delta 都不是规范数据。
- 完成态正文只能提交一次。
- retry 或 provider fallback 必须创建新的 Attempt。
- 新 Attempt 不得复用旧 Attempt 的 partial buffer。

### 3.3 可重放投影

- 客户端 Projection 必须能从 Snapshot 加 Event suffix 重建。
- Event 使用会话内单调 sequence。
- Reducer 只能按稳定 ID upsert，不得按数组最后一项猜测目标。
- 重复 Event 必须是幂等 no-op。
- 乱序 Event 必须缓存、补洞或拒绝，禁止归入当前消息。

### 3.4 有界资源

- 对话历史分页加载，禁止默认加载完整会话。
- 流式事件按帧合并，禁止每个 token 触发整棵 UI 更新。
- 工具输出、进程日志和网络正文必须有内存上限。
- 超限内容写入 Artifact/Blob，事件只携带摘要和引用。
- Markdown 只更新可变尾块，稳定块不得重复解析。

## 4. 组件边界

### 4.1 `nola-domain`

负责：

- 稳定 ID 与 Value Object。
- Conversation、Message、ContentPart、Run、Attempt、Effect、Artifact 等领域类型。
- 状态枚举、错误码和跨平台语义。

禁止：

- IO、异步运行时、数据库、HTTP、进程和平台 API。
- Swift、Kotlin、CLI 或 Web 专用展示字段。

### 4.2 `nola-kernel`

负责纯函数式决策：

```text
State + Command/Event -> NewState + DomainEvents + Effects
```

负责：

- Agent turn 状态机。
- Tool call 生命周期。
- Permission policy 评估。
- Context plan 与 compaction plan。
- 无进展检测和预算控制。
- Memory 候选与使用策略。

禁止：

- 直接执行 Tool。
- 直接请求 LLM。
- 读写 Store。
- 生成本地化 UI 文本。

### 4.3 `nola-application`

负责：

- 用例与事务边界。
- 加载 Aggregate、调用 Kernel、提交结果。
- 创建 Run、Attempt、assistant placeholder 和 Effect outbox。
- 接收 Host Result 并进行幂等提交。
- 生成 Snapshot、Projection 和增量 Event。
- 调度长期任务、同步、记忆维护和数据源刷新。

Application 依赖 Store Port 和 Host Port 接口，不依赖具体 SQLite、HTTP 或平台实现。

当前迁移进度（2026-09-08，尚未满足完整依赖目标）：`nola-store-port` 已覆盖实际 Application 用例所需仓储契约。
Application Runtime、命令分发、收据与投影通过聚合仓储接口执行；SQLite 上传适配器负责路径、延迟打开和连接缓存。
主库与上传适配器现由独立 `nola_application_bootstrap` 装配，Service、移动嵌入式服务和 CLI 临时会话使用同一装配入口。
Application 已删除生产路径打开/重建 API，Runtime 为 `ApplicationRuntime<R: ApplicationRepositories>`；
具体 `NolaStore` 由 bootstrap 装配，Application 的 Store/SQLite 依赖仅用于测试。
Application 仍依赖旧 `nola_core` 和 Host Protocol，完整纯 Kernel 与 Host Port 边界尚未完成。
实际调用和验证范围见 [当前架构核对](V2_CURRENT_ARCHITECTURE_2026-09-08.md)。
上传接口验收包含非 SQLite 仓储注入、查询不创建数据库、损坏文件不覆盖，以及真实 Service 的分块上传和重启恢复；
服务端性能测试不能代替各客户端 UI 与真实模型的端到端验收。

记忆命令、后台记忆整理的输入读取、上下文检查点命令也已依赖独立仓储接口；记忆检索的 SQL 转义由 SQLite 适配器处理。
上下文提交必须先确认 checkpoint 的会话归属写入成功，再替换组件；跨会话 ID 冲突与组件冲突均需完整回滚。
Mac 归档数量原先通过 `LoadCheckpoints` 加载全部组件后计数，现已替换为 `CountCheckpoints` 元数据计数；旧的全量查询入口已删除。
单个检查点仍按 ID 读取。若后续增加检查点历史浏览，应提供有界元数据分页，而不是恢复全量组件列表。
Mac 计数现通过后台任务读取，视图只访问结果状态；可见归档区域在会话切换和运行状态变化时刷新，不随每个流式 delta 查询。
会话 ID 与请求版本共同阻止迟到结果覆盖当前会话；失败与真实零值区分。完整 Mac 编译、截图与交互验收仍待完成。

### 4.4 `nola-store-sqlite`

数据源生命周期用例现通过 `DataSourceRepository` 执行，存储记录类型由 Store Port 定义。
文档索引用例也通过 `DocumentSourceRepository` 执行；Application 在替换前验证 scope/kind 和输入身份，
SQLite 适配器继续负责分区替换事务与检索实现。该拆分没有移除文档全量 List 接口，文档大正文分页仍需后续处理。
文档 Replace 命令的成功回执现只包含提交数量，不回传全部正文；CLI/Web 调用方使用该数量契约。
底层替换事务提交后直接返回输入数量，不再执行全量 List；测试用只允许写入、拒绝正文读取的 SQLite 视图验证这条路径。
输入正文仍通过内联命令传输，不能将有界回执和消除回读等同于分块导入已经完成。
文档搜索摘要的 metadata 最多返回 4096 个编码字节；超限返回空对象并显式设置 `metadata_truncated`。
限制在 SQL 投影中应用，不先把大 metadata 加载到 Rust 再截断；完整读取仍保留原始 metadata，不修改数据库记录。
CLI/Web 的摘要 JSON 使用 `metadataTruncated` 传递此标记。该限制不等于完整文档 List 已有界化。
Web 上下文详情、设置与原生桥接使用同一个数据源权限投影，不得直接把内部 `uploaded_documents` 返回客户端。
受管资源的所有者和正整数授权版本必须能被明确解析；缺失、损坏或无效时拒绝访问，不得默认归属于当前用户。
所有权只能在资源创建流程中显式赋予，读取和权限判断不能补写或猜测所有者。
Web 文档 UI 投影省略正文并返回 `content_characters`、`content_omitted`；列表不再依赖正文计算字符数。
保存已有文档时，缺失 content 表示保留服务端规范正文，显式 content 仍作为更新输入验证；派生字段不写回。
模型上下文使用独立的内容读取权限投影，不因 UI 摘要化而丢失原文。服务端仍会加载完整文档集合，
文档正文按需读取、分页及元数据专用存储命令仍是后续工作，不能将浏览器响应缩小视为整个链路已经有界。
2026-09-08 的真实浏览器验收通过 `scripts/test_web_document_browser_e2e.py` 分别执行 en-US、zh-CN：
隔离 CLI Web 与 Application Service，通过接口创建 200 万字符文档，在页面保存并刷新重开，
保存请求为 456 字节、不携带正文；只读数据库核对规范原文完全一致，页面 JavaScript 错误为空。
该用例不调用 LLM，也不证明模型上下文装配、Mac UI 或千万字符消息恢复通过。
两组截图人工查看仍发现未通过项：英文上下文建议包含服务端生成的中文文案，中文“移除”按钮发生逐字换行。
右侧显示 `0 tokens`、窗口为 1；此用例没有配置真实模型或提交推理请求，尚不能确定该值的来源和有效性，
需要覆盖未知窗口、待估算、已有实际 usage 等状态，不能把占位值展示为已确认的精确统计。
上下文建议现传递稳定的语义标识与数值参数，由 Web 客户端本地化；旧文案拼接和回退已删除。
12 种建议的协议测试、中英文浏览器保存用例及截图已覆盖警告语言与“移除”按钮单行布局。
窗口为 1 的来源已定位到 Web context 路由对未配置值 0 应用 max(1)；现保留未知值，
右侧未知窗口不显示百分比，也不再宣称统计“精确”。这一修复不代表已完成模型窗口解析、
请求前估算与请求后实际 usage 的统一来源；这些状态仍需要独立的跨会话验收。
Web 不再把没有模型窗口的空 Application 用量投影当作已记录请求，避免零值覆盖准备阶段估算；
已知窗口下的真实零值仍保留。预留输出取 Core 的预留值，不把安全余量误算成输出。
上下文建议不再以原始文档长度乘固定比例承诺可回收 tokens；该数字并非压缩计划或执行结果。
报告传递 Core 的 preparation action/reason，Web 区分无法容纳与需要摘要，不把所有超限都视为历史压缩可解决。
Web 请求前的工具估算复用 Agent Host 的初始曝光规则，只计首轮可见 schema；
Deferred 工具仍在完整可用目录中，由模型通过发现与激活机制使用，权限规则不改变可用目录。
这修复的是预览把完整目录当作本轮请求的口径错误，不代表已验证后续发现工具后的实际请求预算。
SQLite 适配器使用原连接及外层事务，不另开写入连接。上下文解析通过批量接口读取数据源与会话覆盖，
不再逐条查询覆盖设置；独立调用使用读取快照，已有事务内的调用保留调用者的事务所有权。
Application 的具体 Store 生产依赖后续已由 bootstrap/Store Port 切分，当前边界见 §4.3；
该进展不代表文档正文读取和全部客户端链路已经有界化。
验收要求包含跨会话覆盖、继承、重启、非 SQLite 仓储以及两次读取之间发生并发提交时的快照一致性。

大文本导入已接通规范正文引用发布、有界读取、ACK 依赖复制和 Mac 分阶段原子归档恢复；
当前实现及专项证据见 [当前架构核对](V2_CURRENT_ARCHITECTURE_2026-09-08.md)。
专项通过不能替代 §20 的五端千万字符、多窗口和完整 UI/性能验收。

负责：

- SQLite WAL 持久化适配。
- Repository 实现。
- 事务、索引、分页、cursor 和租约原语。
- 有界 Blob/Artifact 元数据。

禁止：

- Tool 路由、权限决策、Prompt 组装和 Agent 循环。
- 返回平台 UI 模型。

Store 可以使用同一个数据库文件，但必须按 Repository 拆分模块，不能继续由一个八千行 `NolaStore` 承担全部领域。

删除会话用例通过 `ConversationDeletionRepository` 提交，不能先读取活动 Run 后再独立删除。
Store 必须在同一写事务内检查非终态 Run、删除会话及其从属记录、发布同步 tombstone；
维护型 Run 没有助手消息也必须阻止删除，未知 Run 状态按非终态处理。
`conversation_deletion_e2e` 覆盖另一写入者提交 Run 后的拒绝、终态删除、重复删除和发布失败回滚。
这仍不代表全部会话用例或整个 Application 已解除具体 Store 依赖。

创建会话、重命名、设置更新和标题/设置联合更新通过 `ConversationProfileRepository` 提交。
输入与结果契约归 Store Port 所有，SQLite 适配位于独立的 `conversation_profile` 模块；
Application 保留标题规范化、JSON 对象与时间校验，不依赖 SQLite 实现这些用例。
联合更新的失败回滚测试覆盖标题、设置、revision 和同步发布，不能拆成客户端两次保存。

### 4.5 `nola-runtime`

负责：

- 每会话最多一个活跃 Run。
- Provider stream、Tool future、后台进程和取消句柄。
- Attempt 隔离。
- Effect lease、heartbeat、timeout、retry 和 recovery。
- 有界 channel 和 backpressure。

Runtime 只通过 Application Port 提交结果，不能直接持久化业务状态。

### 4.6 `nola-host`

负责：

- 执行模型、文件、Shell、浏览器、Skill、MCP 和平台 Effect。
- 将供应商协议转换为统一 Host Event。
- 执行 OS 权限检查和 security-scoped resource 访问。
- 生成 Artifact 引用和用户可见的结构化摘要。

Host 不拥有 Conversation，不重放完整历史，不拼接最终助手消息。

### 4.7 Platform Client

负责：

- 提交 Command。
- 订阅 Projection Stream。
- 本地交互状态、窗口状态和动画。
- 原生 SwiftUI、Compose、Web 或 TUI 渲染。
- 展示系统权限入口。

禁止：

- 自行运行 Agent loop。
- 自行决定 Tool 路由。
- 自行合并 provider delta 与最终消息。
- 直接写 canonical Store。

## 5. 依赖方向

允许的依赖：

```text
nola-domain <- nola-kernel
nola-domain <- nola-protocol
nola-domain + nola-kernel + ports <- nola-application
nola-application ports <- nola-store-sqlite
nola-application ports <- nola-runtime
nola-runtime ports <- provider/tool/platform adapters
nola-protocol <- nola-client-sdk
nola-client-sdk <- platform clients
```

禁止的依赖：

- Client SDK 依赖 Application 实现。
- Kernel 依赖 Host Protocol 实现。
- UI 依赖 Store。
- Tool 依赖 UI。
- Store 依赖 Application。
- Platform Client 复制 Kernel policy。

`nola-service-client` 必须只依赖协议和 DTO，不得依赖整个 `nola-application` crate。

## 6. 进程拓扑

### 6.1 macOS

```text
NolaMac UI
    -> Application Client
    -> local Application Service
    -> long-lived Runtime Supervisor
    -> local Agent Host adapters
```

- Application Service 随 App 启动后保持存活，不按命令反复创建连接和进程。
- Agent Runtime 按会话保持状态，不按对话轮次重启。
- Codex、Claude Code、本地模型和远程模型是 Execution Backend，不是另一套会话系统。
- Remote Host 通过相同协议接入，UI 不直接区分本地与远端事件格式。

### 6.2 CLI 与 Web

```text
nola CLI / nola web
    -> same Application Client protocol
    -> embedded or daemon Application Service
```

- Embedded、daemon 和 remote 必须复用相同 Command、Event 和 Projection reducer。
- CLI 不得因为嵌入模式而获得额外 Store 写入路径。
- Web Host 只是 transport 与认证入口，不重新实现 Agent Runtime。

### 6.3 iOS 与 Android

- 移动端可以将 Application Service 以内嵌库形式运行。
- 内嵌模式仍必须经过同一 Application Command 和 Projection 协议。
- HealthKit、Health Connect、联系人、日历和通知作为 Platform Adapter。
- 移动客户端不得保留独立 Prompt Composer、Tool policy 或 Permission reducer。

## 7. Canonical 数据模型

### 7.1 Conversation

```text
id
title
createdAt
updatedAt
activeBranchID
settingsRevision
```

### 7.2 Message

```text
id
conversationID
parentMessageID
runID
role
status
createdAt
completedAt
```

### 7.3 ContentPart

Part 类型：

- text
- reasoning
- toolCall
- toolResult
- file
- artifact
- citation
- status
- error
- compaction
- subtask

公共字段：

```text
id
messageID
ordinal
kind
status
revision
contentRef/content
createdAt
updatedAt
```

### 7.4 Run 与 Attempt

```text
Run: id, conversationID, state, activeAttemptID, sequence, budget, timestamps
Attempt: id, runID, provider, model, state, failure, usage, timestamps

Attempt failure is a typed, bounded user-presentable summary (`code`,
`message`, `retryable`). Raw provider payloads remain only in the diagnostic
Trace. Application joins Attempt chains to their canonical assistant Message
and emits a lightweight `model_attempt` projection Part for failed attempts,
including the successor provider/model when fallback occurred. This is a
query-time fold of the Attempt ledger, not a duplicate MessagePart write and
not reconstruction from retry log strings.
```

Run 状态：

```text
queued
running
waitingPermission
waitingUserInput
waitingToolResult
paused
cancelling
completed
failed
cancelled
```

### 7.5 RunEvent

```text
eventID
conversationID
runID
attemptID
messageID
partID
callID
sequence
kind
payload
timestamp
```

RunEvent 用于顺序、恢复、审计和增量投影。规范实体表与 Event 必须在同一事务提交，Event 不能先于事实发布。

## 8. Command 与 Projection Protocol V2

### 8.1 Command Envelope

```json
{
  "protocolVersion": 2,
  "requestID": "stable-id",
  "clientID": "client-id",
  "expectedRevision": 42,
  "command": {}
}
```

- `requestID` 必须写入持久化 receipt。
- 相同 `requestID` 重试必须返回原结果，不能重复执行。
- revision 冲突返回结构化 conflict，不进行隐式覆盖。

上述持久化 receipt 约束针对业务写命令。Projection 查询不进入写命令
admission，不创建 receipt，也不缓存结果以重放；即使复用 requestID，查询
仍返回当前已提交状态。响应封装中的 requestID 只用于关联请求。
`projection_snapshot`、`projection_event_suffix`、会话窗口及条件窗口更新、
上下文用量和权限投影读取采用此规则。查询禁止携带 expectedRevision；
窗口增量使用 event sequence，历史分页使用 message ordinal，两者不可混用。
只读资格必须由服务端明确的查询类型决定，客户端不能提交 readOnly 标志来
绕过写入幂等与权限检查。

### 8.2 Projection API

必须提供：

```text
listConversationSummaries(pageCursor, limit)
loadConversationWindow(conversationID, anchor, before, after)
loadRunSnapshot(runID)
subscribe(scope, afterSequence)
loadEventSuffix(scope, afterSequence, limit)
```

禁止通过单个完整 JSON 响应传输整个长会话。

### 8.3 Snapshot 与 Event suffix

重连顺序：

```text
subscribe(afterSequence)
loadSnapshot()
apply buffered events newer than snapshot.sequence
continue live events
```

客户端 reducer 必须验证 scope、identity、revision 和 sequence。

## 9. 流式消息协议

### 9.1 提交用户输入

一个事务内完成：

```text
create user Message
create assistant placeholder Message
create Run
create Attempt
append RunQueued event
```

因此用户按下发送后，助手气泡立即出现，不需要等待首 token。

### 9.2 Delta 与完整 Part

- Provider delta 绑定唯一 `attemptID + partID`。
- delta 只更新内存 projection 或低频 chunk checkpoint。
- token 级 delta 不逐条写 SQLite。
- `PartCompleted` 携带同一 `partID` 的规范全文。
- 客户端收到完成事件时替换该 Part 的 preview，绝不 append 全文。
- 完成事件到达后，所有旧 revision delta 都必须丢弃。

### 9.3 Markdown

Markdown 渲染采用：

```text
raw source
    -> streaming block parser
    -> stable blocks + mutable tail
    -> native block views
```

- 标题、段落、列表、引用、表格和代码块获得稳定 block ID。
- 已稳定 block 不重新解析、不重新创建 View identity。
- 未闭合代码围栏立即显示为活动代码块。
- mutable tail 在语义边界批量推进。
- “水底浮现”动画只应用于新稳定的语义短语或 Block，不应用于已有内容。
- 完成态只关闭 mutable tail，不替换整棵 Markdown 视图。

## 10. Tool、Skill 与 MCP

三者统一为 Capability Provider：

```text
CapabilityDescriptor
ToolSchema
Invocation
PermissionRequirement
ProgressEvent
Result
Artifact
```

统一调用链：

```text
Model tool call
    -> Kernel validates bound catalog
    -> Permission evaluates concrete invocation and resource
    -> Runtime dispatches Effect
    -> Provider executes
    -> ToolPart transitions state
    -> Result returns to model context
```

ToolPart 状态：

```text
pending -> waitingPermission -> running -> completed/error/cancelled
```

Provider streaming adapter 必须把 `function.arguments` 字符串 delta 作为原始参数字节顺序拼接，
不能对每个分片再次 JSON 编码。同一 provider slot 的非空 `call_id` 一旦建立不得改变；身份冲突或
未终止的参数流必须 fail closed，不能形成可执行 ToolCall。工具投影来自 Core attested catalog，Host
不得根据用户提示的语言或“搜索”“当前”等词语增删工具。
Provider 返回当前 Bound Catalog 之外的 capability ID 时，Host 必须保留原始 call ID、名称和参数并交给
Core 的 typed unknown-tool/catalog-refresh 恢复；Host 不得先改名、丢弃或降级为通用 provider failure。
参数结构损坏仍必须在执行前 fail closed。

Skill 是带说明、资源和可选脚本的 Capability package，不是第二套 Agent。

MCP 工具转换为相同 Tool contract，不建立独立权限或 UI 事件路径。

每轮可见工具集是不可变 Bound Catalog；下一轮重新生成。历史只保留已调用工具的规范名称、参数摘要、结果摘要和 Artifact 引用。

## 11. 权限模型

权限只评估具体调用：

```text
toolID
operation
normalizedResource
hostID
workspaceID
conversationID
riskClass
```

权限范围：

```text
once
conversation
workspace
persistent
```

默认策略：

- 只读且在已授权工作区内的操作默认允许。
- 可逆写入使用用户选择的权限模式。
- 删除、覆盖、外发敏感数据和不可逆操作需要确认。
- OS 权限与 Nola policy 权限分开表达。
- OS 已授权不能被错误展示为 Nola 未授权。

所有产品端必须提供同一组三态权限模式，wire value 固定为 `safe`、`recommended`、`allow_all`，
缺省为 `recommended`。Safe 对宽泛操作不预授权；Recommended 允许普通可逆操作，但由 Core 对具体
破坏性调用改为 Ask；AllowAll 只属于当前 live interactive Turn，不得被 scheduled、delegated、
replayed 或 maintenance origin 继承。任何模式都不能替代 OS permission、扩大具体资源 grant，或覆盖
Host/Core 的显式 Deny。客户端只投影与传递选择，不得复制风险分类器或根据提示词/语言推断权限模式。
Host 注入模型的运行环境说明必须读取与 Core 决策相同的 typed effective profile；不得从遗留
`auto_approve` 标志或进程环境重新推导一个不同的显示值。

本地持久化适配器可以在读取时迁移历史拼写（当前已知为 `moderate` → `recommended`、
`allow-all` → `allow_all`），但新写入和跨 Host wire 必须只使用 canonical 值。无法识别或类型错误的
本地值必须收窄到 Recommended，并保留同一会话的其余设置，不能因一个权限字段使整份设置解码失败。

权限请求必须挂起原 Tool future。用户响应通过完整 permission wait identity（protocol、nonce、request、
call、run/session、Host、Client 与 tool）加 `decision + scope` 恢复原调用，不能新建一轮模型请求。客户端在
提交期间收到 successor wait 时，旧 handler 与迟到回包都必须失效，即使 Host 复用了 request ID。
Host 内部用于把 Core trace 与同步权限 resolver 汇合的等待表必须支持跨线程生产/消费，并按 canonical
conversation ID 分桶；取出时同时验证 conversation、permission request、tool 与 trace span/call 身份。
客户端的交互卡按 canonical call ID 复用工具行，待响应期间不受 activity 分页/折叠投影隐藏。

## 12. 上下文、压缩与长期记忆

### 12.1 三种数据

```text
Canonical Transcript: 完整、不可因压缩而删除
Model Context Projection: 当前模型实际看到的有界工作集
Long-Term Memory: 跨会话事实、偏好、关系与来源
```

三者禁止共用同一个 message 数组或 summary 字段。

### 12.2 Context Checkpoint

```text
id
conversationID
modelProfileID
coveredThroughMessageID
coveredThroughSequence
summaryPartID
retainedTailStartMessageID
tokenUsage
createdAt
```

压缩由代码选择边界，由 LLM 生成语义摘要，再由代码校验结构与预算。压缩只替换模型工作集，不修改 UI 历史。
每个压缩 Part 必须投影按规范顺序排列的 source item IDs 与该序列的 fingerprint。客户端详情入口以
这些身份定位 Canonical Transcript 中的消息或 Part，并展示 fingerprint 供核验；不得从摘要文字猜测
原文位置。若目标不在当前分页窗口，客户端必须明确触发历史页加载或显示尚未加载，不能跳到相邻消息。
iOS 以 Application `load_conversation_projection_window_v1` 返回的 `before_ordinal` 和 `has_more`
逐页向前读取；每页仅用 source item ID 与 canonical Message/Part identity 比对。命中后把该页合并进时间线再滚动；
`has_more=false`、缺失游标或游标不前进都必须终止并显示原文不可用，读取失败显示可恢复错误。
Reduce Motion 开启时保留相同定位过程，但滚动不得使用动画。
Android 遵循同一协议：Compose 只把压缩 Part 携带的 source IDs 交给 ViewModel，ViewModel 使用
Application 窗口游标加载并合并更早的 canonical projection；UI 不解析摘要寻找目标。系统动画比例为零时，
目标跳转使用即时 `scrollToItem`，其余情况才使用动画滚动。
Web 消息页必须返回与消息列表相同边界的 Application projection window，禁止为来源索引额外物化完整
transcript。浏览器按历史页累积 canonical Message/Part 索引，并在每次 before 游标前进后重新匹配来源；
Reduce Motion 时使用即时 `scrollIntoView`。
CLI 使用 `nola session sources <conversation> [compaction-part]` 读取同一个有界 projection window；省略
Part ID 时选择最新且带 source item IDs 的压缩 Part，指定时只接受精确 Part identity。命令从最新页按
规范游标逐页定位 Message/Part，保留 source ID 顺序并输出 fingerprint 和未找到的 IDs；不得解析摘要文字，
游标缺失、重复、不前进或超过页数上限必须终止。

Provider overflow 必须由 Host 归一化为 typed `context_length_exceeded` 后交给 Core；错误链可能包含
网关诊断、重试元数据和最终 provider 响应等多个 JSON 对象，分类器必须扫描全部 error envelope，不能因
较早的非上下文诊断提前停止。Core 对同一 Turn 的恢复次数设硬上限，并在压缩事务中保留原 submission、
tool state 与 canonical source IDs/fingerprint；超过上限进入确定失败，不能重复提交用户输入或工具副作用。
分类器只读取 `error`/`errors` 等 typed error 容器中错误对象的直接 `code`/`type`/`error_code`；不得根据
本地化 `message`、裸 prose 或错误对象内回显的 request 字段选择压缩恢复。
移动端 provider adapter 同样只读取结构化错误 envelope 的直接 `code`/`type`/`error_code` 字段，将供应商的上下文
超限别名归一化为该 canonical code。自然语言错误消息与回显的请求字段不能触发恢复。此类 4xx 在 Host
内不得进行传输重试；Host 提交一次 typed failure，由 Core 决定有界压缩及同一 Turn 的后续 Attempt。

### 12.3 长期记忆

MemoryRecord 必须包含：

```text
id
kind
content
sourceConversationID/sourcePartID
confidence
validFrom/validUntil
revision
status
```

长期记忆通过语义计划选择，禁止通过自然语言关键词触发。写入是内部操作，不触发文件权限；删除和外部导出遵循产品权限策略。

## 13. 长任务

长任务不依赖聊天气泡、活跃窗口或单个客户端进程存活。

每个任务包含：

```text
taskID
ownerConversationID
state
checkpoint
lease
progress
outputCursor
artifacts
budget
nextWakeAt
```

必要能力：

- 可挂起、恢复、取消和重试。
- 跨 App 重启恢复。
- waitingPermission 与 waitingUserInput 不占用执行线程。
- 输出保存在有界日志或 Artifact 中。
- UI 按 cursor 读取过程，不把全部输出留在内存。
- 学术检索等任务允许独立的较长时间预算。
- 无进展检测基于调用身份、规范参数、结果摘要和状态变化，不使用用户关键词。

## 14. 数据源、Skill 与自动化

三者通过 Application 编排：

```text
Automation trigger
    -> Application creates durable Run
    -> Context planner selects enabled Data Sources
    -> Skill activation contributes instructions/resources/tools
    -> Runtime executes bound capabilities
    -> Results and provenance commit to Store
```

Data Source 保存：

- 配置与授权状态。
- 测试结果。
- 读取方式和数据格式 schema。
- freshness、maximum age 和最后成功时间。
- 默认启用与会话级 override。
- 每次注入上下文的 provenance。

首次添加数据源必须测试成功后才能启用。测试失败不能写入“已可用”状态。

## 14.1 Context、Memory 与长任务实施边界

这一部分是 V2 重构的实施契约。它不是客户端约定，也不是 Store 表结构的直接映射。

### 当前结构性缺口

| 领域 | 当前问题 | 必须收敛到的所有者 |
|---|---|---|
| Context | 压缩摘要、数据源正文、会话开关和 token 用量被不同客户端分别拼装 | Application Context Aggregate |
| Memory | 提取、筛选、写入和注入分散，客户端可能绕过统一计划 | Application Memory Aggregate |
| 长任务 | 聊天气泡、进程、任务记录和恢复状态互相耦合 | Runtime Supervisor + Application Task Aggregate |
| 数据源 | CLI JSON、macOS 服务和移动端权限状态存在平行实现 | Application Data Source Aggregate |
| Skill/MCP | UI 注册表、模型提示和工具执行目录可能出现三个真相 | Application Capability Catalog |

禁止用一个通用 `summary` 字段同时承载压缩上下文、长期记忆和数据源摘要。禁止客户端通过读取多张表自行推断模型实际看到了什么。

### Context Aggregate

Context Aggregate 由两类实体组成：

```text
ContextCheckpoint
    id
    conversationID
    runID?
    modelProfileID
    policyVersion
    coveredThroughMessageID
    coveredThroughSequence
    summaryPartID?
    retainedTailStartMessageID?
    tokenUsage { estimated, measured?, window, promptLimit }
    payloadDigest
    createdAt

ContextComponent
    id
    checkpointID
    kind
    sourceID?
    sourceURI?
    title?
    mimeType?
    authorization
    observedAt?
    maximumAgeMinutes?
    freshness
    contentReference
    contentDigest
    estimatedTokens
    provenance
```

`ContextCheckpoint` 描述某次模型工作集的稳定边界；`ContextComponent` 描述被选入工作集的每个来源。大正文写入 Artifact/Blob，组件只携带有界预览和引用。

授权状态必须是封闭枚举，至少区分：

```text
notDetermined | authorized | denied | restricted | unavailable | notRequired
```

新鲜度必须由 `observedAt + maximumAgeMinutes` 计算，不能由客户端根据展示文案猜测。过期只表示需要刷新，不表示授权失败。

Application 命令按领域表达，不再暴露 Store CRUD：

```text
ContextCommand.Prepare
ContextCommand.CommitCheckpoint
ContextCommand.LoadCheckpoint
ContextCommand.LoadComponents
ContextCommand.InvalidateSource
```

准备流程固定为：

```text
读取会话策略
    -> 解析默认数据源与会话 override
    -> 校验授权和 freshness
    -> 生成有界 ContextPlan
    -> Host 执行需要刷新的读取 Effect
    -> Application 校验结果与 provenance
    -> 提交 checkpoint + components
    -> Runtime 使用已提交 checkpoint 启动 Attempt
```

`Prepare` 可以产生 Effect，但不能直接执行文件、网络、HealthKit、浏览器或平台 API。Host 只能返回读取结果，不能自行把内容注入 Prompt。

### Memory Aggregate

Memory 是跨会话知识，不是历史消息副本，也不是 ContextCheckpoint 的长期缓存。

```text
MemoryRecord
    id
    kind
    content
    sourceConversationID
    sourcePartID
    confidence
    validFrom?
    validUntil?
    revision
    status
    createdAt
    updatedAt
```

Memory 的写入由模型提出候选、代码执行结构校验和去重，最后通过 Application 内部命令提交。内部写入不申请文件权限。用户删除、导出或向外部服务披露记忆时，才进入相应权限策略。

自动 extraction/consolidation 的输出在 Core 标记 job 完成前，必须按持久层规范幂等键
`(namespace, scope, kind, content)` 检查同批重复。不能只校验模型生成的 record ID；checkpoint writer
需要保留 Core 已接受的身份，事后再按内容合并会造成 checkpoint 与 Memory projection 分叉。

内容去重不得改变事实的来源权威性。同一 namespace、scope、kind 与 content 下，后台自动提取的
`agent_inference` 重复项只能确认已有事实，不能覆盖 `explicit_user` 的 provenance、confidence、
summary 或更新时间；用户后续显式编辑和纠错仍可通过 typed Memory 命令更新或 supersede 旧事实。
该约束必须在共享 Store 事务内执行，使所有调用 Application Commit 的 Host 得到一致结果。

Memory 注入必须先生成语义检索计划，再返回带来源的候选记录。字面检索命中旧事实时，Store adapter
必须在严格上限内补齐同 namespace/scope 的 supersession 后继链；纠错事实不需要重复旧事实的关键词，
Core 仍能在置信度、有效期和 token 预算校验后阻止旧事实复活。禁止用用户输入关键词决定是否启用长期记忆，也禁止把全部记忆拼入 system prompt。
当 MemoryPolicy 设置 `maximum_age_days` 时，`observed_at` 是 freshness 判定的必需 provenance；缺失、
非法或晚于本轮权威时钟的 observation time 必须 fail closed，不能借由未知或负 age 绕过过期门禁。
Core 的最终选择必须把 supersession 约束在相同 `(namespace, scope)` 所有权域；即使一个 ContextPlan
同时允许多个域，某域记录也不能仅凭引用另一域的 ID 来隐藏后者。
带 `sensitivity` 标签的记录必须经过本次 MemoryPolicy 的精确 typed allowlist；缺省策略不得向模型注入
任何带标签记录。授权比较只匹配规范标签，不能从内容、用户查询或 memory kind 推断。
平台侧的持久化投影、兼容模型和 Core ABI wrapper 必须逐层保留 sensitivity；将带标签记录转换为
`sensitivity=nil` 等同于绕过默认拒绝。旧客户端缺少 `allowed_sensitivities` 字段时必须解码为空 allowlist。
带 kind 的 typed recall 仍必须遵守结果类型边界：其他 kind 的后继事实可以参与 supersession 冲突消解，
但不能作为命中项返回给调用方。

Code Memory 的 dirty repository revision 必须覆盖普通文件内容、未跟踪/删除/重命名、符号链接和
submodule/gitlink 的嵌套 HEAD 与脏工作区身份。父仓库 porcelain 的稳定 `M <path>` 不能单独证明
submodule 内容未变化；未绑定嵌套 revision 的索引结果不得视为当前代码事实。
Host 必须在 Code Memory 结果进入模型可见 envelope 前，用当前工作区独立校验 path、symbol、revision、
content digest 与 source evidence。若任一事实与当前文件冲突或路径已不存在，原 provider payload 必须隔离，
只暴露带 stale 原因和 typed provenance 的刷新要求；仅追加 warning 而保留旧 snippet 不构成 stale 门禁。
任何结果都必须完整落在 Host 的有界验证集合内；超过 fact 上限时不能静默验证前缀并继续暴露未验证后缀，
而应将整个 payload 标为 `requires_verification` 并隔离，保留上限和原因供刷新或分页处理。

Code Memory 在 CLI、Rust Web、macOS 与 Windows 中是链接进 Agent Host 的 Rust library，不是 MCP server、
sidecar 或单独可执行文件。Host 必须把 `codebase_*` 注册为 builtin capability，并在通用 MCP 路由之前按
Core 的 `CodeIntelligenceOperation` 直接调用；移动端不注册这组能力。构建、安装、benchmark 和运行时均不得
下载、发现、上传或启动 Code Memory companion，也不得用环境变量把调用改道到外部二进制。

原生引擎最低闭环是多语言事实、typed nodes/edges、SQLite generation 与有界查询。项目相对限定名不能包含
机器绝对路径或数据库 row ID；IMPORTS/CALLS 必须携带解析策略与置信度，歧义引用保留 unresolved 而不能猜测。
文件 hash、解析器版本和索引配置共同决定 freshness；I/O 错误、取消或发布失败必须保留上一完整 generation 并
显式记录 coverage gap。工具 schema 只能声明已经实现的参数和结果，不能复制上游 catalog 后静默忽略字段。
全文搜索、snippet 等索引后文件读取仍须重新 canonicalize、验证 workspace 边界与内容 digest，防止符号链接
替换或文件变更把旧行号和新内容拼成伪造证据。

### Long Task Aggregate

长任务必须独立于聊天窗口和单次 Agent Host 连接：

```text
RuntimeTask
    id
    ownerConversationID
    runID?
    state
    currentStep
    checkpointReference
    lease { owner, expiresAt }
    progress
    outputCursor
    artifactReferences
    budget
    nextWakeAt?
    createdAt
    updatedAt
```

状态机至少包含：

```text
queued
running
waitingPermission
waitingUserInput
sleeping
cancelRequested
succeeded
failed
cancelled
```

`waitingPermission` 和 `waitingUserInput` 是可恢复挂起态，不占用工作线程，不创建新的模型轮次。任务输出使用游标分页；聊天消息只保存任务引用、当前摘要和最终结果，不复制完整日志。

### 三者之间的数据流

```text
User/Automation Command
    -> Application 创建 Run 或 RuntimeTask
    -> Context Aggregate 生成并提交模型工作集
    -> Memory Aggregate 提供有来源的候选组件
    -> Runtime Supervisor 调度已提交 Effect
    -> Host 返回 Effect Result
    -> Kernel 产生新的 Domain Events / Effects
    -> Application 原子提交状态、规范消息和投影事件
    -> Client 订阅 Projection
```

任何阶段都不得由客户端保存“补丁版完整消息”作为恢复依据。delta 只用于预览；恢复只能基于已提交 Part、Run checkpoint、Effect receipt 和任务 checkpoint。

## 14.2 Tool、Skill、MCP 与 SOP 的统一能力模型

四者不是四套执行系统：

- Tool 是最小可执行能力，包含输入 schema、风险声明、执行位置和结果 schema。
- MCP 是 Tool 的一种提供者和传输适配器，不拥有独立权限语义。
- Skill 是可启停、可查看、可编辑的指令、资源和允许工具集合，不直接执行副作用。
- SOP 是一种面向长任务的 Skill，额外声明阶段、检查点、完成条件和恢复策略。

统一目录项至少包含：

```text
capabilityID
kind
providerID
displayMetadataKey
inputSchema
outputSchema
riskClass
requiredPlatformCapabilities
availability
version
```

模型只看到本轮 ContextPlan 选择出的有界能力集合。每轮结束后清空可调用集合，仅保留调用身份、规范参数、截断结果、Artifact 引用和 provenance。Tool 名称与参数不能通过 shell 展示前缀影响权限匹配。

SOP 运行固定其创建时的定义与版本；目录新增版本不能改变进行中的运行。只有显式创建或 rerun 才选择新定义，且必须按稳定 SOP ID 选择满足版本约束的最高可用版本，不能依赖目录返回顺序。
进程恢复时，模型上下文只投影 owner 为空的 workspace 级运行，以及 `owner_session_id` 与当前 canonical
session 完全相同的运行；同 workspace 的其他会话运行不得因仍为非终态而进入候选集。该选择只读取
typed owner/run/version identity，不读取用户自然语言，也不因 catalog refresh 或 `tool_search` 改写。

模型为工具调用附加 SOP lineage 后，Host 必须在移除保留参数并执行工具前，把 HostRequest 的真实
`session_id`、当前 workspace、run ID 与 step ID 一并交给 Application 校验。带
`owner_session_id` 的运行只能由同一会话继续；owner 为空的运行才是 workspace 级。工具权限不能代替
lineage 所有权检查，permission wait 的首次请求与恢复请求也必须执行同一检查。

## 14.3 平台边界

所有平台共享同一 Application/Kernel 语义，但 Host 能力不同：

| 平台 | 客户端职责 | Host 能力示例 |
|---|---|---|
| macOS | 原生 Projection UI、权限交互、窗口与 Inspector | 文件、进程、辅助功能、屏幕录制、Chrome 扩展、本地模型 |
| iOS | 移动 Projection UI、系统授权和轻量本地推理 | HealthKit、日历、提醒事项、文件选择、本地模型 |
| Android | Compose Projection UI、系统授权和设备数据 | SAF、日历、通知、本地模型 |
| CLI | 终端 Projection、交互式权限和开发者工具 | 文件、进程、Git、网络、MCP |
| Web | 远程 Projection UI，不持有设备权限 | 通过已认证 Host session 委托能力 |

“共享语义”不等于让所有平台暴露相同工具。Application 根据 Host capability advertisement 生成可用目录；缺失能力必须显示为不可用，不得静默切换到另一条私有实现路径。

macOS 原生客户端不能把 CLI 进程当作业务运行时。CLI 与 macOS 可以使用同一个 Application Service 和 Agent Host 协议，但二者是并列客户端。

## 14.4 单写者迁移顺序

直接 Store 访问按以下顺序删除，禁止长期保留双写或 fallback：

1. Context、Memory、Permission、RuntimeTask 建立完整 Application Command/Query。
2. Tool、Skill、MCP、Data Source 统一进入 Capability Catalog 与 Host Effect。
3. CLI 和 Web 的业务读写改为 Application Service client。
4. macOS、iOS、Android 删除本地规范消息拼装和恢复逻辑。
5. 删除旧仓库、JSON 状态、兼容命令和隐式数据库重建路径。

每一阶段必须先建立规范 Projection，再删除客户端读模型。禁止为了减少改动让客户端通过多个 Query 拼接一个逻辑快照。

## 14.5 本阶段验收条件

- 除 Application Service 外，没有业务模块以读写模式打开 canonical Store。
- Context、Memory 和 Transcript 使用不同实体、命令与投影。
- 每次模型调用都能追溯到一个已提交 ContextCheckpoint。
- 每个外部 ContextComponent 都包含授权、新鲜度和 provenance。
- Skill/MCP/SOP 的启用状态、版本和工具集合来自统一目录。
- 权限挂起后恢复原 Effect，不创建替代模型轮次。
- 长任务在客户端退出和 Host 重连后可以从 checkpoint 恢复。
- 客户端切换十兆字符级长会话时，只加载可见分页和有界尾部。
- 任何 delta 重放、重复 Effect Result 或 Projection Event 都是幂等操作。
- UI、日志和测试能同时显示稳定 ID、Attempt、Effect、Task 和 Projection cursor，以定位重复、遗漏和卡死。

## 15. 同步模型

- 同步复制已提交的 canonical operations/entity revisions。
- 同步服务不是本地功能的前置条件。
- UI projection、delta preview、Markdown cache 和进程日志不参与同步。
- 冲突解决以实体类型、revision 和明确策略处理，不上传整份会话覆盖远端。
- Artifact 大对象与元数据分离同步。

## 16. 性能预算

### 16.1 服务与 IPC

- Command 控制帧目标小于 256 KB。
- Event 控制帧目标小于 64 KB。
- 大内容通过分页、Blob stream 或 Artifact reference 传输。
- 常驻连接复用，不为每条命令重新连接 Unix Socket。
- Service request loop 与 outbound writer 分离，并使用有界 channel。

### 16.2 会话与 UI

- 10M 字符会话启动时只加载摘要与可见窗口。
- 切换长会话不加载全部历史。
- 客户端内存与可见窗口和缓存预算相关，不与完整会话长度线性增长。
- 同时打开多个长会话时，非活动会话只保留轻量 Projection。
- 流事件以约一帧一次的频率归并，相同语义状态不得重复发布。
- Markdown parser 缓存以 block ID 和 source revision 为键。
- 客户端按 canonical owner identity 预索引 Task、Skill 与来源投影；流式重组中不得为每条可见消息重复扫描完整投影。

具体阈值通过性能基线测试确定，不能写死为参考项目的参数。

## 17. 数据库策略

产品尚未发布，因此 V2 不实现旧 schema migration，也不保留兼容读取代码。

同时禁止静默删除用户数据库：

- schema 不匹配时返回 `schema_epoch_mismatch`。
- 开发构建提供明确的“重建测试数据”操作。
- 正式构建在没有显式用户或开发者操作时不得删除数据库。
- 新 schema 使用独立 epoch 和明确文件名，避免旧构建与新构建竞争同一文件。

## 18. 删除清单

V2 落地后必须删除：

- Mac UI 对 `MacApplicationPort.shared` 的直接业务调用。
- Mac UI 对 Local Host 与 Remote Host 的分别归并逻辑。
- 平台侧独立的 assistant delta/final 文本合并器。
- 平台侧重复的 Prompt Composer、Tool Catalog 和 Permission reducer。
- CLI 对 canonical SQLite 的直接访问。
- `nola-service-client -> nola-application` 实现依赖。
- 每轮启动 Agent Host 的路径。
- 完整会话 JSON IPC。
- legacy event 与 canonical event 双发路径。
- 自动重建不兼容数据库的默认行为。
- 按用户文本关键词选择能力或修正路由的逻辑。

删除旧路径是阶段完成条件，不允许长期保留 fallback。

## 19. 实施阶段

### Phase 0：约束与基线

- 冻结新跨层直连。
- 增加 crate dependency architecture test。
- 建立五端共享的 Protocol fixtures。
- 建立当前重复、乱序、权限恢复、长会话和多窗口性能基线。

### Phase 1：Domain 与 Protocol V2

- 建立稳定 ID 与统一实体。
- 建立 Command receipt、Snapshot、Event 和 cursor 协议。
- 建立统一 ContentPart 与 ToolPart 状态机。

### Phase 2：Store 与 Application Service

- 创建新 schema epoch。
- 拆分 Repository。
- 实现事务内实体更新、Event append 和 outbox。
- 实现分页查询与 Projection subscription。

### Phase 3：Runtime 与 Host

- 建立长驻 Runtime Supervisor。
- 实现 Run/Attempt 隔离、取消、permission waiter 和恢复。
- 统一 Provider、Tool、Skill、MCP 和 Platform Adapter。

### Phase 4：Mac 与 CLI

- Mac 改为纯 Application Client。
- CLI/Web 复用同一 Client 协议和 reducer。
- 删除 Store 直连和重复 Host 事件解析。

### Phase 5：iOS 与 Android

- 移动 FFI 收敛为 Application Client 与 Platform Effect Adapter。
- 删除 Swift/Kotlin 重复 policy。
- 使用同一 Projection fixtures 验证行为一致。

### Phase 6：UI 与性能

- 分页时间线与稳定 Part View。
- 增量 Markdown block parser。
- 语义短语浮现动画。
- 10M 字符、多窗口、长任务和低内存压力测试。

## 20. 验收门槛

### 20.1 协议正确性

- 同一 Event 序列在五端得到相同规范 Projection。
- 重复 Event 不改变 Projection。
- 乱序、断线、重连和 cursor 补洞不重复消息。
- completed 全文不会再次追加到 delta preview。
- retry Attempt 不污染前一个 Attempt。

### 20.2 权限与工具

- once、conversation、workspace、persistent 四种 scope 可验证。
- Custom response 挂起并恢复原调用。
- OS 权限和 Nola policy 状态分别正确。
- Bash、文件工具、Skill 与 MCP 使用同一权限评估链。

### 20.3 长任务

- App 退出后任务可恢复。
- waiting 状态不持续占用 CPU。
- 取消能终止真实 Provider、Tool 和后台进程，而非只停止 UI。
- 工具日志与 Artifact 内存有界。

### 20.4 性能与 UI

- 构造 10M 字符规范会话，不调用真实 LLM。
- 多个长会话窗口连续切换并记录帧时间、RSS 和响应延迟。
- 真实 LLM 多轮测试记录首 token、事件顺序、最终 Store 和截图。
- UI 测试比较加载中、流式中、工具中、权限等待、完成和失败状态截图。
- 相同状态事件不得引起气泡 identity 或布局变化。

## 21. 参考项目取舍

### Codex

借鉴：

- 单一 Session submission loop。
- 持久化历史、模型工作集和 UI 投影分离。
- Embedded、daemon、remote 复用同一 App Server 协议。
- 完整 item 单一提交边界。
- 权限 waiter、统一中断和有界后台进程输出。

不照搬：

- JSONL canonical 加 SQLite projection 的双介质结构。
- 面向 TUI 的逐行稳定策略。
- 仅适用于代码工作区的权限模型。

### OpenCode

借鉴：

- Session、Message、Part 规范模型。
- 事务内更新实体和事件，再发布 Bus。
- SSE cursor、客户端幂等 reducer 和按帧批处理。
- Tool、Skill、MCP 统一 contract。
- 增量 Markdown stable block 与 mutable tail。

不照搬：

- 特定 TypeScript、Hono、Solid 或 Tauri 技术栈。
- 固定逐字符动画参数。
- 进程内 PTY 作为数小时长任务方案。

### Claude Code

借鉴：

- 每会话长期存在的 QueryEngine。
- Attempt 隔离。
- 完整消息为权威，delta 仅预览。
- compact boundary 与模型工作集替换。

不照搬：

- 固定轮次限制。
- Tool contract 与 React UI 耦合。
- 云端文件系统作为长期记忆基础。

### Open WebUI

借鉴：

- 发送后立即创建 assistant placeholder。
- message ID 定位的 typed event。
- parent message 分支与取消态落盘。

不照搬：

- 完整 Chat JSON 与独立消息表双真相源。
- 前端维护和上传整棵消息树。
- 每次 delta 读取并重写完整文本。

## 22. 最终判定标准

当以下陈述全部成立时，V2 才算完成：

- 任意平台都无法绕过 Application Service 写 canonical state。
- 任意 Host 都无法直接构造或覆盖完整会话。
- 任意客户端都可以从 Snapshot 与 Event suffix 恢复一致 UI。
- 一条助手回复只存在一个 Message ID 和一组稳定 Part ID。
- Tool、Skill、MCP、权限和长任务共享统一生命周期。
- 完整会话长度不决定客户端常驻内存。
- Core 行为通过跨平台 fixture 一致性验证，而不是依赖五套人工实现保持同步。


## 19. Implementation invariants locked during the hard cut

The architecture generation and each wire/storage identity are different concepts. `Architecture V2` must never be encoded by resetting a mature wire protocol to version `2`.

- Application Service wire protocol advances monotonically from `v5` to `v6`.
- Canonical Store uses schema epoch `2` and schema version `8`.
- Socket identity, FFI protocol identity, Rust clients, native clients, fixtures, and process tests must all derive from the same Application Service protocol constant.
- Normal product startup must never pass a destructive recreate flag. An incompatible Store is surfaced as an explicit recovery condition; only a user- or developer-selected reset may recreate it.
- A command is identified by `(client_id, request_id)`. Reusing that identity with different command content is a protocol conflict.
- The durable receipt is written as `accepted` before execution and reaches one terminal state: `committed`, `rejected`, or `conflict`.
- Until command mutation and receipt completion share one SQLite Unit of Work, a crash between those commits remains `accepted`. Recovery must refresh/reconcile canonical projections and must not blindly execute the side effect again.
- Projection events use one monotonically increasing sequence per scope. Duplicate `event_id` with identical content is idempotent; identity reuse or a sequence gap is rejected.
- Agent work has no arbitrary turn or cycle ceiling. The Runtime stops only on a terminal/quiescent state, explicit budget or cancellation, or a concrete protocol stall where the same committed effects yield no accepted response.
- Boundary user input is acknowledged immediately after Application commits it. A later Host failure must not requeue already committed input.

These invariants are architectural gates, not compatibility shims. New code that bypasses them must fail dependency or contract tests.

## Data Source control plane and context injection

Data sources are durable, managed capabilities. They are not arbitrary JSON blobs and they are not equivalent to context components. A data source describes how data may be acquired; a context checkpoint records the immutable result selected for one model request.

### Canonical aggregate

The Application Service owns the canonical `DataSource` aggregate. At minimum it contains:

```text
DataSource
  id: DataSourceID
  revision: Revision
  kind: DataSourceKind
  displayName: LocalizedDisplayName
  configuration: DataSourceConfiguration
  authorization: DataSourceAuthorization
  validation: DataSourceValidation
  readContract: DataSourceReadContract
  freshnessPolicy: FreshnessPolicy
  defaultEnabled: Bool
  lifecycle: draft | ready | disabled | failed | deleted
  lastSuccessfulReadAt: Instant?
  createdAt: Instant
  updatedAt: Instant
```

`DataSourceConfiguration` contains only typed, source-specific configuration. Secrets are stored by a platform secret provider and referenced by opaque IDs; they must never be persisted in projection payloads, logs, tool output, or conversation messages.

`DataSourceAuthorization` records the capability and current authorization state. Platform permission snapshots are observations rather than authority. The owning Host performs the platform check and returns a typed observation; the Application Service decides the aggregate transition.

`DataSourceValidation` records the latest validation attempt, including its timestamp, status, bounded diagnostic, and a stable validation recipe version. A source cannot enter `ready` or become enabled until its first validation succeeds.

`DataSourceReadContract` records the tested acquisition method and output schema. It includes the executor capability, typed parameters, content type, schema version, provenance requirements, and bounded sample metadata. It must not embed sampled private content merely to prove that validation occurred.

### Commands and events

Clients never replace a complete data-source row. The Application Service accepts intent commands only:

```text
CreateDataSourceDraft
UpdateDataSourceConfiguration(expectedRevision)
RequestDataSourceValidation(expectedRevision)
RecordDataSourceValidation(validationToken, result)
SetDataSourceDefaultEnabled(expectedRevision, enabled)
SetConversationDataSourceOverride(conversationID, dataSourceID, inherit | enabled | disabled)
DisableDataSource(expectedRevision)
DeleteDataSource(expectedRevision)
RequestDataSourceRefresh(dataSourceID, reason)
RecordDataSourceRead(readToken, result)
```

Validation and reads use committed effects. The Host may execute them only after the command transaction has committed. Results carry a one-use token issued by the Application Service so that stale, duplicated, or cross-source results cannot mutate the aggregate.

Each accepted transition appends a typed domain event and advances the aggregate revision. Conflicting revisions are rejected and returned as explicit conflicts; last-writer-wins behavior is forbidden.

### Selection and injection

For each model request, the Context Coordinator resolves sources in this order:

1. Start with sources whose lifecycle is `ready` and whose `defaultEnabled` value is true.
2. Apply the current conversation's explicit source overrides.
3. Exclude denied, unavailable, disabled, deleted, or schema-incompatible sources.
4. Apply freshness policies and request refresh effects when required.
5. Select bounded content under the context budget.
6. Commit an immutable `ContextCheckpoint` containing typed components and provenance.

Conversation overrides are durable conversation settings. They survive view changes, process restarts, and client changes. A UI-local toggle is never the source of truth.

Every injected component records the data-source ID, source revision, read timestamp, content reference, schema version, freshness decision, authorization observation, selection reason, and token usage. This makes the exact model input reproducible without coupling the transcript to live external data.

### Read-path constraints

- A successful test is required on first creation and after a read-contract or credential change.
- Cached content is addressed by immutable content references. Large payloads are not copied into the data-source row or projection event.
- Refresh is deduplicated per source and freshness window.
- A failed refresh may use a still-valid prior snapshot only when the freshness policy explicitly permits stale fallback.
- Authorization denial is not represented as empty data. Empty, denied, unavailable, and failed are distinct typed outcomes.
- Data-source summaries shown in clients are projections. Clients do not query platform stores independently to invent competing status.

## Durable long-task command model

Long-running work is a first-class aggregate whose lifetime is independent from any chat view, stream, process, or client connection. The transcript may reference a task, but it does not own the task.

### Canonical state machine

```text
queued
  -> leasing
  -> running
  -> waitingForPermission | waitingForInput | suspended
  -> running
  -> succeeded | failed | cancelled
```

Terminal states are immutable. Cancellation is a request until the owning worker acknowledges it or the Application Service resolves an expired lease. Retrying creates a new attempt under the same task identity; it does not erase prior attempts or events.

### Ownership and commands

The existing pattern of `Save(RuntimeTask)` or generic full-row upsert is forbidden. It lets clients overwrite canonical state, lease ownership, timestamps, progress, and output references. V2 exposes transition-specific commands:

```text
CreateTask(specification, origin, conversationID?)
ClaimTask(workerID, capabilities, leaseDuration)
RenewTaskLease(taskID, attemptID, leaseToken)
ReportTaskProgress(taskID, attemptID, leaseToken, progress)
AppendTaskOutput(taskID, attemptID, leaseToken, outputChunk)
SuspendTask(taskID, attemptID, leaseToken, reason)
ResumeTask(taskID, reason)
SubmitTaskInput(taskID, interventionID, input)
ResolveTaskPermission(taskID, interventionID, decision)
RequestTaskCancellation(taskID, requestedBy)
CompleteTask(taskID, attemptID, leaseToken, outcome)
AcknowledgeTaskNotification(taskID, notificationID)
```

Only the current attempt holder may report progress, append output, suspend, or complete. Every such command requires the current opaque lease token. The Application Service checks the state, attempt, ownership, sequence, and lease expiry atomically before committing the transition.

### Output, artifacts, and projection

Task output is an ordered append-only stream with bounded chunks and monotonically increasing cursors. A task row does not contain full logs, arbitrary output paths, or mutable result blobs. Durable results use typed artifact references owned by the artifact store.

Clients consume task projections using snapshot plus suffix subscription. Projection data includes current state, normalized progress, current human-readable phase, active capability invocation summary, pending intervention, latest output cursor, artifact summaries, attempt count, and timestamps. Detailed tool parameters and output are loaded on demand from the corresponding invocation/event records.

Reconnection starts from the last acknowledged cursor. Duplicate events are ignored by stable event identity. A dropped stream changes delivery only; it never changes execution state.

### Recovery and resource policy

- The Runtime Supervisor recovers expired leases and committed-but-unpublished effects after process restart.
- Heartbeats are persisted at a bounded cadence rather than for every UI animation frame.
- Progress updates are coalesced without dropping state transitions, permission requests, artifacts, or terminal outcomes.
- Task output and tool output have explicit retention and truncation policies; the complete artifact may be stored separately when required.
- Academic or explicitly long-running work may select a longer execution policy. Other work retains bounded timeouts.
- A foreground request becomes a background task only when duration, disconnection tolerance, or resource needs justify it.
- The Host owns processes, browser sessions, GPU runtimes, and platform resources. The Application Service owns durable intent and lifecycle; the Kernel owns pure transition rules.

## Data source, Skill, and scheduled-task composition

Autonomous orchestration uses the same capability protocol as interactive chat:

```text
Schedule trigger
  -> Application Service creates a run/task
  -> Kernel selects a Skill and its declared capabilities
  -> Context Coordinator resolves enabled data sources
  -> Application Service commits effects and permission interventions
  -> Host executes capabilities
  -> Application Service commits invocation results, memory candidates, artifacts, and task outcome
  -> clients observe projection events
```

A Skill is a versioned instruction-and-capability package, not an alternative execution path. MCP servers contribute capabilities through adapters, not by bypassing the capability catalog. Scheduled tasks reference stable Skill and data-source IDs plus version constraints. They do not copy executable prompts, credentials, or platform-specific command lines into schedule rows.

Before a schedule can be enabled, the Application Service validates that every required Skill version, capability, data source, authorization, and runtime Host is available. The validation result is durable and invalidated when a dependency revision changes.

## Required acceptance scenarios for these aggregates

The refactor is not complete until automated tests prove all of the following:

- A newly added data source cannot be enabled before a successful validation result is committed.
- A conversation override survives restart and is reflected identically in the inspector and composer context indicator.
- A denied source cannot appear as a valid empty context component.
- Concurrent data-source edits produce a revision conflict rather than silent overwrite.
- A repeated validation/read result token is idempotent and cannot apply to another source.
- A client cannot mutate task status, lease, timestamps, progress, or artifact references through a generic save operation.
- A stale worker cannot append output or complete a task after its lease is replaced.
- Restart after effect commit but before publication resumes exactly once without duplicating tool calls or output.
- Permission or user-input waiting suspends the task without consuming model turns or worker capacity.
- A scheduled Skill run resolves the same data-source policy and capability permissions as an equivalent interactive run.
- Ten-million-character transcripts and multiple long conversations use paged projections and bounded resident memory; UI switching does not reconstruct entire histories.

### 自动记忆作业的客户端调度与权限

完成的 Agent Turn 只负责由 Core/Application 原子地产生记忆作业；模型提取由独立后台 Worker
领取。Android 在 Turn 流正常结束后以会话稳定键加入 WorkManager unique work，冷启动后由
`MemoryWorker` 重新打开 Application Runtime；iOS 的 BGAppRefresh 与 BGProcessing 回调扫描
Application 的可领取作业，并由 canonical Memory Worker 执行。两端 Worker 均通过 typed
Application command 领取、心跳、完成或失败，不直接写记忆表。

### 移动端权限等待的进程恢复

Android 打开带 `waiting_for_permission` active run 的本地会话时，必须恢复原 Run，不能创建新 Turn。
新进程先核对 canonical active run 与 invocation-config digest，再等待 Application effect-work schedule
宣布旧 lease 可接管；随后以新的 worker generation、lease owner 与 lease token claim successor delivery。
恢复入口只接受 `request_permission`，并校验 request attempt 与 delivery attempt 完全一致；model、tool 或
其他 effect 不得由该入口自动重放。Coordinator 重新提交 `execution_started` 后展示原权限请求，最终决定
仍通过带当前 delivery 的 HostResponse 提交，因此旧进程和过期 lease 都不能完成该调用。
iOS 冷启动遵循同一契约：`ChatManager` 只在 canonical projection 明确处于
`waiting_for_permission` 时，从 active Run 的稳定 run/message identity 构造恢复描述；
`IOSCanonicalAgentApplication` 核对原 invocation-config digest，等待 effect-work schedule 后以新
worker claim 不超过五分钟的 delivery。`DeviceAgentCoordinator` 只恢复原
`request_permission` envelope，先持久化 `execution_started` 再重建 UI waiter；该入口不创建新
submission，也不重放 model/tool effect。

Worker 的模型请求是内部提取流程，不携带交互 Turn 的权限授权，也不暴露普通工具目录。lease
失效或 ownership token 不匹配时，Application 拒绝心跳和结果。失败提交 `retry_after_ms` 后，
平台调度器必须保持后续唤醒；一次平台任务返回成功不得遗留仅因退避尚未到期而暂不可领取的作业。
Android WorkManager 的首次退避与 Core 的一分钟重试窗口对齐，并在服务报告已提交 retry 时返回
`Result.retry()`，最多执行三次平台 attempt。跨会话后台扫描的预算只限制实际领取的模型作业数，
不能限制检查的会话数；空队列不得让后续 owner domain 的 ready job 在每次冷启动中永久饥饿。

## 单写入者迁移审计与实施门禁

### 审计结论

当前 iOS、macOS、Android 与 Web UI 未发现直接创建 SQLite 连接的生产路径，客户端薄化方向基本成立。
2026-09-08 按实际实现复核：CLI 的 `store::NolaStore` 已是路径与 Application 调用包装，
不是 SQLite 连接；生产持久化调用经过 `CliApplicationPort::Service`。
不能据同名 `NolaStore::open` 调用推断仍有客户端直写。CLI 的 `nola_store`、`rusqlite`
直接依赖现限定为测试，Web 分支输入使用 Protocol 的 revision + boundary 意图；CI 增加直接依赖门禁。
这不代表完整单写者和分层验收已经通过：CLI 仍混合客户端、Web/Agent Host 和监督职责，
并依赖 Application/Service 实现；主库打开和具体 Store 选择已迁独立 bootstrap，
Application 已通过 Store Port 解耦具体 Store，但仍依赖旧 Core。
移动 FFI 已将写命令统一到 durable receipt submit，execute 仅允许 typed query，
并已有 iOS/Android 原生恢复专项验证；这不代表全部移动领域所有权和五端投影已验收。
后续必须继续消除其余边界缺口。

以下是多写入者或绕过事务协议可能造成的风险，不能用锁、重试、去重或客户端补丁代替所有权收敛：

- 事件已执行但状态未提交，或状态已提交但副作用未执行。
- macOS、CLI 与 Web 对同一会话产生不同投影。
- 权限授权、工具结果、长任务进度和消息正文出现重复、遗漏或乱序。
- 多个进程反复打开数据库，放大锁竞争、内存占用和恢复成本。
- 测试只能覆盖单模块结果，无法证明跨进程时序和故障恢复正确。

### 强制边界

生产环境只允许 Application Service 持有 canonical Store 的读写连接。其他组件必须遵循下列约束：

| 组件 | 允许 | 禁止 |
|---|---|---|
| Kernel | 纯状态转换、命令校验、Effect 规划 | IO、数据库、网络、进程、平台 API |
| Application Service | 事务、幂等收据、事件日志、投影、恢复协调 | 执行外部副作用 |
| Runtime Supervisor | 领取已提交 Effect、租约、心跳、结果回报 | 直接修改 canonical Store、绕过命令提交 |
| Agent Host | 模型调用、工具/Skill/MCP 执行、平台能力适配 | 直接持久化消息、权限、任务、记忆、数据源 |
| CLI/macOS/iOS/Android/Web | 发命令、订阅投影、保存纯 UI 偏好 | 打开 canonical SQLite、维护第二份领域状态 |

客户端可持久化窗口尺寸、主题、最近选择等无业务语义的 UI 偏好。会话、消息、Part、运行、工具调用、权限授权、长期任务、记忆、数据源、Skill/MCP 配置和模型配置均属于 canonical 状态，必须通过 Application Service 修改。

### CLI 与 Web Host 拆分

CLI 是 Application Service 的客户端，不是数据库所有者。`nola web` 是同一客户端协议的 Web 入口，不得因为与服务进程位于同一二进制或同一机器而获得 Store 特权。

目标进程关系：

```text
CLI / Web UI / macOS / iOS / Android
                 |
                 | typed command + projection subscription
                 v
          Application Service
                 |
                 | committed effect + lease
                 v
          Runtime Supervisor
                 |
                 | typed host request/result
                 v
     Agent Host / Tool / Skill / MCP / Platform Host
```

本机部署可以把多个角色打包和共同启动，但协议边界、所有权和失败语义不得合并。测试内可直接构造 Store fixture；生产模块不得以“本机优化”为理由直连 Store。

### 迁移顺序

1. 为 CI 增加依赖和源码门禁，禁止客户端及 Host 生产代码引用 `nola_store`、`rusqlite` 或 `NolaStore`。
2. 将会话、消息、投影读取迁移到 Application Service 查询与订阅协议。
3. 将权限授予、工具结果和 Agent run 提交迁移为带幂等键的显式命令。
4. 将长任务改为 Application Service 状态机；Supervisor 只通过租约命令 Claim、Renew、Report、Suspend、Complete。
5. 将记忆、数据源、Skill、MCP、模型配置与计划任务迁移到各自的 typed aggregate 命令。
6. 删除 CLI `store` 领域包装层及 Web 路由的数据库访问；仅保留 Application Service 内部 Store adapter。
7. 最后删除所有兼容双写、数据库回退、进程内直读和隐式重放路径。

迁移期间不得新增桥接双写。尚未迁移的能力必须显式标记为 V1 隔离区，不能继续扩散其依赖。

### 失败与恢复规则

- 每个写命令必须携带稳定 `commandId`；重复提交返回同一 durable receipt。
- Effect 只能在所属事务提交后发布；未提交 Effect 永不执行。
- Host 结果必须携带 `effectId`、`attemptId` 和所有权 token；过期或重复结果不得改变状态。
- 流式 delta 只更新易失预览；最终消息只由一次 canonical finalize 命令提交。
- 服务重启从事件日志、收据和租约恢复，不扫描客户端缓存猜测状态。
- 客户端断线后按投影 revision 补齐 suffix；不得重发已确认写命令来“修复”界面。

### 架构验收门禁

重构完成必须同时满足：

- 客户端与 Host 生产代码不存在 canonical SQLite 连接创建。
- 除 Application Service Store adapter 外，不存在 `nola_store` 或 `rusqlite` 生产依赖。
- 同一用户操作在断线重试、进程崩溃和 Host 超时后最多产生一个 canonical 结果。
- CLI、Web 和 macOS 订阅同一会话时观察到一致的消息顺序、工具 Part、思考 Part 和任务状态。
- Application Service 停止时，客户端只能离线浏览已缓存投影，不能自行写 canonical 状态。
- 一千万字符会话分页和多长会话切换不要求完整载入 transcript，也不触发第二套本地领域数据库。
- 长任务跨客户端退出、Host 重启和服务重启后可继续，且租约过期不会造成双执行。

以上门禁属于架构正确性要求，不得以 UI 已正常显示、单元测试通过或本机单进程可运行为替代。
