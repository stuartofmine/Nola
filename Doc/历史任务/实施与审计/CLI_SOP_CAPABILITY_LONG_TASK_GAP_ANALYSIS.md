> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# CLI SOP、能力编排与长任务差距分析

## 文档状态

- 日期：2026-08-27
- 范围：Nola CLI 的 SOP、工具与 Skill 编排、Goal、子 Agent、后台任务、权限、恢复和终端交互
- 参考实现：Codex CLI、OpenCode、Claude Code
- 性质：架构审计、实施台账与后续改造依据；未标记为“已验收”的项目不得视为完成

## 决策摘要

Nola CLI 当前已经具备 Goal、Plan、Skill、Tool、权限、后台任务、恢复和统一 Work 投影等主要构件，但这些构件尚未全部形成一条由 Core 持有、可持久化、可恢复、可审计的完整工作链。与 Codex CLI、OpenCode 和 Claude Code 相比，主要差距不是能力数量，而是以下四项系统属性：

1. **单一事实来源仍不彻底**：部分运行状态、恢复证据、外部执行结果和 UI 状态仍可能由 Host 或具体执行器单独维护，容易导致重复、遗漏、状态闪烁及冷启动不一致。
2. **长任务还不是统一协议**：后台进程、SkillRun、Goal、权限等待和外部执行器虽然可投影为 Work，但公平调度、资源预算、故障恢复和人工介入仍未由同一个 Supervisor 合同完整覆盖。
3. **副作用安全仍存在边界缺口**：内置 Tool、Runtime Tool、设备桥和 MCP 已进入 Work Step 事务边界，真实 MCP 崩溃窗口已有调用次数证据；Claude stream-json、Codex JSON-RPC app-server 和 OpenCode ACP 均已覆盖真实子进程异常退出、同一 backend session 恢复、迟到 ToolFinished 结算和副作用计数恰为一次。非 Git 工作区的直接文件工具、格式化器、checkpoint restore、带显式输出合同的前后台 Bash，以及 Word/Excel/PowerPoint 文档 Skill 已具备真实资源验证和幂等恢复。剩余边界主要是未声明输出的任意命令及第三方执行器内部无法由 Nola 事务控制或枚举的副作用；这些路径仍只能拒绝不安全重放，不能宣称绝对 exactly-once。
4. **SOP 的结构化程度领先于实际执行闭环**：Schema、依赖、条件、预算、重试、补偿和 verifier 已存在，但复杂分支、并行步骤、版本升级、恢复与真实 Skill 的端到端覆盖尚不足。

因此，后续不应继续通过增加 Slash Command、Prompt 规则或客户端关键词判断来补洞。正确顺序是：

```text
Canonical Turn / Message
  -> Typed Resume Snapshot
  -> Side-effect Transaction
  -> Unified Supervisor
  -> SOP / SkillRun Execution
  -> Work Event Projection
  -> CLI UI and Debug Views
```

## 参考项目对标矩阵

本节只比较可观察的系统能力，不把命令数量、界面相似度或 Prompt 长度当作成熟度指标。Codex CLI、Claude Code 与 OpenCode 的共同优势，是把模型决策、工具执行、用户介入和终端展示约束在稳定的运行协议中；Nola 当前的主要不足是这些能力虽已分别存在，但跨模块闭环仍不够强。

| 维度 | 参考项目的成熟做法 | Nola 当前不足 | Nola 目标 |
| --- | --- | --- | --- |
| 工作协议 | Turn、ToolCall、权限等待、取消和完成具有稳定 identity，事件顺序可重放 | 部分 Host 状态与 UI 状态仍可能脱离 canonical 事件，曾出现重复、覆盖、漏消息 | 所有展示都由 canonical event projection 派生；同一事件只结算一次 |
| SOP/计划 | 计划是运行中的控制面，可随执行结果更新，并保留已完成事实 | SOP 定义较完整，但复杂分支、并行 join、补偿和 verifier 尚未形成确定性运行时 | Core 持有步骤状态转换，Host 只执行 ready step，完成必须有 Evidence |
| 能力选择 | 模型依据工具 schema、描述和当前状态选择能力，失败返回可恢复协议 | Tool、Skill、MCP、Web、Memory 的成本、持续时间、产物和恢复合同不完全统一 | 统一 Capability Contract，禁止客户端关键词路由与 query 改写 |
| 工具历史 | 下一轮保留调用、参数、截断结果和 Artifact 引用，不重复注入完整旧输出 | 大结果和临时状态仍可能增加上下文噪声 | 每轮清空可调用瞬态，只保留结构化执行摘要与稳定引用 |
| 长任务 | 独立 worker、进程组、lease、heartbeat、预算、取消传播和恢复报告 | 显式后台 Shell 已基本进入统一监督，但所有执行路径尚未完全收敛 | 前台、后台、Skill 与外部 Agent 共用 durable supervisor，不因转后台重启 |
| 权限 | 权限请求挂起原调用，授权范围清晰，恢复原 call identity | 历史上存在重复弹窗、Custom 响应旁路和作用域丢失 | waiting_permission 不占运行槽；授权后原步骤恰好恢复一次 |
| 用户介入 | steering 在安全边界生效，可审计、可恢复 | 普通输入、Custom 权限响应和后台任务介入的边界仍需继续统一 | 用户介入持久化为 FIFO command，消费一次并保存游标 |
| 取消 | ESC 可中止当前工作并传播至工具和子进程 | UI 取消与真实进程终止曾存在脱节 | 有界时间终止完整进程树，不产生孤儿和伪完成 |
| 上下文管理 | 运行上下文与完整 transcript 分离，压缩保留任务状态和关键证据 | 长会话可能重复物化、重复解析或把展示数据当上下文事实 | 完整记录持久化；模型窗口采用摘要、Artifact 和近期事件 |
| 终端渲染 | 增量状态稳定，输入区与历史区职责清晰，resize 后可恢复 | 曾出现只剩末尾几行、光标错位、footer 不一致和工具行污染 | 单一 terminal projection；PTY 关键帧与宽字符、窄窗口回归 |
| 故障恢复 | 半截流、进程退出和网络错误不会重复正文或重放未知副作用 | 部分第三方执行器副作用只能检测为不确定 | 安全重试只发生在副作用开始前；开始后进入 reconciliation |
| 可观测性 | Debug 信息能关联 turn、call、worker、budget 与终态 | 日志曾偏文本化，难以定位跨模块时序问题 | 统一 correlation IDs；用户 UI 简洁，Debug 可展开完整证据 |

### 不应照搬的部分

参考项目用于建立质量下限，不意味着 Nola 应复制其产品边界：

- Nola 是 local-first 个人助手，数据源、长期记忆、自动化和跨端一致性比纯编码 Agent 更重要。
- CLI 是独立客户端和 Web Host，不应成为 macOS 或移动端的隐藏实现依赖。
- Core 应提供跨平台一致的工作协议，但不同平台可以提供不同工具集合、权限适配和展示层。
- 不以超长系统 Prompt 代替结构化合同，不以 Shell 包装所有工具，不以输出文本解析代替类型化事件。

## 分阶段验收门禁

以下门禁用于决定一项能力是否真正完成。代码存在、单元测试通过或 UI 能看到入口，都不能单独视为验收。

### P0：协议正确性

- 同一 assistant message 在流式、工具调用、恢复和完成过程中保持稳定 identity。
- ToolCall 从 proposed 到 terminal 只结算一次；未知工具可恢复，不污染下一轮目录。
- 权限等待、用户等待和后台等待均为持久状态，CLI 重启后可继续。
- ESC 能终止当前 Turn、worker 及其完整子进程树。
- 前台转后台沿用同一 task、worker、command PID、日志和 Artifact。

### P1：长任务可靠性

- Agent、Shell、Skill 和外部执行器共享全局与项目级并发预算。
- worker 在命令开始前崩溃可以安全认领重试；命令开始后崩溃不得自动重放未知副作用。
- wall time、CPU、RSS、进程数和输出预算均有真实观测值及明确终止原因。
- 多任务完成能够唤醒公平队列，不依赖用户再按一次 Enter。
- 长任务介入、取消、权限批准和恢复均有真实 PTY + SQLite + 子进程 E2E。

### P2：SOP 执行闭环

- 条件分支决议持久化，重启后不重新选择已确定分支。
- fan-out 步骤可以并行，fan-in 只在依赖 Evidence 全部满足后进入 ready。
- RetryPolicy 区分纯计算重试、幂等副作用和必须人工核验的副作用。
- replan 创建新 revision，保留旧 run/step/artifact/evidence 审计链。
- Goal 只能在 required verifier 全部通过后完成。

### P3：性能与交互

- 一千万字符会话不整体物化到终端；首屏、搜索、追加和退出有稳定基线。
- 三个一千万字符会话连续切换不重复解析全部 Markdown 与工具详情。
- 1 万条工具/Skill 事件使用窗口化列表与惰性详情加载。
- 普通模式与 Debug 模式共享消息和 footer 投影，不维护两套行为。
- PTY 截图覆盖输入、流式文本、工具过程、权限面板、后台任务、resize 与恢复关键帧。

### E2E 最小场景

最终回归至少执行一条包含下列阶段的真实工作流：

```text
真实模型生成计划
  -> 只读工具收集上下文
  -> 权限请求并挂起
  -> 用户选择会话级授权
  -> 写入 Artifact
  -> 一个长步骤转入后台且不重启
  -> 用户 steering 修改后续步骤
  -> CLI 被强制退出并重启
  -> 从相同 run/step identity 恢复
  -> verifier 检查产物
  -> Goal 完成并输出 Evidence
```

测试必须断言数据库状态、事件序列、真实进程、文件或外部副作用计数以及 PTY 可见结果；禁止仅断言模型文本包含某个短语。

## 统一 Shell Supervisor 实施证据（2026-08-27）

本阶段已把显式后台 Shell 和交互式前台转后台路径接入与 Agent 共享的 durable supervisor，并通过真实 CLI、PTY、SQLite 和子进程验证。该结论只覆盖下表列出的合同，不代表整份审计已经完成。

| 合同 | 当前证据 | 状态 |
| --- | --- | --- |
| 显式后台任务由独立 worker 执行 | `background_shell_runs_in_a_real_claimed_worker_with_resource_evidence` 验证 ownership、worker/command PID 与资源证据 | 已验收 |
| Agent 与 Shell 共享全局/项目并发槽 | `background_shell_and_agent_share_the_same_real_concurrency_slot` 验证 Shell 结束会主动唤醒公平队列，无需用户再次输入 | 已验收 |
| 输出预算由真实 worker 强制执行 | `background_shell_output_budget_is_enforced_by_the_real_worker` 验证失败终态、实际输出字节和明确预算原因 | 已验收 |
| 命令开始前崩溃可安全恢复 | `claimed_background_shell_recovers_before_command_start_without_replay` 验证 lease 过期后换 token 认领且副作用只发生一次 | 已验收 |
| 命令开始后 worker 崩溃不得重放 | `killed_shell_worker_after_command_start_requires_reconciliation_without_replay` 验证进程组终止、单一 reconciliation 事件和副作用不重放 | 已验收 |
| Ctrl+B 不重启执行实例 | `foreground_shell_transfer_keeps_the_same_real_worker_command_and_logs` 验证转移前后 task、worker PID、command PID、日志和 Artifact 不变 | 已验收 |
| `auto` Bash 不再使用先启动后落库的旧提升路径 | 短命令由 Supervisor 完成后返回普通前台结果；长命令超过宽限期沿同一 worker 自动释放前台等待；`auto_` 回归 6 项通过 | 已验收 |
| Core Host 的 call-local child 能收到 Ctrl+B | `call_local_child_observes_and_consumes_the_turn_background_request` 验证父 Turn 信号可见且只消费一次 | 已验收 |
| 声明输出在重建后只结算一次 | `background_bash_persists_and_settles_declared_artifacts_once_after_reconstruction` | 已验收 |
| Store 后台任务回归 | `store::background::tests` | 已验收 |
| CLI 与 agent-host 可编译 | `cargo check -p nola_cli --bins --quiet` | 已验收 |

实现修复还纠正了一个真实协议断点：Core Host 创建 call-local cancellation child 时，过去会丢失父 Turn 的终端能力、活跃 listener 和 Ctrl+B 信号，导致真实 PTY 中前台 Bash 继续走直接进程路径，而局部单元测试无法发现。现在 child 继承父 Turn 的交互通道，共享后台请求，并以原子消费清除信号，避免后续工具被错误转入后台。

尚未据此验收的项目包括：多项目高竞争和 worker 强杀下的公平性、正文/推理断流后的 canonical continuation、SOP 分支/并行/补偿/verifier 闭环，以及千万字符终端性能。它们必须继续按各自门禁提供独立证据。

## 全局公平调度与跨项目唤醒审计（2026-08-27）

本轮进一步审计了 Agent 与 Shell 共用 Supervisor 后的多项目竞争行为。结论需要拆成两个层面：Store 的原子公平认领已经成立，Host 的跨项目基础推进也已通过双 CLI Shell 和混合 Agent/Shell E2E；高竞争、worker 强杀与无法预枚举资源的跨机制矩阵仍未整体完成。

### 已完成：Store 层全局公平认领

过去的 `claim_runtime_task_fair` 只会在调用方传入的项目中选取最高优先级任务。在全局并发槽只有一个时，哪个项目的进程先调用 claim，哪个项目就可能先获得资源；项目级公平并不等于全局公平。

当前认领事务已经改为：

```text
BEGIN IMMEDIATE
  -> 为全部 queued task 计算基础优先级与等待老化分数
  -> 每个 project 只保留自己的队首任务
  -> 排除已经用满项目级并发槽的 project
  -> 从全部 eligible project head 中选择唯一 global head
  -> 在同一事务内更新 ownership token、lease 与 running 状态
COMMIT
```

优先级老化避免持续到来的高频短任务永久饿死较早的低优先级任务；项目级 head 和 project limit 则避免单个项目占满全局执行资源。以下 Store E2E 已通过：

- `fair_claim_selects_one_global_project_head_and_ages_waiting_priority`
- `fair_claim_skips_a_project_that_already_uses_its_project_slot`
- `runtime_task_e2e` 中的并发单槽防超卖回归

这些测试证明的是**数据库认领决策的原子性和排序正确性**，并不证明等待项目会被及时唤醒。

### 已完成基础闭环：Host 层跨进程、跨项目队列推进

当前恢复入口仍以调用方项目为边界：

```text
reconcile_supervised_tasks(config)
  -> recover_orphaned_shell_tasks(current project)
  -> sync_shell_runtime_tasks(current project)
  -> recover_orphaned_agent_tasks(current project)
```

任务结束后的 `wake_background_task_notification()` 也是进程内信号。由此会出现一个真实饥饿窗口：

```text
Project A: A1 正在运行，占用唯一 global slot
Project B: B1 进入队列，等待时间早于 A2
Project A: A2 后进入队列
A1 完成
  -> A 进程收到本地 wake
  -> A 尝试认领 A2
  -> Store 正确拒绝，因为 B1 是 global head
  -> B 进程没有收到跨进程 wake
  -> B1 继续停留在 queued，直到 B 再次有输入、轮询或重启
```

这里 Store 没有做错；历史缺口是完成事件没有可靠地推动**拥有 global head 的另一个项目进程**。当前活跃 CLI 使用 durable task event cursor 感知跨进程调度变化，并由 Store 的原子 claim 决定唯一启动者。`two_idle_clis_advance_the_global_fair_queue_without_another_enter` 已证明 A1 -> B1 -> A2 的跨项目 Shell 顺序；`idle_cross_project_agent_follows_shell_through_the_same_global_slot` 已证明项目 A 的 Shell 释放唯一槽后，空闲的项目 B 无需输入即可启动原 Agent task，且每个任务只有一个终态事件。

### 目标协议

全局队列推进必须满足以下合同：

1. 任一 RuntimeTask 释放全局槽后，等待队列无需用户按 Enter 即可继续。
2. Store 是任务顺序、ownership、lease 和终态的唯一事实来源；Host 不维护第二套排队顺序。
3. 启动任务时必须使用任务所属项目保存的执行请求、工作目录、能力目录和权限上下文，禁止用唤醒方项目的 `Config` 代替。
4. Shell、Agent、Skill 与外部执行器使用同一推进协议，不能只为 Bash 增加特殊唤醒。
5. 多个进程同时响应完成事件时，最终仍由 Store 的原子 claim 保证只启动一个实例。
6. 唤醒信号允许合并，但不能丢失“队列可能可运行”这一事实；冷启动也必须能从 Store 重建。
7. `waiting_permission` 若仍持有运行 ownership，不得占用可执行槽；授权后恢复原 task/call identity，而不是重新排队为新任务。

### 推荐实现顺序

首先检查持久化任务请求是否已经足以脱离原 CLI 进程重建 worker：

| 条件 | 实现方式 |
| --- | --- |
| Shell 与 Agent 的执行请求、项目配置引用、权限上下文均已持久化 | 实现全局 durable queue advancer；任一 worker 结束后读取 global head，并按任务自身请求启动对应 worker |
| Agent 仍依赖原进程内 Config、凭证或不可序列化状态 | 在 Store 增加 durable scheduler epoch/event；各活跃 CLI 监听游标变化并只推进自己项目的任务，认领仍由 Store 仲裁 |
| 需要无人值守、跨 CLI 生命周期持续运行 | 由单一 scheduler/daemon 消费 durable event；CLI 仅展示和提交命令，不让每个客户端各自成为调度事实来源 |

优先采用第一种方案，但前提是任务请求真正自包含。若凭证必须留在平台安全存储中，应持久化安全引用而不是复制密钥。不能为了快速唤醒而让 Project A 的进程使用自己的权限和配置启动 Project B 的任务。

### 禁止的修补方式

- 不得恢复为“调用方项目优先”，这会直接破坏全局公平。
- 不得在 Store 拒绝 A2 后允许 A 绕过更早的 B1。
- 不得依赖用户再次输入、打开 `/work` 或重启 CLI 才推进队列。
- 不得用语言关键词判断哪些任务需要长时间运行或应获得更高优先级。
- 不得只增加高频无游标轮询；轮询若作为降级路径，必须围绕 durable event/epoch，有界且可观测。
- 不得由不同执行器分别维护 Agent 队列、Shell 队列和 Skill 队列。

### 必须补充的真实双 CLI E2E

测试必须启动两个真实 CLI 进程，使用不同 `--project`、同一个 `NOLA_DB_PATH` 和同一全局并发预算：

```text
1. CLI A 启动长任务 A1，占用 global slot。
2. CLI B 提交 B1，确认进入 queued。
3. CLI A 再提交 A2，且 A2 的 queued_at 晚于 B1。
4. A1 完成并释放 slot。
5. 不向 CLI B 输入任何字符。
6. B1 必须自动启动，且启动时间早于 A2。
7. B1 完成后 A2 自动启动。
8. 强制终止其中一个 CLI 或 worker，验证 lease/reconciliation 后仍不重复副作用。
9. 检查 SQLite ownership、worker/command PID、事件顺序、输出 Artifact 和孤儿进程。
```

至少再增加一个 Agent/Shell 混合场景，证明全局队列不是按执行器拆分的。现有 PTY harness 会为 `PtyCli` 生命周期持有全局串行锁，因此需要提供仅供该测试使用的多进程模式；这属于测试基础设施调整，不能通过两个顺序 mock 代替真实竞争。

上述基础 E2E 已通过，因此 P1 门禁中的“多任务完成能够唤醒公平队列”对活跃双 CLI 的 Shell、Agent/Shell 混合场景，以及 Goal 激活 Skill 后派生 Shell/Agent 的单槽竞争均标记为**已验收**。Goal 与 Skill 是控制合同，不直接占 Supervisor 槽位；其派生 worker 必须进入同一 durable scheduler。高竞争及任一 Host/worker 强杀下的混合队列扩展矩阵仍为进行中。

## 本轮实施约束：先强化 CLI，不以客户端补丁掩盖协议缺口

本轮 CLI 优化遵循“**不修改 Core 公共合同**”的边界。目的不是否认 Core 的长期职责，而是先把 CLI 自己负责的 Host、Store、进程监督、终端投影和真实 E2E 做扎实。凡是现有 Core/Application 合同已经能够表达的状态，CLI 必须正确消费；凡是现有合同无法表达的状态，本轮只能记录为架构缺口，不得通过关键词路由、字符串去重、私有状态机或 Prompt 约定伪造完成。

### CLI 层可以直接完成

| 领域 | 本轮应完成的工作 | 不得采用的替代方案 |
| --- | --- | --- |
| Host 执行 | 统一工具启动、进程组、取消、超时、输出预算、后台提升和冷启动重建 | 为每种工具另写一套生命周期，或用 shell 命令名称判断风险 |
| Store | 持久保存 RuntimeTask、权限等待、steering、终态通知游标、Artifact 引用和恢复证据 | 只写 Debug 日志，或依赖内存对象在重启后猜状态 |
| 权限 | 按具体 capability、resource、effect class 和任务作用域执行现有授权合同 | 根据用户原始文本的关键词自动放行或拒绝 |
| 能力目录 | 向模型提供准确 schema、成本、持续时间、依赖、结果和恢复说明 | 客户端改写 query，或用中文/英文关键词强制选择 Tool/Skill/Web |
| 终端 UI | 从同一事件投影渲染消息、思考、工具、权限、后台任务和 footer | 用输出字符串去重遮盖重复事件，或让 Debug 模式维护另一套状态 |
| 长会话性能 | 对消息建立轻量索引、窗口化展示、限制瞬时拷贝，并测千万字符和多会话切换 | 截断 canonical transcript，或只测试短 mock 文本 |
| E2E | 启动真实 CLI、SQLite、PTY、子进程和真实模型，注入崩溃与重启 | 仅做 serde、字符串包含、静态 manifest 或固定 ToolCall 测试 |

### 本轮不能在 CLI 私自补造

以下能力如果现有 Core 合同尚未完整表达，应进入后续 Core 设计，而不是在 CLI 新建第二事实来源：

- SOP 的分支决议、fan-out/fan-in、补偿顺序和 verifier 完成门禁。
- Goal、Plan、SkillRun、ToolCall、RuntimeTask、Artifact、Evidence 的权威归属关系。
- canonical message、provisional part、sequence 和 terminal event 的全局一致性。
- 跨执行器 exactly-once 或 needs-reconciliation 的业务判定。
- 跨平台一致的权限语义、Capability contract 和恢复动作。

CLI 可以展示、持久化适配和执行这些合同，但不能自行发明一套不被其他平台识别的语义。

## 可执行缺口清单

### A. SOP 与复杂工作编排

当前最大问题不是缺少 SOP 文件，而是模型和 Host 仍可能把 SOP 当作一次性说明书。CLI 侧应先保证：

1. 模型每轮只看到当前可执行步骤、稳定输入引用、预算和完成条件，而不是整个历史 SOP 的重复正文。
2. 已完成步骤只以结构化摘要、Artifact 和截断结果进入下一轮，临时工具目录和完整旧输出必须清空。
3. `/work` 能沿 `Goal -> Plan revision -> Step -> SkillRun/ToolCall -> RuntimeTask -> Artifact/Evidence` 展示真实归属。
4. 用户介入、权限等待、后台提升和 CLI 重启后，当前步骤 identity 不变，已提交副作用不重放。
5. Skill 或工具不可用时返回结构化 recoverable state，不让模型从自然语言错误中猜下一步。

验收用例必须包含条件分支、两个可并行步骤、一次权限挂起、一次失败重试、一次用户修改计划和一次进程重启。最终完成只能由 required verifier 与 Evidence 闭合触发。

### B. 能力发现与自主选择

Nola 的能力选择必须依赖模型理解 capability schema，而不是精准匹配用户表达。能力目录需要让模型能比较：

- 这是原子 Tool、领域 Skill、MCP、Web、Memory，还是需要 Supervisor 的长任务。
- 调用是否只读、可逆写入、不可逆或具有外部副作用。
- 预期耗时、依赖、授权范围、输出 schema、Artifact 和恢复方式。
- 失败后是刷新目录、申请权限、安装依赖、换执行器、等待后台结果，还是人工核验。

真实模型回归必须覆盖自然跟进、多语言、模糊表述，以及包含 `search`、`current`、`memory`、`skill` 等字样但实际只是架构讨论的输入。断言对象应是模型提出的具体 tool call 及 Core/Host 对该调用的处理，不能断言关键词命中。

### C. 长任务 Supervisor

当前“可以在后台运行”不等于具备可靠长任务能力。CLI 仍需逐项证明：

| 能力 | 验收指标 |
| --- | --- |
| 启动与隔离 | 每个长期任务拥有稳定 task/run identity 和独立进程组；启动阶段可见且不会每轮重复显示“启动环境” |
| 资源治理 | 记录 wall time、CPU time、峰值 RSS、进程树和输出字节；超限终态能指出具体预算 |
| 取消 | ESC、`/work cancel` 和父任务取消在有界时间内结束全部后代，无孤儿进程 |
| 后台提升 | 前台任务提升后沿用同一进程、identity、日志和 Artifact，不重新执行 |
| 人工介入 | steering 持久化、按 FIFO 在安全边界消费一次，重启后不丢失、不重复 |
| 权限等待 | 任务进入 suspended/waiting 状态，不占用运行槽；授权后从原步骤恢复 |
| 冷启动 | CLI 重启后能区分 running、waiting、retry、verifying、completed 和 needs-reconciliation |
| 公平性 | 多项目、多任务竞争时不饥饿，不因高频短任务永久阻塞长任务 |

学术检索等合理长任务可以配置更长 wall-clock，但不能取消 heartbeat、取消传播、输出预算和进度可见性。时限策略应来自 capability/task contract，不来自用户 query 的学科关键词。

### D. 终端 UI 与用户心智

CLI 的默认心智应是“我正在完成一项工作”，而不是“我正在操作 Goal、Plan、Task、SkillRun 四个内部系统”。因此：

- 输入提交后立即创建 assistant provisional item，并显示稳定、可理解的当前动作。
- 思考、工具参数、中间结果和权限请求进入同一时间线；长任务不能只显示 spinner。
- 已完成正文只能追加和定稿，后续 delta 不得覆盖旧消息或导致只剩最后几行。
- 工具展示名、本地化文本和关键参数分离；不显示内部枚举、`tool completed` 或 raw wrapper。
- footer 在空闲、生成中和 Debug 模式使用相同的 context 格式；用户输入时主动让位。
- 权限 Custom 响应回到正常输入流并真正挂起当前工作；`/allow` 必须提供可理解的授权模式选择。
- resize、窄终端、长命令、宽字符和多行输入不得让光标脱离输入行。

这部分不能只靠 snapshot 字符串测试，必须保存真实 PTY frame，并对关键帧或截图做人工/自动视觉检查。

### E. 性能与大规模会话

CLI 性能基准至少包含以下固定场景：

| 场景 | 必须记录 |
| --- | --- |
| 单个一千万字符会话 | 冷加载、首屏、搜索、追加一条消息、峰值 RSS、退出保存耗时 |
| 三个一千万字符会话切换 | 每次切换 p50/p95、峰值 RSS、是否重复解析全部 Markdown/工具详情 |
| 一万条工具/Skill 事件 | `/work` 首屏、展开详情、过滤、滚动与冷启动恢复耗时 |
| 多后台任务 | 4/16/64 个任务时调度延迟、CPU、RSS、通知吞吐和取消耗时 |
| 上下文压缩 | 压缩前后 token、模型摘要耗时、保留 Artifact/Evidence 完整性和恢复结果 |

canonical transcript 必须完整保存，但 UI 不应同时物化全部正文。性能优化应采用索引、分页、窗口化、惰性解析和有界缓存，不得修改 Core 语义或丢弃恢复所需记录。

### F. 本轮优先顺序

1. 修复 terminal projection 与 PTY：消息不覆盖、输入光标稳定、footer 一致、ESC 可取消。
2. 闭合 RuntimeTask 冷启动、权限挂起、steering 和后台提升的真实 Host E2E。
3. 统一 CLI capability catalog 的成本、结果、Artifact、权限和恢复描述，补真实模型选择回归。
4. 完成千万字符、多会话切换和多后台任务性能基准，并保存基线结果。
5. 在上述基础稳定后，再推进 SOP 分支、并行、补偿和 verifier 的 Core 执行闭环；不得由 CLI 私有状态替代。

本轮任何一项只有同时具备实现、真实 Host E2E、故障恢复证据和用户可见行为验证，才能从“进行中”改为“已验收”。

## 当前对标结论：Nola CLI 真正不足的地方

本节是面向后续实现的结论摘要。它不以“是否存在某个类型、命令或页面”为判断依据，而以用户交给 Nola 一项复杂工作后，系统能否自主规划、稳定执行、接受介入、跨进程恢复并以证据完成为标准。

### 1. SOP 目前更像数据结构，还不是可靠的执行语言

Nola 已经定义步骤、依赖、条件、预算、重试、补偿和 verifier，但定义完整不等于执行成熟。当前主要缺口是：

#### 当前实现边界

截至 2026-08-27，Core 中已经存在 `SopDefinition`、`SopStep`、`SopRunRecord` 和 `SopStepRunRecord` 等结构化类型，CLI Host 也会把适用的 SOP 定义序列化后注入模型上下文。但是，现有运行链仍存在一个关键断点：

```text
SOP definition
  -> serialized host guidance
  -> model creates or updates CLI Plan
  -> AgentPlanState / ConversationPlan persistence
```

该断点已完成第二阶段修复：`ConversationPlan` 保留版本化规划身份与 revision 历史，`SopRunRecord` 则成为独立的 canonical 执行状态机。条件分支、fan-out/fan-in、RetryPolicy、补偿和 verifier 由 Core 类型化事件推进，Application 固定定义版本并通过 CAS 持久化。Plan 与 Run 各自只保存稳定引用，不以提示词或显示文本同步状态。真实 CLI 重启会注入当前/可运行步骤的有界投影；联合权限、replan 与 worker 强杀矩阵仍未全部验收。

下一步必须以现有 `ConversationPlan` 为权威计划状态扩展运行字段，而不是新增 CLI 私有 JSON 或依赖 Prompt 回忆：

| 层级 | 必须持久化的 identity 与状态 |
| --- | --- |
| Plan | `run_id`、`sop_id`、固定的 `sop_version`、当前 revision、冻结/重规划状态 |
| Step | 稳定 `step_id`、依赖、状态、`attempt`、`executor_ref`、`resume_key` |
| Output | 实际 `artifact_refs`、`evidence_refs`、验证状态和失败原因 |
| Effect | `tool_call_id`、`work_step_transaction_id`、幂等键和 reconciliation 状态 |

这里的 Artifact 必须是持久引用，不能只是模型总结中的路径文本；resume key 必须指向已经落库、能够在冷启动后验证的恢复点，不能只是“从第 N 步继续”的自然语言。`WorkStepTransaction` 继续负责真实副作用的 prepared/observed/committed 状态，Plan/SOP 只保存对该事务及产物的引用，避免复制结果正文形成第二事实来源。

| 缺口 | 当前风险 | 目标合同 |
| --- | --- | --- |
| 分支与并行 | 条件判断、fan-out/fan-in 主要依赖模型临场维持，恢复后容易丢失已选分支或重复执行 | Core 持久保存分支决议、依赖满足集合和 join 状态；Host 只执行已就绪步骤 |
| 步骤输入输出 | 上一步输出经常以自然语言重新塞回模型，缺少稳定引用 | 每一步输出均形成带 schema、内容引用和 provenance 的 Artifact；下游按 Artifact ID 消费 |
| 重试语义 | “再试一次”没有严格区分模型重算、只读调用重试和副作用补偿 | RetryPolicy 明确 attempt identity、幂等键、可重试错误和补偿动作 |
| 计划升级 | Skill/SOP 更新后，旧运行实例缺少版本迁移或冻结规则 | 运行实例固定 SOP 版本；升级必须显式 replan 并生成结构化 diff |
| 完成判定 | 执行器成功或模型总结仍可能被误当成任务完成 | required verifier 全部通过且 Evidence 链闭合后，Core 才允许完成 |

该缺口的最小验收不是 serde 或 Store 单元测试，而是一条真实重启链：创建带 SOP identity 的 Plan，完成一个产生 Artifact 的步骤，在下一步骤执行前退出 CLI，重启同一会话，验证 SOP 版本、阶段 attempt、Artifact、resume key 和关联 Work Step Transaction 均保持一致；随后只执行下一就绪步骤，不能重复已提交副作用。用户 replan 时必须产生新 revision 和结构化 diff，旧运行记录保留审计但不得被静默覆盖。

对标成熟 Agent CLI，关键优势并不是提供更多预设流程，而是把每一步的输入、输出、状态转换和恢复点都变成可验证协议。Nola 后续不应增加更多仅靠 Prompt 描述的 SOP；应先让现有 SOP 在崩溃、权限等待、用户修改计划和部分失败后仍能确定性恢复。

### 2. 能力很多，但模型看到的“可行动空间”仍不够清晰

Nola 同时拥有 Tool、Skill、MCP、Memory、Web、后台任务和外部 Agent，但能力目录的边界、代价和结果合同仍不完全统一。模型容易在以下位置做出低质量选择：

- Tool 与 Skill 能解决同一问题时，Schema 没有充分表达粒度、成本、预期产物和适用边界。
- 长任务能力与普通工具并列出现，模型可能把“需要持续监督的工作”当作一次工具调用。
- 文件、网络、浏览器和执行器返回格式不统一，模型需要额外回合解释结果。
- Capability 不可用、依赖缺失或权限不足后，恢复动作有时仍靠模型从错误文本猜测。
- 上一轮工具目录和临时结果可能污染下一轮；下一轮真正需要的是调用记录、截断结果和 Artifact 引用，而不是完整旧工具状态。

目标不是通过用户输入关键词预选能力。正确机制是：

```text
User intent
  -> model reads concrete capability schemas
  -> model proposes a concrete call
  -> Core validates capability, resource, permission and work state
  -> Host executes one authorized effect
  -> normalized Result + Artifact + Evidence return to Core
  -> next turn receives compact execution history, not stale tool state
```

Capability Schema 至少需要统一表达：

| 字段 | 用途 |
| --- | --- |
| `effect_class` | 只读、可逆写入、不可逆写入、外部副作用 |
| `expected_duration` | 即时、交互式、后台候选、长期运行 |
| `resource_scope` | 文件、目录、网络域、设备、进程或远端服务 |
| `result_schema` | 模型可直接消费的结构化结果，而不是展示文本 |
| `artifact_contract` | 会产生、修改或验证哪些 Artifact |
| `resume_contract` | 是否可续传、幂等重试或必须人工核验 |
| `permission_contract` | 推荐权限模式、授权范围和授权有效期 |

### 3. 长任务最弱的不是“能放后台”，而是缺少操作系统级监督

成熟实现把长任务视为独立、可观察、受资源约束的工作单元。Nola 已有持久 RuntimeTask、lease、heartbeat、wall-clock 和输出预算，但仍不能只因任务出现在 `/work` 中就认为长任务能力成熟。

尚需闭合的监督能力：

1. **进程隔离**：每个需要 CPU、内存或进程数量预算的任务必须运行在独立进程组或等价隔离域，不能用整个 CLI 进程的资源占用冒充单任务指标。
2. **资源采样**：持久保存 CPU 时间、峰值 RSS、进程树峰值、输出量和墙钟用量，并能解释由哪个预算终止。
3. **取消传播**：ESC、`/work cancel`、父 Goal 取消和进程退出必须传递到全部后代进程；结束后不得残留孤儿。
4. **公平性**：调度不仅要遵循优先级和 FIFO，还要验证多个项目、多个长任务竞争时不会饥饿或超卖资源槽。
5. **人工介入**：用户输入不是旁路聊天消息，而是持久 steering command；只能在安全边界消费一次，并留下确认游标。
6. **提供方故障**：网络断开、限流、半截流、provider 重启和模型切换不能制造重复正文、重复 ToolCall 或丢失已完成步骤。
7. **恢复报告**：冷启动后应明确展示继续、等待权限、等待用户、资源失败、需核验副作用或已完成，而不是笼统显示“任务中断”。

长任务的最小状态机应由 Core/Application 持有：

```text
queued
  -> claimed
  -> running
  -> waiting_permission | waiting_user | suspended | retry_scheduled
  -> running
  -> verifying
  -> completed | failed | cancelled | needs_reconciliation
```

Host 可以杀进程、采样资源和续租，但不能自行把任务标记为业务完成。

### 4. Goal、Plan、SkillRun、RuntimeTask 仍需收敛成一条工作链

当前对象各有用途，但用户心智不应是同时管理四套生命周期：

- Goal 表达“为什么做”和最终完成条件。
- Plan 表达当前版本的步骤、依赖与预期证据。
- SkillRun 表达某个领域执行合同及其 Artifact/verifier。
- RuntimeTask 表达 Host 中实际占用资源的执行实例。

它们必须通过稳定 ID 串联，而不是通过标题、显示文本或最近一条消息关联：

```text
Goal ID
  -> Plan revision
    -> Work Step ID
      -> SkillRun ID / ToolCall ID
        -> RuntimeTask ID
          -> Effect ID
          -> Artifact ID
          -> Evidence ID
```

任何恢复、UI 展开、Debug 日志和完成判定都应沿这条链查询。缺少任一引用时，不应由客户端猜测归属。

### 5. 终端 UI 还没有成为可靠的工作控制面

参考实现的终端体验之所以稳定，不只是渲染更好，而是 UI 消费单一事件流且区分临时态与持久态。Nola 的目标应为：

- 正文、思考、工具调用、权限、后台任务和状态栏由同一 canonical projection 驱动。
- 流式区域只更新当前 provisional Part；已完成消息不可被后续 delta 覆盖。
- 工具进行中显示用户可理解的动作和关键参数，完成后保留可展开详情，不输出内部枚举名或冗余 `tool completed`。
- 输入期间 footer 隐藏或稳定让位，不能与光标、权限面板和长输出争夺终端行。
- 普通模式与 Debug 模式共享相同状态语义；Debug 只增加 identity、sequence、耗时和资源证据。
- resize、超长输出、多个后台任务、权限挂起和 ESC 均须使用真实 PTY 验证。

### 6. 当前测试体系仍容易产生“功能存在但产品不可用”的误判

过去遗漏问题的共同原因不是测试数量不足，而是测试层级与真实故障不匹配：

| 只做这种测试 | 无法证明 |
| --- | --- |
| Schema/serde round-trip | Host 真正执行、恢复和 UI 正确 |
| mock 模型固定 ToolCall | 真实模型能从 Schema 正确选能力 |
| 单进程单任务 | 冷启动、lease、孤儿与重复副作用 |
| Store 单元测试 | CLI 命令、Core、Host、SQLite 是否形成一条链 |
| 输出字符串包含断言 | 光标、闪烁、覆盖、resize 和面板布局 |
| 静态 Skill manifest 测试 | Skill 真能产出可打开、格式正确的文档 |

必须增加以下分层验收：

1. **合同测试**：非法状态、重复 sequence、陈旧 revision、越权资源和未满足 verifier 必须被拒绝。
2. **真实 Host E2E**：启动 CLI 二进制、真实 SQLite、真实子进程、真实权限挂起和重启。
3. **真实模型 E2E**：使用自然、多语言、模糊和跟进式输入，验证模型依 Schema 选择能力；禁止关键词夹具替代。
4. **故障注入**：分别在 effect 前、effect 后、结果持久化前、stream 中和 terminal event 后杀进程。
5. **终端视觉测试**：记录 PTY frame 或截图，检查光标位置、已完成正文稳定性、面板宽度、长输出和 resize。
6. **产物验证**：Word、Excel、PPT、代码和数据分析结果必须实际打开、解析或渲染，并验证格式与内容。
7. **性能测试**：覆盖千万字符会话、多长会话切换、多后台任务竞争，保存 p50/p95、峰值内存和恢复耗时趋势。

### 7. 与参考实现相比应借鉴什么，不应照搬什么

| 参考方向 | 应借鉴 | 不应照搬 |
| --- | --- | --- |
| Codex CLI | 统一 turn/item 事件模型、稳定的流式终端更新、明确审批边界、可中断执行 | 将 Nola 退化为仅面向仓库操作的编码 Agent |
| Claude Code | 长任务中持续展示行动、权限交互简洁、计划与执行自然衔接 | 依赖专有后端隐式状态或把恢复语义留给模型猜测 |
| OpenCode | provider/会话/工具抽象清晰、前后端事件协议解耦、可扩展能力目录 | 为兼容多个 provider 而弱化 Core 的状态与副作用合同 |

Nola 的差异化方向应继续是：数据源驱动、长期记忆、自我总结和跨平台工作。但这些能力只有进入同一 Work、Artifact、Evidence 和权限链路后才构成优势；单独增加数据源、Skill 或页面只会扩大状态分裂。

### 8. 修复顺序与阶段门禁

| 阶段 | 目标 | 进入下一阶段前必须通过 |
| --- | --- | --- |
| P0 | 统一 identity、事件和恢复 | 重启不丢消息、不重复正文、不重复副作用；权限等待可恢复 |
| P1 | 完成独立长任务 Supervisor | CPU/RSS/进程树/墙钟/输出预算、取消和孤儿回收真实 PTY 通过 |
| P2 | 让 SOP 成为执行协议 | 分支、并行、重试、补偿、replan、版本固定和 verifier E2E 通过 |
| P3 | 收敛能力目录 | Tool/Skill/MCP/Memory/Web 使用统一结果、Artifact、权限和恢复合同 |
| P4 | 打磨终端控制面 | 普通/Debug/权限/长输出/resize/ESC 的截图或 frame 回归通过 |
| P5 | 证明差异化闭环 | 数据源 -> 定时任务 -> Skill -> Artifact -> 记忆 -> 后续会话复用真实 E2E 通过 |

禁止跨阶段用 UI 补偿底层缺口。例如，在事件重复未解决前不得用字符串去重隐藏重复正文；在任务没有独立资源隔离前不得仅增加“CPU 使用率”展示；在 Artifact 合同未闭合前不得用模型自述代替产物验证。

### 9. 本轮审计的完成判据

本审计文档本身只定义问题和验收标准，不代表相关能力已经完成。只有对应实现满足以下全部条件，台账才可标记为完成：

- 有明确 Core/Application 类型化合同，而非 Prompt 约定。
- 真实 CLI Host 执行链通过，不是单元测试或 mock service 自循环。
- 至少覆盖一次进程退出后的恢复。
- 涉及副作用时证明没有盲目重放，并保留人工核验路径。
- 涉及模型决策时包含真实模型、多语言和自然跟进输入。
- 涉及终端交互时包含 PTY frame 或截图检查。
- 涉及产物时完成打开、解析或渲染验证。
- 性能与资源结果可量化并持久记录，不能只描述“感觉流畅”。

近期工程决策：

| 决策 | 说明 |
| --- | --- |
| Core 是工作合同权威 | Goal、Plan、Work、SkillRun、ToolCall、权限等待、恢复动作和完成门禁由 Core/Application 定义；Host 不推断状态。 |
| Host 是副作用执行边界 | Host 负责进程、I/O、lease、heartbeat、取消、超时和结果采集；任何变更型执行必须进入 Work Step 事务。 |
| `/work` 是统一控制面 | `/goal`、`/plan`、后台任务等命令可以保留为快捷视图，但不得形成独立生命周期。 |
| 长任务按可恢复工作建模 | 是否转后台由预计时长、交互需求和资源预算决定，不把“使用某个工具”直接等同于后台任务。 |
| 模型从真实 Schema 选能力 | 禁止用关键词、语言列表或客户端 Prompt 重写强制选择 Tool、Skill、MCP、Memory 或 Web。 |
| 完成必须由证据支持 | 模型自述、执行者报告和退出码为 0 均不能单独完成 Goal；必须关联 Artifact、Evidence 和 verifier。 |

当前建议优先级：

| 顺序 | 工作 | 达成标准 |
| --- | --- | --- |
| 1 | 扩展 Supervisor 的第三方不可观察故障覆盖 | 权限等待、定时重试、不确定副作用和受事务控制的外部执行器已经具备真实 CLI 交互；下一步验证公共 Provider 与第三方内部不可枚举副作用只进入 reconciliation，不盲目重放。 |
| 2 | 完成跨平台 Supervisor 合同 | CLI 的多任务公平调度、资源上限、取消传播、孤儿回收和类型化恢复报告已有真实 Host 证据；继续补 Linux/Windows 资源门禁与公共 Provider 联合故障。 |
| 3 | 完成 SOP/SkillRun 执行闭环 | 条件、并行、重试、补偿、replan、版本升级和冷启动恢复通过真实 Host E2E。 |
| 4 | 统一可观察性和终端交互 | 普通模式、Debug、日志与 `/work` 来自同一事件投影；长输出、resize、ESC 和权限挂起稳定。 |

本文后续章节分别保存实施台账、原始差距证据、目标架构和 E2E 验收矩阵。判断能力是否完成时，以第 0 节的验收状态和第 10 节的联合测试结果为准，不能仅依据类型、命令或单元测试已经存在。

## 0. 实施台账

本节记录审计后的实际改造状态。后续章节保留原始差距分析，用于解释设计动机；判断当前能力时以本节和第 10 节验收矩阵为准。

### 0.1 已实现并通过定向测试

| 能力 | 当前实现 | 主要位置 | 验收证据 |
| --- | --- | --- | --- |
| 统一工作投影 | Goal、SOP Run、RuntimeTask、后台进程、外部执行器和 SkillRun 映射为统一 Work；CLI 提供 `/work` 列表、详情与控制。SOP 按规范化工作区可见，重启和切换会话不丢失，`pause/resume/stop` 回写 Core 状态机 | `cli/src/work.rs`、`cli/src/main.rs` | `durable_sop_run_crosses_the_real_host_and_resumes_after_cli_restart` 在两个真实 CLI 进程中验证 `/work` 可见性、同一 run identity 和继续执行 |
| 结构化 SOP | Core 拥有带版本的定义和可执行 Run 状态机；条件决议、attempt、延迟重试、并行 fan-out/fan-in、步骤/全局 verifier、补偿、暂停/恢复/取消均由类型化事件驱动。Application 固定定义版本并以 revision CAS 写入 `sop_runs`；Host 只提交事实。每个新回合只注入非终态 Run 的当前/可运行步骤、预算、完成条件与 Artifact/Evidence 引用，不重复完整历史 | `core/src/agent/sop.rs`、`core/nola-application/src/sop_runtime.rs`、`core/nola-store/src/schema.sql`、`cli/src/store/sop.rs`、`cli/src/tools/mod.rs`、`cli/src/work.rs`、`cli/src/core_agent_runtime.rs` | `executable_run_handles_conditions_parallel_fan_in_retry_and_verification`、`terminal_failure_requires_declared_compensation_to_finish`、`persisted_run_uses_revision_cas_and_recovers_the_pinned_definition`、`durable_sop_run_crosses_the_real_host_and_resumes_after_cli_restart` |
| SkillRun 合同 | Skill 支持 inline/fork、独立预算、required capability 解析、进度、Artifact、verifier、resume point 和完成门禁 | `core/src/agent/capability.rs`、`cli/src/tools/mod.rs` | `capability_skill_contract`；24 个内置 Skill manifest 合同测试 |
| Skill 失败门禁 | 缺失依赖进入可恢复 catalog refresh；未知 Skill 明确失败；部分产物或失败的 required verifier 均不得完成，失败 evidence 保留 | `core/tests/capability_skill_contract.rs`、`cli/src/tools/tests.rs` | `unknown_tool_waits_for_catalog_refresh_and_resumes`、`skill_context_rejects_unknown_exact_name_instead_of_silent_empty_success`、`failed_required_verification_preserves_evidence_and_blocks_skill_completion` |
| 前后台转换 | `Auto` 可提升为持久后台任务；前台任务可由用户请求转入后台且不重启进程、不改变任务 identity | `cli/src/interaction.rs`、`cli/src/tools/mod.rs` | `auto_mode_promotes_one_process_to_a_durable_background_task`、`foreground_host_request_transfers_without_restarting_the_process` |
| 持久 steering | 用户运行时输入绑定 session，在 Core 安全边界通过 durable command FIFO 领取、确认和重放；真实终端 `Enter` 先持久化再进入进程内队列。模型流无法从 provider 字节游标恢复时，Host 显式提交 `ModelStreamRestarted`，Core 丢弃旧 provisional Part 后在同一 effect 上接受替换流，steering 只在下一个 Core 边界消费一次 | `core/nola-host-protocol/src/lib.rs`、`core/src/agent/runtime/submission.rs`、`cli/src/interaction.rs`、`cli/src/core_agent_runtime.rs`、`cli/src/agent_host_driver.rs` | `submitted_runtime_input_is_durable_before_the_next_core_boundary`；`restarted_model_stream_discards_provisional_text_and_accepts_replacement_part`；`steering_survives_cli_process_loss_and_is_consumed_once_at_the_next_core_boundary` |
| Supervisor reconciliation | Host 启动/查询时协调 shell 与 Agent 任务；终态通知持久确认，进程重建后不会重复 | `cli/src/task_runtime.rs` | `completion_notification_is_acknowledged_across_process_reconstruction` |
| 结构化 Plan | Plan step 使用稳定 ID、依赖 DAG、审批状态、能力引用、预期 Artifact、成功条件、Evidence、验证状态和终止原因；完成和执行状态有严格门禁 | `core/nola-application/src/conversation_plan.rs`、`cli/src/tools/mod.rs` | `conversation_plan` 测试、`update_agent_todos` 测试 |
| Plan 审批与并发控制 | `/plan approve/reject` 通过 Application typed command 和 revision CAS 改变审批状态；旧 revision 不得覆盖新计划，批准状态冷启动后保留 | `core/nola-application/src/conversation_plan.rs`、`core/nola-store/src/lib.rs`、`cli/src/store/plan.rs` | `conversation_plan_approval_e2e`、CLI plan approval 测试 |
| Plan 生命周期闭环 | Plan 支持用户编辑、合同冻结、显式重规划、追加式 revision 历史和结构化 diff；冻结后普通 Save、Edit 和 Approval 均不能静默改变合同，CLI 与模型工具共享同一 Application 协议 | `core/nola-application/src/conversation_plan.rs`、`core/nola-store/src/schema.sql`、`cli/src/store/plan.rs`、`cli/src/main.rs`、`cli/src/tools/mod.rs` | `conversation_plan_lifecycle_e2e`、`conversation_plan_approval_e2e`、`update_agent_todos` 回归测试 |
| Plan 绑定 SOP 运行身份 | Plan 当前记录及每个 revision 持久保存 `run_id / sop_id / sop_version`；普通更新不能换 identity，显式 replan 必须创建新 run，diff 保留前后 identity；CLI Host 生成 run ID，模型不能编造或静默覆盖 | `core/nola-application/src/conversation_plan.rs`、`core/nola-store/src/contracts.rs`、`core/nola-store/src/schema.sql`、`cli/src/tools/mod.rs` | `conversation_plan_lifecycle_e2e`、`conversation_plan_cold_restart_e2e`、`plan_stage_runtime_survives_a_real_cli_process_restart` |
| 工具事务原子关联 Plan stage | Work Step Transaction 进入 committed/failed 终态时，Application 读取稳定 `work_id / step_id`，在同一 SQLite transaction 中提交 Tool 终态、追加 Plan revision、关联 transaction ID，并只把通过 Host 协议校验的 `HostArtifact.uri` 写入 Plan；`result_ref` 仅表示完整工具输出存储位置，不再冒充 Artifact。Plan CAS 或 Artifact 校验失败时 Tool 终态一并回滚 | `core/nola-application/src/work_step_transaction.rs`、`core/nola-store/src/lib.rs`、`core/nola-store/src/schema.sql` | `terminal_effect_atomically_links_transaction_and_artifact_to_the_plan_stage`、`invalid_host_artifact_rejects_terminal_commit_and_rolls_back_the_plan_link`；完整 `work_step_transaction_e2e` 4/4 通过 |
| 内置文件工具结构化 Artifact | `ToolOutcome` 直接携带 Host 协议 Artifact；写入、追加、Notebook、移动、删除、符号体替换、文本/行/原子多编辑、跨文件替换、符号重命名和补丁均在真实副作用成功后声明规范 URI 与实际 operation。多文件工具只声明实际 touched-resource 集合；补丁区分 create/edit/delete；移动同时声明源删除、目标移动及真正成功更新的 importer；无变化和 dry-run 不产生 Artifact。Host 不再从 capability 名称、Bash 参数或输出文本猜测产物，事务重放恢复同一 Artifact | `cli/src/tools/mod.rs`、`cli/src/agent_host_driver.rs` | `real_write_file_artifact_is_atomically_linked_replayed_and_restored_for_the_model`、`append_notebook_move_and_delete_artifacts_follow_real_effects_and_survive_reopen`、`structured_single_and_multi_file_edits_link_only_the_real_touched_resources`、`apply_patch_declares_created_edited_and_deleted_resources_and_replays_once`；CLI Work Step 回归 8/8、Application 事务 4/4、Host 协议 7/7、真实 PTY Plan 冷重启 1/1 通过 |
| 格式化真实资源观测 | `format_files` 在启动 formatter 前为受控工作区内的文本资源建立有界流式指纹和 non-Git preimage，执行后重新观测并只声明实际 create/edit/delete；dry-run、无变化和 formatter 未启动均不产生 Artifact。超过 20,000 个文本资源或 32 MiB preimage 预算时在副作用前拒绝，不把计划目标误当成已修改文件 | `cli/src/tools/mod.rs` | `real_format_files_links_only_changed_artifacts_and_replays_without_formatting_again` 使用真实 `cargo fmt`，覆盖实际内容、Artifact、事务重放与冷启动模型上下文 |
| Checkpoint restore 真实资源观测 | Git restore 使用结构化 `git diff --name-only -z` 枚举候选资源，file-preimage restore 使用受管 manifest；两条路径均在恢复前后比较真实文件状态并区分 create/edit/delete。预览与重复幂等恢复不产生伪 Artifact，事务重放不重复恢复 | `cli/src/tools/mod.rs` | `git_checkpoint_restore_links_exact_create_edit_delete_artifacts_and_replays_once`、`non_git_file_mutation_persists_a_bounded_preimage_and_replays_no_write`、`non_git_preimage_restore_removes_a_file_created_by_the_tool_call` |
| Bash 显式输出与后台结算合同 | Bash schema 提供结构化 `output_artifacts[{path,operation}]`；路径必须位于 `cwd` 权限边界，Host 在执行前保存 preimage、执行后验证 create/edit/delete 后置条件，只有完全匹配的真实资源才进入 Artifact。命令文本、stdout 和参数字符串不参与产物推断；未声明输出保持为空。前台调用立即结算；后台调用将声明、执行前状态和稳定 settlement transaction 一并持久化到 RuntimeTask，完成后由 wait、status、通知或冷重建的首个观察者结算一次。崩溃后从事务结果恢复，不重复命令或 Plan revision | `cli/src/tools/mod.rs`、`cli/src/store/background.rs`、`cli/src/store/mod.rs` | `bash_uses_only_explicit_observed_output_contracts_and_replays_them`、`background_bash_persists_and_settles_declared_artifacts_once_after_reconstruction`；后者清空内存任务表两次，验证 SQLite 重建、真实后台进程、Artifact、Plan 原子关联及 revision 不重复 |
| 自动格式化前置条件修复 | Rust 文件只有在有效 Cargo 工程中才执行 `cargo fmt`；孤立 `.rs` 文件不再发生“文件已经改写、随后 formatter 因缺少 Cargo.toml 失败”的分裂结果。项目 formatter 失败时仍沿用原有原子回滚合同 | `cli/src/tools/mod.rs` | `structured_single_and_multi_file_edits_link_only_the_real_touched_resources` 使用无 Cargo.toml 的真实 Rust 文件通过 |
| 副作用 Work Step 事务合同（部分验收） | Store/Application 持久记录 `prepared -> checkpointed -> effect_observed -> committed/failed`，通过 ToolCall 幂等键拒绝重复执行；内置 Tool、已注册 Runtime Tool、设备桥和委托执行器内部工具事件接入同一审计事务，未完成事务进入 `/work`；冷启动自动提交已观测结果，但不重放 checkpointed 的不确定副作用 | `core/nola-store/src/schema.sql`、`core/nola-application/src/work_step_transaction.rs`、`cli/src/store/work_step_transaction.rs`、`cli/src/tools/mod.rs`、`cli/src/agent_host_driver.rs`、`cli/src/work.rs` | `work_step_transaction_e2e`、`mutating_tool_is_checkpointed_committed_and_not_replayed`、`registered_runtime_route_replays_committed_result_without_executing_twice`、`delegated_tool_events_create_auditable_transaction_and_preserve_uncertain_crashes`、`cold_start_commits_observed_result_but_preserves_uncertain_checkpoint`、`uncertain_side_effect_is_visible_in_the_unified_work_projection` |
| 非 Git 文件 preimage（定向验收） | `write_file`、`append_file`、`delete_file`、`move_file`、结构化编辑工具、`format_files` 及带显式输出的 Bash 在普通目录执行前保存版本化文件清单；单文件上限 16 MiB、单次总量上限 32 MiB，超过预算时在副作用前失败。备份保存在数据库相邻的受管目录并关联 Work Step Transaction；`workspace_checkpoint` 可跨进程缓存重建进行 list、diff、默认 dry-run 预览和显式恢复，恢复拒绝绝对路径、父级跳转、符号链接及受管目录外备份，同一 checkpoint 可重复幂等应用且只在真实变化时返回 Artifact | `cli/src/tools/mod.rs`、`cli/src/store/checkpoint.rs`、`core/nola-application/src/workspace_ownership.rs` | `non_git_file_mutation_persists_a_bounded_preimage_and_replays_no_write`、`non_git_preimage_restore_removes_a_file_created_by_the_tool_call`、`non_git_preimage_budget_failure_prevents_the_file_side_effect`；既有 Git 回归 `workspace_checkpoint_isolates_and_restores_changes_since_checkpoint` 通过 |
| MCP 副作用事务、Artifact 与重放 | 真实 MCP Server 子进程参与执行；变更型 MCP 调用进入统一 Work Step 事务，以稳定 ToolCall 幂等键保存结果。MCP 标准 `resource_link`、embedded `resource` 和显式 `_meta["nola/artifacts"]` 被验证为 HostArtifact 并贯通 ToolCompleted、事务、Plan 与冷启动模型上下文；文本块中的 URI 不会被猜测为产物。已提交结果直接重放；MCP 已执行但 Host 尚未记录结果时保留 `checkpointed` 不确定态，重建后拒绝第三次调用 | `cli/src/mcp_host.rs`、`cli/src/mcp_stdio.rs`、`cli/src/agent_host_driver.rs`、`cli/src/store/work_step_transaction.rs` | `result_artifacts_use_only_explicit_mcp_resource_blocks_and_metadata`；`retained_mcp_process_mutation_is_transactional_and_replayed_without_second_call` 使用真实 retained stdio Server，覆盖标准 resource link、Plan/模型冷恢复、提交重放和 `checkpointed` 崩溃窗口 |
| 设备桥结构化 Artifact 与 Host 调度 | 设备结果只允许在 `_meta["nola/artifacts"]` 中声明经 Host 协议验证的规范资源；正文中的 URI 不会获得 Artifact 权限。CLI runtime dispatch 现在把已注册设备工具交给设备 resolver，而不是落入无 handler 的失败分支；结果在已准备的 Work Step Transaction 内提交并原子关联 Plan，冷启动恢复同一 URI | `cli/src/tools/mod.rs`、`cli/src/web.rs`、`cli/src/core_agent_runtime.rs`、`cli/src/agent_host_driver.rs` | `accepts_only_valid_explicit_metadata_and_never_path_like_text`、`device_bridge_preserves_only_explicit_artifacts_from_a_real_waiting_result`、`device_bridge_artifact_crosses_host_transaction_plan_and_cold_context`；覆盖真实等待状态、Host Catalog 调用、事务、Plan 和冷启动模型上下文 |
| 委托执行器内部工具审计与 Artifact（Nola 边界已验收） | 外部执行器报告的 ToolStarted/ToolCompleted 使用 backend、backend session 与外部 call ID 形成稳定事务 identity；恢复进程即使只收到 ToolFinished，也从 Store 恢复 checkpointed 事务并结算。Artifact 只来自显式 `_meta["nola/artifacts"]` 或 Codex App Server 的结构化 `fileChange.changes` 协议，且须通过规范 URI 与 workspace 边界校验；输出正文不参与推断。Claude stream-json、Codex JSON-RPC app-server 和 OpenCode ACP 子进程均在实际写入一次后异常退出，随后以不同父 effect 恢复同一外部 session，副作用计数保持 1；已提交事务重放同一 Artifact，Claude 真实崩溃恢复 E2E 同时验证 Plan 与冷启动上下文 | `cli/src/agent_host_driver.rs`、`cli/src/external_executor.rs` | `delegated_tool_events_create_auditable_transaction_and_preserve_uncertain_crashes`、`real_external_process_crash_resumes_one_session_without_repeating_its_side_effect`、`real_codex_app_server_crash_resumes_one_thread_without_repeating_its_side_effect`、`real_opencode_acp_crash_resumes_one_session_without_repeating_its_side_effect`；本轮 `work_step_transaction_host_tests` 6/6、Codex 协议 7/7、OpenCode 定向回归 5/5 通过 |
| 文档 Skill 最终 Artifact 与 verifier | Word、Excel、PowerPoint 的内置生成器真实创建 OOXML，并由各自 `--validate-only` 路径重新打开包、解析 XML 及校验领域约束。最终文件只通过 `skill_update(action=artifact)` 的显式规范 `file://` URI 注册；Host 验证文件存在、位于活动 workspace 且符合 HostArtifact 协议。`skill_update` 是免用户权限的内部状态写入，但不再错误标记为只读，因此 Artifact 和验证更新进入 Work Step Transaction、原子关联 Plan 并可冷启动恢复。Skill 指令要求最终文件校验后停止文件变更，仅提交声明的 progress、Artifact 和 `completion-audit` evidence | `NolaMac/Resources/NolaBuiltInSkills.bundle/{word,excel,powerpoint}`、`cli/src/tools/mod.rs`、`cli/src/agent_host_driver.rs` | `skill_update_emits_strict_runtime_protocol_payloads`、`office_skills_generate_validate_replay_and_restore_explicit_artifacts`；后者真实生成并二次验证 DOCX/XLSX/PPTX，覆盖规范 URI、Host ToolCompleted、重复投递幂等、Plan 关联与冷启动恢复；Host 事务 8/8、内置 Skill 9/9、CLI bins 编译通过 |
| Goal 终止语义 | Goal 显式保存 completed/scheduled/awaiting_user/failed/cancelled 停止原因；恢复后清除旧停止原因 | `core/src/agent/goal.rs` | `goal::tests` |
| Skill 到 Goal 的证据链 | Skill checkpoint 恢复后完成 verifier，执行报告不能自证 Goal 完成，独立审计引用 verifier evidence 后才允许完成 | `core/tests/skill_goal_lifecycle_contract.rs` | `recovered_skill_output_requires_independent_goal_audit_before_completion` |
| 压缩期间保留 Work | 手动压缩只替换模型上下文，不改变当前 Goal、后台 Task、待处理权限、Skill Artifact 与 verifier evidence | `core/tests/agent_core_host_contract.rs` | `compaction_preserves_live_work_permission_task_and_skill_evidence` |
| 超长会话 Work 性能 | 两个各 1000 万字符、含 Tool/Skill 时间线的会话反复切换时，同时测量会话投影、统一 Work 查询和 durable steering 入队/领取/确认的 p95 | `cli/src/main.rs` | `two_ten_million_character_sessions_restore_and_switch_with_bounded_terminal_views` |
| Host 响应事务结算 | Application 仅在 terminal Host response 到达后确认 effect；progress response 只作为可观察事件持久化，不再提前移除待执行 effect | `core/nola-application/src/lib.rs`、`core/nola-application/tests/turn_transaction_e2e.rs` | `turn_transaction_e2e` 8/8 |
| Goal 暂停、Skill 生命周期与 mailbox 结算 | `goal_complete(needs_user/blocked)` 只结束当前 Goal 执行片段，保留非终态 SkillRun；后台证据贯穿 executor/verifier 多个 turn，仅在 Goal 完成或暂停后确认，后续 turn 不重复注入 | `core/src/agent/runtime/submission.rs`、`cli/src/core_agent_runtime.rs`、`cli/src/main.rs` | `goal_skill_permission_background_restart_and_resume_share_one_work_chain` 完整 PTY E2E |
| 显式 Goal 跨进程发现 | 新进程执行 `/goal resume` 时，如果当前空会话没有 Goal，则按当前 workspace 查找最近的非终态 canonical Goal checkpoint 并切换到其原会话 | `cli/src/session.rs`、`cli/src/main.rs` | 同一 PTY E2E 已验证跨进程发现、恢复和后续暂停；批量恢复快照仍待优化 |
| 恢复前置门禁与人工处置 | `/goal resume` 在任何模型请求前加载权威 ResumeSnapshot，并消费类型化 Supervisor disposition；已观测副作用先提交并重载快照，`checkpointed` 不确定副作用阻止自动续跑，持久权限等待进入结构化权限面板，已调度重试不会重复启动。`/work resolve` 通过 Application typed command 接受 `applied`、`not-applied` 或 `abandon`，强制填写外部核验证据；原事务不得盲目重放 | `cli/src/main.rs`、`cli/src/task_runtime.rs`、`cli/src/work.rs`、`core/nola-application/src/resume_snapshot.rs`、`core/nola-application/src/work_step_transaction.rs` | `goal_resume_blocks_uncertain_side_effect_before_any_model_request`、`goal_resume_restores_pending_permission_before_any_parent_model_request`、`uncertain_side_effect_requires_evidence_and_closes_the_original_identity`；完整 Goal 联合链回归通过 |

### 0.2 当前仍未完成

1. 真实 MCP Server 已覆盖正常提交重放及 `checkpointed` 与 `effect_observed` 之间丢失 Host 结果的故障窗口，恢复时不会盲目发出第三次调用。Claude stream-json、Codex JSON-RPC app-server 和 OpenCode ACP 均已覆盖真实子进程崩溃、同一 backend session 恢复、迟到完成事件和重复完成事件，且外部副作用计数证明未重复执行。该证据只验收 Nola 可观察的事务边界；第三方执行器内部不受 Nola 控制的操作仍只能提供“拒绝不安全重放 + 可审计恢复”，不能宣称绝对 exactly-once。
2. 非 Git workspace 的直接与结构化文件工具、`apply_patch`、跨文件替换、重命名、格式化器及带显式输出合同的前后台 Bash 已生成可恢复 preimage 或真实资源观测并关联 Work/Step/ToolCall。未声明输出的任意命令和第三方执行器内部未通过结构化协议暴露的写入仍不能伪装成已覆盖，必须依靠显式 Artifact 合同、执行器幂等协议或不确定副作用人工处置。
3. 第 10 节跨机制矩阵尚未全部通过真实 CLI Host E2E。`Goal -> Skill -> Permission -> Background -> needs_user -> restart -> /goal resume -> verifier -> needs_user` 与跨进程 steering 均已通过真实两进程 PTY；一次性 typed resume snapshot 已建立 Application 合同，并被 `/goal resume`、Prompt 动态上下文和 Goal 结算共同消费。恢复前置门禁已在模型请求前处理 `AwaitPermission`、`RetryScheduled` 与副作用 reconciliation：持久权限等待不会被孤儿恢复错误改写为 retry，已观测结果自动提交，不确定的 checkpointed effect 阻止自动续跑。跨进程权限卡、typed deny、父 Goal 零提前请求，以及 `applied/not-applied/abandon` 人工处置均已有真实 PTY 或 Application E2E。三种外部执行器传输的进程崩溃、恢复会话和重复内部事件已完成联合验证。委托 Agent 的多任务子矩阵现已覆盖单槽 FIFO、双 worker 重叠运行、预算失败隔离和双任务 Supervisor 强杀恢复；`goal_skill_shell_and_agent_share_one_supervisor_slot` 还覆盖 Goal/Skill lineage 下后台 Bash 与委托 Agent 的单槽竞争。仍缺的是高竞争或 Host/worker 强杀叠加该混合队列，以及无法预枚举文件资源的副作用恢复证据。
4. 单个子 Agent 的输出、墙钟、CPU、RSS 和进程数预算已通过真实 CLI PTY、Agent Host、Core、Application、SQLite 和子会话联合验收：超量输出按 UTF-8 边界截断；悬挂流式子会话在墙钟预算到期后取消；CPU 饱和进程超过 1000 ms 后终止；128 MiB 驻留子进程使进程树超过 100 MiB 后终止；跨进程组的 `bash` 子进程通过 PPID 递归进程树采样被识别并终止。五类任务均结算为资源失败并持久化具体原因，父回合恢复完成，稳定 `spawn_agent` 调用只创建一个持久任务。多长任务混合预算竞争、网络/provider fallback 和外部执行器 identity 的联合故障测试仍不足。
5. 一千万字符会话的 Work 查询和持久 steering 已纳入 CLI 专项测试；仍需在 CI 基准环境长期记录趋势，避免只依赖单次阈值。
6. Application 已提供 `LoadResumeSnapshot` v2：在同一 SQLite savepoint 读视图中返回 `SessionProjection`、active run、会话 RuntimeTask、未读 mailbox、有效授权、待处理权限请求、未结算 WorkStepTransaction 与类型化 Supervisor reconciliation 报告。CLI `/goal resume`、Core Prompt 构建和 Goal 结算复用同一快照；`AcknowledgeResumeSnapshot` 校验单调 checkpoint lineage、task owner 和每个 task 的精确 `updated_at_ms`，既允许同一 Goal 在消费证据后合法推进 checkpoint，又会整批拒绝陈旧 task cursor。

### 0.2.1 Supervisor 公平调度与资源预算进展

本轮已将长任务调度从 Host 的“先查询、再计数、再启动”改为持久 Store 内的原子认领协议。新增的类型化合同包括：

- `RuntimeTaskPriority`：`low / normal / high`。
- `RuntimeTaskResourceBudget`：最大墙钟时长和最大输出字节数。
- `RuntimeTaskScheduling`：优先级、入队时间、预算、已输出字节数和预算耗尽原因。
- `RuntimeTaskCommand::ClaimFair`：在一个 SQLite immediate transaction 内检查全局与项目并发槽，并按“优先级、入队时间、稳定 ID”认领任务。

当前实现具备以下性质：

1. 并发 worker 不能超卖同一个持久并发槽。
2. 同一优先级按 FIFO 调度，高优先级先于低优先级。
3. 失去并发槽的任务保持 queued，不被错误标记为失败。
4. `running` 和 `waiting_permission` 都占用有效 lease；权限等待期间 heartbeat 仍可续租。
5. 持久取消请求会阻止后续 heartbeat，并由实际持有任务的 worker 传播到本地 cancellation token。
6. 输出缓冲按 UTF-8 边界截断，超过预算后请求取消，并将任务结算为资源失败而非普通用户取消。
7. 墙钟 watchdog、输出用量和预算耗尽原因会持久化到任务调度元数据，并通过任务详情投影给 CLI。

为了保持当前数据库 schema 可读，调度合同暂时存储在保留的 `metadata_json._nola_scheduler` 字段中，但只允许通过 Application 的类型化 API 读写；格式错误、负入队时间和零预算会在 Application 边界被拒绝。该选择不是允许 Host 任意写 metadata 的兼容层，而是在正式 schema 版本升级机制完成前避免直接破坏现有数据库。

已通过的定向测试覆盖：类型化合同 round-trip、非法预算拒绝、优先级与 FIFO、公平槽位、并发 worker 不超卖、持久取消阻止 heartbeat，以及 UTF-8 安全的输出预算取消。真实 PTY 进一步覆盖 `subagent_output_budget_is_enforced_and_reported_through_the_real_host`、`subagent_wall_clock_budget_cancels_the_real_child_and_resumes_the_parent`、`subagent_process_budget_terminates_the_real_process_tree_and_resumes_the_parent`、`subagent_cpu_budget_terminates_a_real_cpu_bound_process_tree` 和 `subagent_memory_budget_terminates_a_real_memory_resident_process_tree`。这些测试分别验证超量流式输出、悬挂 SSE、跨进程组子进程、CPU 饱和负载和 128 MiB 驻留负载；均覆盖 CLI 输入、模型 ToolCall、Host 提升、结构化权限、持久子任务、父回合恢复和最终退出，并在 SQLite 中断言稳定调用 identity、具体预算原因及峰值用量。该证据验收单任务五类资源预算切片，不代表多任务混合预算竞争已经完成。

### 0.2.2 真实 PTY 暴露并完成修复的 Host 重放缺陷

资源预算真实 PTY 最初失败，并连续暴露出两个比预算逻辑更底层的协议错误。第一层发生在子 Agent 提升阶段：父任务先收到 `tool_call_progress(sequence=0)`，再收到 `task_promoted(sequence=1)`；Runtime 再次从 durable effect outbox 读取同一未终结 effect 时，旧 outbox 请求没有保存最新 `resume_after_sequence`，Host 因而从 sequence 0 重发进度，Core 正确地以 `non_monotonic_sequence` 拒绝重复事件。

数据库证据显示：

- `host_event_inbox` 已经按顺序且仅一次保存 sequence 0 和 sequence 1。
- 当前 effect 因响应尚未 terminal，正确保持 `pending`。
- `effect_outbox.payload_json.resume_after_sequence` 却仍为空，说明 durable 可执行请求没有随着非终态响应刷新游标。

第一层根因不是 UI 去重、测试夹具或 Host sequence 计数，而是 **Application 接受非终态响应后，只更新了 checkpoint 内的 `last_response_sequence`，没有在同一事务中刷新当前 pending outbox envelope**。修复后，Application 在同一事务中 upsert 最新 envelope，并将 Core 已移除的 pending effect 与可执行 outbox 对账；terminal Turn 则结算全部旧可执行 effect，同时允许新生成的 cleanup/cancel effect 独立入队。

游标修复后，真实 PTY 又暴露第二层错误：已提升的 `spawn_agent` effect 被再次投递时，CLI Host 先执行通用 Work Step 事务恢复。该事务已经保存首次执行的普通工具结果，因此 Host 在 Runtime Task handler 看到请求之前就重放了 `ToolCompleted`。但 Core 在接受 `TaskPromoted` 时已经完成原始 ToolCall，第二个普通完成事件必须被拒绝，错误为 `tool completion does not match an active call`。

第二层修复在 Runtime Tool 执行前增加只读持久任务重放探针。探针仅使用稳定 `call_id` 查询既有 durable task；命中后直接产生 `TaskUpdated`，不再次执行工具，也不把旧的普通事务结果伪装成新的完成事件。首次调用仍经过原有 Work Step 事务和权限边界，因此修复没有放宽 Core 校验，也没有削弱副作用防重。

修复不得采用以下方式：

- 不得放宽 Core 的单调 sequence 校验。
- 不得删除 progress 或把所有 progress 伪装为 terminal。
- 不得只在 CLI 进程内记住 sequence；冷启动后必须仍然正确。
- 不得在 UI 层过滤重复事件来掩盖协议错误。

最终合同为：

```text
Host response(sequence = N, non-terminal)
  -> Core/Application reduce
  -> checkpoint.last_response_sequence = N
  -> same transaction refreshes pending effect outbox envelope
  -> envelope.resume_after_sequence = N
  -> cold Runtime reload
  -> Host next response starts at N + 1
  -> if ToolCall was promoted, resolve stable call_id to durable task first
  -> replay TaskUpdated, never replay the pre-promotion ToolCompleted
  -> terminal response acknowledges and removes pending effect
```

当前验收证据：

1. `nola_application` 的 `turn_transaction_e2e` 9/9 通过，覆盖非终态响应、terminal 结算和中断清理语义。
2. Core `explicit_task_promotion_survives_checkpoint_and_finishes_exactly_once` 通过，证明提升后的 effect 可恢复且只完成一次。
3. 真实 PTY 输出预算与墙钟 watchdog 测试通过；任务最终为 `failed`，分别保存不超过 1024 字节的合法 UTF-8 输出或 1000 ms 墙钟预算，以及对应预算耗尽原因；父回合均得到结果并正常结束。
4. 同一稳定 `spawn_agent` 调用的 durable Agent task 计数严格为 1，证明重投递未产生第二任务。
5. 公平排队、显式取消、权限等待、孤儿恢复以及输出/墙钟/CPU/RSS/进程树限制已有真实 PTY 回归。多任务混合资源竞争、强杀采样器/worker 的故障窗口及所有外部执行器的统一预算仍需补齐，完成前不能把整个 Supervisor 阶段标记为已验收。

### 0.3 架构约束

- 模型从具体 schema 中选择 Tool、Skill、MCP 或 SOP；禁止由客户端根据用户文本关键词强制路由。
- Core 负责状态转换、合同和完成门禁；Host Supervisor 只负责进程、lease、输出、取消、恢复和通知。
- 状态更新使用稳定 ID 和明确引用，不解析 UI 显示文本。
- 执行者报告、模型自述和“命令退出码为 0”都不能单独作为 Goal 完成证据。

### 0.4 目标架构与实施路线

CLI 不应继续把 Goal、Plan、Skill、Tool、后台进程、权限请求和外部执行器作为彼此独立的功能入口维护。目标架构是将它们组织成一条由 Core 持有合同、由 Host 执行副作用、可暂停和可恢复的工作链：

```text
User Intent
  -> Goal Contract
  -> Versioned Plan
  -> Work / Work Step
  -> SkillRun or ToolCall
  -> Permission Decision
  -> Side-effect Transaction
  -> Foreground / Background Execution
  -> Artifact + Evidence
  -> Independent Verification
  -> Goal Terminal Decision
```

各层职责必须保持清晰：

| 层级 | 负责 | 不负责 |
| --- | --- | --- |
| Core | 状态机、合同、稳定 ID、revision CAS、权限判定输入、完成门禁、恢复语义 | 直接执行文件、网络、Shell 或平台 API |
| Application | typed command、事务编排、持久化用例、跨 Store 原子边界 | 根据用户自然语言猜测工具或权限 |
| Host Supervisor | 进程、lease、heartbeat、输出采集、取消、超时、恢复协调 | 自行改写 Plan、Goal 或 Skill 完成状态 |
| Client UI | 呈现 Work 树、进度、权限、Artifact 和恢复动作 | 从显示文本反推运行状态或工具路由 |
| Model | 从实际 Tool/Skill schema 中选择能力、生成计划与参数、根据证据提出下一步 | 绕过 Core 权限与完成门禁、自证执行成功 |

实施顺序如下。每个阶段必须先完成合同和冷启动测试，再接 UI；不得用 UI 状态掩盖底层生命周期缺失。

1. 统一 Plan 生命周期：稳定步骤 ID、版本、冻结、编辑、显式重规划、历史和 diff。
2. 统一副作用事务：每个有副作用的 Work Step 持久记录 `prepared -> checkpointed -> effect_observed -> committed/failed`；重启后不得盲目重复执行。
3. 统一恢复协调：启动时协调 Work、后台进程、权限、SkillRun、ToolCall 和未完成事务，生成可解释的恢复决策。
4. 打通跨机制真实 E2E：Goal -> Skill -> Tool -> Permission -> Background -> Restart -> Resume -> Verify。
5. 增加长任务资源治理：并发公平性、CPU/内存/输出上限、provider/network fallback、超时和取消传播。
6. 收敛 CLI 交互：以 `/work` 为主控制面，其他命令作为特定视图或快捷入口，而不是独立生命周期。

副作用事务边界当前处于“部分验收”：Store 与 Application 已有持久合同，内置 CLI Tool、已注册 Runtime Tool、设备桥和真实 MCP 调用已进入统一边界；外部执行器内部工具调用及其他绕过统一 Tool 执行入口的路径尚未全部纳入，因此不能宣称 exactly-once 已完成。正确语义也不是绝对 exactly-once，而是：执行前持久声明、执行后保存可审计结果、重启时拒绝不安全重放，并由幂等键、checkpoint 或人工恢复解决不确定状态。

### 0.5 完成定义

一项能力只有同时满足以下条件才可从“实现中”改为“已验收”：

- Core/Application 存在类型化合同，非法状态转换会被拒绝。
- 状态持久化后，进程冷启动能够恢复同一 identity、revision 和等待原因。
- CLI Host 真实执行链路已接入，不是仅测试 Store 或 mock service。
- 权限拒绝、用户自定义回复、取消、超时、崩溃和重试均有确定语义。
- Tool/Skill 输出、Artifact、完整结果引用和 verifier evidence 可以追溯到 Work Step。
- 重复事件、重复 delta、重复 ToolCall 或重启重放不会产生重复消息和重复副作用。
- 普通任务、长任务、后台任务和外部执行器在 `/work` 中使用一致状态表达。
- 至少包含一次真实 Host 冷启动 E2E；涉及模型决策的能力还必须包含真实模型回归，不得只用关键词或固定脚本模拟路由。
- 性能测试覆盖超长会话、多工作并行和频繁会话切换，并记录可比较的 p50/p95、内存峰值和恢复耗时。
- 用户可见文本完成本地化，普通模式和 Debug 模式表达一致且不泄漏内部状态名。

### 0.6 真实 PTY 联合链路审计记录

2026-08-26 使用 `goal_skill_permission_background_restart_and_resume_share_one_work_chain` 进行真实 CLI PTY、真实 Agent Host、canonical SQLite 和模拟流式模型服务联合测试。测试不是静态 parser 测试，覆盖以下事件顺序：

```text
/goal
  -> Skill(analyze-data)
  -> bash(execution_mode=background)
  -> permission session grant
  -> durable task identity + mailbox
  -> goal_complete(needs_user)
  -> CLI exit
  -> background process completes
  -> new CLI process
  -> /goal resume
  -> restore Goal + SkillRun + mailbox evidence
  -> model continuation
```

本轮依次发现了四个此前单点测试没有暴露的根因：

1. **progress 提前结算 effect**：Application 在第一条 `ToolCallProgress` 到达时就确认 effect，导致 terminal `ToolCompleted` 失去待处理请求。修复后只有 `HostResponse::is_terminal()` 才确认 effect；Host inbox 继续负责去重和已交付记录。
2. **turn 完成误等同于 Skill 完成**：`goal_complete(needs_user)` 触发 `TurnCompleted` 后，Core 对当前 turn 的每个 SkillRun 自动调用 `Complete`，从而错误触发 `skill_completion_contract_unsatisfied`。现在 `needs_user/blocked` 保留 SkillRun 为 running/resumable，且没有削弱真正完成时的 verifier 门禁。
3. **测试夹具要求重复模型调用**：旧 E2E 在结构化 `goal_complete(needs_user)` 后仍等待第二次模型请求生成 `CHAIN-PAUSED`。该协议会增加延迟并可能制造重复正文。现在工具结果直接形成 Goal 暂停 UI 和 terminal turn，不再为了润色暂停状态重复调用模型。
4. **显式恢复只查询当前空会话**：重启后的 `/goal resume` 原先只查新建的当前会话，尽管原 canonical checkpoint 完整保存了 `GoalStatus::NeedsUser` 和 SkillRun。现在显式恢复按 workspace 查找最近的非终态 Goal checkpoint，再切换到原会话。

当前数据库证据表明持久状态本身完整：Goal 为 `needs_user`，SkillRun 保持 `running`，resume point、执行合同和后台 task identity 均存在。恢复后的模型请求也收到了预期正文。Application Service listener 从固定 `sleep(100ms)` 轮询改为事件驱动 `poll(2)` 后，完整两进程 PTY 已稳定走通。剩余工作是 **恢复热路径结构优化**，不是数据丢失或当前 E2E 阻塞：

- 新进程从 `/goal resume` 到模型请求前串行执行多次 `load_conversation`、`load_checkpoint`、`list_permission_grants`、`runtime_task` 和其他 Application command。
- 原固定轮询会放大每次 Application 往返；事件驱动 listener 已消除该固定等待，但重复查询仍造成不必要的 IPC 和 SQLite 开销。
- 不应通过放宽超时验收。Application 需要提供一次性的 typed resume/read model，原子返回 conversation、checkpoint、permission grants、durable task mailbox 和恢复决策；Host 在一次快照上构建 prompt，避免 N+1 IPC/数据库往返。

下一阶段验收阈值：

| 指标 | 要求 |
| --- | --- |
| `/goal resume` 到恢复快照可用 | p95 小于 500 ms（本地 SQLite、无网络） |
| 恢复快照到模型请求发出 | p95 小于 250 ms |
| 首个模型 delta 到终端可见 | p95 小于 100 ms |
| 同一 permission/task/checkpoint 查询 | 每次 resume 最多一次规范读取，不允许按 Tool catalog 重复查询 |
| 冷启动恢复一致性 | Goal、SkillRun、Permission、Task、mailbox 使用同一 checkpoint/revision 视图 |

### 0.7 动态证据生命周期缺陷

联合链路进一步暴露出一个比恢复耗时更重要的协议问题：后台任务 mailbox、活动 Skill 和其他恢复证据作为 Host 动态上下文注入一个 Core turn，而 CLI Goal 控制器的 executor 与 verifier 实际可能跨越多个 Core turn。旧实现把单个 turn settlement 错当成完整 Goal 执行片段的消费边界；如果模型只输出正文、未给出 `goal_complete`，后续 verifier turn 便看不到同一份恢复证据。

这会产生以下错误行为：

- 同一 turn 内第一次请求知道后台任务已经完成，第二次请求却表现为从未收到结果。
- Skill 名称、resume point 和 Artifact 在重试中丢失，模型只能重新猜测执行位置。
- 为补偿证据丢失而追加提示词或重复查询，会增加首 token 延迟和重复正文风险。
- mailbox 的“已通知”状态实际代表“某次请求已读取”，而不是“当前工作链已可靠消费”，确认语义错误。

该问题现已按以下生命周期修复：

```text
Durable task result / Skill evidence
  -> Application 生成同一 revision 的恢复快照
  -> Core 在每个相关 turn 开始时物化为稳定 ContextComponent
  -> executor、工具续轮和 verifier turn 在同一 Goal execution slice 内复用该证据
  -> Goal execution slice 完成或暂停后统一确认消费
  -> 后续无关 turn 不再重复注入
```

实现约束：

1. Host 不得在第一次模型请求发出后确认 mailbox；确认点必须是 Core turn 的 terminal boundary，或证据已经持久写入 canonical model context 的原子提交点。
2. 同一 turn 内的 continuation、工具结果续轮、格式修复和 Goal verifier 必须使用同一 revision 的恢复快照。
3. 恢复证据必须具有稳定 ID，确认操作幂等；进程在物化后、确认前崩溃时允许再次注入，但不得丢失。
4. 不得通过语言关键词、重复拼接 Prompt 或放宽 PTY 超时掩盖该问题。
5. 回归测试必须覆盖“第一次模型请求未结束 Goal、verifier turn 仍可见 mailbox 与 Skill evidence、Goal 片段结算后下一轮不重复出现”。该合同已由 `goal_skill_permission_background_restart_and_resume_share_one_work_chain` 覆盖。

### 0.8 与参考 CLI 的分层差距及实施优先级

参考项目的优势不在于命令更多，而在于每一层只有一个权威状态来源，并以事件和稳定 identity 连接。Nola 当前应按以下优先级收敛：

| 优先级 | 领域 | 当前主要不足 | 目标状态 | 必须验证的场景 |
| --- | --- | --- | --- | --- |
| P0 | Turn 与消息所有权 | Host 动态上下文、模型 delta、工具结果和 UI 各自维护局部状态，容易遗漏或重复 | Core/Application 持有 canonical turn；Host 只提交有序事件，UI 只渲染投影 | 多次工具续轮、重试、断线重放后正文与工具事件恰好一次 |
| P0 | 长任务恢复 | Goal、Skill、Permission、后台任务分别读取，恢复快照不一致且存在 N+1 往返 | 一次 typed resume command 返回同 revision 的完整恢复快照和决策 | 进程退出、后台完成、冷启动、恢复、验证、再次暂停 |
| P0 | 副作用安全 | 部分外部执行器和非 Git 文件操作缺少 preimage 或不确定状态协议 | 所有变更型调用进入统一 Work Step 事务，未知结果禁止盲目重放 | effect 前后分别崩溃、重复 ToolCall、人工恢复 |
| P1 | SOP 执行 | 结构化 schema 已有，但执行器、UI 和真实 Skill 覆盖仍不完整 | SOP 成为可版本化、可暂停、可重规划、可验证的执行合同 | 条件分支、并行步骤、补偿、版本升级和恢复 |
| P1 | 资源治理 | 多个后台任务可运行，但缺少全局公平性和明确资源预算 | Supervisor 统一管理并发槽、CPU/内存/输出预算、优先级与背压 | 多工作竞争、超量输出、孤儿进程、取消传播 |
| P1 | 权限交互 | 权限模式已存在，但跨 Tool 前缀、custom response 和恢复链路仍易分叉 | Core 对具体 effect 判定；Host 挂起同一调用，用户决定后原地继续 | session grant、custom、拒绝、重启和同资源复用 |
| P1 | 可观察性 | Debug 日志、工具卡片和 `/work` 尚未完全共享事件模型 | 日志、普通 UI、Debug UI 都由同一 Work event projection 派生 | 长任务实时步骤、参数摘要、Artifact、失败原因和完整引用 |
| P2 | 上下文治理 | 压缩、恢复证据、工具结果截断和总窗口显示尚未形成统一预算 | Core 分配 token 预算，保留活跃合同与证据，Host 只提供内容 | 多次压缩后 Goal/Skill/Permission 不丢失，窗口显示一致 |
| P2 | 终端体验 | 输出区域、footer、权限面板和长内容交互仍存在状态竞争 | 单一渲染循环、稳定区域布局、增量更新和可访问交互 | 1000 万字符会话切换、窄终端、resize、ESC、中文输入 |

实施时不得把参考项目的 Prompt、命令名称或 UI 表象直接复制进来。应复用其原则：单一所有权、事件有序、状态可重放、副作用可审计、等待可恢复、完成有证据。每个改造项均需先写失败场景，再修改合同和实现，最后使用真实 CLI Host、冷启动和真实模型完成端到端验收。

### 0.9 本轮审计新增结论

#### 0.9.1 终端 steering 曾存在“测试通过、真实路径未接入”的缺陷

此前单元测试直接调用 `queue_draft_input()`，能够证明该方法会写入 durable command FIFO，但不能证明用户在真实终端按下 `Enter` 时走相同路径。实际键盘处理分支只把文本写入进程内 `state.queued`，进程被强制终止后输入会丢失。

当前修复要求如下：

1. 所有运行时输入必须经过唯一的 `commit_runtime_input` 入口。
2. 有 canonical session 时先写 durable FIFO；只有无法绑定 session 时才允许退化为进程内队列。
3. 写入成功后才能清空编辑区并显示“已排队”。
4. Core 只在下一安全边界领取输入，处理成功后幂等确认。
5. 重启恢复原 session 时只能消费一次；恢复活动 Run 本身不得提前注入下一轮 steering。

这暴露出测试方法上的通用问题：不得只测试内部 helper。涉及终端交互、权限面板、暂停恢复和后台任务时，必须从 PTY 输入、进程强杀、SQLite 冷启动和最终 UI 输出完整验证真实入口。

#### 0.9.2 `SessionProjection` 统一了查询语义，但还不是完整恢复事务

当前 Goal、Task、Memory、Permission 和 Skill 的会话级读取已经统一从 Application `SessionProjection` 派生，`/goal resume` 也可以通过 typed command 按 workspace 找到最近的非终态 Goal。这消除了多套 CLI Store projection 各自解释会话状态的问题。

但完整恢复仍缺以下同 revision 数据：

- RuntimeTask mailbox 及其确认游标；
- 当前 effect 对应的有效 permission grant；
- 未结算 Work Step transaction；
- Supervisor reconciliation 结果；
- 明确的恢复决策，例如继续 effect、等待用户、拒绝不安全重放或进入 verifier。

因此目标不能只是减少查询次数，而应新增 Application 级 typed resume snapshot：在一个一致性边界内返回 session projection、mailbox、权限、未结算事务和恢复决策。Host 只能消费该快照，不得在构造恢复 Prompt 的过程中再次拼接不同时间点的局部查询结果。

### 0.10 可执行整改清单

| 阶段 | 交付物 | 失败条件 | 验收方式 |
| --- | --- | --- | --- |
| P0-1 | Application typed resume snapshot | 任一恢复字段需要 Host 额外查询；revision 不一致 | Application 冷启动 E2E + 两进程 CLI PTY |
| P0-2 | steering 跨进程恰好一次 | 强杀后丢失、注入活动 Run、重复注入后续轮次 | 长流式 Run 中输入、强杀、`/resume`、连续两轮断言 |
| P0-3 | canonical Turn/Message 所有权 | delta、工具结果或重放造成正文覆盖、重复或乱序 | 断线、工具续轮、重复事件和恢复联合 E2E |
| P0-4 | 全副作用事务覆盖 | 外部执行器或 MCP 崩溃后盲目重放 | MCP effect 前后故障注入与调用次数审计已通过；外部执行器真实进程崩溃/恢复仍待联合 E2E |
| P1-1 | Supervisor 公平调度与资源预算 | 单任务饿死其他任务；输出、内存或子进程无上限 | 多 Goal/Skill/后台任务压力测试 |
| P1-2 | 权限挂起统一协议 | custom response 另开流程；session grant 重复弹窗 | allow/custom/deny/restart 的真实 PTY E2E |
| P1-3 | SOP 执行闭环 | SOP 仅存在于 Prompt，步骤状态无法恢复或验证 | 条件、并行、重试、补偿、replan E2E |
| P1-4 | Provider/网络/外部执行器故障恢复 | fallback 改变任务 identity 或丢失上下文 | 超时、401、429、断网、executor 崩溃联合测试 |
| P2-1 | 统一可观察性投影 | 普通 UI、Debug、日志和 `/work` 状态互相矛盾 | 同一事件流的多视图快照测试 |
| P2-2 | 终端性能与稳定布局 | 长回复只剩尾部、footer 竞争、resize 后光标错位 | 千万字符、多会话、窄终端与 resize 基准 |

整改顺序不可交换：先建立恢复和消息所有权，再扩展 SOP 与长任务能力，最后打磨 UI。否则 UI 会继续为不一致的底层状态增加补丁，并制造更多无法重放的隐式状态。

### 0.11 Durable Steering 与 Canonical Message 联合审计

本轮新增了一条真实 PTY 回归链路，用于验证用户在模型仍在输出时提交补充要求、CLI 随后被强制终止、重启并恢复原会话的完整行为。该测试不直接调用内部 helper，而是通过终端按键、两个真实 CLI 进程、Application/Store、模拟流式模型服务和公开的会话恢复命令完成。

```text
long model stream
  -> user presses Enter with steering text
  -> durable command committed before editor is cleared
  -> CLI process is force-killed
  -> new CLI process resumes the canonical session
  -> interrupted Run is recovered first
  -> steering is consumed at the next safe Core boundary
  -> successor Turn runs exactly once
  -> later ordinary Turn must not consume steering again
```

该链路已经证明以下协议修复有效：

1. 真实终端输入通过唯一持久入口提交，不再只进入进程内队列。
2. durable command 的路由使用明确的 canonical session ID，不再错误复用 permission scope。
3. Runtime Driver 在已提交的 Host 响应之后读取安全边界输入，并通过 Application active submission 写回 Core。
4. 只有 committed effect journal 已经排空后才确认 durable FIFO；失败时重新入队。
5. 后继 effect 存在时，Application 不得因为前一 Turn 已经 terminal 就提前关闭 Run。
6. 重启后 steering 对模型只产生一次新的用户意图；后续请求中只能作为正常历史存在，不能再次成为待执行输入。

真实行为 E2E 与 canonical persistence 联合缺陷均已修复。Application 现在依据 Core `InputApplied` 和 Part ownership，在同一 Store transition 中确保后继 user/assistant Message、Part mutation、checkpoint、event cursor、effect acknowledgement 与 Run 状态原子提交。公开 `session show` 冷启动读取严格得到下述 8 条消息，重复事件不会制造第二份 Message。

该缺陷不能在 CLI UI、Host transcript 或恢复 Prompt 中补写消息。正确修复边界必须位于 Application/Store：

- `InputApplied` 形成后继 Turn 时，在同一 Store transaction 内创建稳定 ID 的 canonical user message。
- 后继 Turn 第一次产生 assistant Part 时，在同一 transaction 内创建 canonical assistant message，并建立 `turn_id -> message_id` 的稳定所有权。
- Part delta、消息创建、checkpoint revision、event cursor、effect acknowledgement 和 Run 状态必须原子提交。
- 重复 Host event、重放 transition 或进程崩溃后恢复不得创建第二份消息或重复 Part。
- CLI、macOS、Web 及其他客户端只能渲染 Application 的 canonical projection，不得各自从流式事件重建另一套永久消息历史。

目标持久化结果必须严格保持以下顺序：

```text
1. original user message
2. recovered assistant response
3. steering user message
4. steering assistant response
5. explicit continuation user message
6. explicit continuation assistant response
7. later ordinary user message
8. later ordinary assistant response
```

验收不得只检查终端字符串。必须同时断言：

| 层级 | 必须满足的条件 |
| --- | --- |
| Core | steering 在命名安全边界提升为新 Turn，且只提升一次 |
| Application | Run 在后继 effect 完成前保持 active，terminal response 后正确结算 |
| Store | 冷启动读取到完整、有序、无重复的 8 条 canonical message |
| CLI | `/resume` 后可见完整历史，后续输入不覆盖旧回复 |
| 模型请求 | steering 首次作为新意图出现一次，后续仅作为已提交历史出现 |
| Durable FIFO | 成功结算后为空；失败或崩溃时仍可恢复，不提前确认 |

因此，第 0.10 节 P0-2 已在运行行为和 canonical persistence 两个维度完成验收；P0-3 的“Core 自动提升后继 Turn 的 Message ownership”切片也已验收，但更广泛的工具重试、远程 Host 重放与断线矩阵仍未全部完成。

### 0.12 一次性恢复快照的目标合同

当前 `SessionProjection` 已经收敛会话、Goal、Skill 和部分任务查询，但 Host 恢复时仍需分别读取 mailbox、权限授权、未结算副作用事务和 Supervisor reconciliation 结果。这会同时造成性能和一致性问题：多个查询可能观察到不同 revision，恢复 Prompt 也可能混合新旧状态。

Application 应提供单个 typed resume command，原子返回：

```text
ResumeSnapshot
  session_projection
  canonical_checkpoint_revision
  active_run_and_turn
  goal_and_skill_runs
  runtime_task_mailbox_and_cursor
  effective_permission_grants
  unsettled_work_step_transactions
  supervisor_reconciliation
  recovery_decision
```

`recovery_decision` 必须是类型化结果，而非 Host 根据字符串拼装：

- `continue_effect`
- `wait_for_user`
- `resume_model_turn`
- `verify_artifact`
- `manual_recovery_required`
- `refuse_unsafe_replay`

Host 消费快照后不得为了构建同一次恢复请求再次读取上述局部状态。快照使用一个 revision 和确认游标；确认操作必须引用该 revision，陈旧确认应被拒绝。这样才能同时解决恢复 N+1 往返、权限重复申请、mailbox 提前确认、后台证据丢失和不确定副作用误重放问题。

当前实施状态（2026-08-27）：

- `ApplicationCommand::LoadResumeSnapshot` 已实现，并通过真实 SQLite 冷启动测试。
- Store 使用 savepoint 固定一个 WAL 读视图；快照不会跨多次 Application IPC 拼接不同时间点的数据。
- 已返回 `SessionProjection`、全部会话 RuntimeTask、未读终态 mailbox、有效授权、pending permission request、workspace 内未结算 WorkStepTransaction 和 `ResumeAction`。
- `/goal resume` 已直接用快照选择 canonical 会话和 Goal，旧的 CLI session-ID 查找 helper 已删除。
- 快照被保留到完整 Goal execution slice；executor、工具续轮和 verifier 从同一 mailbox 构建上下文，结算使用快照内的 task IDs 幂等确认，不再重新扫描任务表。
- 快照包含 active run 和 `mailbox_cursor(task_id, updated_at_ms)`；确认通过 Application/Store 原子执行。checkpoint 作为单调 lineage 下界，task owner 与精确 `updated_at_ms` 作为证据 CAS：Goal 合法推进 checkpoint 不会误拒绝确认，快照后 task 自身发生变化仍会让旧确认整批失败。
- Host 在加载快照前先把外部 shell/process 状态协调为 RuntimeTask；真实 PTY 额外断言恢复结束后的 shell mailbox 已被 canonical acknowledge，而不是由旧 metadata 标志伪装为已消费。
- `ResumeSnapshot` v2 已携带类型化 Supervisor 报告。任务被归类为 `Monitor`、`AwaitPermission`、`RetryScheduled`、`DeliverEvidence`、`Settled` 或 `RequiresIntervention`；副作用被归类为 `ReadyToExecute`、`CommitObservedResult` 或 `RequiresIntervention`。
- 冷启动协调使用 workspace-wide 查询，因为 `/goal resume` 在发现 canonical conversation 前仍处于新建空会话；普通 `/work` 和会话 UI 继续严格按 owner session 隔离。
- 真实 PTY 暴露并修复了两类 helper 测试看不见的协议错误：事后重查 pending task 会把本轮从未注入的刚完成任务提前标成已读；checkpoint revision 完全相等会拒绝 Goal 消费证据后的合法状态推进。当前确认只针对本轮真实注入的 task IDs 或已持有的 resume snapshot。
- 联合 PTY 已通过：权限只请求一次，Skill、后台 shell、进程退出、跨进程 `/goal resume`、mailbox、`/work` 和后续不重复注入处于同一真实工作链。该用例耗时约 35 秒；日志显示恢复 Goal 结算仍有多次约 0.5 秒级 Application 提交，功能正确不代表恢复性能已经达标。
- `ResumeAction`/Supervisor disposition 的 CLI 恢复交互已经闭环：`goal_resume_restores_pending_permission_before_any_parent_model_request` 覆盖权限先恢复后续跑；`goal_resume_blocks_uncertain_side_effect_before_any_model_request` 覆盖不确定副作用停止自动恢复并要求 `/work resolve` 提交人工证据；`goal_resume_reports_scheduled_retry_without_starting_it_twice` 覆盖未来重试可见、父 Goal 可继续且任务身份、重试次数和截止时间均不变。受 Nola 事务边界控制的真实外部执行器崩溃恢复证据见 11.5。仍未完成的是恢复结算写放大优化、公共 Provider 非确定性联合故障，以及第三方内部不可枚举副作用的自动验证；这些边界继续拒绝盲目重放，而不是回退到 Host 猜测。

## 1. 结论

Nola CLI 的主要问题不是工具数量不足，而是已有能力没有组成统一、可靠、可恢复的工作系统。

当前已经具备 canonical conversation、稳定消息与 Part、Goal checkpoint、权限挂起与恢复、RuntimeTask、后台 Bash、Skill、MCP、外部执行器和 workspace checkpoint 等基础组件。但这些组件在 CLI 产品层仍由多套入口、状态模型和恢复路径分别管理：

```text
/goal       -> GoalProjection 与独立执行/审计循环
/agent      -> 子 Agent 与 RuntimeTask
/tasks      -> RuntimeTask 查询与控制
/jobs       -> 后台 Bash 进程
/resume     -> 会话恢复
/plan       -> 会话计划摘要
/compact    -> 上下文压缩
permission  -> pending effect 恢复
executor    -> Codex / Claude Code / OpenCode 自有会话
```

这造成三个根本问题：

1. 系统没有统一的 Work/Task 生命周期。
2. SOP 仍是 Prompt 指导，而不是结构化、可恢复、可验证的执行协议。
3. Goal、Skill、Tool、Background、Permission 的跨机制端到端可靠性不足。

因此后续工作的优先级应是整合生命周期和执行契约，而不是继续增加工具数量。

## 2. 审计范围与证据

### 2.1 Nola 现有实现

主要实现位置：

- CLI workflow overlay：`cli/src/host_prompts.rs`
- Host workspace policy：`cli/src/core_agent_runtime.rs`
- CLI 命令和 Goal 执行：`cli/src/main.rs`
- 子 Agent 与 RuntimeTask：`cli/src/task_runtime.rs`
- 后台进程存储：`cli/src/store/background.rs`
- RuntimeTask 存储：`cli/src/store/task.rs`
- 内置 Skill：`cli/src/builtin_skills.rs`
- 工具、workspace checkpoint 和后台 Bash：`cli/src/tools/mod.rs`
- 外部执行器：`cli/src/external_executor.rs`

相关设计文档：

- `Doc/软件架构设计/总体设计/CORE_HOST_ARCHITECTURE.md`
- `Doc/历史任务/参考研究/CORE_HOST_REFERENCE_SYNTHESIS.md`
- `Doc/软件架构设计/模块/10-Goal任务与子代理/GOAL_LONG_HORIZON_ARCHITECTURE.md`
- `Doc/软件架构设计/模块/07-权限与平台/PERMISSION_BOUNDARY_CONTRACT.md`

### 2.2 参考项目

本次对照以下仓库内参考实现：

- `参考资料/codex-main`
- `参考资料/opencode-dev`
- `参考资料/claude-code`

对照重点不是复制产品 UI 或提示词，而是比较任务所有权、状态机、恢复、权限、上下文、Skill、后台执行和可观察性。

## 3. P0：结构性缺口

### 3.1 多套并行任务生命周期

Nola CLI 当前至少存在四类工作对象：

| 类型 | 主要入口 | 主要状态位置 | 典型用途 |
| --- | --- | --- | --- |
| Goal | `/goal` | Core GoalProjection + `main.rs` | 自主长期目标 |
| 子 Agent | `/agent`、`/tasks` | RuntimeTask + `task_runtime.rs` | 隔离执行或委派 |
| 后台 Bash | `/jobs` | background store + 状态文件 | 长时间命令 |
| 外部执行器 | `/executor` | external session ID | Codex、Claude Code、OpenCode |

这些对象没有统一定义以下属性：

- 状态枚举
- 当前步骤
- 进度与最近活动
- 权限等待
- 用户输入等待
- 取消与超时
- 重试与退避
- 输出、Artifact 和完整结果引用
- 恢复策略
- 终止原因
- 用户 steering

后果是同一个用户目标可能同时产生 Goal、RuntimeTask、后台进程和外部 executor session，但 CLI 无法把它们呈现为一棵工作树，也无法通过一个控制面统一暂停、恢复、取消或检查。

### 3.2 SOP 是软提示词，不是执行协议

`cli/src/host_prompts.rs` 中的 CLI workflow 是编号自然语言。`cli/src/core_agent_runtime.rs` 的 workspace policy 也明确将其描述为 soft SOP guidance。

当前 SOP 缺少：

- 稳定 SOP ID
- schema version
- 结构化步骤 ID
- 输入和输出类型
- 前置条件
- 完成条件
- 步骤依赖
- 可并发声明
- 权限需求
- 资源和时间预算
- 重试、补偿与回退策略
- Artifact 契约
- 每一步的验证器
- 中断恢复位置
- SOP 级执行记录

自然语言 SOP 可以改善模型行为，但不能保证执行顺序、恢复位置、幂等性或验收结果。上下文压缩、模型切换、Host 重启后，系统只能从摘要和消息中重新推测执行位置。

### 3.3 Skill 仍以指令包为中心

内置 Skill 目前主要由 metadata、instruction 和资源文件组成。现有测试能够验证 Catalog、物化文件、启用状态和部分指令约束，但尚未形成统一的运行时质量契约。

缺失能力包括：

- 依赖与环境 readiness probe
- required capability 的运行前解析
- inline 与 isolated Agent 的明确模式
- 独立上下文和 token/runtime budget
- 稳定 SkillRun ID 与步骤进度
- Artifact schema
- Skill 专用验证器
- 失败恢复点
- 版本升级和回归记录
- 真实输入输出 E2E

因此 Skill 仍更接近“模型读取的一份操作说明”，而不是“系统调度的可验证执行单元”。

### 3.4 Goal 的计划和证据关联仍偏文本化

Goal 已具备持久状态、lease、heartbeat、暂停、恢复、调度和独立 Auditor，这是良好基础。但 CLI 侧仍承担较多规划和状态拼装：

- 默认 requirements 由客户端生成。
- requirement ID 由文本行顺序生成。
- Goal prompt 由 CLI 拼接自然语言控制段。
- Contract 和执行报告的形成仍高度依赖当前模型回合。
- Auditor 主要形成整体判断，Requirement 与具体证据的逐项关联较弱。

目前缺少：

- Requirement dependency DAG
- Requirement 到 ToolCall、Artifact、测试结果的明确引用
- 可编辑、可版本化的执行计划
- 步骤级预算和终止条件
- 用户对计划步骤的批准、拒绝和重排
- 重新规划时的 plan diff
- 失败步骤的补偿合同

## 4. P1：长任务可靠性缺口

### 4.1 后台任务管理的是进程，不是工作

后台 Bash 当前主要记录 PID、command、cwd、stdout/stderr 路径和 status 路径。这可以管理进程，但不足以表达业务任务。

系统无法稳定回答：

- 当前处于哪个业务步骤？
- 已完成哪些 Requirement？
- 最近一次有效进展是什么？
- 当前是在执行、等待网络、等待权限还是等待用户？
- 已生成哪些 Artifact？
- 进程退出后结果是否已经提交？
- Host 在提交结果前崩溃时能否 exactly-once 恢复？
- 当前操作是否可以安全重试？

后台进程应成为统一 Work 下的一个执行节点，而不是独立的第二套任务系统。

### 4.2 缺少前台运行到后台托管的转换

当前策略要求模型在执行前判断任务是否需要后台运行。但运行时间经常无法预估。

缺失交互包括：

- 用户将正在运行的命令转到后台
- 运行超过阈值后由 Host 建议托管
- 转后台后保持原 ToolCall、Task 和 output identity
- 前后台切换时不丢失取消、权限和结果关联

参考 Claude Code，运行中的 Shell 可以被用户转入后台，而不是要求模型提前准确猜测耗时。

### 4.3 Steering 队列不够耐久

CLI 交互循环可以暂存 queued user input，但主要是进程内状态。它没有形成持久、可恢复、绑定明确工作边界的 steering queue。

需要明确：

- 指令绑定哪个 Work/Task？
- 在工具完成、模型 Step 完成还是 Contract 完成后生效？
- CLI 重启后是否仍存在？
- 多任务并行时如何路由？
- 新指令是否使旧计划失效？
- 已执行副作用是否需要重新审计？

参考 Codex 的 input queue，steering 应只在命名明确的安全边界注入，而不是作为普通聊天文本随时混入。

### 4.4 恢复入口分裂

目前恢复入口包括：

- `/resume`
- `/goal resume`
- `/tasks`
- `/jobs`
- 权限 pending effect resume
- Codex、Claude Code、OpenCode native session resume

技术上分别存在恢复能力，但用户没有统一的“正在进行的工作”页面，也没有一个恢复协调器判断：

- 哪些工作仍真实运行？
- 哪些 lease 已过期？
- 哪些只是进程退出但结果尚未提交？
- 哪些等待权限或用户输入？
- 哪些可以自动恢复？
- 哪些副作用必须先审计再恢复？

## 5. P1：能力编排缺口

### 5.1 `/plan` 不是完整 Plan Mode

当前 `/plan` 更接近查看计划摘要，不是完整的计划生命周期。

目标 Plan Mode 应支持：

```text
理解目标
  -> 只读探索
  -> 生成结构化计划
  -> 用户编辑或批准
  -> 冻结计划版本
  -> 执行选定步骤
  -> 按证据更新步骤
  -> 必要时生成 plan diff
  -> 验收
```

需要补齐：

- 步骤依赖和可并发关系
- 每步工具、Skill、权限和 Artifact 预期
- 计划版本与 diff
- 用户批准和局部执行
- 执行中重新规划
- 计划步骤与 ToolCall、Task、Artifact 的稳定关联

### 5.2 缺少显式 Execution Plan

工具、Skill、MCP、数据源和子 Agent 当前主要由模型逐轮临时选择。需要增加显式能力绑定：

```text
Requirement
  -> selected capability
  -> execution mode
  -> permission scope
  -> expected artifact
  -> verifier
  -> retry policy
```

没有这一层时，常见失败包括：

- 工具正确但调用顺序错误。
- Skill 被调用但没有完整遵循契约。
- 产物已生成但没有验证。
- 长操作错误地留在前台。
- 独立只读操作没有并发。
- 同一数据被重复读取。
- 重试时重复已经产生副作用的步骤。

### 5.3 Workspace checkpoint 没有进入任务合同

`workspace_checkpoint` 已经能够 create、list、diff 和 restore，但它仍是模型可选工具，不是 Goal/Task 合同的一部分。

Work 应稳定记录：

- baseline checkpoint ID
- checkpoint 创建原因
- 哪些步骤发生在 checkpoint 后
- 当前任务修改的文件集合
- 失败后的推荐恢复点
- restore preview
- 是否只回退当前任务产生的修改

否则 checkpoint 只是高级工具，不是可靠事务边界。

## 6. P2：CLI 产品交互缺口

### 6.1 Slash command 暴露内部架构

当前用户需要理解 `/goal`、`/plan`、`/agent`、`/tasks` 和 `/jobs` 的区别。尤其 `/tasks` 与 `/jobs` 是实现概念，不是自然用户心智。

建议收敛为：

- `/work`：查看、切换、暂停、恢复、取消所有工作。
- `/plan`：查看、编辑、批准和版本化计划。
- `/context`：上下文占用、压缩和保留内容。
- `/permissions`：查看和修改权限模式与待处理请求。

旧命令可以保留为快捷入口，但 UI 和帮助应围绕统一概念组织。

### 6.2 缺少统一任务时间线

终端应以稳定 Item/Part 显示工作树：

```text
● 分析项目
  ✓ 查看目录
  ✓ 阅读 4 个文件
  ● 编译目标
    cargo test ...
    运行 38s · 214/320 tests
  ○ 验证产物
```

当前状态分散在 spinner、工具输出、正文、`/tasks`、`/jobs` 和 Goal 状态中。用户无法快速判断当前步骤、最近进展、阻塞原因和下一步。

### 6.3 缺少“为什么还没完成”的解释

长任务状态至少应包含：

- 当前步骤
- 已耗时
- 最近有效活动
- 等待原因
- 是否需要用户介入
- 下一步
- 进度可信度
- 可用控制操作

仅显示 spinner、PID 或 stdout 不能建立用户信任。

## 7. 与参考项目的差距

| 方面 | 参考项目优势 | Nola CLI 当前差距 |
| --- | --- | --- |
| Task 生命周期 | Codex 用统一可取消 Task 表达多种工作 | Goal、Task、Job、Executor 分裂 |
| 稳定消息与 Part | OpenCode 持久化 Message、Part、Todo、Permission | 底层较强，但 CLI 工作投影没有统一 |
| Skill 执行 | Claude Code 支持 inline/isolated、预算、工具范围和进度 | Skill 仍以 instruction 为中心 |
| 后台任务 | Claude Code 暴露 identity、progress、usage、output ref 和 cancel | Nola 更偏向进程管理 |
| Plan Mode | Codex 有明确的计划与执行边界 | 缺少结构化编辑、审批和版本 |
| Permission suspend | OpenCode 以 suspended request 和结构化规则恢复 | Core 较强，但不同任务入口 UI 分裂 |
| Steering queue | Codex/Claude 在明确边界消费队列 | CLI 主要依赖进程内队列 |
| 长任务恢复 | Claude Code 统一任务活动和终止通知 | 各模块分别恢复，缺少协调器 |
| SOP | 三者均把关键流程落入运行结构 | Nola 仍主要依赖 Prompt |
| E2E | 参考实现重视跨状态恢复 | Nola 单点测试多，跨机制链路不足 |

## 8. 目标架构

### 8.1 统一 Work 模型

CLI 产品层应只向用户暴露一种长期工作对象：

```text
Work
├── Plan
├── GoalContract
├── Step
│   ├── SkillRun
│   ├── ToolCall
│   ├── BackgroundProcess
│   ├── ChildAgent
│   └── PermissionRequest
├── Artifact
├── Evidence
└── SteeringQueue
```

建议统一状态：

```text
queued
planning
ready
running
waiting_permission
waiting_user
waiting_external
paused
verifying
completed
failed
cancelled
```

每个节点至少具有：

- stable ID
- parent Work ID
- state 与 terminal reason
- created/started/updated/finished timestamps
- current step 与 progress
- retry policy 和 attempt
- permission/effect correlation
- output preview 和 full output ref
- Artifact refs
- evidence refs
- cancellation token
- resume policy

### 8.2 结构化 SOP

建议 SOP schema 至少包含：

```json
{
  "id": "document.create.word",
  "version": 1,
  "inputs": {},
  "steps": [
    {
      "id": "collect-requirements",
      "dependsOn": [],
      "mode": "inline",
      "requiredCapabilities": [],
      "completion": {},
      "retry": {},
      "artifacts": []
    }
  ],
  "verification": {},
  "compensation": {}
}
```

SOP runtime 不应根据用户关键词强制路由。模型仍从 schema 中选择 Skill、Tool 或 SOP；Core/Host 根据具体调用、资源、权限和当前状态验证执行。

### 8.3 SkillRun

Skill 应成为 Work Step 的一种执行器，并明确：

- Skill version
- execution mode：inline 或 isolated
- input schema
- required capabilities
- runtime/token budget
- allowed tools
- progress protocol
- Artifact contract
- verifier
- resume behavior

### 8.4 长任务 Supervisor

CLI Host 需要统一 Supervisor 管理：

- lease 和 heartbeat
- foreground/background transfer
- child process ownership
- output capture 和 full output ref
- cancellation propagation
- permission suspension
- user steering
- host restart reconciliation
- exactly-once terminal notification
- concurrency 和 resource limits

Supervisor 不拥有 Agent 决策，只负责 Host 执行生命周期。

## 9. 分阶段实施建议

### Phase 1：统一投影，不立即重写执行器

- 定义 CLI `WorkProjection`。
- 将 Goal、RuntimeTask、后台 Bash、ChildAgent 和外部 executor 映射到统一投影。
- 新增 `/work` 列表和详情。
- 保留旧命令作为兼容快捷入口。
- 统一状态、进度、等待和终止原因显示。

### Phase 2：结构化 SOP 与 SkillRun

- 定义 SOP schema 和版本。
- 将现有 workflow prompt 拆为稳定 policy 与结构化 SOP。
- 为 Skill 增加 execution mode、budget、Artifact 和 verifier。
- 在 Work 时间线中展示 SOP/Skill 步骤。

### Phase 3：统一长任务 Supervisor

- 合并后台 Bash 与 RuntimeTask 的 Host 管理语义。
- 支持前台转后台。
- 持久化 steering queue。
- 统一权限等待、取消、超时、恢复和终态提交。
- 增加启动时 reconciliation。

### Phase 4：结构化 Plan 与 Goal

- Plan version、step dependency 和 approval。
- Goal Requirement 与 Plan Step、Artifact、Evidence 关联。
- Auditor 按 Requirement 验证具体证据。
- 支持 replan diff 和局部重试。

## 10. 必须补齐的 E2E 验收矩阵

以下测试应走真实 CLI Host、canonical store 和终端事件，不应只测试静态文本或 parser：

1. Goal -> Skill -> Tool -> Background -> Permission -> restart -> resume。
2. 长任务中用户 steering，CLI 重启后在正确边界消费。
3. Tool 已完成但 terminal event 尚未提交时 Host 崩溃。
4. 后台进程完成但结果提交前 Host 崩溃。
5. 权限允许、拒绝、自定义回复和 conversation grant 的 exactly-once 恢复。
6. 多个长任务并行时的公平调度和资源上限。
7. Skill 依赖缺失、安装失败、部分产物和验证失败。
8. 网络中断、超时、用户取消和 provider fallback 的统一终态。
9. Plan 变更后旧步骤结果不得错误复用。
10. 上下文压缩发生在 Work 中间，仍保留当前合同、权限、任务和证据。
11. 外部 Codex、Claude Code、OpenCode session 与 Nola Work identity 稳定映射。
12. CLI 重启后 `/work` 与真实进程、Goal、Task、Permission 状态一致。
13. 一千万字符会话下切换 Work、查看任务、追加 steering 仍保持有界内存和响应时间。
14. Tool、Skill、MCP、Artifact 和文件操作在时间线中可展开并保持本地化。

## 11. 验收原则

改造完成不能只以“功能存在”作为验收。每项能力必须证明：

- 状态只有一个权威来源。
- 所有更新使用稳定 ID，而不是解析显示文本。
- 每个运行节点最终到达一个且仅一个 terminal state。
- 取消传播到模型、工具、进程和子 Agent。
- 权限恢复原 effect，而不是重发用户消息。
- 重启不会重复副作用。
- 完整 transcript 和 Artifact 不因压缩丢失。
- UI、日志和存储中的状态一致。
- 用户可以理解当前正在做什么、为什么等待以及下一步是什么。

## 12. 优先级

最终优先级如下：

1. 统一 Work/Task 生命周期和 CLI 投影。
2. 将 SOP 从 Prompt 升级为结构化执行协议。
3. 建立 Goal、Skill、Tool、Background、Permission 的跨机制 E2E。
4. 建立长任务 Supervisor 和持久 steering queue。
5. 完成结构化 Plan、证据关联和逐项 Auditor。
6. 在统一架构上继续增加能力与 Skill。

在前三项完成前继续增加工具，只会扩大状态分裂、恢复困难和测试盲区。

## 13. 2026-08-27 RuntimeTask 真实 PTY 证据更新

本轮关闭了长任务 Supervisor 的一组关键缺口。验证均运行真实 `nola`
二进制、真实 PTY、真实 CLI Host、canonical SQLite store 和可控的流式模型服务器，
不是静态 parser 或 mock executor 断言。

### 13.1 已验证

- `subagent_output_budget_is_enforced_and_reported_through_the_real_host`
  验证 Agent 输出预算、UTF-8 安全截断、预算耗尽原因和稳定调用只创建一个任务。
- `subagent_wall_clock_budget_cancels_the_real_child_and_resumes_the_parent`
  验证挂起的真实 SSE 子请求被墙钟 watchdog 取消，父轮次恢复并消费结构化失败结果。
- `stop_task_cancels_a_real_streaming_child_and_resumes_the_parent`
  验证 `stop_task` 经过真实权限面板后传播取消，任务只进入一次 `cancelled`
  终态，父轮次在远端流 EOF 之前恢复。
- `queued_subagents_run_in_fair_order_under_the_real_concurrency_limit`
  统一改为独立 worker 进程后曾暴露跨进程重复 claim 竞争。当前 Supervisor 已将公平 claim
  前移到进程创建之前，并把持久 ownership lease 传给唯一 worker；全局和项目并发上限均为
  1 的真实 PTY 已重新通过。该结论仅覆盖单工作区双任务公平排队，不外推到多项目压力场景。
- `orphaned_agent_task_resumes_same_child_session_after_process_restart`
  验证 Host 进程中断后恢复同一 durable task 和同一 child session，不创建替代任务。

上述重型 PTY 场景均使用 `--test-threads=1` 定向通过。真实 PTY 用例
在测试进程内由全局锁串行持有终端资源；显式串行参数只能降低测试资源噪声，不能代替
生产运行时的跨进程互斥、持久 lease 和原子 claim 合同。

### 13.2 本轮修复的根因

- durable Agent task 的 metadata 包含 `runner_pid`，旧 `stop_task` 逻辑据此把 Agent
  误判为 legacy Shell task，最终返回 `Unknown background bash task`。现已只依据类型化
  `task.kind` 选择执行器，不再根据 metadata 字段猜测任务类型。
- promoted task 的 stable call 重放过去先进入通用 Work Step transaction recovery，旧
  `ToolCompleted` 会被再次投递给已经没有 active call 的 Core。Host 现在先查询 durable
  runtime task；命中 stable `call_id` 时投影 `TaskUpdated`，首次执行仍使用普通事务和权限。
- Application checkpoint 后会以 Core canonical pending effect IDs 对账 outbox，清理 Core
  已经移除的旧 effect，避免非终态恢复把过期副作用再次交给 Host。

### 13.3 尚未关闭

- `RuntimeTaskResourceBudget` 已增加 `max_cpu_time_ms`、`max_memory_bytes` 和 `max_processes`，Application 会拒绝零值并保持 metadata 往返。原生 Nola 后台 Agent 现统一运行在独立 worker 进程中，不再保留线程内执行分支；资源监督器按 PPID 递归收集跨进程组后代，排除采样器自身，并在超限时先 `SIGTERM`、再重采样后 `SIGKILL`。真实 PTY 已验证进程数、CPU、RSS、墙钟和输出预算，并证明显式取消不会误杀 CLI 的 Application Service。`concurrent_subagent_budget_failure_is_isolated_from_a_healthy_peer` 证明双 worker 实际重叠运行时，一个任务触发墙钟预算失败不会终止或污染健康任务；`two_orphaned_agent_tasks_recover_independent_sessions_after_supervisor_kill` 证明 CLI/Supervisor 进程树被强杀后，两任务分别复用原 child session、独立增加 retry 并完成；`goal_skill_shell_and_agent_share_one_supervisor_slot` 进一步验收 Goal/Skill lineage 下 Shell/Agent 的跨机制单槽竞争。尚未关闭的是更大规模混合队列叠加 Host/worker 强杀的压力矩阵。
- 后台 Bash 已持久化可认领 execution spec、ownership/lease 与 pre-spawn claim，并与 Agent 共享全局/项目槽位。`claimed_background_shell_recovers_before_command_start_without_replay` 证明 claim 后、命令启动前崩溃可恢复且不重复执行；`killed_shell_worker_after_command_start_requires_reconciliation_without_replay` 证明命令跨过执行边界后 worker 被杀不会盲目重放；Shell/Agent 单槽和跨项目 durable wake 也已通过真实 PTY。`background_shell_cpu_memory_and_process_budgets_are_enforced_by_real_workers` 进一步证明 CPU、RSS 和进程数预算由真实 Shell worker 强制执行并持久化 exhaustion。`twelve_background_shell_tasks_complete_without_starvation_or_slot_oversubscription` 使用正式 `--yes` 模式，经真实模型协议一次创建 12 个持久后台 Shell task，在全局/项目 3 槽限制下执行真实子进程；全部 task 完成，实际并发至少为 2，区间扫描未超过 3 个槽，且每个 task 恰好一个 `finished` 事件。
- 单项目 Shell 高竞争、无饥饿和槽位不超卖门禁已关闭。剩余范围是多项目混合 Agent/Shell 队列叠加 Host/worker 强杀，而不是基础公平调度或 Shell 资源预算功能缺失。
- provider EOF、`429 + Retry-After-Ms`、`401`、正文/推理 continuation 已通过真实 HTTP/PTY。CLI 现支持显式、有序的 `llm.fallbacks[]`：每个候选包含独立 `provider / endpoint / api_key / model`，不从环境变量或用户文本推断路由；仅当当前候选尚未发布正文或推理且错误属于可恢复传输/限流/服务故障时才切换，认证、请求合同和上下文错误不会被备用 Provider 掩盖。`provider_fallback_preserves_an_active_durable_shell_task_and_identity` 通过真实双 HTTP Provider、PTY、Application/SQLite 和真实后台 Shell worker 验证主 Provider 503 时的接续：活动任务保持同一 task ID、唯一记录、正常完成且产物只生成一次。成功响应会携带实际 Provider namespace/model，continuation 不再错误绑定主 Provider。
- 模型流重启已覆盖 Text、Reasoning 和增量 ToolCall：旧文本与思考内容被清空，旧半段工具参数被清空并取消，替换流使用新的 Part identity，不会拼接两次参数流。
- 权限等待、重启和恢复同一 effect 已通过 Goal/Skill/Background 联合 PTY；`separate_background_agents_keep_task_scoped_permissions_isolated` 覆盖并发任务等待不同资源，session grant 复用由背景 Agent 与混合链路用例覆盖。跨客户端仍需复用同一 effect 决策合同。

### 13.4 模型流与交互状态的跨进程恢复证据

- `ModelStreamRestarted` 是 `ModelStream` effect 的合法非终态 Host 响应。它不创建新 Turn、不更换 effect identity，也不把 provider 无法续传误报为失败。
- Core 会清空旧流的 provisional Text/Reasoning 内容，将旧增量 ToolCall 的半段参数清空并取消，同时清除未提交 provider continuation；替换流必须使用新的 Part identity，并沿用单调 Host response sequence。`restarted_model_stream_clears_reasoning_and_partial_tool_arguments` 覆盖这一合同。
- 真实 PTY 在首个长流中输入 steering 后强杀 CLI，冷启动恢复原回答，再在下一个安全边界消费 steering；SQLite 中消息顺序完整，后续普通 Turn 不会再次消费该输入。
- `goal_resume_restores_pending_permission_before_any_parent_model_request` 和 `goal_skill_permission_background_restart_and_resume_share_one_work_chain` 证明持久权限会先于父模型恢复，且 Goal、Skill、权限和后台任务继续属于同一工作链。
- Skill 依赖、Artifact verifier、部分成功和局部重试的真实端到端覆盖。

因此，RuntimeTask 的取消、基础排队和恢复子集已有强证据，但第 10 节矩阵尚未整体完成，
不能据此宣称长任务体系已经验收完毕。

## 14. 全部 Agent 进程隔离后的新增审计结论

将所有后台 Agent 统一迁移到独立 worker 进程是正确方向：CPU、RSS、进程树、墙钟和输出
预算终于能够归属于具体任务，取消也不会再误杀 CLI 的 Application Service。但该改造同时
证明，原先依赖进程内集合、锁和本地 active-turn 缓存的协调方式并不具备跨进程正确性。

### 14.1 公平队列缺少持久化的原子启动权

当前恢复者会先观察到任务为 `queued`，再创建 worker，由 worker 自行执行 `claim`。观察与
claim 之间存在窗口；多个 CLI/worker/recovery loop 可以同时观察到同一个排队任务，随后各自
启动进程。进程内的 task handle 集合无法阻止另一个进程重复启动。

已观察到的证据包括：

- 同一 RuntimeTask 出现两个不同 ownership token 的 `claim_task` command。
- 第一个 worker 随后报告 `ownership_lost`，第二个 worker 完成任务。
- Core 拒绝迟到 claim，错误为 `Only queued or retry-waiting Tasks may be claimed`。
- 最终任务虽然完成，但已经违反“一个调度决定只启动一个执行实例”的合同。

目标设计必须满足：

```text
atomic reserve/claim
  -> persist ownership token and lease
  -> spawn exactly one worker
  -> worker adopts the persisted lease
  -> spawn failure releases or schedules the same attempt for retry
```

禁止使用以下方式掩盖问题：

- 增加进程内 mutex 或 Set。
- 捕获并忽略重复 claim 错误。
- 仅按最终任务状态去重 UI。
- 依赖 PTY 测试串行执行来保证生产互斥。

### 14.2 Task command 必须按真实 Run 状态选择 active 或 idle 提交

RuntimeTask 的 permission-resolved、terminal update 等命令可以在父回答结束之后到达。当前
边界层仍可能依据 SessionStore 中过期的本地 active-turn 缓存，将这些命令提交到已经关闭的
Run，产生 `active submission has no active Run`。未消费命令随后会影响任务详情、通知和恢复。

正确合同是：

- Application 的 canonical active Run 是唯一权威，Host 本地缓存不能决定提交路径。
- Run 存在时，任务更新可以作为 active submission 进入该 Run。
- Run 已终止时，相同 typed task operation 必须作为 idle submission 独立提交。
- promoted task 必须终结原始 Host request；后续进度通过 durable Task command 传播。
- 每个 command 只能被确认一次，重启后未确认 command 必须可继续消费。

### 14.3 `/work` 与 `/tasks` 的显式管理应采用工作区作用域

显式查看、停止或恢复后台工作属于工作区控制行为，不应因为用户切换了会话而把同一工作区
的任务误报为未知 Bash 任务。路径必须先 canonicalize，处理 `/var` 与 `/private/var` 等同一
目录的不同拼写。

建议拆分两类作用域：

| 作用域 | 用途 |
| --- | --- |
| workspace scope | `/work`、`/tasks` 的 list/status/stop/resume，以及用户主动查看 Artifact |
| owner session scope | 权限 continuation、模型上下文注入、steering 消费和对话内自动提示 |

客户端不得在 Agent 任务查询失败后静默回退为 Bash task 查询；任务类型由持久化 `kind` 决定，
不能由 metadata、命令文本或查询失败推断。

### 14.4 当前实现门禁

在以下真实 PTY 回归全部通过前，Supervisor 的公平调度、权限恢复和跨进程协调保持未完成：

1. 并发恢复者竞争同一 queued task，只能生成一个 ownership token 和一个 worker。
2. worker 在 claim 后、spawn 后和 terminal commit 前分别崩溃，均能恢复同一 attempt。
3. 父 Run 结束后到达的 permission-resolved 与 terminal command 走 idle submission，且只消费一次。
4. conversation grant 被同一会话的多个任务并发复用，不重复弹窗，也不扩大资源范围。
5. 切换会话后仍可从 `/work` 查看和停止同一 canonical workspace 的任务。
6. 普通模式与 Debug 模式展示相同 Task identity、状态和终止原因。
7. 数据库断言同时检查 task、command、effect outbox、ownership token 和 canonical Run，而不是只检查终端文案。

### 14.5 后续实施顺序

1. 在持久 Store/Application 层建立原子 launch reservation 或 pre-claimed lease。
2. 让 worker 接管已持久化 ownership，而不是启动后再次竞争 claim。
3. 按 Application 的真实 Run 状态路由 active/idle task submission。
4. 分离 workspace 管理作用域与 owner-session continuation 作用域。
5. 修复并运行公平队列、权限恢复、取消、孤儿恢复和资源预算组合 PTY。
6. 最后再优化 `/work` UI；禁止用展示层去重掩盖协议重复。

这一组问题再次说明：长任务可靠性不能通过“后台进程可以运行”或“最终拿到结果”证明。
必须同时证明调度唯一、ownership 稳定、状态提交恰好一次、权限 continuation 不丢失，以及
用户在会话和进程重启后仍能控制同一个 Work。

### 14.6 本轮修复与当前证据

当前实现已完成以下协议修复：

- Supervisor 在创建 worker 进程前执行事务化 `claim_runtime_task_fair`；未取得容量或启动权时
  不创建进程。
- `worker_id`、`ownership_token` 和 lease duration 通过内部 worker request 传递，worker 不再
  启动后重新竞争 Store claim。
- 进程创建或 stdin 交接失败会使用相同 ownership token 释放任务进入有界重试。
- 模型流 boundary consumer 在领取 durable Task command 前查询 canonical active Run；Run 已
  终止时不再把命令提交到陈旧 active turn。
- `/tasks` 和 `/work` 的显式任务管理按 canonical workspace 匹配；会话 owner 不再阻止用户
  查看同一工作区的任务，`/var` 与 `/private/var` 等路径别名通过 canonical path 收敛。

当前通过的验证：

| 验证 | 结果 |
| --- | --- |
| `cargo check -p nola_cli --bins --quiet` | 通过；仅有既存 warning |
| 公平队列真实 PTY | 通过 |
| 后台 Agent 权限面板与 conversation grant 复用真实 PTY | 通过 |
| 进程树预算真实 PTY | 通过 |
| 墙钟预算真实 PTY | 通过 |
| 显式取消真实 PTY | 通过 |
| 孤儿 Agent 同 child session 恢复真实 PTY | 通过 |
| Goal/Skill/权限/后台/重启联合真实 PTY | 通过 |
| `nola_store` RuntimeTask E2E | 4/4 通过 |
| `nola_application` Turn transaction E2E | 9/9 通过 |

仍未关闭的范围：

- 多项目混合 Agent/Shell 队列叠加 Host/worker 强杀的压力公平性和饥饿验证；单项目 12 Shell/3 槽高竞争已通过。
- worker 在 pre-claim 后、进程 request 完整接收前被 `SIGKILL` 的专门故障注入。
- 多个任务同时等待不同资源时 conversation grant 的顺序与最小作用域证明。
- provider 断流、退避和 fallback 与 RuntimeTask terminal state 的联合协议。

## CLI 能力强化路线图

### 目标

CLI 不应只是一个能循环调用工具的聊天终端。它应当成为一个可恢复、可审计、可干预的本地 Agent Host：模型负责语义决策，Core 负责协议和状态约束，CLI Host 负责执行、权限、持久化与用户交互。任何能力都不能依赖语言关键词路由，也不能依赖某个客户端进程始终存活。

### 当前主要不足

#### 1. SOP 仍偏向单轮工具循环

当前流程能完成普通的“模型决定 -> 调用工具 -> 返回结果”，但面对跨文件、跨阶段或数小时任务时，缺少稳定的阶段模型。计划、执行、验证、恢复和交付之间的边界还不够明确，导致：

- 模型容易重复探索，缺少基于已有证据的阶段收敛。
- 工具失败后通常由模型临场补救，没有统一的可恢复错误分类。
- 重启后虽然能恢复任务，但未必能恢复任务当时的意图、阶段和验收条件。
- 前台对话、后台任务和子 Agent 之间的结果归并仍可能出现竞态。

目标 SOP 应固定为：

```text
理解意图
  -> 建立可验收目标
  -> 收集最小必要上下文
  -> 形成阶段计划
  -> 执行一个可提交步骤
  -> 验证产物与约束
  -> 持久化检查点
  -> 继续、等待、请求权限或交付
```

计划不是一次性文本，而是持久化状态；每个步骤必须记录输入、工具调用、截断结果、产物、验证结果和下一步。进入新一轮时不应继续携带完整工具集合，只保留调用摘要和必要产物引用。

#### 2. 长任务仍缺少完整的运行契约

长任务需要与聊天消息解耦。一个任务至少应具备以下持久化字段：

- 稳定任务 ID、所属工作区和发起会话。
- 当前阶段、检查点、恢复策略和验收条件。
- worker 身份、所有权令牌、租约、心跳和真实进程树。
- CPU、内存、进程数、运行时长和输出配额。
- 权限等待、用户输入等待、重试、失败、取消和完成状态。
- 产物索引、截断日志及可重放事件。

需要明确区分：

| 状态 | 含义 | 是否占用 worker |
|---|---|---|
| `queued` | 等待公平调度 | 否 |
| `running` | 正在执行可计算步骤 | 是 |
| `waiting_permission` | 等待用户授权 | 否 |
| `waiting_input` | 等待用户补充信息 | 否 |
| `retry_wait` | 可恢复失败后的退避 | 否 |
| `completed` | 产物已验证并提交 | 否 |
| `failed` | 不可恢复或超过预算 | 否 |
| `cancelled` | 用户或策略取消 | 否 |

进程退出不是任务失败的充分条件。Host 必须根据租约和检查点判断是接管、重试还是终止。权限等待也不能伪装成工具错误；它应挂起运行并在用户响应后从原调用恢复。

#### 3. 工具协议需要进一步统一

Shell 不应成为其他工具的隐式替代品。文件读取、目录遍历、搜索、补丁、进程和网络检索应使用稳定的规范工具名及结构化参数。只有确实需要 shell 语义时才调用 shell。

统一后必须满足：

- 展示名、本地化名、权限匹配名和审计名来自同一工具描述。
- 权限按规范化后的具体资源和动作判断，不按自然语言关键词判断。
- Shell 包装层不会破坏命令前缀授权，也不会与专用工具重复执行。
- 未知工具、参数错误和暂时不可用均返回可恢复的结构化错误。
- 工具输出正文、诊断信息、产物和完整输出引用分离存储。
- 模型下一轮只看到必要的截断结果，用户仍可展开完整详情。

#### 4. 权限模型需要与任务生命周期一致

权限模式应维持三档，并在 CLI、Web 和 Mac Host 中使用相同语义：

- 推荐：读取和可逆操作默认允许，仅不可逆或高风险操作确认。
- 安全：扩大确认范围。
- 自主：在明确工作区和资源预算内自动执行。

“本次对话允许”和“本次任务允许”必须是持久化授权租约，而不是某个 UI 面板的临时状态。授权匹配对象应包含规范工具、动作、规范路径或网络范围、会话/任务作用域及过期条件。`custom response` 应将运行置为 `waiting_input`，回到正常输入流，而不是制造一条失败结果。

#### 5. 能力发现和 Skill 编排还不够可靠

Skill 应提供结构化能力声明、输入约束、依赖、预估耗时、权限需求、产物类型和验证方法。模型通过工具 schema 和 Skill 描述自主选择，而不是由客户端对用户文本做关键词匹配。

针对长任务，Skill 还应声明：

- 是否支持检查点和幂等恢复。
- 是否适合后台执行。
- 可并行阶段和最大并发度。
- 依赖安装及环境探测策略。
- 失败后可否降级，以及降级是否影响产物质量。

数据源、定时任务和 Skill 应能够串联：定时任务触发数据源读取，模型选择 Skill 处理，产物和摘要写回任务记录；其中每一步都可查看、暂停和恢复。

#### 6. 上下文管理需要面向任务证据而非消息数量

上下文压缩应由代码和 LLM 协同完成。代码负责预算、分段、去重、不可丢弃项和完整记录保留；LLM 负责语义摘要。压缩结果至少保留：

- 用户目标、约束和验收标准。
- 已完成步骤及验证证据。
- 未完成计划、阻塞项和等待中的权限。
- 关键文件、产物和外部来源引用。
- 最近交互及仍可能影响决策的工具结果。

工具 schema 不应永久累积到后续轮次；每轮重新提供当前可用工具，历史中仅保留实际调用记录和截断结果。手动 `/compact` 与自动压缩必须走同一 Core 契约，并显示压缩前后 token、模型总窗口和保留内容类型。

#### 7. CLI 交互仍未达到可控、低焦虑的标准

用户应始终能回答四个问题：现在在做什么、为什么需要这么久、是否在等待我、如何停止或继续。建议采用稳定的三层显示：

```text
第一层：当前状态和上下文占比
第二层：正在执行的阶段、工具和关键参数
第三层：可展开的完整思考摘要、工具结果和任务日志
```

普通模式减少噪声，但不能隐藏关键中间过程；Debug 模式增加协议事件、租约、检查点、token 和耗时，不应改变基本布局。输入期间 footer 应正确隐藏或避让，`Esc` 必须中断当前前台运行，后台任务则通过任务面板独立管理。

### 与成熟 Agent CLI 的关键差距

值得借鉴的不是某个界面样式，而是以下工程原则：

| 领域 | 成熟实现的共同做法 | Nola 应补齐 |
|---|---|---|
| 执行循环 | Run/Turn/Tool 有清晰事件边界 | 消除终止边界竞态与重复归并 |
| 文件修改 | 先规划读取范围，再生成最小补丁 | 大文件处理在执行前选策略 |
| 权限 | 对具体调用做策略判断，可挂起恢复 | 统一会话、任务、工作区授权租约 |
| 上下文 | 代码预算 + 模型摘要 + 完整日志留存 | 所有平台复用同一 Core 契约 |
| 长任务 | durable queue、lease、heartbeat、checkpoint | 补齐 idle command drain 和阶段恢复 |
| 可观测性 | 事件流稳定，UI 只是投影 | 避免状态栏、工具卡片和正文互相覆盖 |
| 测试 | 真实进程、真实重启、真实协议时序 | 从静态断言升级到故障注入 E2E |

### 分阶段实施优先级

#### P0：正确性

1. 消除 Run 终止、后台命令和消息落库之间的竞态。
2. 保证同一 durable task 在多进程恢复时只被一个 worker 执行。
3. 完成权限挂起/恢复协议，避免重复弹窗和越权执行。
4. 统一工具规范名、展示名、权限名和审计事件。
5. 保证取消可以终止完整进程树且不会遗留 PTY 或子进程。

#### P1：可恢复长任务

1. 引入阶段检查点和验收证据。
2. 将 `waiting_permission`、`waiting_input` 与计算中状态分离。
3. 实现冷启动恢复、租约接管、公平调度和资源预算。
4. 让前台、后台和子 Agent 使用同一持久化 Run/Turn/Tool 模型。

#### P2：能力与体验

1. 建立可审计的 Skill 能力目录和依赖探测。
2. 完成数据源、定时任务、Skill 的自主编排。
3. 统一普通/Debug 模式的信息架构和本地化。
4. 增加长任务进度、耗时解释、产物预览和用户干预入口。

### 必须具备的端到端测试

不能只验证函数返回值。以下场景必须通过真实 CLI/PTY、Store 和 worker 进程验证：

1. 工具调用被持久化声明后，CLI 在 worker 启动前被杀死；新 CLI 在租约过期后仅恢复一次。
2. 两个 CLI 同时恢复同一工作区，只允许一个 worker 获得任务所有权。
3. 后台任务申请权限，用户授予“本次任务允许”后，后续相同资源调用不再弹窗；其他任务不继承。
4. `custom response` 挂起任务，下一条普通输入恢复原调用，消息顺序不乱。
5. `Esc` 中断前台 Run，子进程树、PTY 和租约全部清理。
6. CPU、RSS、进程数和墙钟预算分别触发时，任务进入正确终态并留下诊断。
7. 压缩前后目标、约束、关键证据和待办保持一致，完整 transcript 可重放。
8. 未知工具、错误参数、网络临时失败和工具超时均能恢复，不进入无限工具循环。
9. 中文、英文、混合语言和自然追问走相同工具契约，不依赖关键词路由。
10. 长时间学术检索使用单独预算，其他任务仍受默认超时约束。
11. 一千万字符的历史记录下，启动、输入、工具显示、压缩和会话切换保持在性能预算内。
12. 普通模式和 Debug 模式显示同一任务状态与上下文占比，Debug 仅增加诊断信息。

### 验收标准

CLI 能力强化完成的判断标准不是“可以跑通一个演示”，而是：

- 任意时刻杀死 CLI 或 worker，任务不会重复执行、静默丢失或错误完成。
- 所有高风险操作都能说明具体工具、资源、作用域和授权来源。
- 用户可以观察、暂停、取消、恢复和接管长任务。
- 模型不依赖关键词路由即可正确选择工具和 Skill。
- 上下文压缩后仍能继续执行既定计划并引用已有证据。
- 所有平台对 Run、Turn、Tool、权限和任务状态使用一致的 Core 语义。
- 关键故障均有可重复的真实进程 E2E，而非仅静态或 mock 测试。
## 近期修复与验证证据（2026-08-27）

本节记录本轮针对 CLI 长任务、权限、恢复和交互边界的实际修复。这里的“通过”均指真实进程或 PTY 端到端测试，不是仅检查源码、模拟字符串或绕过 Host 的单元测试。

### 1. 后台 Agent 启动前崩溃恢复

后台 Agent 现在遵循“先持久化认领，再启动独立 worker”的顺序。任务被认领但 CLI 在 worker 启动前退出时，租约过期后可由新进程接管，并继续使用同一个子会话，而不是重复创建任务或丢失执行链。

同时移除了认领阶段写入的伪 `runner_pid`。只有真实 worker 成功启动后才记录其 PID，因此任务列表、恢复判断和进程治理不会把尚未存在的 worker 误判为运行中。

端到端用例 `claimed_agent_task_recovers_when_cli_dies_before_worker_spawn` 验证：

- 原进程在 worker 启动前退出；
- 恢复后的 ownership token 与过期 token 不同；
- 恢复期间记录的 `runner_pid` 对应真实存活进程；
- 完成后清除 ownership token；
- 数据库中只有一个 durable Agent task；
- 恢复继续使用原子会话；
- retry count 正确增加。

### 2. 后台任务权限改为真正的任务作用域

此前后台任务的“本任务允许”实际持久化为 `conversation:<ownerSession>`，并且批准后会批量处理同一 owner session 下的待授权项。这会导致一个后台任务获得的写权限泄漏到之后的其他任务。

现在后台任务权限使用独立 Host 作用域：

```text
host:task:<taskId>
```

该作用域具备以下语义：

- 同一个任务内后续相同操作可复用授权；
- worker 重启或任务恢复后授权仍然有效；
- 授权不会扩散到同一对话中的其他后台任务；
- 待授权项只按 `task_id` 处理，不再按 owner session 批量放行；
- 不伪装成 Conversation 权限，避免触发会话外键约束或产生错误的用户心智。

端到端用例 `separate_background_agents_keep_task_scoped_permissions_isolated` 顺序执行两个独立写任务，验证：每个任务都只出现一次独立权限面板，各自在任务内部复用授权，最终生成四个产物，数据库中存在两个互不污染的已完成 Agent task。

并发写 Agent 仍然受单写者安全约束限制。这是有意保留的安全不变量，不通过放宽并发写入来掩盖权限作用域问题。

### 3. Custom 权限响应的挂起与恢复

此前用户在权限面板选择 Custom 并回到主输入框后，立即输入 `/allow` 可能被追加到已经取消的旧 Run，导致命令被当成旧轮次输入执行，而不是进入权限选择流程。

根因有两处：

- execution cancellation 已请求后，runtime input 仍尝试持久化为当前 Run 的 boundary steering；
- Host 的 `RequestPermission` 处理只进入 permission scope，没有进入当前 execution cancellation scope。

修复后：

- 已取消 Run 不再接收新的 boundary input，新输入进入下一轮本地 FIFO；
- 所有 Host 请求在处理时同时进入 execution cancellation scope 与 permission scope；
- Custom 等待期间流程保持挂起，用户回到主输入框后的下一次输入由权限流程消费，不会污染原 Run。

真实 PTY 用例 `permission_custom_response_suspends_the_tool_until_the_user_answers` 已通过。

### 4. 长任务资源治理、取消、恢复和权限组合验证

以下 13 个真实 PTY 端到端用例已逐项通过：

1. `permission_custom_response_suspends_the_tool_until_the_user_answers`
2. `subagent_output_budget_is_enforced_and_reported_through_the_real_host`
3. `subagent_wall_clock_budget_cancels_the_real_child_and_resumes_the_parent`
4. `subagent_process_budget_terminates_the_real_process_tree_and_resumes_the_parent`
5. `subagent_cpu_budget_terminates_a_real_cpu_bound_process_tree`
6. `subagent_memory_budget_terminates_a_real_memory_resident_process_tree`
7. `stop_task_cancels_a_real_streaming_child_and_resumes_the_parent`
8. `queued_subagents_run_in_fair_order_under_the_real_concurrency_limit`
9. `orphaned_agent_task_resumes_same_child_session_after_process_restart`
10. `claimed_agent_task_recovers_when_cli_dies_before_worker_spawn`
11. `background_agent_reuses_permission_panel_and_task_scoped_approval`
12. `separate_background_agents_keep_task_scoped_permissions_isolated`
13. `goal_skill_permission_background_restart_and_resume_share_one_work_chain`

这组测试覆盖：输出预算、墙钟时间、CPU、内存、进程树终止、流式任务取消、公平排队、孤儿任务恢复、启动前崩溃恢复、任务级权限复用与隔离，以及 Goal、Skill、权限、后台任务、重启恢复组成的完整工作链。

包级验证也已通过：

```bash
cargo test -p nola_store --quiet
cargo test -p nola_application --quiet
cargo check -p nola_cli --bins --quiet
```

最终退出码为 `0`。当前仍有既有未使用代码和 import 警告，但没有编译错误或测试失败；这些警告不作为本轮功能正确性的替代证据，也应在后续代码清理中处理。

### 5. 尚未完成的边界

上述结果不代表本审计全部完成。至少仍需继续处理：

- Plan/SOP 阶段状态、阶段产物和恢复点的统一持久化；
- 更完整的跨进程事务边界和故障注入矩阵；
- 长任务在 CLI、Web 和原生客户端之间一致的可观察性与控制契约；
- 清理当前编译警告，并补足相关回归覆盖。

### 6. CLI 空闲 durable command 主动消费（2026-08-27）

该项已经完成。交互式 composer 现在每 250ms 获得一次有界唤醒，并在 canonical Session 没有 active Run 时主动消费该会话的 durable Agent command FIFO。其边界是：

- active Run 存在时不抢占队列，仍由 `AgentHostDriver` 作为唯一消费者；
- Session 空闲时由 Core Host 适配层恢复/获取 canonical Host，按 FIFO claim、提交、checkpoint 并 acknowledge；
- drain 使用无输出事件接收器，不清空或重绘 composer；
- drain 失败只写入 debug 日志，不破坏当前草稿或抢占终端输入；
- 非 Unix 终端同样使用有界 Crossterm poll，不再无限阻塞。

真实 PTY 用例 `idle_composer_drains_durable_agent_commands_without_keyboard_input` 在完成首轮对话并进入空闲 composer 后，由测试进程直接向 canonical SQLite `agent_commands` 表注入一个合法 Core command。测试不再发送任何键盘输入，随后验证命令在 5 秒内被确认删除，并确认 composer 仍稳定可输入。

相关回归同时通过：

- `populated_context_footer_is_removed_before_the_second_turn_starts`
- `permission_custom_response_suspends_the_tool_until_the_user_answers`
- `background_agent_reuses_permission_panel_and_task_scoped_approval`
- `claimed_agent_task_recovers_when_cli_dies_before_worker_spawn`
- `cargo check -p nola_cli --bins --quiet`

## 2026-08-27 实施记录：Plan 阶段运行态与冷重启恢复

### 已实现

- `ConversationPlanStep` 与 CLI `AgentTodo` 已纳入真实阶段运行字段：`artifactRefs`、`workStepTransactionRefs`、`attempt`、`executorRef`、`resumeKey`。
- `ConversationPlan` 及其每条 revision 已持久保存 `run_id`、`sop_id` 与固定的 `sop_version`。普通 Save、审批、编辑和冻结不能改变运行身份；显式 replan 必须生成新的 `run_id`，并在结构化 diff 中保留前后身份。
- `update_agent_todos` 的结构化 Schema、序列化、格式化、归一化及 Plan 种子投影已统一支持这些字段。
- 非重规划更新不得降低阶段尝试次数，也不得删除已经提交的 Artifact 或 Work Step Transaction 引用。
- CLI Host 现在持有与工具执行共享的 `AgentPlanState`，在每个 Turn 提交前读取最新快照并作为动态上下文注入模型，而不是只在 Host 初始化时生成一次静态副本。
- CLI 进程退出、重新启动并通过 `/resume` 恢复会话后，模型请求能够获得当前阶段、实际 Artifact、Work Step Transaction、执行器、尝试次数和恢复点。
- Work Step Transaction 进入 `committed` 或 `failed` 终态时，会在同一个 SQLite 事务中更新 canonical Plan、递增 Plan revision，并写入 `change_kind=execution` 的 revision。Plan CAS 失败时，事务终态更新也会回滚，避免出现“工具已结算、Plan 未关联”的分裂状态。

### 本轮发现并修复的根因

1. `AgentPlanState` 起初没有绑定当前会话，因此工具侧无法恢复 canonical Plan。
2. 恢复的 Plan 组件曾被加在 Host 构造阶段，但构造阶段只保留提示词身份，动态组件没有进入后续 Turn。UI 因走独立投影能显示 Plan 进度，模型请求却看不到 Plan，形成了“界面已恢复、智能体未恢复”的分裂状态。现改为每轮从共享状态重新投影。
3. SOP 运行身份曾只存在于定义或模型上下文中，Plan 无法证明恢复的是同一版本的运行实例。现由 Host 生成运行 ID，由 Store 和 Application 共同校验 identity 变更规则，模型不能编造或静默替换运行身份。
4. 工具事务与 Plan 阶段曾依赖模型再次调用 `update_agent_todos` 手工关联，存在遗漏和崩溃窗口。现由 Application 在终态结算时按 `work_id + step_id` 自动、原子地写入事务引用；如果结果已有结构化 `result_ref`，同时写入 Artifact 引用。

### 验证证据

- 应用层冷重启：`conversation_plan_lifecycle_e2e::stage_artifacts_transactions_and_resume_point_survive_cold_reopen` 通过，证明 SQLite 关闭后重开仍能恢复阶段运行字段。
- SOP 运行身份：`conversation_plan_lifecycle_e2e` 覆盖普通更新不得替换 identity、同一 run ID 不得用于 replan、显式新 run/version replan、revision 历史及结构化 diff；`conversation_plan_cold_restart_e2e` 覆盖冷启动恢复固定 SOP 版本。
- 事务原子关联：`work_step_transaction_e2e::terminal_effect_atomically_links_transaction_and_artifact_to_the_plan_stage` 通过，证明终态事务、结果 Artifact、Plan revision 和 SOP identity 能在一次提交后跨冷启动恢复。
- CLI 状态机负向测试：`active_plan_stage_cannot_drop_attempt_or_committed_runtime_references` 通过，证明尝试次数和已提交运行证据不可静默回退。
- 真实 PTY 跨进程测试：`plan_stage_runtime_survives_a_real_cli_process_restart` 通过。测试启动真实 CLI 进程、保存会话、退出、直接写入 canonical Plan、启动第二个 CLI、执行 `/resume`，并由模拟模型服务器检查实际请求中的全部运行态字段。
- `cargo check -p nola_cli --bins --quiet` 通过。
- `cargo test -p nola_cli work_step_transaction_execution_tests --quiet` 12/12 通过；其中 formatter、Git/file-preimage restore、前台与后台 Bash 均执行真实文件副作用，不是静态 schema 测试。
- `cargo test -p nola_cli background_lifecycle_tests --quiet` 3/3、`cargo test -p nola_cli store::background::tests --quiet` 1/1 通过。
- `cargo test -p nola_application --test work_step_transaction_e2e --quiet` 4/4、`cargo test -p nola_host_protocol --quiet` 7/7 通过。
- `cargo fmt --all --check` 仍报告本轮之外多个既有文件及 `tools/mod.rs` 的历史格式差异；为避免覆盖并行工作，本轮未执行全仓自动格式化。该项不是编译失败，但仍是仓库级质量门禁欠账。

### 仍未完成

1. **结构化 Artifact 合同已贯通主要受控执行路径，但不能泛化为任意副作用 exactly-once。** `ToolOutcome -> Work Step Transaction -> canonical Plan -> 冷启动模型上下文` 的统一通道、主要内置文件编辑工具、`format_files`、两类 checkpoint restore、前台/后台 Bash、MCP、设备桥、委托执行器标准资源合同以及 Word/Excel/PowerPoint 文档 Skill 已验收，旧的文本、命令参数和 capability 名称推导代码已删除。未声明输出的任意命令和第三方执行器未通过结构化协议暴露的内部写入仍不在该保证范围内。
2. **SOP runtime 基础调度器及权限挂起链已落地。** `SopRunRecord` 已统一驱动条件分支、fan-out/fan-in、RetryPolicy、补偿与 verifier，并通过 Application/Store CAS 和真实 CLI 重启链验证。工具调用以结构化 `run_id + step_id` 关联精确步骤，权限等待由 Application 幂等挂起并在用户决策后恢复。仍未完成的是用户 replan、并行 worker 强杀与补偿失败叠加在同一条真实链上的联合矩阵。
3. **版本升级只允许显式 replan，尚无迁移协议。** 当前规则能拒绝静默升级并保留旧 revision；若未来支持运行中升级，必须定义类型化迁移、兼容检查、失败回退和用户可见 diff，不能由模型自行改版本。
4. **长任务联合故障矩阵仍不足。** 仍需覆盖多任务竞争、租约过期、worker/采样器强杀、权限等待后恢复、provider/network fallback、外部执行器断线，以及这些故障与 Plan/SOP stage 的联合恢复。
5. **真实 Artifact E2E 已覆盖主要内置与外部边界。** 写入、结构化多文件编辑、删除/移动、补丁、真实 formatter、Git/file-preimage restore、前台/后台 Bash、retained MCP、设备桥、真实委托执行器崩溃恢复，以及文档 Skill 的真实 OOXML 创建、重新打开验证、verifier evidence、事务重放和冷启动均已完成对应链路。后续扩展必须继续用资源级后置条件和冷启动证据，不能用模型自述或通用文件工具证据替代领域产物质量。

### 下一阶段实施顺序

1. 将已验证的文档 Skill Artifact/verifier 模式推广到其他会产生持久产物的内置 Skill，并逐个保留领域校验器、事务重放与冷启动 E2E；禁止重新引入基于文本或工具名的产物推断。
2. 让 `SopRunRecord` 驱动单分支、并行汇合、重试、补偿和 verifier 状态推进，每项先写失败场景再实现。
3. 将 Supervisor 故障矩阵与 Plan/SOP stage 联合验证，确认取消、权限等待和恢复不会重复已提交副作用。
4. 最后统一 CLI 时间线、`/work`、Debug 日志和 Web UI 的事件投影；在底层状态合同稳定前，不以 UI 补丁掩盖状态分裂。
# CLI SOP、能力与长任务改造总纲

> 本节作为后续实现、测试和验收的统一依据。它关注的是可泛化的 Agent CLI 能力，而不是某个单一故障的补丁。

## 1. 审计结论

当前 CLI 已具备交互式对话、工具调用、权限审批、后台 Shell、Agent 任务、上下文压缩和 Web Host 等基础能力，但这些能力仍偏向“功能集合”，尚未形成稳定一致的任务执行系统。与 Codex CLI、Claude Code、OpenCode 等成熟实现相比，主要差距不在工具数量，而在以下方面：

1. **SOP 缺少统一状态机**：计划、执行、等待权限、等待用户、后台运行、恢复和结束之间的边界仍不够严格。
2. **能力契约不够统一**：Shell、文件工具、Skill、MCP 和平台工具存在重复入口，名称、权限资源和 UI 表达容易分叉。
3. **长任务不是一等对象**：前台转后台、跨进程恢复、跨项目公平调度、进度观测和结果回收仍有薄弱环节。
4. **宿主与 Core 边界不稳定**：部分生命周期逻辑留在客户端循环中，导致 CLI、Web 和其他 Host 难以复用同一语义。
5. **真实端到端覆盖不足**：很多测试验证了函数返回值，却没有验证真实 PTY、真实进程、持久化数据库、权限挂起和恢复后的用户体验。

## 2. 对比参考项目后的关键不足

| 领域 | 成熟实现的共同特征 | Nola 当前不足 | 目标 |
|---|---|---|---|
| Agent 循环 | 单一 canonical run，事件可重放 | 展示状态、持久化状态和 provider 流偶有分叉 | 所有输出由同一 run/event 流派生 |
| 工具协议 | 工具 schema、权限资源、结果结构稳定 | Bash 与专用工具可能表达同一操作，权限匹配不统一 | Core 规范化具体调用，Host 只执行 |
| 权限 | 决策按具体资源生效，可挂起后原位恢复 | 自定义响应、会话允许、前缀匹配曾出现重复申请 | 权限是 durable suspension，不是失败重试 |
| 长任务 | task/run 有稳定身份、日志、取消和恢复 | 前后台任务历史上存在两套路径 | 统一 Supervisor 管理所有长任务 |
| 上下文 | canonical transcript 与模型窗口分离 | UI 估算、压缩记录和请求消息可能不一致 | Core 维护消息账本与压缩边界 |
| 流式恢复 | provider 中断后可判断是否安全续跑 | 半截流和副作用之间缺少完整恢复审计 | 以副作用边界决定重试、续接或终止 |
| 多项目调度 | 全局公平、项目隔离、租约明确 | 跨项目唤醒和 queued 状态曾被错误终止 | 原子 claim、durable wake、公平推进 |
| 可观测性 | UI 是状态投影，详细事件可展开 | 状态栏、工具行和正文可能重复或闪烁 | 稳定主行 + 可展开事件时间线 |
| E2E | 真进程、真终端、故障注入 | 静态测试和 mock 比例偏高 | 关键合同必须由真实 E2E 覆盖 |

## 3. 推荐的统一执行模型

CLI 不应围绕“这一轮打印了什么”组织，而应围绕一个持久化 `Run` 组织：

```text
UserInput
  -> RunCreated
  -> ModelDecision
  -> ToolCallProposed
  -> PermissionEvaluated
  -> ToolRunning | WaitingPermission | WaitingUser | Queued
  -> ToolResultRecorded
  -> ModelContinued
  -> AssistantMessageCommitted
  -> RunCompleted | RunCancelled | RunFailed
```

核心约束：

- 一个用户请求只能对应一个 canonical run。
- UI 中的思考、工具、权限和正文均由 run events 投影，不能各自维护事实来源。
- 每轮可见工具集在进入下一轮后清空；历史只保留调用名称、规范化参数、截断结果和 artifact 引用。
- 工具产生副作用前必须先写入可恢复事件；产生副作用后不得通过重新调用模型来猜测是否成功。
- `waiting_permission` 和 `waiting_user` 是可恢复状态，不是错误。
- 前台转后台只改变观察和调度方式，不创建第二个任务。

## 4. SOP 设计缺口

### 4.1 规划

当前不足：

- 简单问题和复杂任务之间缺少明确但非关键词驱动的执行策略。
- 大文件、长命令、学术检索等代价应在工具调用前由 schema、资源大小和预算评估，而不是失败后切换路径。
- Skill 的说明、输入输出和依赖不够结构化，模型难以可靠组合多个 Skill。

应实现：

- Core 根据模型给出的具体调用和结构化成本信息评估执行计划。
- 长任务在开始前生成阶段、预计资源、可取消点和产物合同。
- 数据源、定时任务、Skill 和 MCP 使用同一种 capability descriptor，不使用语言关键词路由。

### 4.2 执行

当前不足：

- Bash 与专用文件/搜索工具可能重复表达操作。
- 工具完成结果、状态栏和正文之间容易重复。
- 自动转后台和显式后台任务过去存在不同生命周期。

应实现：

- 工具调用进入 Core 后先规范化为具体资源和动作，再进行权限评估。
- Supervisor 是 Shell、Agent、Skill 长任务的唯一生命周期管理器。
- UI 默认展示当前阶段和关键参数，完整 stdout、stderr、工具 schema 和原始事件按需展开。

### 4.3 权限与用户介入

当前不足：

- “本次允许”“本对话允许”“自定义响应”曾出现语义不一致。
- 自定义响应有时被当成权限答案，而不是让 run 挂起并回到正常输入。

应实现：

- 权限决定绑定规范化后的动作、资源范围、会话和有效期。
- 自定义响应将 run 置为 `waiting_user`，下一条用户输入作为新的语义输入继续同一 run。
- 推荐模式默认允许可逆读取和普通写入，仅对删除、覆盖、外发敏感数据等不可逆操作申请确认。

### 4.4 完成与恢复

当前不足：

- provider 半截正文、推理和 native tool 参数流已通过真实 HTTP 截断及 canonical continuation E2E；CLI 异常退出和已执行副作用则按 durable event、checkpoint 与 reconciliation 合同恢复，第三方不可观察副作用仍禁止盲目重放。
- 完整 transcript、压缩摘要、UI 消息和模型请求可能出现不同步风险。

应实现：

- 先持久化完整 canonical message，再生成 UI 增量和模型上下文投影。
- 流中断时依据最后 durable event 和副作用边界恢复。
- `/resume`、`/goal resume`、重启恢复和 Web 重连复用同一套 run replay。

## 5. 长任务专项差距

### 5.1 已确认问题

- 旧 Host 通过固定周期全量 reconcile 推进任务，能唤醒但效率差，且不是显式调度协议。
- Shell summary 曾把合法的 `queued` 状态映射为失败，导致跨项目队列提前终止。
- 多 CLI 共用数据库时，需要 durable wake，同时必须避免一个项目使用另一个项目的 provider 配置执行 Agent。
- 长任务 UI 尚未形成稳定的“阶段、当前动作、耗时、产物、取消、恢复”模型。

### 5.2 目标架构

- Store 提供全局原子 claim、公平排序、租约和单调事件游标。
- 每个 Host 只执行自己项目可安全重建配置的任务。
- durable event cursor 负责跨进程通知，Store claim 负责最终互斥，通知不承担正确性。
- `queued`、`retry_waiting`、`waiting_permission`、`waiting_user`、`suspended` 都必须保持任务身份。
- 学术研究类任务允许更长预算；其他任务使用有界超时。超时策略由 capability 元数据决定，不按用户语言匹配。

### 5.3 公平性要求

- 同一项目内部按优先级、等待时长和入队顺序推进。
- 多项目共享全局槽位时不得长期饥饿。
- 等待权限的任务原则上不应占用实际执行槽位；批准后重新竞争槽位，但保持原 task/run identity。
- 任务取消、Host 崩溃和租约过期后必须可回收。

## 6. 能力系统重构方向

建议统一能力描述：

```text
CapabilityDescriptor
  id
  displayNameKey
  semanticDescription
  inputSchema
  outputSchema
  sideEffectClass
  resourceResolver
  executionMode
  timeoutPolicy
  resumability
  platformAvailability
```

由模型根据 schema 选择能力，Core 对具体调用做规范化、权限和调度判断。以下做法禁止继续出现：

- 根据“搜索”“当前”“写文件”等词强制路由。
- 为中文、英文或其他语言维护独立工具排除表。
- 客户端改写用户 query 后强制调用某工具。
- 用 Bash 包装已有专用工具，绕开统一权限资源。
- 因 unknown tool 直接终止整轮，而不向模型返回可恢复协议错误。

## 7. 分阶段实施优先级

### P0：正确性

- 统一 canonical run/event/message 账本。
- 完成 Shell、Agent、Skill 的 Supervisor 单路径。
- 修复 queued、waiting permission、cancel 和 lease 的状态合同。
- 完成 provider 半截流与副作用恢复策略。
- 保证 Esc、权限自定义输入和重启恢复不会丢失 run。

### P1：可靠性

- 全局公平 claim 与 durable wake。
- 多项目、多进程和 Host 崩溃恢复。
- 工具结果截断、artifact 引用和完整日志持久化。
- 上下文压缩由代码确定边界、LLM 生成摘要，并保留可重放 transcript。

### P2：用户体验

- 前台与后台统一进度视图。
- 稳定状态主行，不因相同描述重复刷新。
- 工具过程默认可见，细节可展开。
- debug 与普通模式共享布局，仅增加诊断字段。

### P3：能力扩展

- 数据源、Skill、MCP、定时任务统一编排。
- 长任务产物、通知和跨客户端查看。
- Web Host、macOS 和移动端复用 Core 的 run/event 合同。

## 8. 必须具备的真实 E2E

以下用例不能只由 mock 或静态断言替代：

1. 两个真实 CLI、两个项目、同一数据库、一个全局槽位，验证无额外 Enter 也能按公平顺序推进。
2. Shell 与 Agent 竞争同一槽位，验证共享 Supervisor 限制。
3. 前台运行时按 Ctrl+B，验证同一 task identity 转后台并最终回收结果。
4. 运行中按 Esc，验证进程停止、状态落库、UI 恢复输入。
5. 权限选择“本对话允许”后重复访问同一规范化资源，不再弹窗。
6. 自定义响应挂起后，下一条自然语言输入继续原 run。
7. Host 在工具产生副作用后、模型继续前崩溃，重启后不得重复副作用。
8. provider 在 thinking、tool call、assistant text 不同阶段断流，分别验证恢复策略。
9. 触发上下文压缩后继续多轮，验证 transcript、摘要、窗口占比和模型请求一致。
10. 一千万字符会话、多次工具与 Skill 事件、多个长会话切换，验证内存、首屏时间和交互延迟。
11. CLI 启动 Web Host 后断线重连，验证同一 run 的事件顺序、权限和消息完整性。
12. 多语言自然跟进、模糊词和架构审计问题，验证不会触发关键词强制路由。

## 9. 验收指标

- **正确性**：消息不重复、不覆盖、不丢失；副作用至多执行一次。
- **恢复性**：CLI/Host 异常退出后，所有非终态 run 可解释、可恢复或可明确终止。
- **公平性**：高竞争多项目测试中无永久饥饿，排序符合调度合同。
- **响应性**：空闲时不进行固定周期全量 reconcile；事件推进由 durable cursor 驱动。
- **一致性**：普通/debug、CLI/Web/macOS 对相同 run 使用相同状态和消息语义。
- **可观测性**：用户能看到当前阶段、关键工具和等待原因；诊断日志能定位同一 run/task/event。
- **测试真实性**：P0/P1 合同至少各有一个真实进程或真实 PTY E2E，不以仅检查日志字符串代替行为验证。

## 10. 后续执行原则

后续修复应按“合同 -> Store/Core -> Host adapter -> UI projection -> 真实 E2E”的顺序进行。不得用客户端条件分支、语言关键词、额外轮询或重复状态缓存掩盖根因。每完成一项，需要在本文档中记录：问题证据、修改范围、真实测试命令、结果和尚未覆盖的风险。

## 11. 实施台账（2026-08-27）

### 11.1 跨项目 durable queue

状态：**真实 Shell 与混合跨项目 Agent/Shell 基础 E2E 均已通过。**

已修复：

- Store 使用全局原子 fair claim，从每个项目的队首中选择全局候选。
- 活跃 CLI 通过 durable task event cursor 感知其他进程提交的调度变化，不再每 250ms 执行一次无条件全量 reconcile。
- 删除旧 Shell summary 对 Supervisor 管理任务的状态写入权限。旧路径会用陈旧快照覆盖 worker 已提交的终态，并生成重复 `finished` 事件。
- `queued` 不再被 status 文件投影误判为失败。

真实证据：

```text
two_idle_clis_advance_the_global_fair_queue_without_another_enter ... ok
idle_cross_project_agent_follows_shell_through_the_same_global_slot ... ok
```

该测试运行两个真实 CLI、两个项目和同一数据库，在一个全局槽位下验证：

- 不需要额外 Enter 唤醒；
- 执行顺序为 A1 -> B1 -> A2；
- 三个 durable Shell task 均提交终态；
- 每个 task 恰好一个 `finished` event；
- 不存在跨槽位重叠或项目 B 饥饿。

混合测试还证明：项目 A 的 Shell 占用唯一全局槽时，项目 B 的 Agent 保持 queued；Shell 提交终态后，在不向任一 CLI 再输入字符的条件下，Agent 使用自身持久请求启动并完成，`agent.started_at_ms >= shell.ended_at_ms`，两类任务均只有一个 `finished` event。

### 11.2 统一 Agent/Shell Supervisor 槽位

状态：**同一 Host 与跨项目双 Host 均已验证基础单槽合同。**

真实证据：

```text
background_shell_and_agent_share_the_same_real_concurrency_slot ... ok
```

Store 调度合同：

```text
runtime_task_e2e: 8 passed; 0 failed
```

跨项目证据为 `idle_cross_project_agent_follows_shell_through_the_same_global_slot`。仍需补高竞争、Host/worker 强杀和 Goal/Skill 参与竞争的扩展矩阵，不能用基础单槽用例扩大结论。

### 11.3 权限等待与执行槽位

状态：**Store 合同与真实 Agent 权限恢复已验证。**

原问题：`waiting_permission` 保留 worker 租约且计入 running count，用户未决策时会占住全局槽位；如果只从计数排除而批准后直接恢复，又会造成瞬时超配。

当前合同：

- 等待权限时保留 worker identity、ownership token 和 heartbeat；
- `waiting_permission` 不占执行槽；
- 具体权限获批后设置 durable resolved 标记；
- 原 worker 必须通过 Store 的原子 fair resume 重新竞争全局和项目槽位；
- 只有 resume 成功后 Core task 才恢复为 Running；
- worker 退出或租约过期仍走原 task identity 的恢复流程。

证据：

```text
permission_wait_releases_the_slot_and_resumes_through_the_fair_queue ... ok
background_agent_reuses_permission_panel_and_task_scoped_approval ... ok
```

### 11.4 provider 半截流

状态：**根因已修复，半截 native tool、正文和推理的真实 HTTP 截断及 canonical continuation 均已通过。**

已确认问题：低层 `stream_chat` 将“带工具 schema 的整轮”统一视为可重试，即使正文或推理 delta 已经交给消费者。整轮重采样没有撤销已发布内容，会造成重复、拼接或 presentation gate 状态污染。仅有半截工具参数时可以安全丢弃并重试，但必须清空失败 attempt 的本地呈现状态。

当前修复：

- 一旦某 attempt 发布正文或推理 delta，禁止在低层整轮重采样；由上层 continuation 协议处理 `stream_incomplete`。
- 仅连接前失败或仅收到未提交工具参数片段时允许 bounded retry。
- 安全重试前清空 buffered text、reasoning 和 tool-round presentation gate，避免第一次的半截工具调用吞掉第二次正常正文。
- 工具副作用仍只在完整、校验后的 `ModelCompleted.tool_calls` 进入 Core 后执行，partial tool arguments 不成为可执行事件。

已验证：

```text
real_http_partial_tool_attempt_is_discarded_before_clean_retry ... ok
real_http_visible_text_partial_is_not_whole_request_retried ... ok
real_http_visible_reasoning_partial_is_not_whole_request_retried ... ok
real_http_rate_limit_retries_once_without_publishing_partial_output ... ok
real_http_unauthorized_fails_without_retry_or_visible_output ... ok
provider_retry_never_replays_already_published_text_or_reasoning ... ok
retry_after_partial_tool_transport_does_not_suppress_the_clean_answer ... ok
incomplete_visible_stream_continues_into_one_canonical_assistant_message ... ok
incomplete_reasoning_stream_continues_without_replaying_private_reasoning ... ok
```

真实 HTTP 测试声明大于实际响应的 `Content-Length` 后提前断开，覆盖 reqwest 的真实 EOF。未终止的 `write_file` 参数流恰好安全重试一次，partial call 不进入可执行 ToolCall；已经发布正文或推理的 attempt 不做整轮重采样，而是进入上层 continuation。真实 PTY 与 SQLite 进一步证明两次 provider 请求最终只形成一条完整 assistant message，私有 reasoning 不被复制进续写请求。独立 TCP Provider 还验证 `429 + Retry-After-Ms` 只产生一次 bounded retry，且重试前不发布正文；`401` 立即失败，不产生第二次请求或任何可见 delta。

仍待验证：provider 在传输层报告成功、但其自身在外部系统执行了不可观察副作用后才返回半截协议的第三方边界；该情况不能自动推断为安全，必须继续进入 reconciliation。

### 11.5 副作用 checkpoint 与外部执行器恢复

状态：**受 Nola 事务边界控制的外部执行器与 MCP 路径已验证；任意未声明输出命令仍不得宣称 exactly-once。**

本轮重新执行并通过：

```text
real_external_process_crash_resumes_one_session_without_repeating_its_side_effect ... ok
real_codex_app_server_crash_resumes_one_thread_without_repeating_its_side_effect ... ok
real_opencode_acp_crash_resumes_one_session_without_repeating_its_side_effect ... ok
retained_mcp_process_mutation_is_transactional_and_replayed_without_second_call ... ok
work_step_transaction_e2e: 4 passed
```

这些测试使用真实子进程、持久数据库和副作用计数文件，覆盖 checkpoint 后进程异常退出、复用原 backend session、迟到/重复终态事件以及 committed result 重放，副作用计数均保持为一次。无法预枚举资源或第三方内部不可观察的副作用继续采用“禁止盲目重放、进入 reconciliation/人工介入”的合同。

### 11.6 一千万字符会话与跨会话切换

状态：**CLI 与 CLI Web 的无模型性能基线已通过。**

`two_ten_million_character_sessions_restore_and_switch_with_bounded_terminal_views` 构造两个各一千万字符的真实 canonical conversation，包含文件 Tool、Skill 和运行中 Work，然后连续切换 12 次。当前实测：

```text
resume list: 1.43 ms
cold restore: 81.47 ms
switch average: 80.87 ms
switch p95: 82.63 ms
Work query p95: 1.46 ms
durable steering p95: 1.23 ms
terminal projection: 160624 bytes
```

测试同时断言 resume 列表不读取大正文、终端投影小于 400 KB、工具和 Skill 时间线仍可见、每次切换只投影当前会话 Work。恢复列表已从“Application 全量扫描 + 每会话三次查询”的 N+1 路径改为单条有界 SQL 投影，并加入一个更新时间最新但无消息的会话作为回归：空会话会被过滤，不能再因循环提前 `break` 而遮蔽两个更旧的有效历史会话。`two_ten_million_character_web_conversations_restore_through_bounded_windows` 还验证 CLI Web 对两个一千万字符会话按 24 条窗口分页，首窗不超过 60 万字符，并可无遗漏恢复全部一千万字符。

`five_thousand_conversations_list_without_n_plus_one_queries` 进一步构造 5,000 个 canonical 会话，混合两个项目、空会话和有效用户消息。项目内最近 128 条恢复列表在约 12.45 ms 内返回，排序、项目隔离和空会话过滤均通过，证明恢复列表耗时不再随“每会话额外查询次数”线性放大。

`ten_thousand_tool_events_restore_through_a_bounded_terminal_window` 进一步构造 10,000 个交替的 ToolCall/ToolResult canonical 消息。当前实测：

```text
restore: 13.11 ms
projected messages: 96
serialized projection: 12721 bytes
rendered lines: 49
```

测试断言最新工具调用可见、完整结果仍保存在有界 canonical 投影中、原始工具结果正文不泄漏到紧凑历史、最旧事件不会逸出最近窗口，恢复耗时小于 1 秒。恢复热路径同时移除了“先通过另一连接扫描全部会话、再读取最近消息”的双快照实现；会话时间、总消息数、用户轮次和最近窗口现在由同一个只读 SQLite 事务读取，避免并发写入时计数与内容错位，并将万级事件实测恢复时间从约 37.83 ms 降到约 13.11 ms。该证据覆盖 Store/投影、两个千万字符会话切换、万级工具事件、连续真实 PTY resize 和受控 Rust 活跃分配预算；系统总 RSS 边界见 11.19 的限制说明。

### 11.7 SOP runtime 边界

状态：**基础可执行运行时及真实重启链已闭合，复杂联合故障矩阵仍进行中。**

`SopRunRecord` 现由 Core 持有条件决议、每次 attempt、延迟重试、并行 fan-out/fan-in、步骤与全局 verifier、补偿、暂停/恢复/取消以及 Artifact/Evidence 引用。步骤挂起不再只保存一个宽泛状态，而是保存 `Permission`、`UserInput` 或 `External` 类型、权威 `referenceId`、稳定原因和请求时间；恢复必须提交 `Continue` 或 `Replan` disposition，当前等待随后原子转入带证据引用的历史。Application 固定创建时的 SOP 定义版本，并通过 Store revision CAS 提交每个类型化事实；CLI Host 的 `sop_run_update` 不能自行改写状态或伪造 revision。`sop_runs` 是权威持久记录，owner session 只表示 lineage，访问范围由规范化 workspace 决定。

模型可见工具 schema 现在统一提供可选 `_nolaExecution { sopRunId, sopStepId }`。Core 在进入业务 schema 校验前提取该内部 envelope，业务参数、权限资源计算和工具实现均看不到它；关联单独保存在 `ToolCallState`、`PendingPermissionRequest` 和 checkpoint 中，只在 Core 发往 Host 的权限/执行 effect 上重新附加。Application 的 `ValidateInvocation` 按规范化 workspace、精确 Run 和精确运行中 Step 校验，CLI Host 不再猜测“当前步骤”。两个并行 Step 可同时通过各自身份校验，未知 Step、跨 workspace 和非运行状态均返回 `invalid_sop_execution_lineage`，不会执行底层工具。

真实 PTY `durable_sop_run_crosses_the_real_host_and_resumes_after_cli_restart` 启动第一个 CLI，通过真实模型 ToolCall 创建 Run、开始步骤并挂起到稳定的外部请求。`/work` 立即显示 SOP、当前步骤和等待原因。退出后第二个 CLI 在任何恢复模型调用前仍看到同一等待；新回合获得有界 `nola-active-sop-runs-v1` 投影，其中包含同一 external reference。模型随后先提交指向不存在 Step 的真实文件工具调用，Host 返回可恢复的 `invalid_sop_execution_lineage` 且不执行工具；下一轮以正确身份调用 `write_file`，CLI 显示真实权限面板，用户选择仅本次允许后 Application 将同一步骤从 `waiting_permission` 恢复并完成写入。SQLite 最终只有一个 Run、一个 attempt、external 与 permission 两条已解决 wait history，文件内容也验证成功。Core/Application 测试另行覆盖条件、并行、一次失败重试、fan-in、verifier、补偿成功/失败、类型化 permission/user wait、replan 证据、幂等权限等待、精确并行步骤关联与数据库冷重开。

显式 invocation correlation、权限挂起、错误关联恢复、并行步骤精确校验，以及“用户 replan + worker 强杀 + 补偿失败”的联合恢复矩阵均已闭合，证据分别见本节与 11.13。尚未关闭的是多项目高竞争叠加 Host/worker 强杀、不可预枚举外部副作用，以及运行中定义升级的显式新 Run UX。运行中版本升级仍禁止静默迁移，只允许显式创建新 Run 并保留旧审计记录。

当前回归证据：`nola_core` 241/241、`nola_store` 53/53、两个 CLI binary 各 820/820、真实串行 PTY 61/61，全部通过。PTY 统计包含上述故障注入、权限等待、冷重启链、11.14 的正常条件/并行/verifier 链和 11.15 的跨项目混合压力强杀链；测试数据库最终断言 revision=7、attempt=1、resolved wait history=2。模型工具 schema 变化使用测试自带生成器更新 canonical Part lifecycle fixture，并在完整 Core 回归中再次验证无漂移。

### 11.8 Goal、Skill 与混合 worker 竞争

状态：**控制合同与执行资源的边界已验证。**

`goal_skill_shell_and_agent_share_one_supervisor_slot` 使用真实 PTY、真实 CLI Host、本地 TCP Provider 和持久数据库启动 Goal，激活内置 `analyze-data` Skill，并在同一模型轮次派生后台 Shell 与委托 Agent。全局与项目并发槽均设为 1。测试验证：

- Skill 激活信息进入后续模型请求，Goal/Skill lineage 未丢失；
- Shell 与 Agent 均成为 durable task，而 Goal/Skill 本身不伪占执行槽；
- Agent 的 `started_at_ms` 不早于 Shell 的 `ended_at_ms`，证明二者共享同一全局 Supervisor 槽；
- 两个 worker 各自恰好提交一个 `finished` terminal event；
- Goal 可在 worker 运行期间稳定进入 `needs_user`，不破坏后台执行。

这关闭了“Goal/Skill/后台 Bash/Agent 单槽竞争”基础矩阵，但不替代多项目高竞争、运行中 Host 强杀与 worker 强杀组合压力测试。

### 11.9 权限挂起与用户取消

状态：**CLI 基础挂起协议已通过真实 PTY。**

本轮重新执行：

```text
permission_custom_response_suspends_the_tool_until_the_user_answers ... ok
goal_resume_restores_pending_permission_before_any_parent_model_request ... ok
escape_interrupts_a_live_stream_and_returns_to_the_composer ... ok
```

结合 `permission_wait_releases_the_slot_and_resumes_through_the_fair_queue`、`background_agent_reuses_permission_panel_and_task_scoped_approval` 和跨进程 typed deny/session grant 证据，CLI 的 custom response 会挂起原调用并回到主输入，持久权限在重启后先于父模型请求恢复，等待权限不占执行槽，Esc 可取消活动流并回到 composer。仍需跨客户端共享的是同一 Core/Application effect 决策合同，而不是复制 CLI 输入状态机。

### 11.10 代码与 LLM 联合上下文压缩

状态：**手动压缩与多周期自动压缩的真实续轮、活动 Goal/Permission 冷恢复均已验证。**

`compact_command_animates_until_the_summary_finishes` 现在不只检查动画。真实 PTY 先完成两轮对话，`/compact` 由代码选择归档边界并向真实本地 TCP Provider 发送包含 `loss-minimizing continuation checkpoint` 和 `Archive JSON` 的摘要请求；Provider 返回结构化 Goal/Next Steps 摘要后，测试再发送一轮用户输入，并断言下一次模型请求实际包含该 LLM checkpoint 与新输入。`skill_decision_survives_completed_tool_transport_compaction` 和 `compaction_prompt_requires_verbatim_continuation_anchors` 同时验证 Skill 决策与 continuation 锚点不会因已完成工具 transport 压缩而丢失。

这证明手动路径符合“代码确定边界、LLM 生成摘要、完整 transcript 保留重放、后续轮次消费摘要”的合同。`repeated_auto_compaction_preserves_goal_permission_and_exactly_once_resume` 进一步关闭了多次自动压缩叠加活动 Goal、权限挂起、Host 强杀与冷恢复的基础长周期门禁，详细故障与证据见 11.20。仍未覆盖的是公共 Provider 非确定性故障、外部执行器内部不可枚举副作用和跨平台系统资源与多任务联合压力。

### 11.11 长流式输出与 Markdown 终端投影

状态：**固定终端尺寸下的真实 PTY 基线已通过。**

```text
long_stream_remains_in_scrollback_and_never_reenables_a_scroll_region ... ok
streamed_ragged_markdown_table_does_not_split_into_an_empty_table_and_raw_rows ... ok
```

长流不会重新启用只保留末屏的 scroll region，历史正文保留在终端 scrollback；不完整 Markdown 表格在流式边界不会先生成空表再泄漏 raw rows。

`dynamic_pty_resize_preserves_draft_stream_cancellation_and_next_command` 进一步通过真实 PTY 的 `TIOCSWINSZ` 和 `SIGWINCH`，在同一 CLI 进程内依次执行 `100×24 -> 42×12 -> 132×36 -> 48×14 -> 120×32`。测试断言输入草稿与 footer 在窄/宽窗口间保留，流式正文在 resize 后继续输出，Esc 仍可取消，随后 `/help` 和 `/exit` 保持可用，且没有重新启用 terminal scroll region。CLI composer 现在显式消费 `Event::Resize`，清除旧几何帧后按实时终端尺寸重绘同一 draft/cursor/footer 投影。

相关回归同时通过：

```text
composer_edits_at_the_visible_input_cursor_and_keeps_command_loop_alive ... ok
long_stream_remains_in_scrollback_and_never_reenables_a_scroll_region ... ok
streamed_ragged_markdown_table_does_not_split_into_an_empty_table_and_raw_rows ... ok
```

因此动态 PTY resize 的基础运行门禁已关闭；权限面板和后台任务详情的 resize 瞬间已补充真实 PTY 帧级门禁，详见 11.18。受控 Rust 活跃分配预算见 11.19；跨终端实现的像素级截图比较仍属于 P3 扩展矩阵。

### 11.12 后台 Shell 资源预算

状态：**CPU、RSS、进程数、墙钟与输出预算的真实 worker 基线已完成。**

`background_shell_cpu_memory_and_process_budgets_are_enforced_by_real_workers` 在同一真实 CLI Turn 中启动三个后台 Shell：CPU 忙循环、64 MB 常驻 Python 进程和创建三个子进程的 Python 父进程。三个任务分别设置 `max_cpu_time_sec`、`max_memory_bytes` 和 `max_processes`，真实 worker 在约 0.8–1.7 秒内终止超限进程树，三条 durable task 均进入 `failed`，且 `_nola_scheduler.budget_exhaustion` 分别保留 CPU-time、memory 与 process-count 证据。结合 `background_shell_output_budget_is_enforced_by_the_real_worker` 和既有 wall-clock timeout，Shell 基础资源治理已与 Agent 使用同一预算合同。
## 2026-08-28 implementation and E2E update

This pass converted several previously inferred scheduler guarantees into exercised CLI behavior without changing `core/`:

- Agent creation now persists a queued Host task first, commits `TaskPromoted` through the Application/Core transaction, and only then allows the Supervisor to claim it. Workers can no longer publish `Running` or `Completed` ahead of the queued Core projection.
- The committed promotion is recorded per task as `core_promoted`. A multi-tool response therefore cannot start task B when only task A's promotion has committed. Persisted sessions may additionally recover the same fact from Core's task projection; ephemeral sessions use the committed Host marker.
- Agent heartbeat cadence is derived from the lease duration instead of being fixed at ten seconds. Short leases used by recovery tests no longer cause healthy workers to be reclaimed repeatedly.
- Idle interactive CLIs retain event-cursor wake-ups and perform a bounded one-second authoritative-state reconciliation. This covers external durable commands and the cross-process edge where a terminal event becomes visible immediately before its shared scheduler slot is released.
- The global Supervisor was exercised with 12 real background Shell tasks under a concurrency limit of three. All tasks completed, observed overlap was at least two, maximum overlap never exceeded three, and each task emitted exactly one terminal event.
- Agent recovery was exercised repeatedly with two independent durable child sessions after the original Supervisor process tree was killed. Child-session identity remained stable and both results were recovered without duplicate claims.
- Cross-project mixed scheduling was exercised with a running Shell task in one CLI and a queued Agent task in another CLI sharing a single global slot. The Agent started only after the Shell released the slot, without additional keyboard input.
- `pty_interaction` 当前包含 63 项并全量通过。新增 SOP 双进程重启场景验证 `/work`、有界运行投影和同一 Run 继续执行；新增正常条件/并行/verifier 链验证 fan-in 门禁；新增两个真实 CLI、两个项目、12 个混合 worker 和一次真实 worker 强杀的联合压力链，并增加授权对等 Host 在原 Host 整体强杀后的混合任务接管，以及双项目主备 Provider 并发故障链。最终全套结果为 63 passed / 0 failed。

The passing evidence is not a declaration that every gap in this audit is closed. The first-class SOP base runtime is now implemented and exercised across a real CLI restart. The combined SOP permission/replan/kill matrix is closed by the evidence below; broader mixed-project pressure stability and screenshot-level resize coverage for permission/background-detail panels remain open. Cross-provider fallback while a durable Shell task is active is covered by a real two-provider PTY E2E; broader multi-project/provider pressure remains part of the stress matrix rather than a missing base protocol.

### 11.13 SOP 调用谱系、运行时对账与补偿失败闭环

状态：**权限、后台 worker 故障、用户重规划与补偿失败的组合矩阵已通过真实 PTY。**

本轮不再依赖当前 UI、工具名称或“当前步骤”推断后台任务属于哪个 SOP 步骤。Host 在 Core 接受具体 ToolCall 后生成 `ToolInvocationContext { sop_run_id, sop_step_id }`，并通过 call-local scope 传递给工具执行器；后台 Bash 和委托 Agent 将同一 identity 持久化为任务元数据中的 `_nolaExecution`。业务工具参数在进入执行器前移除该内部字段，只有 Host effect 携带经验证的谱系，防止模型伪造或业务参数泄漏。

Application 新增幂等 `EnsureWait` 命令。相同 `(kind, reference_id)` 的恢复重放不增加 revision；只有 identity 精确匹配且仍在运行的步骤可以新建等待。worker lease 在命令开始后过期时，Supervisor 会先终止真实进程组，再按任务中持久化的谱系把对应步骤挂起为 `user_input` reconciliation；如果 Application 无法提交等待，任务不会提前伪装为已完成或可安全重试。

`killed_shell_worker_after_command_start_requires_reconciliation_without_replay` 现在覆盖以下单条真实链路：

```text
建立真实 CLI conversation owner
  -> 创建含 anchor 与两个并行 worker 的 durable SOP
  -> Bash 权限请求挂起 worker-a
  -> 用户批准后恢复同一步骤和同一调用
  -> 后台 Bash 产生一次可观察副作用
  -> SIGKILL worker，Supervisor 终止其命令进程组
  -> worker-a 进入持久 reconciliation wait，worker-b 保持运行
  -> /work 展示完整不确定性原因
  -> 用户选择 replan
  -> 恢复并失败 worker-a
  -> Core 取消不可能继续的 worker-b
  -> 启动 anchor 的声明补偿
  -> 补偿失败，Run 进入 failed
```

最终数据库断言 revision=12，权限等待与 reconciliation 等待均保留在 `waitHistory`，对账等待 disposition 为 `replan`，失败步骤、取消的并行步骤和失败补偿均为确定终态，补偿 attempt=1。任务元数据仍指向原 `run_id/worker-a`，`reconciliation_required` 事件恰好一条，命令 PID 已终止，文件副作用内容恰好一份，没有自动重放。

该 E2E 同时暴露并修复了 Core 的终态不变量：过去一个并行步骤进入不可重试失败和补偿后，其他步骤可能继续显示 `running`。现在进入补偿时，所有 queued/running/waiting/verifying 同伴统一转为 `cancelled`，未结束 attempt 记录结束时间和结构化取消原因；已完成步骤仍保留，作为补偿选择依据。

最终验证：Core 全量通过（`nola_core` 241/241、`nola_store` 53/53 及全部集成测试），两个 CLI binary 各 820/820，真实 PTY 串行 63/63。Core 的虚拟六小时性能门禁在与 CLI 全量测试并行争用 CPU 时曾超时，停止竞争后串行全量立即通过；没有放宽阈值或删除性能断言。

### 11.14 SOP 正常条件/并行/verifier 链与任务终态原子性

状态：**正常执行链已通过真实 Host；Shell 终态投影与事件不再撕裂。**

`conditional_parallel_sop_requires_fan_in_evidence_and_verifiers_in_the_real_host` 通过真实 PTY 和 Application 工具合同执行一条非故障 SOP：可选条件解析为 `false` 并进入 `skipped`，两个并行步骤同时进入运行态，右分支先完成；此时提前启动 join 必须返回 `step_not_runnable`，且 Run revision 不得变化。左分支完成后 join 才可运行并提交 Evidence，随后步骤 verifier `join-check` 和全局 verifier `final-check` 依次通过，Run 最终进入 `completed`。数据库断言 revision=10、两个 verifier 均为 `passed`，证明条件、fan-out、fan-in、Evidence、步骤 verifier 与最终 verifier 不是孤立单元能力，而是同一真实 Host 链上的执行门禁。

该用例与完整 PTY 回归还暴露了一个此前被“增加等待时间”掩盖不了的任务终态竞态：Shell 已写入唯一 `finished/completed` 事件，状态文件也为 `exited(0)`，但 UI 通知线程持有旧的 `running` 快照，并通过整行 `Save` 把 `durable_tasks.status` 覆盖回 `running`。结果是命令真实成功、事件真实完成，而 `/tasks` 和等待逻辑永久看见运行中。

修复后的提交边界为：

1. worker 在仍持有 ownership lease 时准备有界结果摘要、输出路径、资源诊断和 unread 状态；
2. Application 的 `FinishWithEvent` 在同一 SQLite immediate transaction 中校验 ownership，一次性提交 terminal status 和唯一 terminal event；
3. UI 完成通知使用 `AcknowledgeNotification` 对 `metadata_json.ui_notified` 做字段级原子更新，不再读取旧任务后整行回写；
4. 自动后台 grace 从“任务入队时间”改为“首次观察到真实 command started”，worker 排队或系统负载不再把短命令错误提升为后台任务；
5. `terminal_event_and_notification_ack_are_atomic_and_never_restore_a_stale_status` 直接保留旧 running 快照，先完成任务再确认通知，断言任务仍为 `completed`、terminal event 恰好一条、`ui_notified=true`。

真实混合回归 `goal_skill_shell_and_agent_share_one_supervisor_slot` 随后稳定通过：Shell 完成释放统一槽位，Agent 才开始，终态不会再回退。最终全量结果为 Core 全套通过、两个 CLI binary 各 820/820、真实串行 PTY 63/63。多项目混合 Agent/Shell 高竞争叠加单 worker 强杀由 11.15 闭合，Host 整体强杀后的授权跨项目恢复由 11.16 闭合，多 Provider 联合故障由 11.17 闭合。

### 11.15 多项目混合高竞争与 Shell worker 强杀

状态：**两个真实 Host 的混合压力与单 worker 强杀门禁已关闭；Host 整体强杀接管见 11.16。**

`mixed_cross_project_contention_survives_one_shell_worker_kill_without_starvation` 启动两个真实 CLI，分别绑定两个项目并共享同一 SQLite 调度账本。项目 A 提交 6 个后台 Shell，项目 B 提交 6 个委托 Agent；全局限制为 3 槽，每项目限制为 2 槽。测试等待指定长 Shell 的 worker PID 与 command PID 均真实出现，再校验 `SIGKILL` 返回成功。最终结果为 11 个任务 `completed`、被强杀 Shell `failed` 且仅有一条 `reconciliation_required`，所有 12 个任务均存在开始和结束时间，没有 queued/running 遗留。

测试按任务时间区间分别重建全局和每项目并发曲线，断言全局峰值不超过 3、每项目峰值不超过 2，且两者实际峰值均至少为 2，避免“完全串行所以自然不超卖”的伪通过。每个正常任务恰好一条 `finished` 事件；故障任务恰好一条 reconciliation 事件。长 Shell 的文件只包含 `started`，command PID 已不存在，证明不确定副作用没有被自动重放或放任完成。

第一次真实执行暴露了生产根因，而不是夹具等待不足：`NOLA_SHELL_TASK_LEASE_MS` 最小可设为 1 秒，但 Shell worker 心跳固定每 10 秒发送一次。另一个活跃 Host 会把健康短租约 worker 当作过期任务，杀掉其命令；原 worker 随后又提交普通 `finished/failed`，造成两个终态所有者竞争。Shell 心跳现与 Agent 使用同一派生规则：间隔为 lease 的三分之一，并限制在 100 ms 到 10 秒。修复后，健康 worker 持续续租；只有测试明确强杀的 worker 在 lease 到期后进入一次 reconciliation。

完整回归还暴露了两个进程所有权问题。第一，隔离 worker 过去只有独立进程组，仍属于交互终端 session，CLI 退出时后台任务可能收到挂断信号；worker 现通过 `setsid()` 脱离终端。第二，租约回收过去只信号 command PGID，无法兜底覆盖逃逸后代；终止路径现先快照 PPID 后代树，再同时信号进程组和每个后代 PID。压力夹具也不再用接近排队耗时的 8 秒命令制造故障窗口，而使用明确长于整个排队阶段的命令，保证 `SIGKILL` 注入发生在执行中，不能以自然完成伪装成回收。

本轮最终验证为两个 CLI binary 各 820/820、真实串行 PTY 63/63。该证据关闭多项目混合队列、高竞争、公平推进、槽位不超卖和单 Shell worker 强杀的联合门禁；多 Provider 同时故障见 11.17。

### 11.16 授权跨项目 Host 整体强杀接管

状态：**原 Host 整体进程树强杀后的授权对等 Host 接管门禁已关闭。**

`authorized_peer_host_takes_over_mixed_tasks_after_the_origin_host_is_killed` 使用共享 canonical SQLite 启动两个真实 CLI Host。项目 A 的原 Host 先提交两个 Shell 与两个委托 Agent，并确认长 Shell 已启动、至少一个 Agent 子会话的 provider 请求已在途；项目 B 的对等 Host 随后启动，且只通过显式 `--allow-dir project-a --allow-dir project-b` 获得接管边界。测试强杀原 Host 整棵进程树，不向剩余 Host 发送额外业务输入。

Supervisor 不会扫描数据库后执行任意 workspace。它只为当前 `project_root` 和规范化后落在显式 `allowed_dirs` 内的非终态任务构建 workspace 配置；未授权路径即使存在于共享数据库也不会被领取。接管后的断言包括：

1. 已开始且副作用不确定的 Shell 只产生一次 `reconciliation_required`，文件只有 `started`，不会重放或补写 `finished`；
2. 已正常完成的短 Shell 恰好保留一条 `finished`；
3. 原 Host 已领取的 Agent 保留同一 `child_session_id`，租约恢复后 `retry_count >= 1` 并完成；
4. 强杀时仍排队的 Agent 同样保留稳定 child identity，由对等 Host 首次领取，`retry_count = 0`；
5. 四个任务全部进入可解释终态，没有 queued/running 遗留或重复 terminal event。

该真实门禁最初发现 Supervisor 只对当前 Host 的 `project_root` 运行 orphan recovery，导致共享账本中的授权跨项目任务永久滞留。修复将 workspace 发现纳入 Supervisor reconciliation，同时保持显式目录授权边界。定向接管测试耗时约 7 秒；最终完整验证为 `nola` 820/820、`nola-agent-host` 820/820、真实串行 PTY 63/63。该结论不扩展到未授权 workspace。

### 11.17 双项目多 Provider 故障与持久任务一致性

状态：**主 Provider 并发故障叠加持久 Shell、委托 Agent 和共享 Supervisor 的真实门禁已关闭。**

`concurrent_cross_project_provider_fallback_preserves_tasks_and_side_effects` 启动两个真实 CLI、两个项目、四个本地 Provider listener，并共享同一 canonical SQLite。全局 Supervisor 为 2 槽、每项目 1 槽。项目 A 先由主 Provider 创建一个真实后台 Shell；命令运行期间下一回合的主 Provider 返回 503，显式配置的 fallback 返回正文。项目 B 的父模型 proposal、父 continuation 和两个委托 Agent 子会话均先收到主 Provider 503，再由自己的 fallback 完成。

该测试不通过 prompt 关键词选择 Provider；Host 按配置中有序的 fallback 合同处理具体、可重试的 503，且只在当前候选尚未发布用户可见正文时切换。测试断言：

1. 项目 A 只有一个稳定 Shell task identity，声明的文件副作用内容恰好一份；
2. 项目 B 只有两个稳定 Agent task，父响应和两个子结果均不重复；
3. 三个任务全部完成，每个任务恰好一条 `finished`，没有 fallback 造成的重复 terminal owner；
4. 两个项目在共享全局槽位下均推进完成，没有 Provider 故障把 RuntimeTask 重新创建或绕过项目上限；
5. 失败候选的内部重试已通过 `NOLA_LLM_MAX_RETRIES=0` 关闭，证据只验证跨 Provider fallback，而非同 Provider retry 偶然成功。

首轮测试还验证了 CLI steering 边界：只看到“任务已排队”正文但尚未回到输入态时发送文本，会作为运行中 steering 入队，而不是开启新 Provider 回合。测试随后改为等待明确的下一输入边界，未修改产品语义或延长 Provider 超时。定向 E2E 约 7 秒，最终真实串行 PTY 63/63；两个 CLI binary 在本轮生产改动后各 820/820。尚未覆盖的是三个以上 fallback 候选同时抖动、限流退避跨分钟运行和真实公网 Provider 的非确定性故障注入。

### 11.18 权限面板与后台任务详情的动态尺寸门禁

状态：**真实 PTY 帧级 resize 门禁已关闭。**

权限确认卡和用户询问卡过去只处理键盘事件，终端在卡片打开期间收到 `Event::Resize` 时不会按新宽度重排，导致长命令、选项说明与底部提示可能保留旧几何。两条交互路径现在都显式消费 resize 事件，使用实时终端尺寸重绘同一状态，并重新计算已渲染行数；不会退出权限等待、改变选中项或伪造用户决定。

`permission_custom_response_suspends_the_tool_until_the_user_answers` 在真实权限挂起期间执行 `100 -> 46 -> 118` 列变更。窄屏帧断言完整保留“仅本次允许 / 返回输入框 / 拒绝”操作，所有可见行不超过 46 列；宽屏帧断言权限标题只出现一次。随后 Custom 输入、挂起恢复和 `/allow` 仍走原交互协议，证明 resize 没有提前完成 effect。

`shell_jobs_are_persisted_and_visible_through_unified_tasks` 打开真实后台 Shell 的 `/tasks <id>` 详情后，先在 composer 输入未提交的 `/help`，再执行 `116 -> 48 -> 116` 列变更。测试断言草稿、footer 和稳定 task identity 在重排中保留，窄屏行宽不溢出；回车后 `/help` 正常执行，下一模型回合继续消费同一后台任务 mailbox。测试夹具只通过 `TIOCSWINSZ` 模拟终端行为，不再向每个后代进程额外广播合成 `SIGWINCH`。

该用例连续执行三次均通过；加入 11.19 的大会话资源门禁后，完整回归为 `nola` 820/820、`nola-agent-host` 820/820、真实串行 PTY 64/64。这里的“帧级”指对真实 PTY 当前屏幕文本、宽度和重复元素做断言，不等同于不同终端字体与渲染器上的像素截图。CLI 主进程的受控分配预算见 11.19，而不是复用后台 worker 的 RSS 预算测试冒充前台低内存覆盖。

### 11.19 千万字符会话的受控分配预算

状态：**CLI 自身 Rust 活跃分配与 macOS 真实进程树 RSS 门禁均已关闭。**

macOS 对当前进程设置有限 `RLIMIT_AS`、`RLIMIT_DATA` 和 `RLIMIT_RSS` 均返回 `EINVAL`，因此测试没有把无效的 `ulimit` 包装成通过。Debug 构建现提供显式测试预算 `NOLA_TEST_ALLOCATION_LIMIT_BYTES`：全局分配器对 alloc、zeroed alloc、dealloc 和 realloc 维护当前活跃字节，并在新增活跃分配超过预算时拒绝分配；未设置变量时使用系统分配行为，release 构建不包含该测试控制面。

`ten_million_character_session_restores_under_a_controlled_memory_limit` 首先用真实 CLI、Provider 和 canonical SQLite 创建会话，再把一条 canonical 助手消息扩展为精确 1000 万字符。第二个真实 CLI 进程在 256 MiB 活跃 Rust 分配预算下打开 `/resume`，切换到该会话，恢复 composer，执行 `/help` 并正常退出。断言包括：15 秒内恢复可交互、PTY 累计输出小于 2 MB，因而没有把完整正文物化到终端。用例还先以 1 字节预算启动同一二进制并要求专用状态码 70，证明预算器确实生效而不是环境变量被静默忽略。

macOS PTY 现在同时以 `/bin/ps -axo pid=,ppid=,rss=` 持续采样 `/usr/bin/script` 包装进程和全部 CLI 后代，按实时父子关系汇总 resident set，并设置 384 MiB 进程树上限。采样器必须至少取得一个有效样本；CLI 异常退出时会随根 PID 消失自动结束，不能因 `ps` 失败静默通过。三次独立定向运行的峰值分别为 141.5、140.8、140.9 MiB，耗时为 4.52、4.59、4.76 秒。384 MiB 保留链接器和系统库差异余量，同时足以发现把 1000 万字符在多个进程内重复物化的数量级回归。

Rust 分配门禁约束 Nola 全局分配器管理的存活内存；进程树 RSS 则补充统计 SQLite、系统库及其他绕过 Rust 分配器的 resident pages。它们仍不包含系统全局文件缓存、终端 App 自身和树外服务，也尚未替代 Linux/Windows 的平台门禁。后台 Shell/Agent 的真实 RSS 与进程树预算继续由 11.12 的 worker E2E 单独保证。

### 11.20 多周期自动压缩、Goal 权限挂起与冷恢复

状态：**真实双进程 PTY 门禁已关闭。**

`repeated_auto_compaction_preserves_goal_permission_and_exactly_once_resume` 使用两个真实 CLI 进程、canonical SQLite、真实 Core Runtime Driver 和本地流式 Provider 执行以下链路：

```text
三轮大上下文
  -> 至少两次自动 LLM checkpoint 压缩
  -> 启动 Goal 与原执行 contract
  -> append_file 进入 permission.ask
  -> 权限面板可见且 Core host effect 已持久化
  -> 强杀原 CLI 进程
  -> 新 CLI 进程 /goal resume
  -> 恢复原 Run 和原 permission effect
  -> 用户 Allow once
  -> append_file(create=true) 恰好执行一次
  -> goal_complete(needs_user)
  -> 后续 Goal slice 消费最新自动压缩摘要
  -> 无活动 host effect、无 queued/running Run
```

该链路依次暴露并修复了三类根因：

1. **Goal 恢复误提交新输入。** Core checkpoint 已经保存原活动 Run 和 `permission.ask` effect，但 `/goal resume` 直接进入 `ensure_core_goal_started` 并提交 idle Goal command，触发 `idle Core submission cannot overlap an active Run`。CLI 现在先调用 `resume_agent_core_pending_with_cancellation`，按 frozen input 继续原 Run；恢复本身不创建第二个 Turn。
2. **旧进程 Goal lease 阻塞新 Host。** 原 CLI 被强杀后，Goal lease 仍由 `interactive-goal:<pid>:<uuid>` 持有且有效期较长。CLI 只在 owner 格式可验证且 PID 已死亡时，以原 owner 向 Core 提交 `Release`，然后由新进程 `Claim`；存活 worker 的 lease 不会被抢占。
3. **恢复结果错误归入新 contract。** 恢复路径现在从同一 Goal projection 取得仍为 `Executing` 的原 contract，将 Emitter 的正文、Goal signal、terminal reason、排队输入和 context usage 写回该 contract。只有不存在可恢复 Run 时才创建新 round；不会把已持久化 effect 误标为“宿主中断”后重新规划。

前台权限的权威恢复事实位于 `core_checkpoints.state.host.pending_host_effects`，不是后台 Agent 使用的 `permission_requests` 表。E2E 在权限 UI 出现时解析真实 checkpoint，要求存在 `state=dispatched`、`capability_id=append_file` 和稳定 `permission_request_id`；完成后要求活动 effects 中不再存在 permission request。文件最终内容严格为一行 `automatic-compaction-once`，证明恢复未重复副作用。

测试夹具同时遵守真实工具合同：新目标使用 `append_file(create=true)`，大上下文使用带唯一序号的语义片段，避免用重复字符串误触重复输出保护。定向链路连续执行三次均通过；加入 Provider 半截流联合门禁后的当前完整回归为 `nola` 820/820、`nola-agent-host` 820/820、真实串行 PTY 67/67（273.56 秒）。

该门禁关闭的是 CLI 内置工具在 Core checkpoint 可恢复边界上的确定性链路，不应扩大为所有副作用的绝对 exactly-once。外部执行器内部调用、不可预枚举远端副作用、非 macOS 平台 RSS、跨终端像素截图以及真实公共 Provider 的联合故障仍需单独验收。

### 11.21 定时重试的冷恢复与去重

状态：**真实双进程 PTY 门禁已关闭。**

`goal_resume_reports_scheduled_retry_without_starting_it_twice` 先通过本地流式 Provider 创建一个真实 `needs_user` Goal，退出首个 CLI 后，在 canonical Store 中写入属于该会话、状态为 `retry_waiting`、重试时间为未来一小时的 RuntimeTask。第二个 CLI 冷启动并执行 `/goal resume`，必须先展示稳定 task identity 和“已安排重试，不会重复启动”，随后只恢复父 Goal。

测试完成后直接检查 canonical SQLite：该 task 仍只有一条，状态仍为 `retry_waiting`，`retry_count=2` 未被提前消费，`retry_at_ms` 未被重写。双阶段 Provider 只收到初次 Goal 和恢复父 Goal 两个请求，因此未来任务没有在恢复 preflight 中被误启动。该用例与权限等待、不确定副作用人工介入共同覆盖 `ResumeSnapshot` 当前三类需要用户可见协调的恢复动作。

该门禁不意味着重试一定成功，也不允许 Host 在截止时间到达前自行执行；真正到期后的 claim、lease、预算与错误分类继续由 RuntimeTask Supervisor 合同控制。

### 11.22 Provider 半截可见流与持久任务联合故障

状态：**协议级真实 PTY 联合门禁已关闭；公共 Provider 的外部不可观察行为仍未关闭。**

`incomplete_provider_stream_preserves_active_durable_work_and_one_answer` 使用真实 CLI、canonical SQLite、后台 Shell worker 和本地 OpenAI-compatible SSE fault injector。第一轮由模型调用后台 Bash，真实子进程延迟四秒后创建 Artifact；任务仍为 `running` 时，第二轮 Provider 返回 HTTP 200 和一段已对用户可见的正文，但在 terminal SSE event 前断开连接。CLI 不重放整个请求，而是按 continuation protocol 携带一次已显示前缀继续请求，最终只提交一条组合后的 canonical assistant message。

门禁同时断言：可见前缀只渲染一次；后台 RuntimeTask 始终只有一个稳定 ID；任务完成后 Artifact 内容只有一份；Provider 续接没有复制或重启后台副作用。定向用例连续三次通过，随后完整真实串行 PTY 为 67/67，耗时 273.56 秒。

该测试覆盖 HTTP/SSE 传输层“成功响应后半截协议”的确定性故障注入，但不会伪装成公共 Provider 内部状态验证。如果 Provider 在其不可观察的外部系统中产生副作用后才断流，Nola 仍无法根据网络结果断言是否已应用；这类调用必须具备外部幂等键/查询接口，或进入 `RequiresIntervention`，禁止自动盲目重放。

### 11.23 Application Service IPC 放大与测试隔离

状态：**CLI Host 可消除的重复健康检查已移除；Core 语义写入未私自合并。**

生产持久端口此前在 `CliApplicationPort::persistent` 构造时调用一次 `ensure_running`，紧接着 `ApplicationServiceClient::execute` 为保证“发送前可恢复、发送后不盲重试”又调用一次。大量短生命周期查询因此每条命令发生两次 Unix socket health IPC。CLI 现在让构造保持无副作用，只保留 `execute` 前的权威健康检查；服务若已退出仍会在真正发送命令前启动，不会在丢失响应后重放。每条 `application_command` 完成日志新增 `duration_ms`，可以按 command type 定位剩余慢提交。

本轮没有把 Goal 的 `Start`、`Claim`、`PlanContract`、`BeginExecution`、`RecordExecution` 和 `Audit` 合并成 Host 私有写入：这些是可恢复状态机中的不同语义边界，若要原子批处理必须由 Core/Application 提供类型化命令，CLI 不得绕过。当前优化减少的是冗余 IPC，不宣称消除了所有 SQLite revision 写入。

全量单测同时暴露两个原有测试合同问题：Chrome native screenshot 测试仍把 JSON `ToolOutcome.text` 当旧纯文本解析，现改为验证 JSON 内的人类文本与结构化 PNG Artifact；动态本地模型 catalog 的隔离子进程会继承并行测试设置的 `NOLA_DB_PATH`，现显式清除数据库和 Application Service 可执行环境，避免父子进程并发初始化同一 SQLite。修复后 `nola` 820/820、`nola-agent-host` 820/820，真实串行 PTY 67/67（272.65 秒）。

### 11.24 外部执行器进程树生命周期统一（2026-08-29）

本轮关闭了 Claude Code、Codex App Server、OpenCode ACP 及其探测命令在取消、超时、协议失败时只终止直接子进程的问题：

- `external_command` 在 Unix 平台统一创建独立进程组；
- 取消、握手失败、协议空闲超时、模型目录探测结束和认证探测超时统一经过 `terminate_external_child`；
- 终止流程先向整个进程组发送 `SIGTERM`，短暂宽限后发送 `SIGKILL`，最后回收直接子进程；
- 新增真实进程测试 `external_executor_termination_kills_descendants_and_leaves_no_orphan`：假执行器启动一个长时间子进程，终止执行器后验证父进程和后代均不再存活；
- 外部执行器协议测试 `28/28` 通过；`nola` 与 `nola-agent-host` 完整单测均为 `827/827` 通过。

进程生命周期修复后，外部执行器进一步接入 CLI 运行配置中的 `external_execution` 预算合同。该合同支持 `max_wall_time_seconds`、`max_output_bytes`、`max_cpu_time_seconds`、`max_memory_bytes`、`max_processes` 和 `max_line_bytes`，默认墙钟为 24 小时，避免把学术或长任务误设为短请求。Claude stream-json、Codex App Server 和 OpenCode ACP 共用同一预算监控器与任务运行时的 PPID 进程树采样；stdout 使用小容量同步通道和单协议行上限，stderr 持续排空但只保留 64 KiB 诊断预览，防止未消费输出造成内存无界增长或子进程阻塞。CPU、RSS 与进程树采样当前由 Unix 进程接口实现；非 Unix 平台仍执行墙钟、总输出量和单行协议帧预算，平台级资源采样属于后续工作。

新增证据包括：运行配置解析测试、超长协议行有界拒绝测试、真实进程树超进程数检测，以及 `real_stream_json_executor_budget_failure_propagates_and_reaps_process_tree`。最后一项通过真实假 Claude 子进程执行完整 `run_stream_json_process`，启动后代后触发进程数预算，断言调用方获得明确预算错误且整个进程组消失。该证据关闭 CLI Host 可控制的外部执行器墙钟、CPU、RSS、进程数和输出预算接线；第三方执行器内部逃逸到 Nola 进程树之外的服务和不可枚举远端副作用仍不在此合同内，继续禁止盲目重放。

### 11.25 SOP 定义快照与显式新 Run（2026-08-29）

本轮关闭“定义升级后旧 Run 的语义不透明、用户只能原地继续”的缺口，并保持 Core/Application 已有的版本固定合同不变：

- CLI Terminal SOP 升级为 v2，在执行与验证之间新增 `checkpoint` 步骤，要求长任务先持久化 resume、Artifact 和副作用证据；短任务明确记录不需要继续点；
- 已创建的 Run 仍完整保存启动时 `SopDefinition`，后续推进、恢复和验证只使用该快照，不用当前 Host 定义重解释旧步骤；
- 注入模型的 active SOP context 现在明确提供 `definitionPinned`、`latestAvailableVersion` 和 `definitionUpdateAvailable`，并禁止模型把新版定义套到现有 Run；
- `/work <run-id>` 显示固定版本以及可用的新版本，并给出本地化的显式新 Run 指引；
- `/work rerun <run-id>` 只接受 completed、failed 或 cancelled 的旧 Run，复制原始输入、使用当前定义和新 UUID 创建独立 Run；活动 Run 必须先停止或完成，避免两个版本并行产生未知副作用；
- 当前定义不存在或版本低于旧 Run 时拒绝重跑，不进行隐式降级或兼容迁移。

真实 PTY + SQLite 测试 `finished_sop_can_be_rerun_with_new_definition_without_mutating_old_snapshot` 预置已结束的 v1 Run，验证详情显示 v1→v2，执行 `/work rerun` 后数据库同时保留不可变的 cancelled v1 和使用相同输入的新 v2 Run。完整回归结果为：`nola` 827/827、`nola-agent-host` 827/827、macOS PTY 68/68（279.24 秒）。本轮未修改 `core/`。

### 11.26 外部执行器接入统一 Supervisor（2026-08-29）

审计确认 Claude Code、Codex App Server 和 OpenCode 虽已有进程树与资源预算，但委托入口此前直接在线程中启动进程，不创建 `RuntimeTask`、不调用 `claim_runtime_task_fair`；`/work` 仅根据配置中的外部 session id 合成一个 `Ready` 项。因此它们能够绕过 Agent、Shell 和 Skill 使用的全局/项目并发预算，也没有权威的 queued、running 和 terminal 记录。

本轮将委托型外部执行器接入同一 durable Supervisor 合同：

- 启动进程前先持久化 `kind=external_executor,status=queued` 的 RuntimeTask，并记录 backend、model、parent effect、Host pid 和可重放边界；
- 使用与 Agent、Shell 相同的 `claim_runtime_task_fair` 和 `NOLA_MAX_CONCURRENT_TASKS` / `NOLA_MAX_PROJECT_TASKS`，拿不到槽时保持 queued，绝不先启动外部进程；
- 认领后启动独立心跳；失去 lease 或收到任务取消时触发同一 `ExecutionCancellation`，由外部执行器的进程组终止合同回收后代；
- 成功、失败和取消都以 ownership token 提交 durable terminal event 并释放槽；RAII 异常退出标记 `reconciliation_required`，不把未知副作用自动重放；
- Host 在拿槽前退出时，Unix 平台根据持久化 Host pid 将旧 queued 任务安全取消并记录 `safe_to_retry=true`；拿槽后 lease 过期则标记失败并要求核验，记录 `safe_to_retry=false`；非 Unix 平台在没有可靠进程身份 API 时保留 queued 记录而不误杀其他活跃 Host；
- 外部执行默认 24 小时 wall time、64 MiB 输出、12 小时 CPU、8 GiB RSS 和 128 个进程的预算现在同时写入 RuntimeTask 调度记录，不再只存在于进程监控器内部；
- 删除 `/work` 基于配置合成的伪外部执行器项，改为只展示 RuntimeTask 账本中的真实状态。

新增真实竞争测试 `external_executor_waits_for_the_same_fair_slot_as_a_shell_task`：先以 `kind=bash` 的任务占满唯一全局/项目槽，验证外部任务保持 queued 且真实子进程未创建；释放 Shell 槽后外部任务才认领、启动实际 `sh` 子进程并提交 completed。该测试同时模拟拿槽前 Host 消失，验证旧 queued 记录转为 cancelled 且留下可安全重试事件。主租约接入后的完整 macOS PTY 为 68/68（277.26 秒）；最终 `nola` 与 `nola-agent-host` 单测均为 828/828。本轮未修改 `core/`。

### 11.27 Windows 原生进程资源治理与 CI 门禁（2026-08-29）

11.24 的外部执行器 CPU、RSS 和进程数预算此前仅在 `cfg(unix)` 下采样；Windows 实际只执行 wall time、输出量和协议帧预算，且取消时只能终止直接子进程。现已补齐 Windows 原生合同：

- `windows-sys` 作为 `cfg(windows)` 条件依赖，不进入 Apple、Linux 或 Android 构建；
- ToolHelp `CreateToolhelp32Snapshot` / `Process32FirstW` / `Process32NextW` 枚举 PID/PPID，以与 Unix 相同的聚合算法确定完整后代集合；
- `GetProcessTimes` 汇总 kernel + user FILETIME 为 CPU 毫秒，`K32GetProcessMemoryInfo` 读取工作集字节；无法打开的系统进程仍计入进程数，但 CPU/RSS 记为零，不因权限不足丢掉后代数量；
- 外部执行器资源监控在 Unix 和 Windows 都执行 CPU、RSS、进程数预算；Shell、Agent 后代和外部执行器取消在 Windows 按后代再根进程的顺序调用 `TerminateProcess`；
- 新增平台无关 FILETIME 换算测试以及 `cfg(windows)` 的真实当前进程采样测试。

本机已安装 `x86_64-pc-windows-gnu` Rust target，但完整 Nola 交叉检查在依赖层被缺失的 `x86_64-w64-mingw32-gcc` 阻断：先后由 bundled C 依赖和 `ring` 构建脚本失败，未进入 Nola Rust 源码，因此不记为产品编译失败或成功。独立、同版本 `windows-sys` Win32 探针已在该 Windows target 成功交叉编译，验证新增 feature、句柄类型和 API 签名。

为获得完整依赖图与运行时证据，`.github/workflows/rust-build.yml` 新增 `windows-latest` CLI job：检查两个二进制，并分别执行 `nola` 与 `nola-agent-host` 全量单测；其中 Windows 条件测试会实际采样 runner 进程。workflow YAML 与 Cargo 的 `cfg(windows)` dependency 元数据已验证。当前 macOS `nola` / `nola-agent-host` 回归均为 829/829；Windows CI 结果必须以远端 runner 的实际状态为准。本轮未修改 `core/`。

### 11.28 长程任务、MCP 与上下文管理专项对比审计（2026-08-29）

状态：**审计完成；长任务基础合同较强，但 MCP 尚未进入该合同，上下文仍缺少协议轨迹与跨压缩任务预算门禁。**

#### 11.28.1 审计范围与判定方法

本节只依据真实代码和测试，不以 README 的功能清单作为完成证据。对标来源为：

- Codex：`参考资料/codex-main/codex-rs/core/src/unified_exec/`、`core/src/tasks/`、`core/src/context_manager/history.rs`、`codex-mcp/src/connection_manager.rs` 与对应测试；
- OpenCode：`参考资料/opencode-dev/packages/opencode/src/session/compaction.ts`、`session/run-state.ts`、`session/status.ts`、`mcp/index.ts`；
- Claude Code 参考实现：`参考资料/claude-code/src/query.ts`、`services/compact/`，以及后台 monitor 的工具合同；
- Nola：CLI 的 `task_runtime.rs`、`store/task.rs`、`mcp_host.rs`、`mcp_stdio.rs`、`agent_host_driver.rs`、`context_budget.rs`、`session.rs` 与真实 PTY/E2E。

判定分为三档：实现存在；状态、权限、副作用、取消和恢复在同一合同中闭合；真实进程、传输或模型边界的 E2E 能复现故障并断言结果。单元测试只证明局部转换，不等同于可靠性成立。

#### 11.28.2 总结结论

| 领域 | Nola 当前判断 | 与参考实现相比的关键差距 | 优先级 |
|---|---|---|---|
| 长程任务 | durable RuntimeTask、全局公平认领、租约、心跳、资源预算、强杀恢复、权限挂起与对账已形成较强基础 | MCP 和部分外部能力仍绕过 Supervisor；任务预算未跨多轮压缩累计 | P0/P1 |
| MCP | 支持 stdio 与 Streamable HTTP、分页发现、目录变更标记、显式 artifact、输出落盘引用、非只读调用禁止盲重放 | 调用是前台阻塞且进程内；无取消/进度协议、无 durable invocation、无健康状态、无 OAuth/elicitation，缺少真实 MCP E2E | P0 |
| 上下文 | Core+LLM 联合压缩、摘要无收益回滚、恢复模式拒绝无效摘要、输出空间预留、最多八次收敛 | 未证明 thinking/tool trajectory 在压缩后仍合法；摘要请求自身无独立输入预算；缺少跨压缩累计任务预算与失败熔断 | P0/P1 |

最重要的架构判断是：**不要重写长任务 Supervisor，也不要为 MCP 新建另一套任务表。MCP invocation 必须成为现有 RuntimeTask/Work step 的一种执行类型。**

#### 11.28.3 长程任务对比

Nola 已经优于“仅后台进程表”的实现：RuntimeTask 具有持久状态、owner、lease、heartbeat、取消请求、终态事件和通知确认；Shell、委托 Agent 与外部 executor 共用公平槽位；worker 在命令开始前崩溃可安全接管，开始后不确定副作用进入 reconciliation；CPU、RSS、进程数、墙钟和输出预算能够终止真实进程树；guidance、权限等待、SOP lineage 和补偿均有持久投影；真实 PTY 已覆盖多项目竞争、Host 强杀、provider 故障、千万字符会话和跨进程恢复。

参考实现提供的可迁移模式：

- Codex unified exec 在初始 yield 前保存 live process，保证 turn 中断不会终止后台进程；同一 process id 支持后续 stdin、增量输出和终止。
- OpenCode session runner 把 busy、cancel、idle transition 放在统一 runner 生命周期，避免会话重入。
- Claude Code 后台 monitor 禁止前台轮询，后台事件主动进入对话；监控必须覆盖成功、失败、取消和超时，短窗口事件批量通知。

Nola 剩余缺口：

1. **P0：MCP 长调用绕开 Supervisor。** `call_tool` 在 Host turn 内同步阻塞；RuntimeTask 的租约、心跳、资源预算、取消、通知和冷恢复均无法覆盖它。
2. **P0：取消存在语义断层。** MCP capability 声明为 `BestEffort`，但 HTTP/stdio 请求不接收 `ExecutionCancellation`，也没有 MCP cancelled/progress 协议。Esc 只能等待 timeout。
3. **P1：缺少跨压缩累计任务预算。** 当前预算主要约束单次请求、worker 和输出；长任务经过多次压缩后没有累计模型上下文/调用成本边界。
4. **P1：任务事件进入模型上下文仍需按新事件水位、最终状态和可操作异常投影。** 不能反复注入完整日志，也不能只保留模糊状态。
5. **P2：后台通知缺少节流指标。** 应记录合并前后事件数、丢弃原因和最后通知水位，证明高频进度不会污染上下文或饿死前台 Turn。

#### 11.28.4 MCP 对比

Nola 当前正确部分：stdio 使用 argv/env 直接启动；HTTP endpoint 禁止 URL 内嵌凭证且设置 timeout；initialize/initialized、session id、JSON/SSE、分页和四类 catalog 已覆盖；transport error 后丢弃 client 但不自动重试当前 call，避免重复副作用；MCP schema 通过冻结 catalog 按需激活；输出有 8K 预览、完整引用和显式 artifact；非只读调用标为 `RequiresRecoveryDecision`。

与 Codex/OpenCode 相比，缺口集中在运行时：

1. **P0：没有 first-class connection lifecycle。** 当前是进程内 registry 加可选 client；没有 starting/ready/auth-required/degraded/failed/stopped、required server、启动取消和可查询健康快照。Codex 对 pending、shutdown、失败原因与 reconnect 有独立测试。
2. **P0：没有 durable MCP invocation。** server/tool/call id、参数摘要、权限决定、开始水位、progress、终态、输出引用和副作用不确定性没有作为 RuntimeTask event 持久化。
3. **P0：缺少真实传输 E2E。** 当前主要是响应解码、稳定 id、transport 选择、artifact 和 catalog 单元测试；没有真实 stdio/HTTP fake server 覆盖断线、timeout、取消、重启和冷恢复。
4. **P1：目录刷新会阻塞 Turn。** turn 开始时同步刷新 catalog；慢或异常服务器会增加首 token 延迟。需要后台刷新、上一版可用 catalog、generation 和 per-server failure。
5. **P1：缺少认证与交互协议。** 目前主要依赖静态 header/env；没有 OAuth 状态、登录恢复、elicitation 挂起及同一 invocation 恢复。
6. **P1：服务端 annotations 被直接提升为执行策略。** `readOnlyHint` 影响 Investigate/Mutate、并发和重放安全。annotation 只能是提示，必须允许本地策略覆盖。
7. **P1：没有 MCP progress、sampling、roots、ping 等能力协商结果。** 长工具不能报告结构化进度。
8. **P2：连接关闭不可完整观察。** stdio Drop 能结束子进程，但 HTTP session 没有用户可见 shutdown 生命周期。

#### 11.28.5 上下文管理对比

Nola 当前正确部分：context window 来源可追踪；prompt limit 预留 completion 与 safety margin；provider 报错只会单调收紧窗口；Core 决定 archive，LLM 生成摘要，再以 archive id/digest 应用；摘要无收益时正常模式回滚、恢复模式失败；最多八轮；最新用户输入、tool catalog 和组件指标被保留；完整记录与模型投影分离。

参考实现值得吸收的机制：

- Codex 结构化保存 reasoning、tool call/output 等 item，删除旧 item 时同步删除 call/output 对端；world state 用 baseline/diff 注入，history rewrite 有 version。
- Claude Code 在全局摘要前依次执行 tool-result budget、snip、microcompact、context collapse；明确保持完整 assistant trajectory，并在中断时补 synthetic tool result。
- OpenCode 把 compaction 持久化为 message/part，保留 previous summary，超限时可重放当前用户请求，对摘要自身超限给出确定终态。

Nola 剩余缺口：

1. **P0：缺少 trajectory 不变量的压缩 E2E。** 尚未构造 reasoning → parallel tool calls → partial results → interruption → compact → cold resume，断言 provider 字段合法、每个 call 恰好一个 result、顺序不重复且不遗漏。
2. **P0：摘要请求自身没有独立输入预算。** `summarize_context_archive` 直接把 archive messages 序列化为一个用户消息，只调整输出上限；极大工具结果可能使解决超限的请求自身超限。
3. **P1：分层裁剪合同和测试不足。** 需要证明 full output ref、artifact、错误码、验证状态、路径和最后工具结果在 micro-prune 后仍可恢复。
4. **P1：没有跨 compaction 的累计 task budget。** 压缩后 prompt 变小，但总模型输入、输出、工具结果和摘要成本未进入同一持久预算。
5. **P1：重复压缩失败只有单次八轮上限，没有跨 Turn 熔断。** 相同 archive/digest 可能在后续 Turn 重试，需要失败计数、退避和用户可见恢复策略。
6. **P2：token 估算仍是启发式。** 没有 provider tokenizer 或 API usage 回填后的误差校正。
7. **P2：数据源 context 缺少差量合同证据。** 应证明未变化来源不重复注入，过期或授权变化只更新对应 component。

#### 11.28.6 推荐实施顺序

**P0-1：MCP invocation 接入现有 Supervisor**

- 新增 `runtime_task.kind = mcp`，不新增平行任务表；
- 持久化 server/tool/call id、参数 digest/有界预览、effect、replay safety、permission 和 output ref；
- 短调用可快速完成，超过前台 grace 后自然投影为长任务；
- 发送前失败可重试，发送后未知必须 reconciliation，绝不盲重放 mutate call。

**P0-2：把取消和进度接入 MCP transport**

- HTTP 与 stdio request 接收统一 cancellation；
- 支持 progress notification，按 200–500 ms 合并为 RuntimeTask event；
- Esc、`stop_task`、Host shutdown 和 lease expiry 走同一取消路径；
- 服务器不支持取消时，终态必须是 cancellation requested 或 reconciliation required，不能伪装 cancelled。

**P0-3：补真实 MCP E2E**

- fake stdio server 与 fake Streamable HTTP server；
- 覆盖分页、catalog change、progress、timeout、用户取消、server crash、Host crash、恢复、只读安全重试和 mutate 禁止盲重放；
- 断言 UI event、RuntimeTask、Core tool result 与完整输出引用一致。

**P0-4：建立 context trajectory 门禁**

- 构造 reasoning + 并行 tool call + 部分 result + interrupt + auto compact + cold resume；
- 验证 thinking/tool pairing、顺序、id 和最终 answer 唯一；
- 同一用例覆盖 OpenAI-compatible `reasoning_content` 回传要求。

**P1：摘要输入分层预算、累计 task budget 与压缩熔断**

- summary 前对每条工具结果结构化裁剪，保护 artifact/full output ref、错误码、路径、命令和验证结论；
- archive 仍过大时分块摘要并用 digest 链合并，禁止无界 JSON 直接进入 summarizer；
- RuntimeTask/Work 持久化累计 model input/output、summary 和 tool bytes，compaction 后不清零；
- 相同 archive digest 连续失败进入退避或等待用户。

**P1：MCP 健康、认证和策略覆盖**

- 每个 server 暴露 generation、状态、最后成功/失败、失败原因和认证动作；
- catalog 后台刷新，交互 Turn 使用最后一个已验证 generation；
- 支持 OAuth/elicitation 挂起恢复；
- 本地策略覆盖 server annotation。

#### 11.28.7 必须新增的验收矩阵

| 场景 | 必须断言 |
|---|---|
| MCP 长调用后 Esc | 取消到达 transport；Turn 不再阻塞；终态不伪造；下一次输入可立即处理 |
| mutate MCP 调用发送后 Host 强杀 | 冷恢复进入 reconciliation；远端副作用不自动重放 |
| read-only MCP 调用发送前崩溃 | 同一 invocation id 安全接管；结果只提交一次 |
| catalog change + 慢服务器 | 当前 Turn 使用上一已验证 catalog；后台生成新 generation；首 token 不被阻塞 |
| MCP OAuth/elicitation | 持久挂起；用户响应后恢复同一 call；不重复请求模型或工具 |
| reasoning + parallel tools 后压缩 | provider 字段合法；call/result 一一配对；无重复、遗漏和孤儿 result |
| 超大工具结果触发压缩 | summarizer 输入受限；保护字段与 full output ref 保留；摘要请求不超限 |
| 连续三次摘要失败 | 相同 digest 不无限重试；原 transcript 未损坏 |
| 长任务跨三次压缩 | 累计 task budget 单调减少；达到上限后确定停止 |
| 高频后台进度 + 前台对话 | 进度合并；前台无饥饿；上下文只注入新水位和最终状态 |

#### 11.28.8 禁止的修补方式

- 不为 MCP 新建独立 scheduler、权限表或恢复状态机；
- 不用用户文本关键词判断是否进入长任务、调用 MCP 或压缩；
- 不因 server 声明 `readOnlyHint` 就跳过策略审计；
- 不通过增加 timeout 掩盖不可取消的阻塞调用；
- 不在摘要失败时删除 transcript 或截断最新完整工具轨迹；
- 不整体复制参考项目，只吸收经过 Nola 合同验证的机制。

#### 11.28.9 P0 实施进度（2026-08-29）

本轮已完成第一条可运行纵切，范围限定在 CLI Host，不修改 `core/`：

- MCP 调用已复用现有 `RuntimeTask`、公平认领、lease、heartbeat、终态事件、通知和冷恢复合同，任务类型为 `mcp`；WorkStep 继续作为参数、effect 与结果的事务边界。
- mutate MCP 在发送后失败、取消或 Host 失联时进入 `reconciliation_required`，不会自动重放；发送前取消以确定失败结束。
- `ExecutionCancellation` 已下沉到两种 transport。stdio 在原请求循环发送 `notifications/cancelled`，短暂排空响应后关闭连接；Streamable HTTP 用同一 session 和 request ID 并发发送取消通知。取消后仍按副作用未知处理，绝不把“通知已发出”误写成远端操作已撤销；HTTP 原响应 worker 可能继续至远端终态。
- stdio 与 Streamable HTTP 的 `notifications/progress` 均已实时进入 Supervisor，按 250 ms 节流，摘要和 detail 分别限制为 240/4000 字符。HTTP 响应采用有界增量读取，SSE 完整帧一到达即发布进度，不等待远端最终结果；分帧使用字节边界，避免 UTF-8 多字节字符跨网络分片时破坏索引或丢失事件。
- heartbeat 线程启动失败会提交 `start_failed` 终态，不再遗留一条假 running 任务。
- 摘要请求新增独立输入上限 `NOLA_CONTEXT_SUMMARY_INPUT_TOKENS`，默认 48K tokens、范围 8K–200K；已有摘要与最新完整消息优先保留，并显式记录省略的旧消息数量和 archive digest。
- 新增 CLI trajectory 回归：并行工具调用和两条结果原子归档，中断说明与最新用户消息保留；序列化模拟冷启动后再次准备上下文，顺序和内容不变。
- read-only MCP 的过期 lease 现在由既有 Application scheduler 转入 `retry_waiting`；同一 Core effect 再投递时按 `parent_effect_id + server + tool` 接管原 RuntimeTask id。mutate 调用仍进入 `reconciliation_required`，不会被该接管路径重放。
- Core checkpoint 已经持久化跨 compaction 的 `cumulative_usage`，摘要 usage 会进入同一累计值且 compaction 不清零。CLI 另持久化工具输出字节与摘要输入/输出成本；活动 Goal 配置 `NOLA_TASK_TOKEN_BUDGET` 后，在新执行片开始前以累计模型 token 加工具结果估算执行确定性门禁，普通会话不受隐式上限影响。
- CLI Host 以摘要输入 digest 持久化失败身份；同一 digest 连续失败三次后在再次请求 LLM 前返回 `compaction_fused`，不同 digest 可进入新的恢复尝试。该账本与累计成本均通过关闭并重开真实 SQLite 验证。

本轮真实边界测试：

| 测试 | 边界 | 结果 |
|---|---|---|
| `retained_mcp_process_mutation_is_transactional_and_replayed_without_second_call` | 真实 stdio MCP、事务重放、mutate 不重复执行、RuntimeTask 完成态 | `nola` 与 `nola-agent-host` 均通过 |
| `cancelled_call_never_reaches_transport` | 发送前取消 | 通过 |
| `cancellation_after_send_returns_promptly_and_marks_side_effect_unknown` | 真实 Streamable HTTP 握手、中文 SSE 实时 progress、远端已收到 mutate、前台快速取消、副作用未知 | 两个二进制均通过；约 1.27 秒含等待远端测试进程收尾，取消返回断言小于 700 ms |
| `retained_stdio_call_sends_protocol_cancellation_and_discards_connection` | 真实 stdio 进程、实时 progress、协议取消 request ID、迟到响应隔离 | `nola` 与 `nola-agent-host` 均通过 |
| `retained_stdio_mutation_crash_is_not_retried_and_discards_connection` | 真实 stdio Server 在 mutate 写入后以状态 17 崩溃、连接失效、隐式重试隔离 | `nola` 与 `nola-agent-host` 均通过；真实副作用计数保持 1 |
| `retained_stdio_mutation_timeout_is_not_retried_and_discards_connection` | 真实 stdio Server 在 mutate 写入后挂起、请求超时、进程清理与连接失效 | `nola` 与 `nola-agent-host` 均通过；真实副作用计数保持 1 |
| `http_mutation_disconnect_is_not_retried_after_transport_send` | 真实 Streamable HTTP Server 返回截断 SSE 后断开、mutate 禁止隐式重试 | 两个二进制均通过；远端副作用计数保持 1 |
| `external_executor_waits_for_the_same_fair_slot_as_a_shell_task` | 真实 SQLite、公平 slot、read-only 过期 lease 冷接管、mutate reconciliation | 两个二进制均通过；read-only 复用原 RuntimeTask id，mutate retry count 保持 0 |
| `killed_host_is_cold_recovered_by_replay_safety` | 独立真实 Host 完成 Streamable HTTP MCP 初始化、目录发现并将 `tools/call` 发送到远端后被强杀，第二 Host 冷恢复 | 两个二进制均通过；read-only 复用原 task id 并取得远端稳定结果，远端调用总数为 2；mutate 远端调用总数保持 1，进入 reconciliation 且不重试 |
| `summary_input_budget_keeps_existing_summary_and_latest_complete_messages` | 超大 archive 的摘要输入预算 | 通过 |
| `parallel_tool_trajectory_survives_compaction_and_cold_resume` | 并行工具轨迹、压缩、序列化冷恢复 | 两个二进制均通过 |
| `cumulative_cost_and_digest_fuse_survive_cold_reopen` | 三轮 compaction 成本累计、三次同 digest 失败熔断、SQLite 冷重开、换 digest 恢复 | 两个二进制均通过 |
| `cold_host_rejects_fused_digest_before_contacting_the_model` | 全新 Host Driver 冷重开真实 SQLite 后，在模型调用之前执行同 digest 熔断 | 两个二进制均通过；不可达模型端点未被访问，直接返回不可重试的 `compaction_fused`；换 digest 仍可恢复 |
| `three_compactions_do_not_reset_the_task_budget` | 三轮累计模型成本、工具成本合并、达到上限确定停止、非 Goal 不受隐式门禁 | 两个二进制均通过 |

仍未完成，不得在发布说明中声称已解决：

- HTTP 取消确认和 stdio 取消后不关闭 session 的安全复用；
- catalog 后台 generation、健康状态、OAuth/elicitation；
- 结构化 micro-prune 与分块摘要；累计模型/摘要/工具成本账本和同 digest 失败熔断已经接入，但任务上限的真实 Goal 多轮 E2E、用户可配置入口及预算剩余 UI 仍需补齐；
- OpenAI-compatible `reasoning_content` 在同一 trajectory E2E 中的真实 provider 验证。
