> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Nola V2 Module Contracts

状态：Normative

当前整体改造顺序、参考资料对比和验收证据见
[`MODULAR_REDESIGN_2026-09-28.md`](MODULAR_REDESIGN_2026-09-28.md)：第8节是本轮goal
累计交付与当前验收状态，第9节保留分批成功/失败历史。本文仍是Normative合同，
已实现源码、阶段性通过与完整迁移必须分别记录；未迁移的边界仍是缺口。

本文把 [`ARCHITECTURE_V2.md`](ARCHITECTURE_V2.md) 的分层原则落实为可检查的模块契约。
阶段性施工记录不得修改这里的所有权、输入输出和依赖方向；实现尚未满足契约时，应记录为迁移缺口。

## 1. 模块化原则

1. 一个模块只有一个状态所有者和一个变化原因。
2. 输入与输出使用领域类型，不以 JSON 字符串、日志文本、UI 标题或数据库行跨越业务边界。
3. 状态只能通过构造器和命令方法改变。调用方不能取得可变内部集合、数据库连接、stream buffer 或 reducer 状态。
4. 跨层依赖面向端口。Application 定义用例需要的端口，Store、Host 和平台实现端口。
5. 组合优先于继承和全局单例。Swift/Kotlin 使用 protocol/interface 注入，Rust 使用窄 trait 与拥有状态的 struct。
6. 文件拆分必须对应职责拆分；仅建立 `parts` 文件并共享全部内部状态不算完成模块化。
7. 每个公开接口都必须写清调用前置条件、成功输出、失败类型、幂等键和资源上限。

### 外部工作区访问合同

默认执行目录、任务目标目录、可访问目录集合、操作批准是四种不同事实，不得互相推导。
`workspace_access { root: absolute_directory }` 是目录扩权的显式能力：Core 按具体输入生成
permission request；Mac 只解码该能力的固定 schema，通过内置申请面板取得用户决定。
平台适配器另行确认 OS 可访问性；仅当当前沙盒确实要求额外 OS 授权时，才进入系统授权
流程。内置批准不伪造 security-scoped bookmark，已有 OS 访问能力也不替代用户批准。
Host 在该能力成功后更新本会话 allowed roots，再由模型继续原任务。
普通 read/write/bash 的批准不扩大 roots；推荐权限模式也必须确认目录扩权。
UI 禁止递归扫描任意工具参数猜测目录，禁止从用户文字提取授权。

越界返回结构化 `workspace_access_required` 和恢复能力，不执行原操作，也不是“目标不存在”
或“目标为空”的证据。无法从 shell 安全确定具体资源时，不自动授权父目录，由模型依据任务
提交精确目录。批准 access 不批准后续写入、不改变当前 Host 默认 cwd；后续工具显式带 root/cwd。
Mac 的持久书签和会话 OS 句柄由平台适配器持有，不是 Core 的操作 permission grant。
切换可见会话不释放其他会话句柄；目录选择校验先于持久化；异步恢复不能覆盖新选择。

Mac 以每会话 workspace access record 保存多个 OS 书签和独立 primary ID；附加授权不改变
默认 cwd。启动每轮 Host 前重新激活全部书签，只把激活成功的目录写入不可变 launch roots。
失效书签不得被路径字符串替代为权限；保留记录以供重新授权，不静默删除。取消、选错目录、
书签生成失败不得新增权限；授权时不接受比请求目录更宽的父目录。切回 sandbox 会清除本会话
书签与句柄，但必须等运行结束，不能把 UI 状态变更冒充对活跃 Host 的权限撤销。
Mac 对 `workspace_access` 向 Core 仅交付 allow-once；目录的持久授权只归书签 registry，
不得再保存一份 conversation tool grant，否则清除书签后旧工具 grant 可绕过 OS 重新授权。
验收必须从空 session workspace 访问外部 fixture 开始，不能只测预先配置好的仓库。

### 原生交互汇合与取消合同

Core 权限投影与 Host nonce challenge 没有跨流顺序保证。两侧事件均须触发幂等汇合，
按会话、generation、request/call/capability 标识匹配；只有最新 Core pending 集合中的
请求可展示，challenge 不能单独创建授权事实。旧 revision、已取消请求及停止中的
generation 不得复活权限卡片，异步查询返回后必须再次验证身份与最新投影。

原生停止信号必须同时设置 execution cancellation 和唤醒所有交互等待，不伪造权限
决定。Host 退出前由 Core/Application 提交 Interrupt 并持久化终态；UI 在确认前仅展示
正在停止。强制退出兜底只能作用于仍登记的同一个子进程，不能仅依据裸 PID 是否存在。
验收包含双向事件乱序、旧事件重放，以及真实 Host 在等待授权时停止后的数据库终态。

Native 协议输出由一个有序 writer 持有。Core 把 permission trace 交给 Host 补充 nonce
时，必须在同一 writer 中预留该 trace 的发送位置；后续投影排在该位置之后。Host 提交
challenge 时填充预留位置，并在同一互斥区同步写出、flush challenge 及排队投影，然后
才允许进入交互等待。禁止直接 stdout 旁路和“已送达 ID + 下一条事件触发排空”。
输出失败、队列超过 1024 项/32 MiB 或交互仍被其他预留位置阻塞，必须返回传输错误，
不能宣称已交付后无限等待。恢复历史运行时不得创建新运行的输出预留位置。
测试必须在不发送决定、不取消、不注入后续事件的情况下先收到 challenge 和匹配投影；
Mac 验收必须经实际 WorkspaceModel 呈现 pending card 后才可答复，禁止原始 challenge
直通自动批准绕过 UI 消费条件。

输出产物与验证目标必须通过同一冻结资源描述映射到 Core 资源身份。`bash` 的
`output_artifacts` 对象数组须声明 `path` 资源字段并以 `cwd` 解析相对路径，不能只识别
字符串形式的 validation_targets，更不能把已声明的具体产物降级成未知资源 `*`。
operation 等元数据不是资源；完成判定仍要求验证证据覆盖实际变更资源。

### 模型完成判定事实

模型完成判定中的“没有审计报告”和“报告不合格”必须是不同事实。适配器不能把缺省报告
转为空字符串再制造失败；是否必须审计由 required/present 字段决定，violations 只描述
实际存在的报告。否则无需审计的普通长回答会绕过重复续写的丢弃分支，产生重复正文。

## 2. 规范运行链路

```text
Platform UI
  -> Presenter / ViewModel
  -> Typed Client SDK
  -> Application Command | Query | Subscription Port
  -> Application Use Case + Unit of Work
  -> Pure Kernel Decision
  -> Store Commit + Effect Outbox
  -> Runtime Supervisor
  -> Host Adapter
  -> Application Result Command
  -> Projection Stream
  -> Presenter / ViewModel
```

任何模块都不能跳过中间所有者。例如 UI 不消费 provider delta 作为历史，Host 不写消息，Store 不决定工具，Runtime 不直接提交业务 SQL。

## 3. Rust 服务端模块

### 3.1 `nola-domain`

| 项目 | 契约 |
|---|---|
| 输入 | 已验证的标量、Value Object、领域命令数据 |
| 输出 | 领域实体、不可变投影值、稳定错误码 |
| 内部功能 | ID、Conversation、Message、Part、Run、Attempt、Effect、Task、Memory、Permission 等领域类型与不变量 |
| 接口 | 无 IO 的构造器、校验函数和状态查询 |
| 禁止依赖 | SQLite、HTTP、异步 runtime、文件、进程、平台 UI、provider SDK |

### 3.2 `nola-kernel`

| 项目 | 契约 |
|---|---|
| 输入 | `State + Command/Event + Policy Facts` |
| 输出 | `NewState + DomainEvents + Effects` |
| 内部功能 | turn、tool、permission、context、compaction、memory selection、loop guard、goal/skill/SOP 状态机 |
| 接口 | 确定性 reducer 与纯策略函数；同一输入必须得到同一语义结果 |
| 禁止依赖 | Store、网络、文件、时钟读取、随机数、进程、平台 API、本地化文本 |

Kernel 需要时间或 ID 时，由调用方在命令中提供。Kernel 不能通过 Host 结果字符串猜测工具状态。

### 3.3 `nola-application`

| 项目 | 契约 |
|---|---|
| 输入 | 带 request ID、actor、scope、expected revision 的 typed command/query |
| 输出 | command receipt、typed query result、projection revision/event、提交后的 Effect |
| 内部功能 | 用例编排、事务、aggregate load/reduce/save、幂等、outbox、恢复协调 |
| 接口 | `ConversationCommands/Queries`、`RunCommands/Queries`、`ContextPort`、`MemoryPort`、`TaskPort`、`PermissionPort`、`CapabilityPort` |
| 禁止依赖 | 具体 SQLite 连接、provider SDK、UI 类型、Shell、平台权限 API |

每个用例只依赖所需的窄端口。完整仓储并集只允许出现在 wire command dispatcher
适配器中；Runtime、receipt admission 和具体业务函数不得取得该并集。

当前 Application 的仓储边界如下：

| Application 模块/端口 | 输入 | 输出 | 内部功能与边界 |
| --- | --- | --- | --- |
| `ApplicationCommandExecutor` | `ApplicationCommand` | JSON-compatible command result 或 typed failure | Runtime 使用的命令执行端口；隐藏 dispatcher 的完整仓储并集 |
| `ApplicationCommandRepositories` | 四类 command-family repository capability | dispatcher 可调用的 repository adapter | 仅供 wire dispatcher 编译期装配，不作为业务 use-case 参数 |
| `application_command/conversation` | conversation family command、`ConversationCommandRepositories` | JSON-compatible command result | 归档、正文、会话设置与 projection；不能取得 Agent/catalog/operations ports |
| `application_command/agent_runtime` | runtime family command、`AgentRuntimeCommandRepositories` | JSON-compatible command result | Run、Effect、task、permission、memory、SOP 与 trace；不能取得 catalog/sync ports |
| `application_command/capability_catalog` | catalog family command、`CapabilityCatalogRepositories` | JSON-compatible command result | Skill、MCP、model、context、data/document source 与 product setting |
| `application_command/operations` | operations family command、`OperationsCommandRepositories` | JSON-compatible command result | automation、workspace ownership、lease 与 replica sync |
| `ConversationCommandRepositories` | 会话、归档、正文、投影相关 repository ports | 会话 command 所需读写能力 | 不授予 agent runtime、catalog 或后台运维能力 |
| `AgentRuntimeCommandRepositories` | run、task、permission、memory、SOP、trace、effect ports | Agent command 所需读写能力 | 不授予模型/MCP/数据源配置或同步能力 |
| `CapabilityCatalogRepositories` | Skill、数据源、设置、model、MCP、context、document ports | capability catalog command 所需能力 | 不执行 Host capability，不决定模型选择 |
| `OperationsCommandRepositories` | automation、workspace ownership、replica sync ports | 后台运维 command 所需能力 | 不读写聊天正文或 Agent 热状态 |
| `CommandProtocolRepositories` | command receipt、transaction ports | command admission 所需 durable protocol 能力 | 不执行业务 command，不读取 capability catalog |
| `RuntimeExecutionRepositories` | `ApplicationCommandExecutor`、transaction/receipt provider、Run effect、delivery、maintenance 及恢复所需窄端口 | command result 与 Runtime 恢复/投递结果 | Runtime 不直接取得完整 command-family trait union；裸 durable flush 与 received receipt 共用 canonical transaction owner |
| `ReceivedCommandRepositories` | Runtime execution、receipt/transaction、projection journal ports | durable receipt、mutation 和 projection journal | 只用于 command-envelope admission 的原子边界 |
| `skill_catalog` | `SkillRepository`、typed `SkillInput` 或 Skill identity | `Skill`、启停/删除结果、`SkillCatalogProjection` | 独占 Skill CRUD 和 Host descriptor 投影；不取得 Session、Run、Store 实现或 Host 执行能力 |
| `session_projection` | `SessionProjectionRepository` / `RemoteSessionRepository`、conversation/Host identity、Core snapshot | 严格校验的 `SessionProjection` 或 remote mirror | 独占本地 checkpoint/远端 mirror 解码、identity/cursor 校验与历史 catalog 脱敏；不执行 Host、不恢复 reducer |
| `maintenance_turn` | `RunCreationRepository` / `RunEffectRepository`、typed maintenance submission 或 memory job identity | accepted maintenance Run、claimable job handles、frozen worker plan | 独占维护回合 admission 与 memory worker 查询；拒绝交互输入和重叠 active Run，不取得完整 dispatcher/store |
| `assistant_turn` | `RunCreationRepository + CanonicalHistoryRepository`、stable submission identity、Core config 与用户/自动输入 | `AcceptedAssistantTurn`、首批 Host effects 与 canonical checkpoint | 独占普通/内部 assistant turn admission、canonical context 恢复、初始 effect staging；拒绝缺失 submission ID、重叠 Run 和无外部 effect 的 transition |

Conversation command 的核心写侧通过借用式 `SqliteConversationWriteRepository` 组合
profile/settings、branch、conversation deletion 与 transcript append/edit；这些写入共享
Application 所有的 Unit of Work，`NolaStore` 不再直接实现对应 Store Port。Revert 暂不并入
该对象：现有 revert planning 同时读取 canonical history 与 active Run，必须先把 planning query
提取成独立窄读端口，才能在不把 projection/read 能力重新授予 writer 的前提下完成切换。
Remote session、archive 与 projection feed/read 也保持独立边界。

Command admission 通过借用式 `SqliteCommandProtocolRepository` 将 receipt 与 transaction
控制保留在同一 Unit of Work；`NolaStore` 不再直接实现这两个端口。Projection journal
暂不并入该对象，因为当前 `ConversationProjectionRepository` 仍把
`ProjectionJournalRepository` 作为 supertrait，且只读 projection window/feed 共同依赖该
复合端口。后续必须先把 journal cursor/event read 从 conversation projection port 分离为窄
read port，再移除 `NolaStore` 的 projection-journal 直接实现；不得以另一连接或独立事务规避
此依赖。

### 3.4 `nola-store-port`

| 项目 | 契约 |
|---|---|
| 输入 | Application 定义的 repository command/query DTO |
| 输出 | 持久化领域记录、CAS/冲突结果、有界分页 |
| 内部功能 | Repository trait、Unit of Work trait、cursor 与 receipt 契约 |
| 接口 | 按 bounded context 拆分的 trait；不得公开 SQL、`rusqlite::Row` 或连接对象 |

### 3.5 `nola-store`

目标文件结构：

```text
src/
  connection.rs          # SQLite 打开、schema gate、事务原语
  conversation/          # 消息、Part、标题、分支、删除、读取窗口
  run/                   # Run、Attempt、Effect outbox、恢复
  permission/            # grant、request、resume
  context/               # checkpoint、正文引用、compaction source
  task/                  # durable task、lease、event cursor
  memory/                # record、usage、consolidation persistence
  capability/            # Skill、SOP、MCP、model profile
  automation/
  sync/
```

| 项目 | 契约 |
|---|---|
| 输入 | Store Port DTO 和事务上下文 |
| 输出 | 记录、分页、CAS 结果、明确 `StoreError` |
| 内部功能 | SQLite WAL、schema、索引、事务、row codec、分页、lease 原语 |
| 接口 | 每个 Repository 独立 `impl`；`NolaStore` 只作为连接/事务容器和适配器装配根 |
| 禁止依赖 | Provider、Tool/Skill 选择、Prompt、UI 投影格式、本地化 |

`NolaStore` 不再增加新的跨领域公开方法。新增能力先定义 Store Port，再放入对应 bounded context 模块。

当前 Store 的文件级 bounded context 契约如下。它们仍共享由 `NolaStore` 私有持有的 SQLite
连接，因此这一阶段只建立源码所有权和审查边界；它没有完成 repository 对象封装，也没有
消除全能力 Store facade。

| Store 模块 | 输入 | 输出 | 内部功能与边界 |
| --- | --- | --- | --- |
| `runtime_task_claim_store` | task record、worker identity、claim time、lease deadline | 持久化 task、attempt 或 lease/CAS 结果 | task admission、查询、公平 claim、resume 和 lease renew；不处理运行结果 |
| `runtime_task_lifecycle_store` | 已 claim attempt 的 progress/suspend/complete/recover mutation 与事件 | CAS 后 attempt/task、恢复结果、task event journal | task 状态迁移、重试与事件游标；调用 claim 模块的私有 insert 原语，不拥有 permission request |
| `runtime_task_permission_store` | permission request、resolution 和时间 | request snapshot 或更新状态 | 只拥有 runtime task 的 permission request 表和校验 |
| `automation_store` | automation definition、schedule claim、heartbeat、retry/completion mutation | durable definition/run 与 lease/CAS 结果 | `SqliteAutomationRepository` 私有借用 owner Unit of Work；`NolaStore` 不再实现 `AutomationRepository`；不以进程退出码解释业务成功 |
| `data_source_store` | source contract、conversation override、usage disclosure | source/override/usage record | `SqliteDataSourceRepository` 私有借用 owner Unit of Work，并同时封装只读 supertrait；`NolaStore` 不再暴露 data-source repository；不执行检索或选择数据源 |
| `document_source_store` | scope、document index replacement/search request | document record、summary 与有界结果 | `SqliteDocumentSourceRepository` 私有借用 owner Unit of Work；`NolaStore` 不再暴露 document-source repository；不解释或执行检索工具 |
| `agent_command_store` | session-scoped command、consumer lease identity 和期限 | enqueue/claim/release/acknowledge 结果 | 保证 command 顺序、租约和幂等消费；不执行命令内容 |
| `trace_journal_store` | 带 trace/run/conversation/turn identity 的有序事件 | append 去重结果、按 conversation/run 查询的事件流 | 只拥有 append-only trace journal 与 cursor 查询；不解释展示文本 |
| `work_step_transaction_store` | work-step preparation/advance mutation、expected revision 和可选 plan link | durable transaction、CAS 或 incomplete recovery set | 原子推进工作步骤及 plan 关联；不执行 Host effect |
| `sop_run_store` | 已校验的 SOP run record 与 expected revision | run snapshot、CAS 更新和有界列表 | 只持久化 SOP execution record；不选择 SOP 或判断步骤完成 |
| `mcp_store` | MCP server record、删除时间与 identity query | revisioned registry record、列表和 sync outbox | `SqliteMcpRepository` 私有借用 owner Unit of Work；`NolaStore` 不再实现 `McpRepository`，MCP 用例不能取得其他 capability；不连接 server、不路由工具 |
| `workspace_checkpoint_store` | workspace root、git revision、untracked set 和 checkpoint identity | immutable checkpoint、按 root 列表、删除结果 | `SqliteWorkspaceOwnershipRepository` 同时封装 checkpoint 与 session lease 的创建、读取和删除；删除/释放不再混入 Run recovery maintenance；不执行 Git 或文件系统操作 |
| `session_run_lease_store` | session ID、process identity、owner 和 acquisition time | lease acquisition、snapshot/list、owner-checked release | 只协调跨进程 session run 排他权；不管理 run 状态机 |
| `product_setting_store` | setting key、validated JSON value、更新时间 | revisioned setting 或删除结果与 sync outbox | `SqliteProductSettingRepository` 私有借用 owner Unit of Work；`NolaStore` 不再实现 `ProductSettingRepository`；不解释配置业务语义 |
| `model_store` | provider/runtime scope、model profile mutation、runtime report | profile/runtime snapshot、revision 和 sync outbox | `SqliteModelRepository` 以私有 `&mut NolaStore` 借用同一 Unit of Work；`NolaStore` 不再实现 `ModelRepository`，模型用例不能取得其他 bounded-context 能力；不调用 provider |
| `conversation_plan_store` | conversation/SOP/run identity、todos、expected revision 和 change metadata | current plan、CAS result、revision history | conversation plan 的原子修订；不执行 SOP step |
| `checkpoint_store` | conversation checkpoint、expected revision 和同事务 memory records | stored checkpoint/memories、revision 或 conflict | checkpoint 与关联 memory 的原子持久化；不生成或压缩上下文 |
| `effect_outbox_store` | run/effect identity 与 acknowledge time | pending effect、单条 effect 或 acknowledgement 结果 | 只读取和确认已提交 outbox；不派发 Host effect |
| `skill_store` | immutable version、editor mutation、enable/delete command | revisioned Skill、editor snapshot、列表和 sync outbox | `SqliteSkillRepository` 私有借用 owner Unit of Work；`NolaStore` 不再实现 `SkillRepository`，Skill 用例不能取得其他 capability；不根据用户文本选择或激活 Skill |
| `permission_grant_store` | typed scope、capability/resource patterns、decision 和 revocation time | grant snapshot、effective grants 或 CAS result | 持久化授权事实；不从提示词推断权限 |
| `sync_mutation_store` | operation resolution、replica cursor、validated remote change page | applied/skipped count、monotonic cursor 和原子提交 | `SqliteReplicaSyncRepository` 私有借用 owner Unit of Work；`NolaStore` 不再实现 `ReplicaSyncRepository`；不拥有传输或重试调度 |
| `turn_admission_store` | validated local/internal/maintenance/remote turn command | conversation/message/run identities、ordinals 和 committed initial state | 原子创建 turn 所需记录；不调用模型或 Host |
| `conversation_projection_store` | idempotency identity、expected revision、typed projection/checkpoint | projection receipt、ledger count 和 revision | 原子提交 projection/checkpoint；不生成展示模型 |
| `conversation_summary_store` | 可选 conversation identity | ordered summary list 或单一 summary | 聚合只读 conversation metadata；不加载正文窗口 |
| `conversation_transition_store` | transition identity、run expectation、events/parts/effects mutation | committed transition receipt 和 next event sequence | local/remote transition 的单事务提交；不执行 effect |
| `context_checkpoint_store` | checkpoint、ordered context components 和 provenance | reconstructed checkpoint、component list 和 count | `SqliteContextRepository` 私有借用 owner Unit of Work；`NolaStore` 不再实现 `ContextRepository`；不选择内容或执行压缩 |
| `conversation_read_store` | conversation identity、ordinal cursor、message/byte limits | bounded conversation/window/tail/settings/checkpoint snapshot | 强制有界读取和稳定 cursor；不拼装 UI projection |
| `presentation_input_store` | conversation identity 与 selected message IDs | local/remote presentation、permission、context usage 所需的最小输入 | 只读取 Application 所需字段；不执行 ownership 决策 |
| `remote_session_store` | run identity、remote mirror/projection record | invocation config、mirror/context usage/projection snapshot | remote session durable mirror；不访问远端 transport |
| `active_run_store` | conversation 或 run identity | active run 与 immutable conversation/assistant ownership | 只读 run ownership；不改变 run 生命周期 |
| `schema_store` | 新建 SQLite connection 的 application ID、version、epoch | compatible initialized schema 或 `IncompatibleSchema` | 只负责首次 schema 建立和兼容性 gate；入口仅对父模块可见 |

下一步由 Bootstrap 构造窄 repository adapter，并让 Application 用例只接收对应 Store Port。
在 repository 对象和私有 Unit of Work 落地前，不能把上述文件拆分称为 god object 已解决。

### 3.6 `nola-runtime-supervisor`

| 项目 | 契约 |
|---|---|
| 输入 | 已提交 Effect、lease、取消/恢复命令、资源预算 |
| 输出 | 有身份和序列的 Host result/progress、heartbeat、terminal reason |
| 内部功能 | worker 生命周期、公平队列、backpressure、timeout、retry、process tree cleanup |
| 接口 | `claim(effect)`、`renew(lease)`、`report(result)`、`cancel(identity)`、`recover(identity)` |
| 禁止依赖 | 会话消息拼装、业务 SQL、Prompt、客户端状态 |

RuntimeTask 的 retry/finish 与 renew/progress 必须携带同一必填typed lease witness：
task、原attempt、worker generation、token、最近接受的revision/expiry。
worker持有Claim/Resume绑定的mutable handle，不按task ID从registry借后继身份。
Store在一个事务中验证当前执行权、注销leased attempt、转换task及记录事件；
取消标记优先于正常重试/完成，仍有效的owner可以提交cancelled。
过期recovery由持久expiry事实独立授权，不是无witness worker fallback。
采用边界、实际迁移状态和验收门槛见
[DURABLE_RUNTIME_DESIGN.md](../../软件架构设计/模块/05-执行与恢复/DURABLE_RUNTIME_DESIGN.md)。

### 3.7 `nola-host`

| 项目 | 契约 |
|---|---|
| 输入 | typed Host Effect、冻结 capability descriptor、permission decision |
| 输出 | typed Host Event/Result，保留 correlation ID 和 cursor |
| 内部功能 | provider、文件、Shell、浏览器、Skill、SOP、MCP、平台能力适配 |
| 接口 | 每种能力一个 adapter；catalog 只描述能力，不决定模型是否调用 |
| 禁止依赖 | Application repository、客户端 reducer、基于自然语言关键词的路由 |

所有 Host adapter（包括跨进程设备能力）必须返回同一个结果信封：typed output、outcome、
correlation ID、可选 artifact 列表和可选 capability activation。adapter 已经持久化或生成
资源时，必须返回稳定的 canonical URI、MIME type 和 operation；不能只把资源信息写进
display 文本或 JSON output，再要求模型复制路径来完成 Skill/SOP 协议。

Skill 完成汇报分为三个有序阶段：激活并读取指令、执行并取得 Host 证据、根据证据提交
内部 completion records。刚激活的 Skill 不得立即收到 completion checklist；必需 artifact
未由 Host 注册、工程验证仍待完成时，也不得用 checklist 抢占实际执行或验证。

Skill源选择由所属Runtime持有，区分managed默认、显式禁用和指定目录；目录/schema、
prompt、权限预检、实际加载及worker/recovery消费同一份选择，不使用进程可变根目录。
元数据发现不授予通用文件访问，激活资源的精确身份来自成功Host结果的结构化数据，
不得从展示文本或有损路径还原。执行仍复验当前Host可用性和精确Skill ID。
不可变选择不等于文件字节封存或永久授权；当前实现及验收边界见
[整体整改8.8](MODULAR_REDESIGN_2026-09-28.md#88-skill-的-runtime-所有权整改2026-10-02)。

CLI 的目标拆分为 `client`、`runtime-supervisor`、`host`、`web-adapter` 四个模块；可打包为同一二进制，但不能共享可变业务状态。

#### 3.7.1 发现、证据和完成的边界约束

- 发现：静态与动态 schema 先合并，再共用匹配、缺失恢复和 activation 路径；Host 冻结
  目录限制优先于名称候选。基础本地操作直接可见，授权仍由具体调用触发。
- 证据：adapter 生成声明范围内的完整结果；通用输出层负责持久化与预览裁剪。
  不允许 adapter 的截断预算消耗掉后续 staged/untracked 区段，再把空区段报告为无变更。
  输出保存失败是显式错误，但不暗示已经执行的副作用被回滚。
- 终态：`ModelCompleted` 是单次采样结束；`TurnCompleted` 是回合协议结束，不是任意
  用户目标的成功凭据。已登记的 validation、recovery、Skill completion 义务必须先
  满足，或进入明确的失败/拒绝/取消路径。输出超限续写无进展不能越过这些义务。
  没有注册为结构化目标的自然语言任务，不得伪装成运行时已独立验收；通用语义验收也不能
  用“提交/搜索/current”等关键词生成执行义务或授权。
- 权限：用户任务范围、具体调用许可、待澄清的产品选择是不同概念。目录可见不等于允许，
  文字讨论权限不等于批准；真正的 allow decision 只恢复被冻结的那次调用。

回归证据包括超出预览/旧工具上限的 diff、后置 staged 与 untracked、二进制及特殊路径、
完整输出保存失败、Unicode 分页、动态/静态目录缺失恢复的一致性、非英文自然跟进、
歧义词、不合法工具恢复，以及真实模型的只读/授权/跟进会话。真实供应商不可用时必须
记录未验证项，不能用 mock 通过替代真实会话通过。

## 4. Apple 客户端模块

### 4.1 文件布局

```text
NolaMac/
  App/                    # composition root、scene、lifecycle
  Application/            # typed SDK facade、commands、queries、subscriptions
  Models/                 # projection/view data，不含 IO
  Presentation/           # reducer、format policy、animation policy
  Features/Chat/          # Chat feature state、actions、coordinator
  Features/Conversation/  # list/title/navigation
  Features/Inspector/
  Hosts/                  # macOS capability adapters
  UI/Components/          # 小型无业务状态 View
```

iOS 使用同一职责划分；平台能力实现不同，Application 与 Projection 语义相同。

iOS Chat 的首批状态边界如下：

| iOS Chat 模块 | 输入 | 输出/动作 | 内部功能与边界 |
| --- | --- | --- | --- |
| `IOSRemoteTaskObservation` | conversation identity、远程 event operation | 单调 cursor 与订阅生命周期 | generation 区分 A→B→A 的不同订阅实例；不解析 projection、不写 Store |
| `IOSHostPermissionSession` | immutable Host request、conversation identity、current predicate、send port、用户 decision | permission session state、已提交 decision 或取消 | 唯一拥有 continuation、replacement token、发送重试与清理；不访问 Store、Provider 或 UI |
| `IOSContextUsageLedger` | conversation ID、Core/Host usage evidence | 会话级权威 token evidence | 只拥有内存索引；持久化恢复仍由 ChatManager 的 query 边界完成 |
| `IOSMemoryWorkerScheduler` | 有序 conversation IDs、跨会话 job budget、run closure | 聚合的 bounded run report | 去重、公平扫描并验证 report 不变量；不持有 UI 或 repository |
| `ChatTextPolicy` | 原始模型/标题文本 | 规范化文本 | 纯函数；不向全局 `String` 暴露 feature-specific API |
| `ChatManagerState` | 无 IO | public error 与 processing state | 稳定 UI 契约；不持有运行任务 |

### 4.2 接口

```swift
protocol ConversationCommanding {
    func submit(_ command: ConversationCommand) async throws -> CommandReceipt
}

protocol ConversationQuerying {
    func projection(id: ConversationID, window: MessageWindow) async throws
        -> ConversationProjection
}

protocol ConversationSubscribing {
    func revisions(id: ConversationID) -> AsyncStream<ProjectionRevision>
}
```

`View` 输入为不可变 display model，输出为用户 action。`FeatureModel` 在 `@MainActor` 上拥有 UI 状态；Service/Host 回调先转换为 typed projection update，再进入 Feature reducer。View 不打开 Store、不解析 Host 文本、不持有 provider stream。

## 5. Android 客户端模块

### 5.1 文件布局

```text
android/.../com/nola/
  application/            # command/query/subscription facade
  domain/                 # Android 侧稳定 projection value
  data/                   # SDK/Room/HTTP adapter
  host/                   # Android capability adapters
  feature/chat/           # state、action、reducer、ViewModel、screen
  feature/conversation/
  feature/inspector/
  ui/components/
```

### 5.2 接口

```kotlin
interface ConversationCommands {
    suspend fun submit(command: ConversationCommand): CommandReceipt
}

interface ConversationQueries {
    suspend fun projection(id: ConversationId, window: MessageWindow): ConversationProjection
}

interface ConversationUpdates {
    fun revisions(id: ConversationId): Flow<ProjectionRevision>
}
```

`ViewModel` 只做 feature 编排和 coroutine 生命周期管理。纯 reducer、permission payload、tool presentation、context usage 和 title policy 放到独立文件并单测。Compose Screen 只渲染 state、发送 action。

聊天 feature 当前建立的协作者契约如下：

| Android Chat 模块 | 输入 | 输出 | 内部功能与边界 |
| --- | --- | --- | --- |
| `ConversationLoadCoordinator` | load/invalidate 信号、request generation、processing state | 单调 generation、stale-safe commit 判定 | 独占历史定位 Job 的取消与生命周期；不读取 repository |
| `ChatTurnRuntime` | generation Job、device coordinator、Core part/task/snapshot event | 可取消运行句柄、归约后的 `ChatUiState` | 独占当前生成与设备执行生命周期；不持久化、不保存重复的 UI 文本 buffer |
| `ChatPermissionSession` | 已验证 Host/device request、用户授权意图 | 当前可处理 request、重复提交 guard、会话授权偏好 | 只保存瞬态交互状态；不做权限策略和 grant 持久化 |
| `RemoteTaskObservation` | provider、conversation ID、选择/重连 predicate | task projection event、单调 cursor | 独占一条订阅、退避重连和取消；不改变 canonical history |
| `AgentTraceCoordinator` | conversation/message identity、trace event | 稳定 turn identity、成功归约后的 trace projection | 独占 trace session 与 durable append 顺序；不格式化 UI 文本 |
| `AndroidAttachmentImportCoordinator` | SAF content URI 批次 | 保序的 `AndroidAttachmentImportResult`（成功附件与逐 URI 失败） | 独占持久读取授权、30 MB bounded read、名称/MIME 查询、文档/图片解码、digest/Base64 与导入遥测；不拥有 composer 状态、不提交消息 |

`ChatViewModel` 仍负责消息提交、provider/local run 编排和部分数据源编辑。后续拆分必须让
这些流程依赖 application use-case 接口，不能把现有字段再次搬入一个新的大 coordinator。
`ChatScreen` 通过参数只接收 `AndroidAttachmentImporter`；默认具体实现由
`AndroidChatFeatureComposition` 在 feature composition boundary 创建和记忆。Screen 不再构造
或识别 `ContentResolver`/codec coordinator，preview、测试和后续 App DI 可替换该接口。

## 6. 文件与可见性门禁

- 新生产文件建议不超过 500 行；超过 800 行必须说明单一职责为何仍成立。
- 已有超过 1000 行的文件只允许净减少，禁止继续加入新的领域职责。
- 一个文件最多拥有一个主要 stateful type；紧密关联的 value types 和私有 helper 可以共置。
- Rust 默认私有，跨 crate 才使用 `pub`，crate 内使用 `pub(crate)`，父子模块协作用 `pub(super)`。
- Swift/Kotlin 默认模块内可见；实现细节用 `private`，测试通过行为接口覆盖，不为测试扩大生产可见性。
- 禁止新增 `shared`、全局 mutable registry 或 service locator；composition root 是唯一装配位置。
- 文件名必须表达职责：`*Policy`、`*Reducer`、`*Repository`、`*Adapter`、`*Coordinator`，不能继续使用无边界的 `Utils`、`Helpers`、`ManagerParts`。

## 7. 第一阶段迁移

第一阶段只改变文件和封装，不改变 wire schema、数据库 schema 或用户行为：

1. 把 macOS workspace 中的纯策略和 reducer 移出 View 文件。
2. 把 Android chat 中的 permission、tool presentation 和 response reducer 移出 ViewModel 文件。
3. 把 `nola-store` 的连接、错误和事务所有权移入 `connection` 模块，根文件只重导出公共 Store facade。
4. 为三个拆分分别运行现有行为测试和编译；禁止通过更改断言弱化行为。

Android context preparation 作为下一条已建立的对象边界：`ChatViewModel` 只提供当前
conversation projection、route、模型预算和启用的资源；`ChatContextPreparer` 返回准备后的
Core context、回答依据和 usage。摘要调用、archive commit、数据源披露分别由
`ChatContextSummaryService`、`ConversationArchiveWriter`、`ChatDataSourceUsageRecorder`
封装，均位于 `agent/context`，不得把 repository 或 Android `Context` 传回 ViewModel 的
纯 reducer。后续若这些能力需要替换实现，应在 DI composition root 为它们建立窄接口。

CLI 的 thinking、工具运行与完成回执属于 transcript，不属于 composer/footer。
活动行与正文共用 history insertion anchor，动画帧仅原位刷新，不追加历史；thinking 结束
只保留一次静态记录，工具返回立即在同一追加位置留下回执，不等待最终回答或回合结束再搬运。
底部 frame 仅拥有输入、必要的 footer 和交互面板。缩放重放、权限面板切换与完成转换必须
保持过程记录顺序且不重复；PTY 验收须在模型最终回答尚未返回时断言命令回执已在主区域。
此路径已由 28 项终端单测及 5 个针对性 PTY 场景验证，覆盖活动/完成位置、正文追加、
中文高度切换、动态缩放/取消和权限返回。高度切换用例必须等新尺寸的光标帧，不能把旧
画面的 placeholder 当成缩放完成；草稿、页脚、历史记录的唯一性断言保持不变。

macOS transcript 已形成独立 presentation component：输入只允许消息 projection、历史窗口
状态、运行状态和 action closures；内部拥有 viewport intent 与原文定位 UI；不得读取
Store、解析 Host envelope 或持有 provider stream。

`MacTranscriptScrollController` 是 transcript 滚动意图的唯一所有者：`latest` 跟随内容底部、
`reading` 保持消息 ID 与消息内像素偏移、`locating` 执行显式原文定位。AppKit 布局通知只能
触发当前意图的几何校正，不能推断用户开始/结束滚动。真实输入才可进入阅读状态；回到底部
按钮、用户滚到底部和切换会话可恢复跟随。每次用户操作或会话切换推进 generation，使旧的
异步定位与排队校正失效。不得从消息内容、运行终态分别启动 sleep/yield 补滚循环。
布局增高（包括终态后的 Markdown 排版）与历史 prepend 均遵守同一契约；嵌套代码块的滚轮
不应改变外层 transcript 意图。原生适配器必须在视图真正进入 NSScrollView 后绑定并在移除时
解除观察；仅测试脱离 SwiftUI 的控制器不能证明视图绑定正确。

回归至少覆盖实际 NSScrollView 的延迟多次增高、手动回到底部、阅读锚点前插和旧定位失效。
这些测试不等同于真实模型整轮会话验收；工具生成、执行准入和可见展示的容量边界须另行验证。

单条 macOS 消息展示已形成 `MacMessageView` feature component；跨 message、workspace、
thinking 与 trace 共用的运行状态动画归 `MacChatStatusViews` 所有。共享 presentation view
保持无 Store/Host 依赖，调用方只能传入不可变展示值。

macOS 运行过程展示也已按输入契约拆开：`MacThinkingProcessView` 只接收有序 trace projection
和 streaming 状态；`MacTraceView` 只接收单条 trace，并通过环境 action 打开文件引用。
两者不得解析 Host envelope、查询 Store 或改变 run 状态。

`MacComposerView` 只接收 immutable `MacComposerDisplayState` 与稳定的
`MacComposerActionSink`。AppKit 文本编辑器通过 action-backed `Binding` 更新 draft；IME、附件
拖放/粘贴、语音输入、style、发送和停止均输出 typed action。Canvas 是唯一把这些 action
映射到 `MacWorkspaceModel` 的 composition point。`MacHostPermissionView` 只接收 permission
request 和 allow/deny/guidance actions；它不得直接调用 Host 或持久化授权。

macOS Inspector 已按文件拆分；`MacChatInspectorView` 负责页签导航和通知到 UI 状态的转换；Overview
进一步拆为 context metrics、response settings、working context 和 data-source access 四个
View。Knowledge、Browser、Activity 各自拥有独立文件。所有 Inspector 文件保持在 500 行
以内。其外部输入由 `MacInspectorDependencies` 明确列出：prompt templates、data registry、
platform data、web sources 和 Chrome messaging port。多数 Inspector 子 View 目前仍直接
绑定完整 `MacWorkspaceModel`，因此表格描述的是目标接口，尚不能视为已经完成的编译期
边界；后续要建立 immutable display state 与 typed actions。Chrome 生命周期通过
`MacInspectorChromeMessaging` 与 `MacInspectorChromeModel` 转成可观察 projection 和 action。

| Inspector 模块 | 输入 | 输出/动作 | 内部状态与失败边界 |
| --- | --- | --- | --- |
| `MacChatInspectorView` | workspace projection、初始页签、`MacInspectorDependencies` | 页签选择、路由到子模块 | 只拥有页签和 Chrome projection model；通知只触发 refresh/navigation |
| `MacInspectorContextSummaryView` | context/cache display projection | 无业务写入 | 只格式化 token、window 和 cache changes |
| `MacInspectorResponseView` | response projection、prompt templates | 选择 prompt/style、打开 Prompt Library | 写入仅通过 `MacWorkspaceModel` command |
| `MacInspectorWorkingContextView` | attachment/workspace/permission projection | preview、选择 workspace、切换 retrieval/memory/permission | macOS 文件展示失败不改变会话规范状态 |
| `MacInspectorDataSourcesView` | registry/platform/web projections、conversation ID | 设置或清除 conversation override | Registry 拒绝写入时保留其 canonical policy |
| `MacInspectorKnowledgeView` | knowledge/wiki/recall projection | 编辑、保存、搜索 | 自己拥有 knowledge 子页签；保存/搜索失败显示 model projection |
| `MacInspectorBrowserView` | browser projection、`MacInspectorChromeModel` | 切页、展开、安装/打开扩展 | Chrome 调用由 adapter 吸收并刷新 typed snapshot |
| `MacInspectorActivityView` | execution location、Chrome snapshot | 打开 capability/extension/diagnostics scenes | 无业务状态 |

Workspace presentation 已完成文件拆分。Composer 的窄边界已经落地；Sidebar 与 Canvas 仍可
直接访问完整 `MacWorkspaceModel`，后续继续把它们缩到 projection/action composition：

| Workspace 模块 | 输入 | 输出/动作 | 内部功能 |
| --- | --- | --- | --- |
| `MacWorkspaceShellViews` | color scheme、window title | AppKit window configuration | 背景材质和窗口 chrome，不读取会话状态 |
| `MacConversationSidebarView` | conversation list projection、selected section | 选择/新建/重命名/置顶/删除/打开窗口 | 分组和搜索仅使用已有 projection |
| `MacSidebarComponents` | conversation display value、selection/running flags | 纯 ButtonStyle/View 输出 | hover、selection 和状态动画 |
| `MacConversationCanvas` | selected conversation projection、run/input/permission state | transcript/composer/user-input actions | 只负责编排会话主画布和 overlay 高度 |
| `MacComposerView` | `MacComposerDisplayState`、`MacComposerActionSink`、focus binding | draft/submit/stop/import/remove/style typed actions | 只拥有 importer/drop/hover/voice/confirmation/focus 瞬态 UI；不读取 WorkspaceModel、Store 或 Host |
| `MacConversationCanvasComponents` | readiness、search、empty/input display values | readiness navigation、搜索、建议与回答动作 | 小型状态 View，不拥有 Store/Host |
| `MacRemoteTaskObservationCoordinator` | conversation/task identity、远程 task event stream | task projection 或结构化失败 | 每个会话一个订阅；拥有 cursor、重连、terminal 判定与 cancel 生命周期，不写 UI/Store |
| `MacConversationWindowCache` | canonical repository、conversation ID | owned bounded window model、existing window、eviction | 唯一拥有 conversation window cache；不协调异步加载或写 Feature state |
| `MacConversationArchiveScheduler` | conversation ID、delay、save closure | monotonic revision callback、cancel/invalidate | 唯一拥有 debounce Task 与 revision；不读取会话或 Store |
| `MacConversationResidencyPolicy` | accessed conversation、全量 byte counts、protected IDs | ordered eviction IDs | 唯一拥有 residency/LRU 顺序和资源上限；不直接删除消息 |
| `MacContextArchiveCountState` | conversation ID、request identity、可选 count | stale-safe count/failure state | 只拒绝过期异步结果；不发起查询 |
| `MacRemoteRunRecovery` | generation/conversation/run/message identities | immutable recovery correlation value | 无 IO、无可变全局状态 |
| `MacLocalRunRecovery` | generation/conversation/run/message identities | 本地传输失联后保留的精确关联值 | 只读状态检查不重新提交用户消息、不调用远端恢复；本值不授予接管执行权 |
| `RunRecoveryInspectionRepository` | 精确Run ID | 同一读快照的持久恢复事实 | 不推断OS活性、不读工具正文或secret，不决定重试；Application另验conversation/assistant身份并分类 |
| `MacRunRecoveryInspection` / `MacRunRecoveryPresentation` | Application已验证的恢复观测 | typed事实及本地化说明 | 缺失记录不等于未提交；未知状态拒绝解析，展示不授予接管或重试权 |
| `MacRunCompletionStatus` | 已确认的completed/failed/cancelled | 活动记录及通知的完整三态输入 | 不由进程退出推断；取消不能降格为failed，通知不得将取消伪装为成功 |
| `MacRunCoordinator` | generation、submission identity、接受/取消/已提交终态证据 | preparing/dispatching/running/recovery/finish ownership | 执行生命周期及一次性终结所有者；不解析正文，不将transport EOF当Run终态 |
| `MacRunSubmissionTicket` | 固定conversation/submission/generation | 原子claim、准备撤回、明确未提交拒绝、accepted | 真正process/HTTP提交边界的一次性门闩；提交后的网络不确定性不能退回未提交 |
| `MacRunSubmissionSnapshot` | send时的消息、上下文选择、目录授权选择及Host target | 不可变异步准备输入 | 冻结选择而非冻结权限事实；解析时重验撤销，不纳入之后新增授权 |
| `MacProjectionObservationCoordinator` | conversation及lease、观察任务 | cancel/drain、当前lease验证 | 新旧观察所有权隔离，旧任务不能注销新任务；不拥有Run终态或消息正文 |
| `MacAssistantPresentationSession` | 已提交正文快照、reveal lease | semantic frame、visible text、完成frame | 唯一拥有accepted/visible/parser/pacing/未展示队列；暂停不丢已接收内容，替换废弃旧lease |
| `MacLocalAgentProcessSession` | 单个Process、所属pipes、submission ticket | launch、写入、退出事实、幂等资源回收 | 只管理该进程资源；启动后传输失败不能伪造未提交或canonical终态；不拥有消息呈现 |
| `MacRemotePermissionIdentity` | generation/conversation/trace/run/request/call identities | immutable permission stream correlation values | 无 IO、无可变全局状态 |
| `MacAutomationApplicationClient` | typed automation definition/run commands and queries | canonical automation/run DTO 或 typed failure | macOS Automation/Activity feature 的窄 Application 用例端口；不暴露 generic command envelope、Store 或其他 capability family |
| `MacSkillCatalogApplicationClient` | built-in Skill inputs、editor revision、visibility revision | canonical Skill/editor projection 或 typed failure | Skill registry 所需的同步、启停、恢复、移除、列表与 setting 查询；不授予 Automation/Conversation/Run 能力 |
| `MacLocalModelApplicationClient` | local model profile、expected parameter revision、profile identity | canonical model profile、CAS revision 或 typed failure | Local model configuration 所需的 upsert、参数 CAS、单项/列表查询；不授予 runtime process、provider、Skill 或 Conversation 能力 |
| `MacDataSourceApplicationClient` | draft/config、expected revision、validation token、override 与 usage | canonical source/override/usage、CAS result 或 typed failure | Data Source registry 的生命周期、验证、启停、覆盖与有界 usage 用例；不授予 Conversation body、Run、Model 或 Automation 能力 |
| `MacContentUploadApplicationClient` | upload/owner identity、byte count、offset、bounded UTF-8 chunk | upload progress、read-back chunk 或 typed failure | 保留逐 chunk ACK 与 resume prefix 验证；不提供一次性大正文写入 |
| `MacArchiveRestoreApplicationClient` | directory revision、restore identity、ordered manifest entry/digest/count | staged progress 或 atomic restore result | 组合 Content Upload，并独占 begin/stage/commit restore 用例；不暴露 generic command envelope |

`MacApplicationPort` 已实现按用例族拆出的 `MacAutomationApplicationClient` 和
`MacSkillCatalogApplicationClient`。Automation/Activity 的定义与 run 生命周期只依赖前者，
`MacBuiltInSkillRegistry` 只依赖后者；`MacAutomationStore` 已不再持有 concrete port。
`MacLocalModelConfigurationStore` 另只依赖 `MacLocalModelApplicationClient`。三个协议均为
`nonisolated Sendable`，避免 Swift 默认 MainActor 把同步 Application 边界污染成 UI actor API。
Data Source Registry、Archive Restore Writer 和 Transcript Body Uploader 也分别只依赖上述窄
client。Upload owner/progress/chunk DTO 已与接口共置。所有 seam 仍由 3,626 行 facade 实现，
其他 capability family 还需继续拆分；`MacDataSourceContract` 的纯值与 presentation extension
目前仍同处 Registry 文件，后续物理迁移时不得把本地化 title 放入 Application contract。

下一阶段把已拆 Store 文件封装为窄 repository 对象，并继续将 Mac/iOS/Android
FeatureModel 分成 command、projection 和 presentation coordinator。

Application Runtime 的 generic type 已收窄为 `RuntimeExecutionRepositories`，并通过
`ApplicationCommandExecutor` 调用 command dispatcher；完整仓储并集只存在于 dispatcher
adapter。下列独立用例继续使用更小的 Rust trait bound：

| Application 用例 | 仓储输入 | 输出 |
| --- | --- | --- |
| `command_receipt` | `CommandReceiptRepository` | typed command receipt 或读取错误 |
| content upload ownership | `ArchiveRestoreRepository + ConversationQueryRepository` | upload progress/chunk 或 ownership error |
| projection feed | `ConversationProjectionRepository + ConversationQueryRepository + ProjectionJournalRepository` | snapshot、cursor、event suffix |
| projection journal | 同上 | 同一事务中的 projection mutation event |

新查询和 use case 禁止使用 `ApplicationCommandRepositories` 作为方便的总约束；只有
把完整 `ApplicationCommand` 翻译到 command family 的 wire adapter 可以使用它。

## 8. Harness Step Runtime

Agent harness 使用三个不可互换的作用域：`Session` 保存规范会话与 durable 状态，`TurnContext`
冻结一轮用户意图可见的环境，`AgentStepContextSnapshot` 冻结一次物理模型采样使用的工具投影、
invocation policy 和 prompt/cache identity。Host 只能验证并执行 StepContext，不得从 prompt、
model 名称或用户文本重建这些值。

模型请求前统一由 Core `context_normalizer` 保证 tool call/result 配对；它只修改 request working
set，不修改 lossless transcript。中断 call 获得确定性的 synthetic error，orphan result 被移除，
并行 result 按原始 call 顺序进入下一次模型请求，因此 Host 完成时序不会改变 request digest。
Provider adapter 只负责 wire encoding，不拥有规范历史修复策略。

Provider transport 以 `ProviderFailure` 传递 status、retry-after、provider/model identity、
same-target retryability、fallback eligibility 和 bounded diagnostic。HTTP header 不得再编码进错误
字符串；Application/UI 使用稳定 code/message，原始诊断只进入受限日志。

工具 admission 统一通过 `ToolExecutionRuntime`。并行性和 interrupt behavior 只读取冻结的 Host
descriptor；未知或不支持取消的 capability 使用 block 语义。模型流中的 partial arguments 是
transport state，只有完整并通过 schema validation 的 call frame 才可执行。所有 Completed、Failed
和 Cancelled 路径在 reducer terminal boundary 统一完成或丢弃仍开启的 Part，禁止 UI 永久停留在
loading 状态，也禁止已提交正文被撤回。

资源关联与循环进展只读取 Host descriptor 中的 `loop_progress` 和 `resource_projection`；Core
不得维护 path、selector、dry-run 或具体 capability ID 的猜测表。Completion policy 统一通过
`evaluate_completion_contract` 同时决定 response contract 与 presentation visibility，调用方不得
分别复制条件树。动态 tool protocol 长文不得逐 Step 重发；稳定规则进入 cacheable prompt prefix，
Step 只携带 typed snapshot、状态差量和必要证据。

本地诊断记录 `host_model_preparation` 与 `provider_first_event` 等分段耗时，至少包含物理采样轮次、
消息数和工具数。Provider retry/fallback 的 UI projection 必须携带稳定 failure code、失败目标、
下一目标、status 与 retry-after；用户可以看到回退过程，raw provider body 只进入有界诊断日志。

### 模型工具批次容量与结果证据边界

Core 的 `model_tool_batch` 定义跨 Host 的单响应容量：当前最多 32 个原生工具调用。
它不是整轮调用配额，也不是 scheduler 并发数；后续批次应根据前一批真实结果重新规划。
工具路由、权限和只读/变更判断仍由具体 capability schema 决定，不能由用户语言推断。

- Provider 适配器在分配第 33 个工具槽之前拒绝响应，包括缺少 call identity 的碎片。
- 原生 Host 将工具参数草稿留在 transport staging；只有通过容量检查的完整响应才提交 Core。
  非流式完成同样检查。超限整批不执行、不截取前缀、不制造工具结果；以稳定错误码
  `model_tool_batch_limit_exceeded` 进入最多两次的协议纠正，并明确告知模型该批没有执行。
  容量错误不触发透明重试或 provider fallback；纠正耗尽后提交失败终态。
- Core 再次校验完整批次，防止其他 Host 绕过；仍发送参数碎片的 Host 也受工具 Part 数量边界
  约束。超限关闭模型 effect 并结束所有草稿，不能留下永久 pending 的卡片。
- 该边界限制单响应计划规模，不解决跨批次的重复调查；重复检测、执行调度、权限等待仍是
  独立责任。界面历史的分组/虚拟化也不能用此上限代替。

Mac 过程展示按 canonical 顺序只合并相邻同类工具，禁止把旧组移动到最新调用位置。
每页最多 24 项，旧页以末项 ID 固定，新的流式事件不改变其内容；前后翻页和返回最新均为
显式交互。所有历史仍可逐页访问；失去锚点不能悄悄返回实时尾部。该窗口在分组之前应用，
因此展开工具组也不会一次创建数百个详情视图。

跨批次相同调查用实际成功返回的结果建立证据指纹：同一规范化调用连续获得三次相同结果
（允许中间穿插其他调用）后进入证据收束阶段，不再继续开放调查工具。结果发生变化则重置
该调用的计数；成功变更开启新的观察周期。此机制不猜测用户意图，不按中文/英文词语路由，
也不把不同资源的相同文本误认为同一个调用。大规模但持续获得新证据的合法调查仍可进行。

工具结果的完整保存、模型取证与界面预览是三个不同边界。大结果保留 `full_output_ref`、
有界预览与准确 continuation byte offset；存在未读取尾部并不意味着必须逐页读完。
普通调查优先检索与按需范围读取；只有任务需要穷尽覆盖时才遍历全部页，并如实报告覆盖缺口。
这遵循参考资料 OpenCode 的保存原文/有界预览设计，而不是把完整 Git diff 反复推回模型或 UI。

进程级故障回归使用 121 个不同调用的 SSE 响应：纠正成功时只完成新计划，数据库没有被拒绝
批次的工具 Parts；连续超限时最多三次物理请求并持久化失败。Core 合同测试另行验证绕过 Host
的完整批次和碎片超限，不得因执行并发有限就声称生成和展示也有界。

### 8.1 结果存储与调用准入的所有权

CLI 终端呈现只有一个帧所有者。帧状态必须记录实际绘制的终端宽高，不能用新高度推算
旧帧的位置。尺寸变化后，终端自身的 reflow 结果不是权威状态：清理可见视口，并从
有界的已提交正文尾部重新排版，再绘制唯一的草稿和状态栏；草稿不得进入回放缓存。
回放缓存最多 256 KiB，仅用于展示，不代替会话存储。正文写入可临时设置滚动区域，
但绘制输入区前必须恢复全屏绝对坐标与滚动范围，绘制期间禁用自动折行，结束后恢复。
模型正文起始的空换行只在展示投影中抑制，不改写 canonical 正文或段落内部换行。
回归须检查缩放后当前屏幕中中文草稿、状态栏和已提交回复各只有一份，不能仅断言
历史输出包含某个字符串。

正文与底部交互区是两个不同的布局区域。参考 Codex 的 `streaming/controller.rs`、
`insert_history.rs` 和 `bottom_pane/mod.rs`：稳定正文追加到历史，尚可变的正文尾部
锚定在历史插入位置，不属于 composer/footer 的底部帧；提交尾部只改变其可变性，
不得改变它的显示位置。权限卡属于底部交互区，暂时替代输入框，由同一绘制器处理
缩放、清理和恢复，不能通过独立 stdout 写入与保存/恢复光标接管整屏。超高权限卡
必须支持分页查看，不得静默丢弃命令或授权范围；外部输入暂停期间的历史提交必须
排队并在交接时写入，不得因为暂停绘制而丢弃。

冷恢复必须先解决物理投递所有权，再重绑定 Core 的逻辑请求。Host 从 Application 的
`LoadRunDeliveryOwnership` 读取当前 attempt ledger，而不是从 UI、旧 session lock 或
checkpoint 猜测工作进程。只有所有已知本机 Runtime owner 均确认消失，才提交带完整
attempt/generation/owner/token 的 `RecoverObservedOrphanedRun`；Application 在同一写事务
中核对身份未变，拒绝新 owner 或未观察到的并发 attempt。未知 owner 或仍存活的 PID
保守交给租约期限，不抢占。已开始的非幂等操作仍进入 outcome-unknown，不能自动重放。
恢复执行结束必须与普通回合执行相同的 composer handoff：交接本地排队消息、持久 FIFO
中尚未消费的输入、未提交草稿及 usage。不能因入口是 `/resume` 就丢弃恢复期间的新输入。

验证债务来自明确采用的工作流契约或具有验证要求的能力契约，不由一次普通写入自动产生。
有此义务时，Core 追加 Host 声明的 validator，但保留原有工具目录，允许后续写入和修复。资源范围匹配的
验证证据仍是最终完成的门槛，不能把“需要验证”变成隐式的工具授权变更。

最新 29 调用的 Mac 审计取消案例证明：批次容量保护不能代替结果协议和调查收敛。
五函数审计样例只作为 smoke test，不作为原始规模用例的完成依据。

模型上下文中的工具事务保留决策轮次的因果顺序：一批 call 的结果齐全后立即封闭该批，
再投影下一批 call。并行结果只允许在所属批次内按 call 顺序整理。不得因两轮之间没有
自然语言消息，就把后续决策的 call 移到先前结果之前；这会将串行证据链伪造成巨型并行批次。
该归一化只作用于模型请求，不修改 canonical transcript，并须满足重复归一化幂等。

持久化确认与 UI 投影形状是独立契约。流式文本、参数和工具进度的命令回执必须原子提交
checkpoint、增量事件和去重结果，但不得在每条回执里重复保存整段对话 snapshot。
完整 snapshot 留给状态边界；增量事件负责即时展示。冷启动从 canonical projection 恢复，
重放同一请求仍返回原回执，不重复追加文本。不能通过取消持久化来降低写入量。
模型网络读取与持久化消费之间使用有界队列；只合并已到达、相邻且同通道的文本，
不人为等待凑批，不跨越 reasoning/text、工具参数或重试边界。重试与模型 attempt 等
控制事件仍等待 durable acknowledgement 后推进。消费者拒绝时取消生产者，不能把
提交失败当成 provider 故障而重试。Host 将一个已合并文本批次作为一个持久化增量，
不能再拆回每个短句各写一次 checkpoint。队列文本最多 1 MiB，单次消费最多 16 KiB。
私有候选与审查正文仅在 `ModelCompleted` 中提交一次；Host 不得在生成结束后将其拆成
展示增量逐条提交、再让 Core 丢弃。通过审查的最终答复仍使用正常流式呈现。
子任务输出预算记录实际观察到的正文峰值字节数，与 UTF-8 截断后保留的预览长度分离。
Application 用前者判定 `output_bytes` 超限；不能只保存低于上限的截断长度，再由错误
文案声称超限。重启后观察用量不得倒退，截断后的用户正文仍保持有界。
任务租约续期只由 Application 的任务账本裁决，续期间隔随租约长度调整。父 Core 的
任务心跳是展示快照，最多每五秒发布一次，不承担第二套租约授权。Host 每次轮询最多
领取一条 durable boundary command，FIFO 余项留在账本中供后续调度；不能把处理所有
心跳作为确认模型响应的前置条件。终态、取消与权限仍通过持久化边界确认。

验收程序与客户端都必须按权威消息协议处理文本替换：只追加的 Trace 是审计日志，
不能据此重建最终正文。流式验收使用 Core Part 的单调游标与完整快照归约，追加时保留
已提交 Markdown 节点，撤回时重置正文，再独立与数据库终态核对，不能以终态覆盖掩盖
流式差异。取消在 Run 准入前可以没有终态事件，但仅在持久会话确实不存在时接受该结果；
已准入 Run 必须产生 canonical cancelled，不能把通用 CancellationError 视为成功。
宽 Markdown 表格按列内容设置有界可读宽度并水平滚动，不强制挤进视口。

`ToolContext` 显式注入 `ToolOutputStore`。内置工具、MCP 和代码分析的完整结果保存到
`<database>.outputs/<session digest>/`，范围读取、列表、搜索均限制在当前会话目录。
存储模块只处理原子保存、路径边界和 UTF-8 分页，不依赖权限线程局部变量、模型路由或 UI。
同一会话重启后引用可继续使用，不同数据库或会话不能读取彼此结果。旧缓存不自动删除。

`tool_output_read` 返回结构化 `tool_output_slice`，包含内容、总字节数、实际返回字节数、
精确下一游标和 `has_more`。分页同时满足原文大小与 JSON 编码后响应预算，避免结果页再次
被截断保存。普通文件读取误用当前会话结果引用时，返回明确的专用读取工具恢复契约，
不扩大工作区授权。模型必须消费实际游标，不按请求大小自行计算。

Skill/SOP 与后续操作混在同一批次时，Host 整批拒绝并反馈未执行的调用 ID；有限次数内
要求先单独激活、读取指令后重规划，不静默挑选调用。Core 对重复调用和超预算调用保留
原始身份，生成 `tool_execution_skipped` 错误终态与模型结果，不创建 Host 执行请求。
排队状态中的准入错误必须能随 checkpoint 保存与恢复。

SOP 执行引用与授权分离：Core 从业务参数中提取并持久保存完整 run/step 引用，
格式错误必须生成未执行的错误结果，不能去掉引用后执行原操作。Turn 开始时的 SOP
快照仅是模型上下文，不是运行期授权表；同一 Turn 内的 start/resume 会改变合法步骤。
Host 在权限请求前及实际执行前，依据权威持久 SOP 状态校验 owner session、workspace、
run 与 step。不能让 Core 的陈旧快照与 Host 的实时状态成为两套互相矛盾的授权源。

模型请求的并行偏好必须跨越 Core → Host → Provider 边界，不能只保存在 invocation plan。
OpenAI-compatible 请求携带 `parallel_tool_calls`，Anthropic 请求使用
`tool_choice.disable_parallel_tool_use`。超限恢复同时关闭并行偏好、要求下一次只规划一个
必要调用；仍保留完整响应的容量校验，不能假设 Provider 一定遵守偏好，也不能执行超限前缀。

纠正预算按模型决策轮次计数，而不是按同一批次中的失败调用数计数：模型看到反馈前，
七个并行参数错误不是七次拒不纠正。下一决策轮的兼容纠正调用成功（包括只读调用）即为
可观察进展，清除该失败周期；同批次中预先安排的无关成功不能假装已经消费了错误反馈。
诊断仍保存每个调用的失败，不能为了降低计数丢弃结果。

`host.cancel` 的权限目标是具体 effect ID。冷恢复后找不到旧 worker 时可幂等确认其静止，
不得升级为取消当前 Turn。新 Turn 的取消上下文只能在旧 Turn 的控制效果结清后绑定。
“明确恢复旧任务”与“新输入取代失联任务”是不同入口；后者保留旧 Run 的取消终态，但必须
能正常执行新 Run。
`host.cancel_task` 同样只提交指定 durable task 的取消请求，不能触碰前台 LLM/工具的
Turn 取消令牌。回执的 `cancel_requested` 必须反映持久化结果；存储失败返回可恢复错误，
不能先取消别的执行再伪造成功确认。

工具终态区分 `succeeded`、`deferred`、`failed` 与 `cancelled`。`deferred` 仅证明前台
调用已交付持久后台工作，不证明该工作的修改或验证完成；Core 不据此增加 mutation
计数或提前建立完成后的验证义务。后台 Shell 返回带确切 task ID 的结构化回执，
包括自动／手动转后台；TaskPromoted 使用相同语义。后台任务自己的生命周期与结果结算
继续独立推进。权限和副作用准入仍按原始请求评估，不能因回执为 deferred 放宽权限。

最终答复审查不是测试专用能力。生产目录对具有调查语义的能力声明
`investigative_evidence_audit`，Core 按具体调用的 effect 决定是否开启私有候选、证据审查、
最终呈现三个阶段；例如同一个 bash 能力的 inspect 与 validate 不应触发相同的收束路径。
审查按具体主张而非文件名排除无证据的结论，并核对范围、语言和输出格式。进程成功与工具
完成只证明协议收束，不能证明审计结论正确。真实工作树验收必须同时检查最终语义证据。

工具决策轮次预算与最终报告协议分离。执行预算关闭后，要求证据审查且没有待完成执行义务
的 Turn 仍可在最多六个工具禁用请求内完成候选、审查和最终呈现（包含协议纠正）；计数随
checkpoint 恢复。不得仅因进入最后报告就无条件判失败，也不得重新开放工具、绕过验证/
纠正/Skill 完成义务。其他未完成执行仍保留明确的预算失败状态。
预算收尾不能覆盖当前阶段的候选、JSON 审查协议或审查结果；请求模式和实际消息必须一致。
采样计数只在模型 effect 真正发出时递增，规划中转入上下文压缩不能消耗最后一次报告机会。

验收顺序为存储/状态机单测、真实 CLI 进程协议回归、隔离的真实工作树变更审计、Mac 同类
用例及滚动/Markdown 验证。不得将前三者中的局部通过表述为全部完成。

### 工具禁用阶段的响应协议边界

#### 压缩请求与热缓存前缀（2026-10-09）

缓存命中率记录与显示：`PromptCacheUsage` 保存最近一次物理请求的输入总量、读取量、
写入量、万分比 `hit_rate_bps` 与 `is_compaction`。命中率为缓存读取 token / 该请求输入
总 token；缓存写入不算命中。Anthropic 的普通输入、缓存读取和缓存写入组成分母；缺少
必要计数、零分母或不合理计数时比例为未知，不伪造 0%。不使用当前上下文占用作分母，
也不平均多次请求的百分比。

该事实随 TokenUsage、Core checkpoint 和 `context_updated` 事件持久化，经已提交 Run
用量读取接口投影；远端记录的摘要校验包含缓存事实。Mac 概览、CLI 状态栏、Web 概览和
iOS 上下文面板显示最近请求比例；Mac/Web 显示读取量 / 输入量及压缩标识。历史事件保留
历次记录，概览只显示最近一次，不冒充会话累计命中率或实际费用节省。

缓存统计验收：Rust CLI 缓存相关 13 项、实时事件转发 1 项、Domain 比例计算 1 项、
Core checkpoint 1 项、Application 本地/远端恢复 9 项通过；Web 265 项测试、类型检查
和构建通过；iOS 投影契约 18 项通过。真实 provider 的实际命中率和费用尚未做付费验收。
Mac 缓存快照新增测试通过；整组 contracts 已通过 620 项后停滞在无关的
`testDeadOwnerLockIsRecovered`，已中止，不能记为全量通过。首次测试启动时应用包内
`nola-service` 被系统终止（status 9）；上述重跑通过环境变量指定工作区服务，未修改
应用签名或打包配置，包内服务启动问题仍待单独排查。

Core 继续拥有归档范围、保留尾部、摘要预算与 checkpoint 提交；Host 不重新决定归档内容。
Provider transport 在成功采样后保留进程内的实际请求体见证（包含实际 fallback route），
Host 按会话与配置身份持有最后一份见证；重启后不从逻辑历史伪造缓存命中。请求体和密钥
不进入 checkpoint 或指标，见证的 Debug 输出必须脱敏。

- 热缓存分叉复制完整实际请求，只追加一条 user 摘要指令并收窄输出上限；tools、system /
  instructions、完整 messages / input、tool_choice、thinking 和缓存标识不重建。
  摘要指令限定 Core 选中的连续归档区间，后续尾部不纳入替换范围；引用锚点是非可信数据。
- 会话或配置改变、没有缓存读写证据、请求超过四分钟保守窗口、归档无法唯一对齐、强制
  工具调用、窗口/思考预算不足或保守 token 成本估计不合算时，走不请求缓存写入的独立摘要。
  四分钟是本地资格判断，不是 provider TTL 承诺；成本估计不是价格或节省金额测量。
- 分叉仅尝试一次；空文本、工具调用、截断或 provider 错误触发独立摘要回退。取消与
  持久化 admission 失败不重试。压缩返回的工具调用永不进入 Agent 执行循环。
  独立摘要也必须通过完整文本检查才能提交。成功回退的总 usage 包含被拒绝分叉的已知费用。
- 每次尝试记录 `compaction_request_metrics`：策略、回退原因、耗时、usage、缓存读写量、
  归档条目数和请求摘要哈希，不记录正文。HTTP 回归比较三种协议的实际前后请求体。
- Responses 透传已配置的 prompt cache key / retention，但“Responses 兼容”不代表支持
  `/responses/compact`。服务端 opaque compaction 尚未接入，必须有独立能力契约后才能启用。
- 没有主请求见证的独立旧入口保持冷摘要路径；压缩后的摘要仍由现有 user checkpoint 数据
  包装注入，不能提升为 system 指令。此改动不宣称所有 provider 都必然缓存命中。

本轮验收：19 项压缩相关测试通过（含 Chat Completions / Responses / Anthropic 的实际
HTTP body 对比、Host 无效分叉回退与累计 usage），`nola-agent-host` 编译检查通过且无编译
警告。CLI 全量结果为 1538 通过、3 失败、3 忽略；以下会话日志测试单独串行重跑仍失败，
不应把全量结果记为通过：`lost_notification_queue_is_seeded_from_committed_heads_once_per_run`、
`same_run_subscriber_return_seeds_commits_made_without_a_presenter`、
`encrypted_http_reads_exact_committed_journal_and_rejects_forged_identity`。
尚未用真实收费 provider 验证压缩请求的缓存命中率和费用节省。

#### 执行前的对象检查与完成后的审查分工

模型审查不是权限授予，也不是通用任务正确性的证明。`execute` 只能建议继续原始任务，
不能把“提交下”扩大成暂存所有未跟踪文件。参考本地 Claude Git 工作流的逐文件暂存、
暂存区检查，以及 Codex 的独立 ReviewTarget / findings 设计，Nola 将具体 Git 对象检查
放在有明确语义的 `git_commit` 操作内，而不是把该工作流强加给所有 Shell 操作：

- `paths` 只选择本次提交的完整工作树文件版本；未选中项是调用范围之外，不等于禁止资源。
- 私有 index 计算期望树；原生 `git commit --only` 提交选中项，保留其他暂存项。新增文件仅在实际执行阶段登记 intent-to-add。
- 检查实际父提交、树、路径集合、选中工作树版本及未选中 index 项。已有部分暂存的选中文件也会提交其完整工作树版本，schema 必须明示。
- 分步、部分提交由模型组合普通调用，无需新增批次模式或给其余源码编造排除理由。二进制格式不自动等于禁止资源。
- 前置条件失败与执行后不确定失败分开报告；执行后失败可能已有提交或 index 变化，不自动回滚或盲目重放。

其他 Git 语义可以使用经过具体权限检查的 Shell。工具选择不能给予额外权限，敏感路径保护与
执行隔离仍独立生效。路径策略不等于系统调用沙箱，不能声称能拦截所有间接脚本或未知秘密。
Git hook、过滤器和并发写入也不是此回执所能隔离的；未选中 index 保留不证明任意 hook 没有其他副作用。

`accept` 的检查项必须非空、逐项 eligible 且含非空证据解释；空 JSON 报告及仍有 rejected/unknown
项的报告不能发布候选。该条件能排除空审查，并不等价于证明语义覆盖；模型仍必须区分文件清单、
已保存未读取的 diff、实际读取的源码和验证结果。没有缺陷不代表没有需要核实的覆盖要求。

`request_user_input` 属于会话控制面，其 schema 不携带 `_nolaExecution`。模型附带的 SOP 元数据
不传给 Host，问题本身仍经过正常 schema/权限检查。普通执行工具的 SOP lineage 检查保持严格；
此规则不能用于丢弃写入工具的错误归属后偷偷执行。

交互工具在非交互环境返回 unresolved question 时，Core 直接交付问题并记录 `needs_input`，
不能因为工具调用本身成功就把任务标记为普通完成。Shell 策略拒绝敏感资源应返回
`sensitive_shell_resource`，而不是 `workspace_access_required`；扩大目录授权不能解决内容策略。
范围内 `git check-ignore` 是路径元数据查询，不读取被查询文件内容；允许该查询并继续检查
目录边界，但不放开 `cat .env` 或 `git diff -- .env` 的内容访问。

若模型在 mutation 的验证义务未清除时尝试结束，下一次采样进入 validation completion
阶段：`RequiredTool`，只投影纯 Validate 能力（混合能力的 effect selector 收窄为验证取值）
及用户交互能力。Core 调用入口同样拒绝 Inspect/Mutate 冒充该轮验证，不仅依赖模型遵守
收窄后的 schema。首次写入后仍可继续正常编辑；仅在提前结束触发的恢复轮收窄。真实验证
失败则回到 Correcting，保留修复能力。阶段及未验证资源随 checkpoint 保存，预算耗尽与
用户拒绝不能被此恢复流程绕过；提交前的检查不能证明提交后的结果。

`RequiredTool` 不是供应商必然遵守的保证。供应商仍返回纯文本时，不能将其当成验证证据，
也不能自动代替模型执行猜测出的命令。有限恢复耗尽须保留失败与已发生的副作用；真实验收
同时检查提交对象集合和完成状态，安全拒绝、部分提交或单测通过均不算整轮任务成功。

审查入口与成功发布必须分开：执行预算未耗尽时，候选即使仍有验证、恢复或 Skill 义务，
也进入独立无工具审查，避免验证先决条件阻断对漏做任务的纠正。审查 `execute` 恢复正常
执行并撤销旧的 validate-only 重试状态，但保留所有未完成义务；`accept` 不能清除义务，
有义务时转回受限完成恢复，沿用有界恢复次数。真实提问与明确阻塞不等价于成功，可以交付。
预算、用户拒绝与最终成功的验证要求不因提早审查而放宽。

验证范围是有方向的覆盖关系，不是双向路径重叠；验证一个子文件不能清除整个目录的义务。
后续无需验证的 mutation 也不能覆盖或清除已有义务。Shell 的 validate/mutate_and_validate
调用必须给出非空 `validation_targets`，缺失或无效时执行前返回 `validation_scope_required`
及 `tool_execution_skipped`，不能执行之后静默留下无法解释的验证债务。范围声明仍不是通用
语义证明；是否真正满足用户要求须结合实际结果审查，不能据此保证任意提交的文件集合正确。

完成协议采用明确的 `decision` 与必填 `outcome`，不再以 rejected/unknown 标签的文本
子串作为验收条件。`accept/answered` 仅适用于本轮没有成功 mutation 调用的回答；存在这些
调用时，接受报告必须使用 `accept/action_completed`，关联本轮
Host 确认的成功、未跳过的 mutation 调用 ID；模型仍须核对结果是否满足请求后置条件。
只读调用、其他回合调用和正文命令均不能充当动作成功记录。`clarify/needs_input` 保留
被审查的具体问题并回到输入栏；`blocked/blocked` 交付说明但结束为失败。二者不能伪装成
动作完成。`execute/incomplete` 回到正常执行/权限流程。模型的任务分类仍是语义判断，
不能把“存在成功调用”泛化成任意任务已验收。

`accept`、`clarify`、`blocked` 必须无未解决 violations，Core 原样发布被审查候选，不再
启动最终自由生成。`revise/incomplete` 只允许产生私有新候选，新候选必须重新审查；最多
两次修改，修改计数与协议错误计数分离。工具关闭阶段不发布候选/审查 JSON/修改稿的流式
文本。Task outcome 同步进入 checkpoint、Core snapshot、TurnCompleted 事件及 CLI done
reason；`needs_input` 是回合交付结束，不是动作成功。

审查请求必须与执行助手隔离：仅保留独立审查协议作为 system instruction，执行策略、
原始会话、工具证据及候选统一序列化为不可信数据。不得把原始用户追问继续作为审查器的
当前任务消息，也不得混入执行助手的实时 system 指令。上下文预算按实际审查投影计数。

`workspace_status` 必须显式标识是否覆盖仓库根目录，并分别给出 tracked/untracked 条目数；
untracked 已包含于 changed_entries，不能再次相加。子目录空 diff 不能证明全仓库干净，
根目录可见性也不构成自动扩大访问权限的授权。

完成审查不能以“模型已经调用工具”为前提。冻结的已启用能力声明 evidence audit 时，
从首轮起将候选正文保留为私有数据；候选即使没有工具调用，也进入结构化任务/证据审查。
审查的 `execute` 决定表示原任务尚未执行或缺少必要证据，清除候选与报告标志，回到
普通执行阶段并保留有界纠正计数和诊断。审查本身不调用工具、不授权工具、不根据用户
文本关键词选择工具。`accept`/`revise` 仅用于已有证据足以完成回答的情况，问候和基于
已有上下文的解释不要求工具。最多两次退回执行；预算关闭后不能重开，必须明确失败。
禁用或未启用的能力不能单凭其注册信息触发审查。计数与反馈随 checkpoint 保存恢复。

Core 必须依据实际发出的模型请求工具投影判断是否允许工具调用。工具列表为空时的
原生工具调用属于模型响应协议违规，不得提交为工具执行、权限请求或工程恢复义务。
保留当前候选/审查报告，在相同阶段有界重试；重复违规明确失败。预算耗尽与用户拒绝
权限的终止规则仍优先，不得为修复协议违规重新开放工具。

`workspace_status(include_diff=true)` 将 staged/unstaged 完整 diff 分别保存到会话输出存储，
在摘要前部返回 `diff_outputs` 引用与字节数，源码内容不充当元数据节。未跟踪文件不在
Git diff 中，必须单独读取。状态统计与确定性规则扫描仅是取证入口，不代表语义审计完成。

## 9. 验收

### 工具结果、授权与验证义务

共享 `nola_host_protocol::tool_result::ToolResult` 将业务数据、展示文本、执行回执和完整输出引用
分开。CLI 顺序、并行和事务恢复路径使用相同结构；禁止根据工具名称解析展示文本取得控制数据。
执行回执单独记录状态、资源 URI、变更前后版本与后置条件。`Completed` 仅表示调用结束，不能
单独证明任务完成。未迁移适配器的原始结果不具备回执权限，其保留字段在 Core 边界移除。

内置工具的提示权限域由 Core `tool_permission_domain` 定义，Host 不再维护独立豁免清单。
Core 授权经内部 `ExecutionAuthority` 传递；模型参数 `_nola_approved_once` 不具有授权效力。
授权不能绕过资源边界和输入校验；专用操作的语义前置条件不应变成其他工具的强制工作流。

写入范围与验证范围分别投影；未知写入单独计数，不以 `*` 假装具体资源，也不能被通配验证清除。
组合写入验证只可证明本次范围，不能顺带消除此前债务。路径身份不得通过后缀相同认定相等。

`git_commit` 是可组合的选中文件操作：校验实际提交父版本/树/文件集合及未选中 index 项，
返回本次实际版本回执，不要求声明整个仓库的排除项。失败可能已产生副作用时必须明确报告，
不能伪装未执行或自动回滚用户内容。授权 Shell 仍可执行其他 Git 语义，但不能伪造同等验证回执。

当前边界：未迁移适配器仍可返回不带回执的数据；通用 Shell 的声明范围不是语义验证证明；
Git 回执不证明文件选择符合用户意图。所有适配器迁移与真实模型端到端通过前，不宣称全面验收完成。

2026-09-21 验收记录：Core/Host/协议/内核 605 项、CLI 单测 1187 项（2 项忽略）、
输出存储端到端与新增事务结果恢复测试通过。隔离真实模型提交追问未通过：模型将已读取的源码
列入排除项，最终审查在 240 秒期限内未收敛。该案例不能以 Git 事务成功代替任务验收；
剩余工作是提交范围的意图/证据核对及完成审查收敛，不能放松范围断言或忽略该源码来让测试变绿。

后续定位确认：源码内容保留于 Core 工具结果，不能归因于输出丢失。持久化的私有改稿含伪工具
协议，却因 `ReviseDraft` 分支先于候选格式检查而进入再次审查。候选准入必须先于首次保存和
改稿替换；无效候选不覆盖原候选/审查报告，按有界计数重试或失败。审查 JSON 引用被拒绝的
协议作为证据不等于候选执行协议，不应误拒绝。此修正通过 591 项 Core/Host/内核测试，
当时尚未重跑真实模型验收。随后同一单轮提交场景重跑通过；不代表三轮追问或稳定性验收通过。
该次修正尚未解决排除路径的理由与证据绑定，后续整改见下文。

权限/数据整改补充：缺省免提示能力仍产生权限主体并评估所有匹配规则；没有匹配规则或仅由
产品 Allow 默认档派生确认时，使用免提示默认值。配置的 Ask/Deny 不得被空权限域跳过。
顺序、并行及委托结果在推进响应序号前
检查外层 outcome 与执行回执的一致性；矛盾状态、成功状态下的失败检查和未知结果协议版本
必须拒绝，原待处理响应保留，允许更正响应使用同一序号重试。iOS/Android 工具响应使用
`outcome` 和 v1 结果信封，不再发送旧的 `is_error` 响应字段（历史 ToolResult 的布尔字段除外）。

Host 完整输出引用有独立字段，事务保存与恢复必须保留；业务数据或展示正文不能提供该引用。
UI 的摘要投影与模型证据投影独立，不能为了隐藏内部协议而缩减模型实际收到的证据。
委托执行出口也使用共享结果信封，而不是只发送已恢复结果的 text。

版本化验证账本：Core 将资源身份与 Host 观察的写后版本一起保存到 checkpoint；只有成功检查
回执中的身份和版本同时相等，才关闭对应义务。旧版本、其他路径、大小写不同的路径不能
获得验证信用；再次写入即使不新增验证要求，也要更新已有义务的版本。失败验证保持
Correcting；声明的 validation_targets 不能独自标记期望产物已验证。没有观察到版本的
写入保留未知版本义务，明确报告证据限制，不通过反复验证或重复写入掩盖它。

文件变更回执由 Host 读取实际文件计算 SHA-256，删除路径使用 absent:v1。Shell 声明的
output_artifacts 经既有产物差异检查后产生同样的版本回执。Shell validate 在命令前后采集
目标快照，只有退出码为零且快照一致才产生检查项；mutate_and_validate 的标签不能证明
内部写入/检查顺序，必须再单独验证。快照限制 4096 个条目、单文件 64 MiB、总大小预算
256 MiB，拒绝目标或遍历条目为符号链接；建议指定实际修改文件，删除项需显式列出。
宽泛目录中测试产物的变化会使快照不一致，不能因此放松为“命令成功即验证通过”。这些
快照不是操作系统隔离或并发写入锁；exit-zero/stable-resource 检查也不证明命令语义正确。

历史的 `excluded_paths/exclusion_reasons` 两类排除设计已废弃：它错误地把单次操作范围与
整个任务范围等同，导致部分提交、自然追问和多步组合被拒绝。当前契约只接收选中路径；
任务是否漏做由用户目标与实际结果的语义审查判断，不以全仓文件分类器代替。

### 开放需求与有界操作的分层

用户目标不由工具名、能力名或语言关键词分类；模型依据目标、上下文和 schema 组合操作。
Core 只评估具体调用的权限主体、资源、输入及结果协议，不枚举用户需求。
Skill/SOP 是可选的复用契约：用户明确要求时遵循，否则由模型判断采用是否有帮助；多步任务
或语义相似不自动强制激活。已采用的契约仍须履行，不能在失败后静默丢弃其验证义务。
普通内置写入不自动生成构建/测试任务；memory_save 等写入也不以工具名称自称验证成功。
用户要求的验证、显式验证调用和已有工作流义务仍然有效，后续普通写入不得清掉旧版本义务。

操作成功、后置条件证据和用户任务完成是三个不同判断。Host 回执只证明实际观察到的事实；
声明的输出范围、进程退出码和模型的完成措辞均不具备额外授权或通用正确性证明能力。
验收用例检验组合、顺序、保留范围和恢复等不变量，不将有限用例变成产品中的需求路由表。

前一轮验证：Core 377、Host 合同 103、共享协议 17、内核 113 项通过；CLI 1188 项通过、
2 项忽略；输出存储端到端 1 项和 Android Host 合同 5 项通过。iOS 本轮仅验证 Swift 语法，
未完成模拟器/真机测试。隔离真实模型单轮提交回归在 240 秒截止前尚未收到最终审查结果，
整体未通过；实际提交包含全部 3 个目标文件，保留候选答复也与其一致。这不能替代回合完成验收。

版本账本/排除证据整改后的验收：`cargo test -p nola_core --tests` 共 613 项通过（含
383 项单元测试、103 项 Host 合同测试及其余集成合同）；共享协议 17、内核 113 项通过；
CLI 1192 项通过、2 项忽略；完整输出存储端到端 1 项通过。生命周期生成基准同步新增的
版本账本、未知范围计数和完成 outcome 字段，未删除原断言。debug CLI 已重新构建。

同一 openrouter/free 配置下，隔离真实模型的单轮提交及完整三轮“只读审计→根目录澄清→
提交”均通过原有精确断言：README.md、cli/scripts/note.txt、native_host_output.rs 全部
提交，.env 和 cli/scripts/nola 未进入索引且内容不变，完成状态为 action_completed。
单轮证据目录后缀 r3jhr375，三轮证据目录后缀 nfcra7io；两者均位于临时目录，不修改
本仓库历史或用户会话数据库。此结果证明这两个场景本次通过，不等同多供应商稳定性、
全部平台适配器或 GUI 端到端验收。

开放组合整改本轮验证：CLI 1197 项通过、2 项忽略；Core 及集成合同 614 项通过；
debug CLI 已重建。专用 Git 操作验证了分步选取、未选中 index/工作树保留、完整工作树
版本、字面路径、删除/符号链接及 hook 失败。不修改本仓库历史或生产会话数据。

真实模型组合验收仍未通过：在独立临时仓库里，第一轮模型取消了未选中源码的暂存；
补充通用状态保留指导后，第二轮仍用 Shell 提交把未选中暂存源码纳入提交。断言未放宽，
也未新增按用户语句匹配的路由规则。此结果暴露的剩余缺口是：用户要求保留的状态尚未
成为执行层可强制检查的任务约束。工具自身的有界回执和权限审批不能替代这一层，提示词
也不构成状态保护机制。不能以本轮单测通过宣称任意组合已经安全或全面整改完成。

#### 已有暂存状态的进程级保留边界

Shell Host 在授权检查后、启动进程前，让 Git 适配器读取工作目录及授权根目录的实际仓库
状态。发现已有暂存对象时，适配器声明 Git 元数据为只读资源，并提供保持 index 的能力契约；
不读取用户语句，不按命令关键词判断写入，也不相信模型声明的 inspect/validate 是只读证明。
通用 `ProcessPreservation` 只接收资源路径、恢复能力和 Host 环境配置，不解释 Git 工作流。
macOS 后端将只读资源编译为继承到子进程的写入限制，同时禁止改名包含该资源的父目录。
实际 git-dir、common-dir 及 linked-worktree 的 `.git` 指针都受到保护；只读 Git 查询禁用
可选 index 刷新。没有已有暂存对象时，此适配器不额外限制 Shell。

显式选中路径的 `git_commit` 由 Git 适配器执行和核对，其能力不是关闭通用进程保护的参数。
它允许目录内的绝对路径与相对路径，规范化目录别名但不跟随叶子符号链接。模型不得通过
批准标记、修改 effect 标签或改用嵌套脚本取消只读约束。执行失败仍可能有其他副作用，
不能把进程受限等同于整个命令未执行或自动回滚。

当前后端范围是 macOS 前台 Shell 进程及其子进程。没有隔离后端的平台，以及尚不能持久
携带约束的后台路径，遇到此约束时启动前拒绝；前台不得自动移交为无约束后台任务。
这不是所有能力统一沙箱，也不是任意用户语义的自动形式化：其他客户端、MCP/原生写入
适配器、外部服务、独立并发写入者和任意 Git hook 副作用不在此机制的保证范围内。

完成标签按已有调用回执记账，而不是按用户语句分类；它不额外授予权限，也不证明目标语义
正确。完成审查遗漏或伪造 effect_call_ids、用 answered 隐藏已有修改调用，属于审查协议错误：
保留同一私有候选，有界重试无工具审查，不能将错误报告自动改成 execute 后重放成功操作。
只有有效审查明确发现仍有工作要做，才允许恢复原任务执行。回归测试覆盖中英文请求、真实
修改后错误标签、虚构回执、更正后的完成，以及审查重试期间不重新开放工具。

本次验证：Core 及集成合同 615 项通过；CLI 1200 项通过、2 项忽略。进程保护测试覆盖
直接写入、嵌套 Shell、目录改名、硬链接、指针文件和路径引号；已批准或标为 inspect 的
修改调用仍受保护。权限恢复测试改为引用真实 Host 回执，仍断言原操作只执行一次。

隔离真实模型 openrouter/free 验收：单次提交、三次分步提交均通过精确文件集合、内容、
父提交链及 action_completed 完成状态断言。分步提交保留原有 unrelated.rs 的暂存对象
及其不同的未暂存内容；.env 与构建产物未纳入提交。证据目录后缀分别为 iszwd8eo、
7_og__mv。此前 300 秒运行超时记录保留；通过运行使用 600 秒上限，不表示满足 300 秒
延迟目标。单次提交用例明确要求只提交已有变更、不创建或修改文件，避免把该测试约束
留给模型猜测；产品没有对应的关键词路由。两个用例本次通过不代表多模型稳定性验收。

- 依赖方向符合本文第 3–5 节。
- 拆分后的模块可独立描述输入、输出和失败，不依赖调用方内部字段。
- 原有行为测试无需特化用户语句或语言即可通过。
- `MacChatWorkspaceView.swift`、`ChatViewModel.kt`、`nola-store/src/lib.rs` 行数持续下降。
- 新代码通过对应格式化、编译和单测。

macOS 日常架构验证使用 `./scripts/test_mac_layers.sh contracts`。会启动、暂停、强杀或重启
真实宿主进程的恢复测试只进入显式 `host-e2e` 或 `full` 层；测试脚本必须在自己的生命周期
内抑制并恢复 CrashReporter 对话框设置，不能让自动化测试阻塞在系统 UI 上。
