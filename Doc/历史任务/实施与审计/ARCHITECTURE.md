> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Nola 总体架构合同

版本：2.0-draft  
状态：Normative  
适用范围：Core、CLI、Web、macOS、iOS、Android、Windows、Agent Host 与后端

## 1. 目的

工作区访问遵循“目标 ≠ 默认 cwd ≠ 可访问 roots ≠ 操作批准”。外部目录扩权必须通过
`workspace_access` 的具体资源授权，Native OS 授权先于 Core 决定交付；Host 只在该能力
成功后更新本会话 roots。普通工具的批准不能绕过路径边界，客户端不能根据任意参数或
自然语言猜测目录授权。详见 `ARCHITECTURE_V2_MODULE_CONTRACTS.md` 的外部工作区访问合同。

本文定义 Nola 的总数据所有权、运行边界和跨平台合同。专题文档可以补充具体协议，
但不得改变本文定义的事实源、写入顺序和层级职责。

模块输入输出、运行/任务/证据状态、恢复流程、性能测量与纵向验收的当前细化契约见
[可执行架构设计](../../软件架构设计/README.md)。其[审计账本](../2026-10-19_当前任务归档/AUDIT.md)明确区分
已验证事实与待整改目标。文档中的 MUST 不是现有实现已达标的声明；历史续补不覆盖这些契约。

本次重设计解决的不是单一客户端缺陷，而是下列系统性问题：

- 完整会话同时存在于 Core checkpoint、平台聊天存储和 Host session 中。
- UI、Provider、Host 和 Core 都可能创建或完成一条 Assistant 消息。
- Context 压缩结果可能被误用为完整聊天历史。
- Trace 同时承担诊断日志、聊天恢复和 UI 状态，语义过载。
- CLI SQLite、Mac JSON、iOS 文件存储和 Android Room 使用不同业务模型。
- 数据源、自动化、Skill、权限和记忆在各平台重复实现策略。
- Core 中仍存在文件系统、系统时间和平台目录等非确定性 I/O。

## 2. 不可违反的系统不变量

1. 每个设备上的 `Nola Store` 是该设备产品数据的唯一事实源。
2. 完整聊天记录不会因上下文压缩、模型切换、Host 重连或 checkpoint 恢复而改变。
3. 一次用户提交只准入一个 Run 和稳定的初始 Assistant 消息身份；同 Run 的后续消息必须经显式消息准入，不能由 UI/Host 自行创建。排队的下一次用户提交属于独立 Run。
4. Provider chunk、Host event 和 Trace 不得直接成为 UI 状态。
5. Core 决定语义，Host 执行副作用，Store 提交事实，UI 只读投影。
6. 所有外部副作用都必须在持久化 transition 后才允许派发。
7. 重复、乱序和重放事件不能产生重复消息或重复副作用。
8. Context 是从完整数据派生的模型输入，不是聊天记录。
9. Trace 是可观测性数据，不是业务数据事实源。
10. 平台差异只能体现在 Host capability，不得复制 Core 策略。
11. 模型、执行单元、Host 和权限模式是四个正交配置维度。
12. 客户端不得通过关键词匹配重写请求、强制工具、选择 Skill 或授予权限。

## 3. 总体分层

```text
┌──────────────────────────────────────────────────────────────┐
│ Product UI                                                   │
│ Chat / Data / Tasks / Memory / Skills / Settings            │
└──────────────────────────────┬───────────────────────────────┘
                               │ commands + queries
┌──────────────────────────────▼───────────────────────────────┐
│ Application Layer                                            │
│ use cases / transaction boundary / projection subscriptions  │
└───────────────┬──────────────────────────────┬───────────────┘
                │                              │
┌───────────────▼──────────────┐  ┌────────────▼──────────────┐
│ Nola Store                   │  │ Agent Core                │
│ canonical ledger + views     │  │ deterministic reducer     │
│ SQLite + blobs + schema      │  │ policy + planning         │
└───────────────┬──────────────┘  └────────────┬──────────────┘
                │ committed effects/events      │ effects/responses
┌───────────────▼───────────────────────────────▼──────────────┐
│ Agent Host                                                   │
│ model transport / tools / OS APIs / external executors       │
└───────────────┬──────────────────────────────────────────────┘
                │
┌───────────────▼──────────────────────────────────────────────┐
│ Platform and Infrastructure Adapters                         │
│ Keychain / TCC / SAF / Health / launchd / WorkManager / Web  │
└──────────────────────────────────────────────────────────────┘
```

后端位于本地产品边界之外，只提供显式联网能力：模型中转、付费、同步、消息渠道和
远端调度。后端默认不是聊天历史权威来源。

## 4. 模块职责

### 4.1 Nola Domain

跨平台稳定类型和规则：

- Conversation、Message、MessagePart、Attachment、Artifact
- Run、RunEvent、ToolCall、PermissionRequest、TaskHandle
- ContextSnapshot、MemoryRecord、DataSource、Automation
- Skill、MCP descriptor、Model profile、Execution profile
- ID、revision、cursor、provenance、retention 和错误分类

Domain 不依赖 UI、数据库、Provider SDK 或操作系统 API。

### 4.2 Agent Core

Core 是确定性状态机，负责：

- Session/Turn 生命周期和输入队列
- 能力目录、工具生命周期和并发屏障
- 权限策略计算
- Context 计划、压缩计划和 Token 预算
- Skill/MCP 语义、长任务晋升与取消
- 记忆写入语义和冲突规则
- 产生有序 Domain Event 与 Effect
- 验证 effect correlation、sequence、lease 和 replay policy

Core 不负责：

- 文件、SQLite、网络或系统时钟 I/O
- Provider 和工具的真实执行
- UI 文案、动画或本地化
- OS 授权弹窗
- 永久保存完整会话

所有时间、随机种子、平台能力快照和外部结果均由 Host 作为输入提供。Core 当前的
文件型 persistence、`Utc::now()` 调度和平台目录发现应迁出。

### 4.3 Nola Store

`Nola Store` 是产品拥有的本地持久化服务，由共享 Rust `nola-store` 模块提供当前 schema、
事务和查询语义。CLI 直接链接；Apple 平台通过稳定 C ABI；Android 通过 JNI。
平台可以提供文件保护、备份和密钥，但不得重新定义表语义。

Store 负责：

- append-only Domain Event ledger
- 完整消息和 Part 的物化视图
- Core checkpoint、inbox/outbox 和 event cursor
- 数据源快照、索引元数据和 freshness
- Memory、Skill、MCP、Automation、Task 和权限授权记录
- Blob/Artifact 引用与生命周期
- 跨设备同步 outbox 和 tombstone

Store 不保存 Provider API key；只保存 `credential_ref`，真实密钥进入 Keychain、
Android Keystore、系统凭据服务或进程注入的安全存储。

### 4.4 Application Layer

Application Layer 是唯一写入协调者：

- 接受用户 intent
- 开启 Store 事务
- 调用 Core 计算 transition
- 原子保存 Domain Events、物化视图、checkpoint 和 outbox
- 提交成功后派发 Host effects
- 将 Store projection 发布给 UI

平台 ViewModel 不允许分别修改消息数组、工具数组和运行状态。

### 4.5 Agent Host

Host 执行 Core effect：

- 模型请求与流式传输
- 文件、Shell、Git、浏览器、计算机控制和平台数据工具
- Skill 资源加载与 MCP transport
- Context component 读取
- OS 权限查询与请求
- 长任务进程和取消
- 外部执行单元适配：Codex、Claude Code、OpenCode 等

Host 返回相关 response，不创建业务消息、不决定权限持久范围、不压缩聊天记录。

本机 in-process Host、捆绑后台 Host 和远端 Host 使用同一协议。CLI 是一个产品客户端，
不是 Mac 原生 Chat 的架构依赖。

### 4.6 UI Projection

UI 只订阅 Store 的物化投影：

- Chat 读取 Message + MessagePart + RunActivity
- 输入框与右侧上下文面板读取同一 ContextSnapshot
- 任务中心读取 TaskHandle + TaskEvent
- 数据中心读取 DataSource + Snapshot + Validation
- 权限面板读取 PermissionRequest + Grant

UI 可以保存纯展示偏好，例如展开状态和滚动锚点，但不能保存第二份执行事实。

## 5. 唯一事实源矩阵

| 数据 | 唯一事实源 | Core checkpoint 是否包含 | Host 是否长期保存 |
| --- | --- | --- | --- |
| 完整消息 | Nola Store | 否，只保存 cursor/ref | 否 |
| 流式正文 | Nola Store MessagePart | 仅开放 Part 状态 | 仅未 ACK 重放缓冲 |
| 工具调用与用户可见结果 | Nola Store | 当前 Run 生命周期 | 仅执行临时态 |
| 模型上下文 | ContextSnapshot | 当前 snapshot ref/state | 否 |
| 压缩摘要 | Nola Store | summary ID | 否 |
| 权限规则与授权 | Nola Store | 当前 Run 所需投影 | OS 状态实时查询 |
| 数据源定义与快照 | Nola Store | 只含选中引用 | Connector 临时缓存 |
| 长期记忆 | Nola Store | 当前 recall projection | 否 |
| Skill/MCP 配置 | Nola Store | 当前 capability projection | transport 临时态 |
| 自动化与任务记录 | Nola Store | 当前活动任务投影 | scheduler lease |
| 诊断 Trace | Diagnostic Store/日志 | 否 | 可短期缓冲 |
| 密钥 | 平台安全存储 | 否 | 只在调用期间读取 |

## 6. Canonical Store 模型

当前逻辑表如下；物理表可以按查询与事务边界组织，但语义不得合并：

```text
conversations
conversation_settings
messages
message_revisions
message_parts
attachments
artifacts

runs
run_events
tool_calls
permission_requests
permission_grants
core_checkpoints
effect_outbox
host_event_inbox

context_snapshots
context_components
context_summaries
memories
memory_revisions

data_sources
data_source_connections
data_source_read_recipes
data_source_validations
data_source_snapshots
data_source_items
index_chunks

skills
skill_versions
mcp_servers
capability_bindings

automations
task_runs
task_events

models
execution_profiles
sync_operations
```

大文本、二进制工具输出和文档产物进入 content-addressed blob store。表中保存 digest、
size、MIME、provenance 和 retention；聊天附件只引用明确交付的 Artifact。

## 7. 会话与流式输出事务

### 7.1 用户提交

一次发送必须在一个事务中完成：

1. 写入 User Message。
2. 创建稳定的 Assistant Message，状态为 `queued`。
3. 创建 Run，绑定 `conversation_id`、`user_message_id` 和 `assistant_message_id`。
4. 保存本轮 ContextSnapshot 引用。
5. 调用 Core 创建 Turn transition。
6. 保存 transition、checkpoint 和 effect outbox。
7. 提交事务后才派发模型 effect。

因此用户按下发送后，Assistant 气泡立即出现；不依赖首 Token 才创建。

### 7.2 Host 事件

每个 Host response 至少包含：

```text
session_id
run_id
turn_id
effect_id
request_id
event_id
sequence
payload
```

处理顺序：

1. inbox 以 `event_id` 去重。
2. Core 验证 owner、lease、effect 和 sequence。
3. Core 生成 Domain Event 和新 checkpoint。
4. Store 原子写入 event、MessagePart 物化结果、checkpoint 和下一批 outbox。
5. 事务成功后 ACK Host 并发布 UI projection。

若提交失败，不得 ACK。运行实例必须从最近已提交 checkpoint 恢复，Host 从最后 ACK
序号重放。

### 7.3 文本增量

- 首个正文事件创建一个稳定 Text Part。
- 后续事件只能追加或修订该 Part 的尾部。
- Markdown 使用稳定 block identity 和一个 mutable tail。
- 完成事件只关闭 Part 并校验最终 digest，不再写一份“最终全文”。
- 如 digest 不一致，请求缺失区间重放，禁止用最终全文覆盖整个气泡。
- 为降低写放大，Store 可在 16-50 ms 内合并连续文本增量，但崩溃恢复不能丢失已 ACK 内容。

### 7.4 完整消息定义

必须持久化所有用户可见内容：正文、可展示思考摘要、工具过程、参数、结果摘要、附件、
Artifact、错误和取消状态。Provider 私有 reasoning、内部 continuation 和协议噪声不作为
用户消息保存；必要的 continuation 以加密、短期 Run 状态保存并设置明确 retention。

## 8. Context、压缩与长期记忆

完整历史永不压缩。Context Engine 从 Store 构造不可变 `ContextSnapshot`：

```text
system instructions
conversation summary refs
selected complete recent turns
active goal/task state
memory recalls
selected data-source snapshots
tool schemas
token accounting
```

Context 压缩只创建新的 `context_summary` 和 `context_snapshot`，不删除或改写 Message。
每个快照记录输入消息范围、模型、策略版本、Token 估算/实测值和来源 digest。

长期记忆是独立实体：

- 后台 Memory Worker 只消费已提交消息。
- 记忆包含 provenance message IDs、版本、置信度、敏感级别和 supersession。
- 内部记忆写入无需 Agent 权限，但必须审计、可查看、可撤销。
- Recall 结果进入 ContextSnapshot，不复制进聊天历史。
- Recall 在投影层使用 `nola-memory://record/<record-id>` typed internal reference；客户端以本地化的“长期记忆/上下文”语义展示并打开记忆中心，不把 provenance 文本冒充 URL，也不为无 destination 的条目展示跳转 affordance。

## 9. 工具、Skill、MCP 与权限

### 9.1 统一 Capability Catalog

Core 只接受 Host 发布的 typed descriptor：schema、risk、route、platform、parallelism、
cancellation、replay 和 output contract。模型从 schema 选择能力；不存在客户端关键词路由。

Skill 是版本化 capability package，包含说明、资源、所需工具、预算和验证合同。MCP 是
外部 capability transport。二者最终都投影到同一 Catalog 和 Tool lifecycle。

`tool_search` 搜索同一份冻结的 Host Catalog，并从 descriptor 物化 schema；不得只搜索
静态 ToolSpec、只注入 MCP，或按 provider 类型维护旁路目录。原生 Code Memory、设备桥接、
MCP 与内置能力遵守同一发现合同。通过类型化发现结果激活或已经成功执行的 capability，在当前
Turn 的后续 model step 中保持可见和可执行，直到 Turn 结束；完成轮次可以隐藏尚未选择的调查
工具，但不得撤销已经激活或执行过的能力。

Host 先把静态定义与运行时 schema 合并成一个可用目录，再执行精确选择、语义排序与
缺失恢复。相同 capability ID 使用当前 Host schema；缺失名称、候选与重试信息不能因
schema 来自内置、MCP 或设备而改变。未知运行时过滤元数据不得被当成满足过滤条件。
候选只来自当前允许暴露的目录；候选建议不是自动调用，也不是权限授予。
CLI/桌面 Host 的首轮小集合包含 `read_file`、`workspace_status`、`bash`，使基础读取、
工作区检查和执行不依赖名称猜测；其他扩展仍按需发现。初始可见性不改变权限评估。

### 9.1.1 完整证据与有界预览

完整工具结果由 Host 输出层保存，给模型和 UI 的预览单独裁剪。生产工具不得先无提示地
丢弃内容，再把残余交给输出层命名为 `full_output_ref`。`git_diff` 不提供内容截断参数；
`paths` 和 `include_untracked` 定义查询范围，范围内的 staged、unstaged、untracked
patch 均完整保留，二进制文件按 Git 的 binary diff 语义报告，不伪装成文本。
完整 patch 不代表包含二进制原始字节，也不代表模型已经审查了全部变更。

超限输出持久化失败必须返回显式错误，保留“操作可能已发生、输出不可完整取得”的区别；
不能把失败降级成成功预览或让模型据此声称无变更。恢复变更操作前先检查实际状态。
读取 cursor 是 UTF-8 字节边界，`next_offset` 指向实际已返回内容的末尾；不得在分页边界
损坏中文或 emoji。`has_more=false` 仅表示该结果文件读到末尾，不证明其他工具结果、
整个工作区或用户任务已经完成。优先按范围查询和搜索，避免机械读取无关的全部证据。
diff 首部保留完整文件清单，避免大文件 patch 隐藏后续文件的存在。单结果搜索共用一个
文件路径头，匹配行及重叠上下文只输出一次，并保留行号和字节偏移；导航结果不应被重复
路径挤满，再发生不必要的二次预览裁剪。

调查阶段由 Core 的 typed capability effect、规范化参数 fingerprint 与连续 batch 状态控制。不同资源、不同分页范围或不同结构化 selector 构成新的证据，不能因累计次数被误判为循环；相同 typed tool batch 连续三次才进入 loop checkpoint。checkpoint 暂时撤销调查能力，但显式保留 `request_user_input`，把重复能力、次数以及“继续一次 / 改变方案 / 停止”投影给用户；回答后清除本次 checkpoint 并从保留证据继续。该判断不读取自然语言关键词，也不维护工具名路由黑名单。桌面 Host 的有限模型迭代预算是异常协议循环的外层保险，最后一次请求为无工具归纳并明确报告未完成状态。

`request_user_input` 的 session identity 由 Host control state 持有，不依赖执行线程的 thread-local。工具即使在 worker 线程运行，也必须把问题事件送回原 session；不可用回退保留全部未决问题，使客户端或最终回复仍能把决定权交还用户。

### 9.2 Harness Step Runtime

一次物理模型采样是一个不可变 Step。Core 在 effect 中发布版本化 `StepContext`，其中包含
Turn context、冻结的工具投影、step index，以及 invocation policy、稳定前缀、模型前缀和请求
快照 digest。Host 必须整体解码并验证这些不变量，不得从 prompt 文本或分散的 settings 字段
重建语义。工具 schema 的身份和 token 成本在投影生成时计算；稳定前缀和 provider cache identity
跨同一 Turn 的 Step 复用，动态尾部单独组合。

送入 provider 前，Core 只对临时 model context 做规范化：孤立 ToolResult 被丢弃，中断的
ToolCall 补确定性的 synthetic result，并行完成的 ToolResult 按原始 ToolCall 顺序投影。该过程
不得修改 canonical transcript。工具的准入、并行 batch、取消和不可取消阻塞统一由
`ToolExecutionRuntime` 根据冻结 descriptor 决定；任何 completed、failed 或 cancelled 终态进入
投影前，所有开放的流式 Part 必须统一完成或丢弃，已经展示的非空部分答案必须保留。

Provider 错误使用 typed `ProviderFailure` 跨重试和 fallback 边界传递，至少包含稳定 code、用户
消息、retry/fallback 决策、HTTP status、Retry-After、provider/model identity 和有界诊断。UI 明确
展示失败位置及回退目标，原始响应只进入受限日志。循环进度、资源映射、preview/durable 语义全部
来自 capability descriptor 的 `loop_progress` 与 `resource_projection`；Core 不按工具名、参数名或
自然语言猜测。是否继续调用工具、进入审计或展示正文由 Kernel 的 `CompletionContract` 统一决策。

恢复与 fault-injection 测试必须覆盖：乱序并行结果、缺失/孤立结果、pending effect 重放、断流、
不支持取消的运行中工具、失败/取消时的部分文本，以及 retry/fallback 的 typed failure 投影。

### 9.3 权限

权限分两层：

- Agent approval：Nola 是否允许模型执行具体操作。
- OS authorization：平台是否允许进程访问资源。

用户可选 `strict`、`recommended`、`unrestricted` 三种策略。`recommended` 默认放行安全
只读操作，仅对破坏性、不可逆、广泛访问、敏感外发或未知能力询问。

Grant 使用规范化 scope 持久化：conversation/run/global、capability、resource、operation
fingerprint、expiry 和 constraints。“本次对话允许”必须在同一 scope 内稳定命中，不能因
`bash cat`、`read_file` 或展示名不同而重复询问。OS 授权仍由平台实时查询。

内部确定性操作，例如 Memory CRUD、消息物化和 Context bookkeeping，不进入 Agent 权限面板。

## 10. 数据源与索引

DataSource 是数据、任务和 Context 的共同边界：

```text
Definition -> Connection -> Validation -> ReadRecipe -> Snapshot -> Items -> Index
```

- 首次添加必须执行真实连接测试。
- 成功后保存读取方式、schema、样例摘要、授权、freshness 和内容格式。
- Snapshot 不覆盖上一版本；使用 revision 和 observed_at。
- 默认启用、会话可选和禁用是显式 policy，不由 UI 临时数组决定。
- Context 只能引用通过授权和 freshness 门禁的 Snapshot。
- Connector 留在平台 Host，policy、contract 和 Store schema 跨平台共享。

健康、日历、文件、网页、浏览器、MCP 和远端 Host 数据都遵循同一模型。平台不支持的
Connector 返回 `unsupported`，不能伪造 0 或 `authorizationNotDetermined` 数据。

## 11. 自动化、定时任务与长任务

Scheduler 分为三个部分：

- Core Schedule Planner：纯函数，输入 `now`、timezone、definition 和上次 watermark。
- Platform Trigger：launchd、BGTaskScheduler、WorkManager、CLI daemon 或服务端调度。
- Task Runtime：通过标准 Agent Run 执行并写入 TaskEvent。

自动化不能拥有独立 prompt 拼接器、权限系统或会话格式。它只创建带 `input_origin`、
ContextSnapshot 和 Skill bindings 的标准 Run。

只有预计长时间运行、需要脱离当前 Turn 或显式被模型/用户晋升的工作才创建 TaskHandle。
短工具始终留在消息气泡内。后台任务使用 lease、heartbeat、checkpoint、取消和 result ref。

## 12. 模型、执行单元与 Host

以下维度必须正交：

| 维度 | 示例 |
| --- | --- |
| Model source | API、MLX、GGUF、远端模型 |
| Model profile | model ID、context window、temperature、top-k 等 |
| Execution backend | Nola、Codex、Claude Code、OpenCode |
| Host location | 本机、后台 Host、连接宿主机 |
| Permission mode | strict、recommended、unrestricted |

切换标题栏选项只修改会话配置，不隐式启动模型或进程。运行前由 Host 返回 availability、
启动阶段和具体错误。Provider、local model 和外部 executor 最终都必须产生相同的 Core
response 合同，不能绕过 Store 直接更新 UI。

## 13. Trace 与可观测性

重新定义当前 Trace 责任：

- Domain Event ledger：业务事实，参与恢复和 UI 物化。
- Diagnostic Trace：从 Domain Event、Host timing 和 transport metadata 派生，用于日志和诊断。
- Presentation Event：Store projection 的瞬时通知，只用于减少 UI 查询延迟。

Diagnostic Trace 可以采样和按 retention 删除；删除 Trace 不能改变聊天、任务、权限或工具
结果。UI 状态栏不得从日志字符串或工具展示名推断运行状态。

## 14. 后端与同步

后端默认负责：

- 官方模型代理和额度
- OAuth、订阅和支付
- 可选加密同步
- 消息渠道 webhook/lease
- 用户明确启用的远端调度

本地 outbox 以 operation ID 幂等同步。消息追加天然合并；标题和设置使用 revision；删除
使用 tombstone；消息编辑创建 revision。服务端不能用短 Context 或远端 Host checkpoint
覆盖完整本地会话。

未经明确上传同意，消息、Memory、数据源内容和文件不得离开设备。同步设计必须区分
元数据、加密内容和 Provider 必需上传内容。

## 15. 跨平台代码组织目标

```text
core/
  nola-domain/       stable types and invariants
  nola-agent-core/   deterministic reducer and planning
  nola-store/        SQLite schema, migrations, ledger and projections
  nola-protocol/     Host/FFI/wire contracts

hosts/
  shared/            generic effect loop, retry, replay and outbox driver
  macos/             AppKit/TCC/files/browser/process adapters
  ios/               iOS frameworks and scoped resources
  android/           Android APIs, SAF and WorkManager adapters
  desktop/           CLI/Linux/Windows process and filesystem adapters

clients/
  macos/ ios/ android/ cli/ web/ windows/
```

现有目录可以渐进迁移，不要求一次重命名。约束是依赖方向：

```text
UI -> Application -> Domain/Store/Core -> Protocol
Host adapters -> Protocol/Domain
Core -X-> Store implementation, UI, OS, Provider
```

## 16. 当前 schema 与协议策略

Nola 尚未发布，因此只实现当前 schema 和当前协议，不提供迁移或向后兼容。

- `core/nola-store/src/schema.sql` 是本地产品数据库的唯一 schema。
- `backend/worker/migrations/0001_current_schema.sql` 是 D1 的唯一 schema。
- 不存在旧 JSON、Room、SQLite 或 Host transcript importer。
- 不存在字段别名、缺字段默认值、双路由、双写、fallback store 或旧事件 parser。
- schema 版本不匹配时明确失败；开发和测试环境重新创建数据库。
- Application Service 是唯一产品事实写入口，所有平台只提交类型化命令并消费投影。
- Agent Host 只接受当前 envelope、canonical identity、typed ContextComponent 和 Part 事件。
- LLM 网关透明传输；query 重写和联网检索只能由模型显式工具调用触发。
- Provider adapter 通过注册式 dialect parser 将文本通道中的工具协议归一化为类型化 ToolCall 或 `protocol_error`。协议命名空间与根元素属于版本化契约；Host orchestration、客户端和 View 不得通过协议字符串、错误拼写样本、自然语言关键词或模型名称推断协议状态。
- 无效控制帧不得进入 assistant 正文或 canonical history；恢复、重试、模型失败和 fallback 必须使用结构化事件，并以本地化状态向用户明确展示发生了什么。
- CLI 的实时展示同样必须把模型重试作为持久状态行提交，显示经过清洗的失败原因和失败模型到回退模型的身份链；不得只使用会被下一帧覆盖的 spinner，也不得展示 endpoint、密钥或原始 provider body。重试序号、目标耗尽状态和失败分类通过 Host progress metadata 进入 Core Trace，客户端不得从展示文本反解析控制状态。

### 16.1 模型文本工具协议边界

模型供应商可能把工具调用编码在文本增量中。该内容必须在 Provider adapter 内完成语义通道分类，不能先发布为 `assistantDelta`，再由 Host、reducer 或 View 根据字符串撤回。协议适配器以注册式 dialect 声明命名空间和根元素，并将完成的输入归一化为普通文本、有效工具调用或无效控制帧。

实现约束：

1. 可能是协议标签前缀的增量必须跨 transport chunk 暂存，直到能够确定通道；网络分块不能改变解析结果。
2. 进入已注册命名空间后，完整帧只由对应 parser 校验。合法帧转换为类型化 ToolCall，不产生 assistant 正文；非法或残缺帧产生 `protocol_error`，原始载荷不得进入正文、canonical message 或历史重放。
3. dialect 的命名空间、根元素和语法属于协议契约。具体错误拼写、语言关键词、用户提示词和模型名称不得成为分支条件。
4. Host orchestration、客户端 reducer 和 View 不得使用 `contains("DSML")`、`contains("tool_calls")` 或错误样本列表推断协议状态。新增协议只能通过注册 dialect 和实现 parser 扩展。
5. Host 对拒绝的控制帧发布结构化恢复事件。客户端明确显示失败位置、恢复动作以及后续模型 fallback，不显示原始协议载荷，也不因整体请求最终成功而删除失败记录。
6. Parser 回归必须覆盖合法、残缺、错误 envelope、标签空白变化、逐字符、固定及随机分块、协议前普通正文和普通 XML-like/Markdown 文本；断言任何控制帧都不会短暂进入可见正文。

平台代码可以保留 UI 偏好、OS 授权句柄、模型文件、浏览器 manifest、日志和临时 artifact，
但这些资源不得复制 canonical conversation、permission、memory、Skill、MCP、data source、
automation 或 runtime task 状态。

## 17. 架构验收门禁

必须具备以下自动化证据：

1. 1000 条完整历史在多次 Context 压缩后逐字节不变。
2. Provider 重复、乱序、断线重放不会产生重复文本、工具或消息。
3. 在每个事务边界杀进程，重启后只恢复已 ACK 内容且不重复副作用。
4. 相同 fixtures 在 CLI、Mac、iOS、Android 和 Web 产生相同物化消息。
5. 切换会话、模型、Host 和执行单元不会改变旧消息。
6. “本次对话允许”在规范化 scope 内不会重复弹窗。
7. 数据源过期、拒绝和不支持不会被渲染成 0 或可用。
8. 自动化使用标准 Run，产生与交互 Chat 相同的工具、权限和 Artifact 记录。
9. Memory 写入具有 provenance、去重和撤销测试。
10. Diagnostic Trace 全部删除后产品数据和 UI 仍可完整恢复。
11. Store 和协议解码器拒绝缺字段、未知旧字段和错误 schema 的 fixture。
12. 至少一个真实 LLM E2E 覆盖文本、工具、权限、取消、重连、压缩和继续会话。

## 18. 明确禁止的实现

- 用 Core model context 覆盖本地完整消息。
- 用最终全文替换已经流式形成的 Assistant 气泡。
- UI 直接消费 Provider delta 或自行合并 Host 历史。
- 从 Trace 日志字符串重建工具或消息状态。
- 在 View/ViewModel 中保存第二份 canonical messages。
- Core 直接访问文件系统、SQLite、环境变量、系统时钟或平台密钥。
- 自动化、Memory Worker 或 Skill 自建一套 Agent 循环。
- 通过语言关键词选择工具、Skill、Memory、Web 或权限。
- 把工具输入临时文件自动提升为用户附件。
- 将 API key、数据库密码、长效 token 或测试密钥写入设计文档。

## 19. 架构决策摘要

1. 完整产品数据属于 Nola Store，不属于 Core checkpoint 或 Agent Host。
2. Core 继续拥有 Agent 语义，但变为真正确定性的 reducer。
3. Store 采用共享 Rust schema 和跨平台绑定，消除四套数据模型漂移。
4. Domain Event ledger 替代 Trace 成为恢复和物化依据。
5. Agent Host 是唯一副作用边界，CLI 只是其中一个客户端/适配器。
6. Context、Memory、DataSource、Task 都是独立实体，通过引用进入 Run。

## 20. Application Service、压缩与资源发现补充合同（2026-08-24）

### 20.1 Application Service 生命周期

Application Service 是按 canonical 数据库地址复用的设备级单写服务，而不是某个 CLI、
Agent Host、窗口或单次请求的子进程。连接方可以启动或复用服务，但不得用自己的 PID、Turn、
HTTP 请求或窗口生命周期控制服务退出。短生命周期客户端退出后，其他客户端必须能够继续通过
同一 socket 和数据库提交命令；只有显式应用关闭、服务管理命令或数据库实例废弃才能终止服务。

该规则保证：

1. 同一数据库始终只有一个产品事实写入者。
2. `conversations`、`messages`、`message_parts`、`runs` 与 `core_checkpoints` 在同一事务边界内推进。
3. Host 断开只影响未完成 effect 的租约与恢复，不会关闭 Store，也不会丢失已提交投影。
4. 客户端只能通过 Application Command/Query 与版本化 wire envelope 访问产品数据。

### 20.2 压缩工作集所有权

压缩结果属于 Core 的可替换 `model_context` 工作集，不属于完整 transcript，也不是客户端生成的
摘要消息。完成一个 Turn 时可以清除该 Turn 的临时 reasoning、工具交换和草稿，但不得清除刚生成
的 Compaction item；它必须跨 Turn sealing 保留，直到被更新的压缩工作集显式替换。完整消息继续
由 Store 的 Message/Part ledger 保存，任何压缩、恢复或模型切换都不得改写它。

### 20.3 数据源内容分层

DataSource Definition、Connection、Validation、ReadRecipe、内容格式、freshness 与 Snapshot/Item
引用由 Nola Store 持久化。Host 按 ReadRecipe 读取外部资源：一次性原始响应只存在于受限执行过程，
除非数据源策略明确创建 Snapshot/Blob，否则不得进入 Message、checkpoint 或日志。已提交给用户的
Assistant Text Part 是独立产品事实，即使其内容来自数据源，也必须保留在完整 transcript 中。

因此，隐私门禁检查的是原始凭据、注入文本和未选择的原文是否泄漏到 checkpoint，而不是删除已经
提交给用户的答案。数据源首次添加必须完成真实读取验证，并保存可复用的 ReadRecipe 与内容格式；
验证失败不能把空值、零值或 `authorizationNotDetermined` 伪装成成功快照。

### 20.4 Rust 产物发现

客户端构建脚本和 E2E 不得假设 `cli/target/debug`、`cli/target/release` 或任何固定 Cargo target
目录。产物目录必须从当前 workspace 的 `cargo metadata` 解析，再结合 profile 和 target triple
定位。该合同同时适用于 Xcode 打包、Shell/Python E2E 和外部执行器测试，避免不同工作区配置使用
了过期或错误二进制。
7. 先迁移 Mac Chat，再逐步收敛其他平台，不进行不可验证的大爆炸重写。
