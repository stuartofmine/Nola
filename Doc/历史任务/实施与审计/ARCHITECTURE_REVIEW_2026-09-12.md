> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Nola 全局架构审计：2026-09-12

本文是对当前工作区的整体审计，基线是 [`ARCHITECTURE_V2.md`](ARCHITECTURE_V2.md)、
[`V2_CURRENT_ARCHITECTURE_2026-09-08.md`](V2_CURRENT_ARCHITECTURE_2026-09-08.md)
以及当前 Cargo/Swift/Kotlin 源码。它描述真实边界和迁移顺序，不把某个专项测试通过
当作 V2 已完成。

## 结论

V2 的“命令、投影、仓储端口、租约、正文引用”已经形成不少可用部件，但状态所有权
还没有完成迁移。当前系统同时存在两套中心：新的 Application/Kernel/Port 链路和旧
`nola_core::agent` 运行时。它们通过 `AgentCoreSnapshot`、`CoreCheckpoint`、Host
Envelope 和大量 `serde_json::Value` 相互传递，因此 crate 名称看起来分层，实际行为仍
是一个跨层的运行时大对象。

这也是最近问题反复出现的共同原因：上下文、记忆、工具发现、SOP、长任务恢复、模型
重试和 UI 投影各自有局部实现，却没有一个唯一的状态拥有者。继续逐文件补条件会增加
第三套规则；应先按下面的所有权和依赖顺序收敛，再做功能优化。

本次复查新增两份规范文档：[`ARCHITECTURE_V2_MODULE_CONTRACTS.md`](ARCHITECTURE_V2_MODULE_CONTRACTS.md)
逐模块定义输入、输出、内部职责、端口和禁止依赖；
[`../../软件架构设计/工程规范/CODING_STANDARDS_RUST_SWIFT_KOTLIN.md`](../../软件架构设计/工程规范/CODING_STANDARDS_RUST_SWIFT_KOTLIN.md)
把 Rust、Swift、Kotlin/Android 官方规范转成 Nola 的可执行约束。下面的施工必须以这两份
契约为准，文件变小本身不代表边界已经成立。

本轮已完成两项边界施工：Application 和 Runtime Driver 不再直接依赖旧 `nola_core`，
旧 Core 只通过明确命名的 `nola_legacy_agent_adapter` 进入；`PermissionInstruction`
从 Host Protocol 下沉到 Domain，SQLite Store 不再为了校验持久化的指导文本而依赖
Host Protocol。两项施工是迁移接缝，不是声称旧 Core 已删除。

## 第二遍复查：Skill、原生工具与自动化终态

`MacBuiltInSkillEndToEndTests.testAutomationSkillEndToEnd` 暴露的缺失输出并非单一调度
问题，而是三个接口错位叠加：

1. Core 在 Skill 刚激活、模型尚未按新指令做出一次决策时，就把所有待完成
   `skill_update` 记录放进 system context。弱模型或确定性测试 Host 会优先填写清单，
   跳过实际写入和验证。
2. `design-automations` manifest 要求 `automation-record` artifact；CLI 和 macOS 原生
   `nola_orchestrate` 已经持久化记录，却只返回 JSON 文本。更深一层，Mac 设备桥即使
   返回 artifact，Rust Host 也固定丢弃为 `artifacts: Vec::new()`。
3. `MacAutomationStore.runScheduledAutomations` 以子进程退出码作为运行成功条件。旧链路
   因此能在没有目标文件、没有真实 verifier evidence 时写成功 receipt。

本轮已修复前两项：Skill completion 提示只有在至少一次激活后模型决策、必需 artifact
已经由 Host 注册且外部 artifact/工程验证门槛满足后才出现；CLI 与 macOS 原生自动化
持久化返回 `nola://automation/<id>` artifact；设备桥统一解析并校验 artifact envelope。
同一端到端用例随后真实执行了 `Skill -> write_file -> read_file -> skill_update`，并通过。

第三项仍需在 Runtime/Automation 边界完成：runner 必须返回 typed terminal receipt，
其中包含 Core terminal status、失败分类和已验证 deliverables；AutomationStore 不能继续
从退出码或日志文本推断业务成功。这应与 Run/Attempt/Effect 状态所有权迁移一并完成。

本次重新统计后的主要文件是：`nola-store/src/lib.rs` 1,544 行、
`MacChatWorkspaceView.swift` 1,384 行、`MacWorkspaceModel.swift` 4,565 行、
`nola-application/src/lib.rs` 3,611 行、`MacApplicationPort.swift` 3,626 行、
`application_command.rs` 972 行、iOS `ChatManager.swift` 3,028 行、
`ChatViewModel.kt` 2,254 行。旧审计中的拆分前数字仅作为基线，不再代表当前文件大小。

`nola-application/src/lib.rs` 中的 Skill catalog 和 Session projection 用例现已分别迁入
80 行的 `skill_catalog.rs` 与 275 行的 `session_projection.rs`。前者只取得
`SkillRepository`；后者只取得 authoritative session/remote mirror ports，并继续严格校验
checkpoint identity、cursor、revision 和实际 SkillRun evidence。根文件只声明并重导出这些
公开用例；wire schema、Store schema 和历史可选 capability 脱敏语义均未改变。

Memory maintenance 的 job readiness、frozen worker plan 和 maintenance Run admission 也已迁入
183 行的 `maintenance_turn.rs`。查询只依赖 `RunEffectRepository`，写入只依赖
`RunCreationRepository`；Application 在进入 Core 前拒绝交互输入与重叠 active Run，Host 无需
解码 checkpoint 或复制重试策略。

普通 assistant turn admission 现位于 244 行的 `assistant_turn.rs`。输入明确包含 stable
submission identity、Core config 与规范消息 identity；输出包含 canonical checkpoint、事件、
trace、snapshot 和首批 Host effect。该模块只取得 Run creation 与 canonical history 两个端口，
并在写入前拒绝重叠 Run、缺失 submission ID 或没有外部 effect 的 transition。

macOS Application facade 的首个用例族也已形成编译期接口：120 行的
`MacAutomationApplicationContracts.swift` 拥有 automation definition/run DTO 与
`MacAutomationApplicationClient`。`MacAutomationStore` 的定义和 run 生命周期、
`MacActivityStore` 的最新 run 查询改为依赖该窄接口，`MacApplicationPort` 负责 wire 适配。
随后又迁出 89 行的 `MacSkillCatalogApplicationContracts.swift`。内置 Skill registry 的同步、
启停、恢复、移除、列表和 setting 查询只取得 `MacSkillCatalogApplicationClient`；
`MacAutomationStore` 因而不再持有 concrete `MacApplicationPort`。其他 macOS feature 仍有直接
依赖 facade 的路径，不能据此宣称整个 Application client 已完成接口隔离。

Local model profile DTO 与四个必要用例又迁入 43 行的
`MacLocalModelApplicationContracts.swift`。`MacLocalModelConfigurationStore` 只取得
`MacLocalModelApplicationClient`，覆盖 profile upsert、参数 revision CAS、单项读取和列表读取。
Application client 协议显式使用 `nonisolated Sendable`，不把 Swift UI actor 隔离传播到同步
服务边界。

Data Source registry 现通过 74 行的 `MacDataSourceApplicationContracts.swift` 取得 13 个明确
用例，revision、validation token、时间戳与 usage limit 都是显式输入。Registry 不再保存或
构造 concrete `MacApplicationPort`；本地化 title 仍留在 presentation/service 文件。

Archive restore 与 transcript body upload 通过 87 行的
`MacArchiveRestoreApplicationContracts.swift` 分成继承关系明确的两个 client。Writer 仍拥有
restore identity、directory revision、manifest sequence/digest/count 校验；Uploader 仍拥有
deterministic upload ID、32 KiB UTF-8 chunk、resume read-back 和逐字节 ACK。没有把它退化成
一次性大正文 API，upload DTO 同时从 facade 根文件迁到该 contract 文件。

Composer 也已落实文档原先要求的 display/action seam：54 行的
`MacComposerFeatureContract.swift` 定义 immutable state、typed action 和稳定 action sink；
`MacComposerView` 已无 `MacWorkspaceModel`、Store 或 Host 引用。`MacConversationCanvas` 是当前
唯一把 action 映射到 WorkspaceModel 的 composition point，长期 draft 没有复制到 View state。

## 第二阶段文件边界：Android Chat Context

第二遍文件审计确认 `ChatViewModel` 仍直接拥有六类上下文职责：构建 Core components、
读取健康平台数据、解析 attachment、调用摘要模型、提交 compacted archive、记录数据源
披露。它们共享的是一个明确的“准备模型上下文”输入输出，并不属于 UI 状态生命周期。

现已提取可注入的 `ChatContextPreparer`：输入为 conversation、messages、route、模型预算、
启用的 Skill/文档/MCP 投影；输出为 `AndroidPreparedContext`、回答依据集合和 context usage。
归档提交与数据源使用记录由该对象在自己的 repository 边界内完成。`ChatViewModel.kt`
从 2,678 行降到 2,413 行，不再构造 `AndroidContextCoordinator`，也不再持有
`AndroidDataSourceRepository`。Android debug/release 编译和两套单元测试均通过。

复查发现首轮拆分仍把 context composition、摘要模型 IO、archive commit 和数据源披露放在
同一个类里，而且文件仍位于 `ui/screen/chat`，因此只是降低 ViewModel 行数，还没有形成
正确的依赖方向。现在整组上下文能力已迁入 `agent/context`：`ChatContextPreparer` 编排 Core
prepare，`ChatContextSummaryService` 封装摘要模型调用，`ConversationArchiveWriter` 独占
归档边界校验和持久化，`ChatDataSourceUsageRecorder` 独占披露记录。`ChatViewModel` 只注入
preparer 并传递 request，preparer 不再直接依赖 `ModelService`、`ConversationRepository` 或
`AndroidDataSourceRepository`。Android debug/release 编译及单元测试再次通过。

Compose 的附件选取回调原先还直接持有 `ContentResolver`、SAF 持久授权、bounded stream、
PDF/DOCX/文本/图片解码、Base64、digest 和 intelligence ingestion。现已迁入 286 行的
`AndroidAttachmentImportCoordinator`：输入是 URI 批次，输出是保序的成功附件和逐 URI
失败；`ChatScreen.kt` 从 3,140 行降到 2,925 行，只合并 typed result 与 composer UI 状态。
随后增加 `AndroidAttachmentImporter` 并由 `AndroidChatFeatureComposition` 创建默认实现；
`ChatScreen` 只接收可替换接口，不再直接构造具体 coordinator。这个边界仍属于 Android Host
附件导入能力，不进入 Core，也不改变 canonical message/attachment schema。

## 第二阶段文件边界：macOS Transcript

`MacTranscriptView`、其 AppKit scroll observer、窗口扩展、跟随最新和压缩原文定位现在位于
独立的 `Features/Chat/MacTranscriptView.swift`。它的输入是 conversation ID、消息投影、
运行状态、历史分页状态与用户 action closures；输出只有滚动/导航 UI 和 action 回调。
原 `MacChatWorkspaceView.swift` 从 6,416 行降到 5,990 行。macOS 编译以及
`MacAssistantPresentationVisibilityTests`、`MacChatWorkspaceBehaviorTests`、
`MacLargeConversationPerformanceTests` 全部通过。

测试编译同时确认纯持久化值 `PromptTemplate`、`PromptTemplateSnapshot`、
`PromptResponseStyle` 和 `ModelParameters` 被默认 MainActor 隔离。它们现已显式声明为
`nonisolated`，可发送的模型参数同时满足 `Sendable`，对应 Swift 6 Codable/Equatable
告警已消除。仍有 Markdown cache 测试的 closure sendability、少量废弃 API 和未使用结果
告警；这些应按独立并发/测试迁移处理，不能用关闭 warning 或放宽 actor isolation 掩盖。

`MacMessageView` 也已从 workspace 文件移入 799 行的独立 feature 文件，输入保持为单条
message projection、streaming/activity 状态和 task actions。编译发现它依赖的 breathing
indicator 与 pending process view 同时被 workspace/thinking/trace 使用，因此这两个组件
被提取为 105 行的 `MacChatStatusViews.swift`，没有通过扩大原文件 `private` 范围制造新的
隐式耦合。`MacChatWorkspaceView.swift` 进一步降到 5,097 行；同一组可见性、行为和大会话
性能测试再次通过。

继续复查 workspace 后，剩余的运行过程、工具结果、权限与输入区也各自拥有清晰的事件
边界。它们现已分别迁入 `MacThinkingProcessView.swift`（trace projection + streaming）、
`MacTraceView.swift`（单条 trace + 文件打开环境 action）、`MacHostPermissionView.swift`
（request + allow/deny/guidance actions）和 `MacComposerView.swift`（workspace draft/attachment
commands）。拆分时编译暴露出 `MacPressFeedbackButtonStyle` 是 thinking、permission 和
composer 共用却藏在 workspace 的私有依赖；该原语已归入 `MacChatStatusViews.swift`，而非
扩大 workspace 内部类型的访问范围。主文件从 5,097 行降至 3,889 行，新文件均不超过
500 行，macOS 编译通过；对应 presentation、workspace behavior 和大会话性能测试通过。

第三次复查继续处理 Inspector。原来的单个 Inspector 约 1,230 行，同时读取 Prompt、平台
数据、Web Source、Chrome 状态并渲染四个页签。现在 `Features/Inspector` 按职责包含 shell、
Knowledge、Browser、Activity、context metrics、response settings、working context、data
sources、Chrome adapter 和共享展示组件；每个文件均低于 500 行。Chrome 以及其余四项业务
服务由 `NolaMacApp` 构造 `MacInspectorDependencies` 后注入，Inspector feature 中不再读取
这些服务的 `.shared`。`MacChatWorkspaceView.swift` 因而从 3,889 行降到 2,331 行。

随后按同一 presentation 边界迁出 workspace shell、conversation sidebar、sidebar
components、conversation canvas 与 canvas components。Sidebar 只接收 conversation list
projection 和用户命令，Canvas 只编排 transcript、permission、user-input 与 composer；窗口
chrome 和 hover/status 动画分别留在纯展示组件。五个新文件均低于 350 行，
`MacChatWorkspaceView.swift` 进一步降到 1,384 行。

## 实际依赖图

下表只列 Cargo 的生产依赖；测试依赖没有混入。它来自 `cargo metadata --offline
--no-deps --format-version 1`，所以不是按目录名称推断。

| 包 | 现在直接依赖 | 判断 |
| --- | --- | --- |
| `nola_domain` | `serde`, `serde_json` | ✅ 领域值对象和协议事实，仍有少量不透明 JSON，需要继续收口 |
| `nola_kernel` | `nola_domain` | ✅ 无 IO 的纯决策层；完整 Agent 状态尚未迁入 |
| `nola_store_port` | `nola_domain`, `serde` | ✅ 按 bounded context 定义持久化端口；Application 在用例侧组合所需端口 |
| `nola_host_protocol` | `nola_domain`, 序列化/URL | ✅ 传输契约；不应继续承载持久化专用类型 |
| `nola_protocol` | `nola_domain`, 序列化 | ✅ Client/Service wire 契约 |
| `nola_legacy_agent_adapter` | `nola_core` | ⚠ 唯一允许新层触碰旧 Core 的迁移接缝 |
| `nola_application` | Domain、Kernel、Protocol、Store Port、Host Protocol、legacy adapter | ⚠ 已无具体 SQLite，但仍把旧 Core 状态和 Host 类型当作 Application API |
| `nola_store` | Domain、Store Port、Kernel、SQLite | ⚠ 已无 Host Protocol；仍把 Kernel 的业务准入算法放在存储适配器调用 |
| `nola_application_bootstrap` | Application、Store、SQLite | ✅ 具体装配位置正确 |
| `nola_runtime_driver` | legacy adapter、序列化 | ⚠ 是副作用泵，但输入输出仍是旧 Core Envelope |
| `nola_runtime_supervisor` | Protocol、Service Client | ✅ 进程级等待/取消边界；Run/Attempt 完整生命周期仍在旧路径 |
| `nola_service` | Application、Bootstrap、Domain、Protocol | ✅ 服务是写者；内部仍持有大 Application Runtime |
| `nola_mobile_bindings` | Bootstrap、Application、Kernel、Domain、Protocol、`nola_core`、模型 Host | ❌ 移动 FFI 仍直接保留旧 Core ABI 和 Application ABI 两套入口 |
| `nola_cli` | 上述几乎所有运行时、Host、Web、Store 装配和模型库 | ❌ 一个 crate 同时是客户端、Web、Host、工具执行器、恢复协调器 |

`nola_core` 自身仍公开 26 个模块（`core/src/lib.rs:20-47`），并同时导出旧 Agent、
记忆、上下文、调度、规则、模型管理和移动 FFI（`core/src/lib.rs:52-97`）。因此它是
事实上的 god crate，而不只是一个纯 Kernel。

## 运行时所有权

目标链路要求“Client → Application Service → Kernel → Runtime Supervisor → Host →
Store/Projection”。当前真实链路如下：

```text
macOS / Service / Mobile FFI / CLI
       ├─ ApplicationCommand / Projection
       ├─ 旧 AgentCore::restore / try_new / submit_at
       ├─ HostRequestEnvelope / CoreCheckpoint
       └─ SQLite Repository / receipt / outbox
```

`ApplicationRuntime<R>` 的文档说它独占 Store（`core/nola-application/src/runtime.rs:1-25`），
但它同时缓存可变的 `AgentCore`（同文件的 `RuntimeSupervisor` 和
`reduce_*_with_core` 调用，约 `:200-238`）。这使 Application 既是用例协调器，又是旧
Reducer 的运行时容器；Kernel 无法成为唯一状态转换者，冷恢复也必须理解旧 checkpoint
格式。

`CommittedRuntimeDriver` 的设计方向是对的：只派发已提交 Effect，不能有未提交工作队列
（`core/nola-runtime-driver/src/lib.rs:69-114`）。但它的 Port 仍返回
`CoreCheckpoint<AgentThreadState>` 和旧 Host Envelope，因此它只是把旧边界搬到一个新
crate，并没有把 Runtime 变成“Application Port + typed Effect”的独立层。

`nola_runtime_supervisor` 已经拥有取消、等待和 worker 生命周期接口，但它只通过
Service Client 观察 `RuntimeEffectWorkSchedule`。真正的 Run/Attempt/Effect 状态仍由
Application 调旧 Core 和 Store 方法装配，持续 Supervisor、跨进程 lease 和 Host 不可用
恢复尚未统一成一个状态机。

## 封装和面向对象问题

Rust 不需要用类继承表达业务，但需要用模块私有状态、受控构造、不可变输入和端口隔离
表达“谁拥有行为”。当前有四类封装缺口。

### 1. 类型被重新导出成跨层公共状态

`nola_application/src/lib.rs` 在文件顶部直接导入旧 Core 的 capability、context window、
Host、Part、Permission、Protocol、Checkpoint、Task 和 Memory 类型
（`:132-155`），然后把它们用于公共命令、回执和投影。外部平台可以因此绕过 Application
不变量，直接构造旧 Core 的 Part/Checkpoint。`nola_legacy_agent_adapter` 现在把这条路
集中到一个可审计接缝，下一阶段必须把这些类型逐一替换为 Domain/Application DTO，而
不能继续向 adapter 添加模块。

### 2. Application 的仓储并集必须停在 wire adapter

旧 `ApplicationRepositories` 把约 35 个会话、运行时、配置和运维端口授予 Runtime 与
command admission。它现已删除；仓储目录建立了 conversation、agent runtime、capability
catalog、operations 和 protocol 五组组合 trait。前四组现在已经分别成为 conversation、
agent runtime、capability catalog 和 operations 四个
dispatcher 模块的实际参数。总入口以穷举的 `ApplicationCommandFamily` 分类 wire command，
每个子 dispatcher 只取得自己的 family trait。`ApplicationCommandRepositories` 并集只由
wire dispatcher adapter 使用；Runtime 依赖 sealed `ApplicationCommandExecutor` 加
恢复/Effect 所需窄端口，receipt admission 依赖 `ReceivedCommandRepositories`。

这同时解决了 Runtime、command receipt 和 family handler 的越权依赖。wire enum、family
分类和协议转换仍集中在 `application_command.rs`；后续若该文件继续增长，应拆版本化 wire
schema，而不是把 repository 并集重新传入子 handler。`nola_application` 的 119 个单元测试
及全部该 crate 集成测试通过。

### 3. `NolaStore` 仍是数据库 god object

Store 根文件已从约 9,488 行降到 1,544 行。runtime task claim/lifecycle/permission、
automation、data source、agent command、trace journal、work-step transaction、SOP run 和
MCP registry，以及 workspace checkpoint、session run lease、product setting、model 和
conversation plan、checkpoint、effect outbox、Skill、permission grant 和 sync mutation 的
SQL `impl` 已按输入输出迁入独立文件；turn admission、conversation projection/summary/
transition、context checkpoint、conversation read、presentation input、remote session、active
run 和 schema 也已形成独立模块。根文件已不再定义公开 `NolaStore` 方法，仍保留共享 SQL
列常量、row codec、校验函数和 sync identity helper；这些内部支撑代码还需归入各自模块。
这些方法仍挂在同一个公开 `NolaStore` 上。Repository port 的存在和文件拆分都没有自动
产生对象封装；要让边界成立，必须把
`SqliteUnitOfWork` 私有化，只由 Bootstrap 创建各聚合 repository，并禁止业务代码拿到
裸连接或全能力 Store。

### 4. 平台层大量使用全局对象和超大对象

macOS 有 28 个 `static let shared`（25 个 Swift 文件），`MacChatWorkspaceView.swift`
拆分前有 12,311 行，`MacApplicationPort.swift` 拆分前有 3,826 行；iOS 的 `ChatManager.swift`
已从 3,252 行降到 3,028 行，`IOSApplicationRuntime.swift` 有 2,250 行；Android 的
`ChatViewModel.kt` 有 3,184 行。全局单例把服务生命周期、缓存、权限和 UI 状态绑在一起，
也使测试很难建立独立的 Application Client。应让根组件装配依赖，Feature/ViewModel 只
持有窄接口和投影 reducer。

构造参数也在泄漏职责。例如 `MacLocalAgentRunRequest` 同时携带 endpoint、apiKey、
context window、workspace、数据库覆盖、Application Port、Skill/MCP 目录和权限模式
（`NolaMac/Services/MacLocalAgentRuntime.swift:10-37`）。模型路由、凭据、Host 沙箱、
Application 写入和 UI 请求应该是不同的对象；把它们放进一个 request struct 并不会形成
封装，只是把耦合搬到了调用方。

### 5. 文件布局没有表达已经存在的概念边界

本轮开始前，`MacChatWorkspaceView.swift` 同时定义 conversation/browser/message/attachment
值对象、工具与思考展示策略、滚动窗口算法、Workspace 状态机和全部大型 View；
`ChatViewModel.kt` 同时定义 `ChatUiState`、纯 reducer、Host permission/tool 格式和所有
Repository/Provider 副作用；Store 的 memory SQL 方法与其 row codec、同步投影、校验函数
相隔约 1800 行。结果是改动一个展示字段也会重新理解持久化、恢复和 Host 生命周期。

第一批文件级迁移后的客观变化如下。数字用于判断迁移方向，不作为架构完成标准：

| 原热点 | 复查前 | 当前 | 已形成的职责文件 | 仍未解决 |
| --- | ---: | ---: | --- | --- |
| `MacChatWorkspaceView.swift` | 12,310 行 | 1,384 行 | workspace layout、window/overlay navigation、deferred reader | workspace shell 仍直接订阅进程内通知，需由 typed update adapter 替代 |
| `MacWorkspaceModel.swift` | 内嵌于 View | 约 4,565 行 | Feature state、window cache、archive scheduler、residency policy、archive count state、remote run identities、remote task observation | 仍同时编排附件、Local/Remote Run、权限和主会话 projection，应继续提取 collaborator |
| `ChatManager.swift` | 3,252 行 | 3,028 行 | public state/error、text policy、context usage ledger、remote task observation、Host permission session、memory-worker scheduler | 仍同时编排本地权限队列、本地/远程模型、Core run 和消息提交，应继续迁移到 Application Client/use case |
| `ChatViewModel.kt` | 3,183 行 | 2,254 行 | context use case、load/turn/permission/remote-observation/trace coordinators、state/presentation policy | 构造器依赖和 coroutine 入口仍过多；消息提交、Provider/Host 执行与数据源编辑仍需收敛 |
| `application_command.rs` | 1,574 行 | 972 行 | conversation、agent runtime、capability catalog、operations dispatchers | wire enum、family 分类和多组协议转换仍集中；后续按 wire schema/version 拆分 |
| `nola-store/src/lib.rs` | 约 9,500 行 | 1,544 行 | 全部公开 Store 方法已迁入 bounded-context 文件，schema 入口只对父模块可见 | 根文件仍有共享 codec/validation；已拆模块尚未成为独立 repository 对象 |

拆分过程还暴露了两个隐式接口：浏览器状态通知原来是 View 文件中的 `fileprivate`
常量，但 Workspace Model 也在发布它；Host 本地化函数跟随 conversation value 被误移入
Models。前者已经收口到 Chat feature event 文件，后者留在 UI 展示边界。这类编译错误
说明原来的文件私有并不等于封装，真正的模块契约必须按调用方向放置。

## 各层的职责核对

### Domain 和 Protocol

这两层目前是最接近目标的部分。Domain 已承载稳定 ID、Part/正文引用、同步事实、权限
指导等跨平台语义；Host Protocol 重新导出 Domain 的 `PermissionInstruction` 后，不再
需要让 Store 依赖传输 crate。剩余问题是 `serde_json::Value` 仍可从 Domain/Protocol
边界一路传到 Application；它适合外部 provider 的不透明 payload，不适合 Message、Run、
Effect、Permission、Memory 和 UI projection 的规范字段。每个跨层 Value 都应有版本化
的 typed wrapper 和大小/身份校验。

### Kernel 和旧 Core

`nola-kernel` 的 25 个模块、约 7,723 行已经覆盖 admission、权限规则、上下文预算、
压缩规划、模型完成策略、Run recovery、Effect delivery 和投影 reducer；这是正确的纯
函数方向。但旧 `core/src/agent` 仍有约 37,936 行，最大的 submission/runtime/tool
driver/turn 文件分别约 2,780/2,251/2,438 行，并持有完整 Agent 状态机。当前最危险的
不是算法重复，而是两边都能解释同一个 checkpoint、Part 和 tool lifecycle。必须按
状态所有权迁移：先定义 Kernel 的 `AgentState + Command -> Decision`，再让旧 Core 只
做 ABI/解码适配，最后删除旧 reducer；不能长期保留“双 reducer + 互转”。

### Application

Application 的模块数量增加了，但根 `lib.rs` 仍约 4,286 行，包含命令分发、Core 恢复、
投影装配、Host response、memory worker、权限、SOP 和附件等行为。它应该只做四件事：
加载聚合、调用 Kernel、在一个 Unit of Work 中提交事件/Effect/receipt、发布 projection。
Provider 错误分类、工具执行、Prompt 组装、Markdown 解析和平台本地化都不应进入这个
层。完成迁移后，Application 的公共 API 应只暴露 `ApplicationCommand`、typed receipt、
projection 和 Host/Application Port，不暴露 Core checkpoint 或 provider envelope。

### Store 和 Store Port

端口已覆盖很多用例，且 Bootstrap 已是具体 SQLite 的装配点，这是可保留的方向。缺口有
两个：一是 `nola-store` 仍调用 `nola_kernel` 的 message/Part/body/effect 准入算法，
说明存储适配器在做业务决策；二是 Store 的所有 repository trait 最终仍汇聚成可任意
调用的 `NolaStore`。远程同步的决策应由 Kernel/Application 先产生，Store 只执行带版本
和身份证明的 SQL 事务；若某个校验确实是数据库约束，应下沉为 Store Port 的持久化
不变量，不要从 Kernel 反向调用。

### Host、Runtime 和模型 Provider

Host 应该是 Effect 执行器和供应商协议转换器，不能决定工具路由、权限 grant、记忆选择或
会话标题。Runtime 应拥有 Attempt 隔离、lease、取消和 backpressure，不能拥有业务 Store
写权限。现在 CLI 的 `agent_host_driver_parts/orchestration.rs` 约 5,490 行，
`llm_parts/streaming.rs` 约 4,042 行，`external_executor.rs` 约 4,153 行，表明 Host、
Provider、工具和重试仍在一个进程包内交叉调用。应先把 Provider response 转成一个小的
typed `HostEvent`，再由 Application 提交；provider JSON、SSE、音视频和 shell 日志不应
穿过 Application 公共 API。

### 客户端和平台

- **macOS**：已有 Application Service Client 和 Projection，但本地 Agent 请求仍把
  凭据、模型路由、Skill/MCP、Host 权限和 Application Port 组合在一起；`shared` 单例
  使服务依赖无法替换。客户端应只发 Command/订阅 Projection，Agent Host 作为独立
  进程通过 Effect 运行。
- **iOS**：同时存在 `AgentCoreRuntime`、Prompt Composer、Tool Catalog、Tool Policy、
  Memory Selection、Context Manager 和 Rust FFI。模型和工具选择逻辑仍可在客户端执行，
  会与 macOS/CLI 产生行为分叉。移动端只应保留 UI 状态、模型下载和 OS 权限适配；能力
  目录、SOP、记忆召回和上下文压缩交给 Application/Kernel。
- **Android**：`NolaDatabase` 直接声明 `ScheduledActionEntity` 和
  `ExecutionResultEntity`（`android/app/src/main/java/com/nola/data/local/db/NolaDatabase.kt:14-28`），
  `ScheduledActionRepository` 直接对 DAO 做 upsert/delete（`:12-40`）。这些是计划任务
  和运行结果的领域写入，绕过了唯一 Application 写者；Room 应退回 UI 缓存/通知投影，
  任务创建、claim、heartbeat、finish 必须成为 Application Command。
- **CLI/Web**：`nola_cli` 直接依赖 Application、Runtime Driver、Supervisor、Service、
  Host Protocol、旧 Core 和本地模型/工具库（`cli/Cargo.toml:23-56`），同时拥有交互
  客户端、Web transport、Host 执行和恢复协调。应拆成 composition root、Client/UI、
  Agent Host、Web adapter 四个明确模块或 crate。

### Code Memory

`nola-code-memory` 作为原生进程内能力的方向是正确的：引擎拥有索引、语法抽取、图查询
和 SQLite，不依赖 MCP 子进程（`core/nola-code-memory/src/lib.rs:1-22`）。它应位于
Host Capability Adapter，Application 只看到 `CodeMemoryPort` 和有界结果。当前语言
grammar 仍以 Cargo feature 方式静态链接 67 个实现。运行时已经通过 thread-local parser
按线程、按语言首次使用时创建，并根据 `available_parallelism` 有界并发索引；不能再把这
两项列为未实现。剩余性能工作是裁剪默认发行包的静态 grammar 集、按项目语言限制索引
任务和量化启动/链接/产物体积。Verilog 保留在支持集，低于使用率门槛的语言不进入默认
集合。索引数据库也要和会话 Store 明确隔离，不能让 Code Memory 通过通用 `NolaStore`
取得业务写权限。

## 需要保留的好设计

1. Application Service 作为单写者、receipt 和 outbox 的目标没有问题；Bootstrap 也已将
   具体 SQLite 装配移出 Application。
2. Delta 是可丢弃预览、完整 Part 才是规范内容；正文引用、范围读取和有界投影符合长
   文本设计。
3. `CommittedRuntimeDriver` 只派发已提交 Effect，lease/heartbeat/owner/token 具备
   可恢复基础。
4. Kernel 的纯 admission、context/compaction 规划、模型完成和权限规则已经提供了
   迁移目标，不需要再新建一套平行规则。
5. 最近新增的 provider quota gate、严格工具协议恢复和 artifact convergence checkpoint
   说明错误分类应继续留在 Host/Application 协议边界，而不是在 UI 中猜测失败原因。

## 统一迁移顺序

顺序按“先所有权、再类型、最后平台表现”排列。每一步删除被替代路径后才算完成，不能
以转发函数或兼容别名代替。

### 0. 边界门禁（本轮开始）

- 保留 `nola_legacy_agent_adapter`，禁止新的 Application/Runtime 模块直接引用
  `nola_core`。
- Domain 拥有跨 Store/Host 共用的中立值对象；Store 不依赖 Host Protocol。
- 在 `workspace_dependency_architecture` 中锁定这两条生产依赖规则。

### 1. 迁移 Run/Attempt/Effect 状态所有权

- 在 Domain 定义 typed Run、Attempt、Effect、HostEvent 和失败分类。
- Kernel 接收完整状态和命令，返回事件与 Effects；不返回旧 Core Snapshot。
- Application 只提交 Kernel 决策，Runtime 只 claim/lease/execute 已提交 Effect。
- 旧 Core 通过 adapter 读取同一 typed 状态；每迁移一个状态机就删除一份旧 reducer。

验收：冷启动恢复、取消、租约过期、重复 Host response、Provider quota、未知副作用和
跨进程重启都只经过一条状态转换路径。

### 2. 收缩 Application API 和仓储权限

- 为四个 command-family dispatcher 增加独立 fake repository 合约测试，并继续把具体 use
  case 从 family 组合 trait 收窄到单一 repository port；总并集只保留在 sealed wire adapter。
- 把 `ApplicationRuntime` 拆成 `ApplicationService`（命令/事务）和私有 `RunSession`
  （热状态缓存），热状态只可由 supervisor 使用，不能成为公共 DTO。
- 将 Core Part/Checkpoint/Host 类型替换为 Application/Domain DTO，逐步删除 adapter
  模块引用。

验收：编译期无法从客户端构造 Core checkpoint；非 SQLite fake repository 可覆盖每个
用例；每个命令的写集合在单一 Unit of Work 中可列举。

### 3. 重构 Store 为聚合适配器

- `NolaStore` 只保留私有连接和 Unit of Work；repository 实例由 Bootstrap 装配。
- 将 Kernel 的业务准入移回 Application/Kernel；Store 只做 SQL 事务、CAS、索引和大小
  限制。
- 统一正文、JSON、Artifact 和 Code Memory 的 Blob/引用读取端口，禁止全量 List 回读。

验收：Store crate 不依赖 Kernel；每个 repository 可单独测试；事务日志能证明 checkpoint、
event、effect、receipt 同提交；大正文查询保持有界。

### 4. 收敛 Host/Runtime/CLI

- Provider 适配器只输出 typed HostEvent；工具/SOP/记忆由能力目录和 Kernel 选择。
- Runtime Driver 依赖 Application Port 与 Host Port，不认识旧 Core 类型。
- CLI 拆出 Client/UI、Web adapter、Agent Host、Service composition root；Web 不直接
  调 Host 或 Store。

验收：一个真实模型会话、工具失败重试、Skill/SOP 激活、代码记忆自主调用和 artifact
  完成均能从 trace 看见同一 Run/Attempt/Effect 身份。

### 5. 移动和原生 UI

- iOS/Android/macOS 统一 Command/Projection SDK；客户端不再组 Prompt、工具目录、权限
  决策或记忆召回。
- Android Room 仅保存通知和 UI cache，计划任务改走 Application。
- macOS 取消业务单例注入，按场景装配 Application Client、Projection Store、Host
  process 和平台能力。
- Markdown 只重绘变化的尾块，投影更新按帧合并；完成事件不再二次替换规范正文。

验收：五端同一事件序列得到相同 projection；长会话分页、多窗口切换、恢复和动画帧率
分别测量，不能用 Service/Rust 性能数字代替 UI 验收。

## 本轮施工和验证

代码改动：

- 新增 [`core/nola-legacy-agent-adapter`](../../../core/nola-legacy-agent-adapter)，Application
  和 Runtime Driver 的旧 Core 入口全部改为该显式接缝。
- 新增 [`core/nola-domain/src/permission_instruction.rs`](../../../core/nola-domain/src/permission_instruction.rs)，
  Host Protocol 只重导出该领域类型，Store 移除 Host Protocol 依赖。
- 更新 [`core/nola-application/tests/turn_transaction_e2e.rs`](../../../core/nola-application/tests/turn_transaction_e2e.rs)，
  用真实 `AgentCore::checkpoint` 生成完整 schema fixture，保持生产 checkpoint 校验严格。
- 扩展 [`core/nola-domain/tests/workspace_dependency_architecture.rs`](../../../core/nola-domain/tests/workspace_dependency_architecture.rs)，
  锁定 Application/Runtime Driver 不得直接依赖旧 Core、Store 或 SQLite，且 legacy adapter
  必须是一条单向依赖。

已运行：

```text
cargo check -p nola_application                         PASS
cargo check -p nola_runtime_driver                     PASS
cargo check -p nola_store                              PASS
cargo test -p nola_domain                               PASS (14 unit + 6 integration)
cargo test -p nola_host_protocol                        PASS (13 unit)
cargo test -p nola_domain --test workspace_dependency_architecture PASS (10)
cargo test -p nola_application --lib                    PASS (118)
cargo test -p nola_application                          PASS (all suites)
cargo test -p nola_runtime_driver                       PASS (2 + 11)
cargo test -p nola_store                                PASS (all suites)
cargo check --workspace                                PASS
cargo check -p nola_cli --bins                          PASS
```

这组结果只证明本轮边界改动可编译且门禁生效，不代表完整 V2、五端 UI、真实模型或
Terminal-Bench 已完成。下一次施工应从第 1 阶段的 typed Run/Attempt/Effect 所有权开始，
而不是再向客户端或旧 Core 增加局部转发逻辑。
