> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Task 通知交付实现与验证范围

本节细化 GOAL-05；通知交付不等于用户阅读、模型消费或父 Goal 验收。

## 已实施的 Agent / Shell Task 链路

1. `take_runtime_task_notifications` 读取任务和精确 terminal receipt / pending permission suspension。
   查询不再设置 `ui_notified`，不修改模型 mailbox。
2. `RuntimeTaskNotificationDelivery` 必填 owner、task、occurrence、consumer、command。
   owner 使用显式可空身份：`None` 只匹配真实无所属会话的 Shell，不是 Agent/其他会话的通配符。
   occurrence 使用 `terminal:<receipt_id>` 或 `permission:<request_id>`；客户端不能用时间戳代替身份。
   CLI consumer 为 `cli-interactive`，command 从 task 与 occurrence 稳定生成。
3. `NotificationDelivered` 为只读 Application 查询。`AcknowledgeDelivery` 是写命令，参与原有 UoW。
4. Store 在同一个事务里核对 canonical owner、当前终态 receipt / 未决权限 occurrence，
   将 `notification_delivered` 追加到既有 `durable_task_events`，不另建任务状态库。
   同 consumer/command 相同内容重放返回未新增；不同内容冲突。相同 occurrence/consumer
   必须重用原 command，不通过换 command 创建第二交付记录。不同 consumer 独立交付。
5. CLI 每条通知调用可靠 presenter：提交到本地展示历史，准备实际帧、`write_all`、`flush` 成功后才 ACK。
   旧 `terminal_notice!` / 普通动画 dispatch 会忽略 I/O 错误，不能作为 ACK 边界。
   外部 prompt 占有终端或窗口无有效尺寸时可靠 presenter 返回错误，不 ACK。
6. 一批中第 N 条失败不会撤销前 N-1 条已提交交付；N 及之后仍可重放。
   ACK 重放不追加 task event，防止 ACK 自己形成无限 supervisor wake。

权限面板由明确决议结束其 pending occurrence；拒绝/取消不是通知 ACK，
延迟的旧 permission occurrence 不能 ACK 后来的请求，即使二者时间戳相同。

Shell 的 finish/cancel/expired 事务原来已经调用统一 receipt 记录函数，但查询只接受 agent，
因而没有 Shell receipt。现将该事实记录拓展到 `shell_process`，使用原不可变
`runtime_task_terminal_receipts`、原 terminal source/attempt/水位，不用普通事件或时间猜测终态。
receipt 插入失败必须回滚整个终态及租约变更；不能先完成任务再补伪造通知 ID。
Application 对 terminal receipt 的后继必须按 canonical task kind 分流：Agent 才能进入
父模型 terminal reconciliation；Shell receipt 不能因“存在回执”就成为 Agent 子任务结果。
Shell 通知携带强制 delivery 信封，与普通可读 summary 分型；不在内存保留第二个 consumed boolean。
旧 `AcknowledgeNotification(task_id, permission_request_id)`、`ui_notified` 更新和
`completion_notified` 消费逻辑已删除。模型 status/list/wait 不确认 UI 通知。
通用 AppendEvent、worker finish 和 expiry-with-event 都禁止写入保留的
`notification_delivered` 类型，不能借诊断事件绕过身份校验伪造 ACK。

## 崩溃与错误

- 读取后 / 写输出失败 / flush 失败：没有 ACK，下次查询仍返回。
- 输出成功、ACK 前退出：可能重复显示，提供至少一次交付，不承诺终端 exactly-once。
- ACK 成功后重启：同 consumer 不再返回该 occurrence，原 Task 与 receipt 仍可查。
- 结果查询、模型消费、UI ACK 相互独立；UI ACK 不修改 Task 的 unread、status 或 updated_at。

## 已知边界，不作为完成声明

- 历史 Shell 终态若没有不可变 receipt，不能猜测/制造旧执行证据：读取通知明确报错；本批
  没有修改用户数据库或自动迁移这些旧记录。协议切换需要独立处理数据保留策略。
- 前台可转后台的 Shell 仍有进程内 `notify_on_completion` 展示资格开关；重启时当前
  hydration 将未确认终态恢复为可通知，可能显示额外前台完成提示。这不是交付确认，
  不丢失原结果；要统一前后台展示策略仍需独立契约，不以假 ACK 隐藏提示。
- Agent 列表仍按 project 扫描和逐 Task 查询；分页与积压指标未落地。
- Mac/移动端尚未接入新 consumer 接口，不能宣称跨端全部完成。
- terminal write/flush 只证明输出交付，不证明用户看到/读懂；没有真实终端进程 kill 验收。
  用户滚动离底时新通知可能不在当前帧可见区域；本实现不擅自改变用户滚动位置，
  因而 ACK 的精确定义是本地展示历史接收及输出帧成功，不是“通知文字已在屏幕可见”。
- 原交互循环的读取错误展示仍需整体验收；不能将空队列当成没有任务。

## 回归锚点

- Store `latest_permission_continuation_uses_attempt_order_not_random_id`：真实 SQLite 权限挂起、
  两个同时间戳 occurrence、读取后重开、双 consumer、错误 owner/occurrence、ACK 幂等和事件游标不变。
- Store `shell_terminal_receipt_is_atomic_and_notification_owner_is_not_a_wildcard`：Shell terminal
  receipt 写入故障回滚状态及 attempt、nullable owner 精确匹配、旧 receipt 拒绝、双 consumer、
  重开幂等和普通事件入口不能伪造 ACK。
- StorePort `unowned_notification_requires_explicit_null_not_a_missing_owner`：缺 owner 字段拒绝解析；
  明确 null 的类型合法性不等于任意 Task 的授权，Store 继续检查实际归属。
- CLI `single_terminal_result_primitive_preserves_independent_ui_delivery`：真实准入/终态 receipt，
  模型先消费、只读预览、模拟 renderer BrokenPipe、不丢 UI 通知、显式交付及 ACK 重放。
- CLI `completion_notification_is_acknowledged_across_process_reconstruction`：实际后台 shell，
  丢弃进程内任务映射后仍重放，显式 ACK 后重建不重复。
- Renderer `durable_notification_presentation_propagates_write_and_flush_failure`：分别注入写入与 flush 错误。
- CLI `production_permission_wait_requires_exact_store_grant`：真实权限挂起后将独立模型 mailbox
  置为已读，UI 仍取得精确 permission occurrence；ACK 后不重复，不妨碍原权限决议与恢复。

上述单测不等于真实 UI、所有消费者或完整 E2E-09 已通过。

第一批隔离 `NOLA_HOME`、使用 16 MiB 测试线程栈的执行记录：Store `runtime_task_e2e`
30/30；CLI `notification` 过滤集 16/16；独立 UI 交付专项 1/1；最后补充的真实权限
wait / unread 回归 1/1。首次 Shell 重建测试失败揭示“只恢复 live Task”的缺陷，修复
hydration 后重跑通过；不把此前空列表测试当成持久 ACK 证明。未运行完整 CLI 或真实 GUI。

第二批 Shell 精确交付契约验收（同样隔离 `NOLA_HOME`，16 MiB 栈）：

- Store `runtime_task_e2e` 串行 31/31；随后补齐保留事件伪造拒绝，最新 Shell receipt 专项 1/1。
- StorePort 显式 null owner 契约 1/1。
- CLI 最终 `notification` 过滤集 16/16；真实 permission wait / unread 1/1。
- CLI 三项权限/工具暴露回归各 1/1：`internal_permission_rules_do_not_exempt_private_memory_from_consent`、
  `desktop_authorization_allowlist_preserves_deferred_model_exposure`、
  `web_context_estimate_uses_host_exposure_without_removing_deferred_tools`。
- Application `terminal_commands_require_full_witness_and_receipt_failure_restores_authority` 最新 1/1：
  Shell 收据不得送入 Agent 父任务队列，故障注入仍能回滚实际 receipt 和 authority，不放宽收据检查。

早期并行 Store 测试暴露 `SQLITE_BUSY_SNAPSHOT (517)`：Task 创建的裸 savepoint 先读后写，
与归档后台写事务竞争；事务修复及并行复验由归档模块负责。本节串行通过不替代其并行验收。

## RunResult V2 消费迁移（代码已接线，待最终集中验收）

Agent Task 的 `sealed` 是执行生命周期终态，不是业务完成；不再复制子 Run 的旧
`completed/failed/cancelled` 生命周期。绑定 Run 的 Task 仅通过精确 Run seal 结算，
receipt 保留完整 `RunResult`、`result_digest` 与独立 `budget_failure`。资源超限不会
覆写子结果；父 Goal 的要求仍需独立核对，不因子任务封存或 `Answered` 自动满足。
未建立 Run 的启动失败和取消可保留失败/取消执行记录，但 worker 不得制造
`completed` 或 `sealed`。Shell 进程退出码终态不冒充 Agent RunResult。

父 Task 投影 IPC 使用协议 2，必需显式携带 `run_result` / `result_digest` 字段：
`sealed` 时二者必须存在且摘要、child conversation 匹配；无 Run 的失败/取消为 null。
Core 将其保存到 `TaskDurability`，精确重放同时核对 receipt、结果及摘要。普通 Host
TaskUpdated 不能越过收据路径制造 Agent 完成。任务容量、通知 occurrence、模型 mailbox
及 ACK 均识别 `sealed`，而 UI `ActivityState::Sealed` 与业务完成显示分开。

本轮新增/更新验收入口（尚未执行，不能引用前述历史测试通过数代表本轮）：

- `task_run_binding_e2e`：真实 Core 终态 → Run seal → Task receipt → 父投影，保存原结果摘要，
  包括资源超限、回滚、重开及幂等，不改变子结果。
- `agent_task_terminal_receipt_contract::sealed_task_preserves_each_business_outcome_without_claiming_parent_acceptance`：
  六种结果纯 reducer 透传、缺结果/篡改拒绝，不宣称该纯结构 fixture 有 Store 证据权威。
- `task_terminal_receipt_e2e::unbound_agent_cannot_manufacture_completion_or_sealed_result`：
  SQLite worker 伪成功/伪封存拒绝，状态、attempt、receipt 不变。
- Application Task 显示与 Host 交付结算测试：`sealed` 可结束 Host delivery，不能推断业务成功。

### Mac 结果显示与恢复观察（本轮实现，待统一验收）

Mac Run 协调器、活动历史和通知直接消费六类 RunResult，不再经由旧三态 rawValue 转换。
“已回答”与“行动已完成”分别显示；“未完成”“受阻”“失败”归入需要关注，取消不被显示为失败。
活动视图同时使用文字、图标与语义颜色，不能仅凭颜色识别结果。自动化进程退出单独记录
`execution_finished`，不据此伪造行动验收；自动化 partial 显示未完成。

恢复查询在 Store 的固定读快照中读取精确 Run seal；`sealed` 但没有有效 seal 是错误。
`RunRecoveryObservation::Terminal` 保留完整 `result`、`result_digest` 和完成时间。
取消响应使用成对的 `terminal_result` / `result_digest`，取消命令后的终态仍重新读取已提交 seal；
Core Turn::Completed、进程退出或取消请求已接纳都不能生成业务成功结果。客户端只用该响应判断
是否重新查询/结束观察，不将其变成新的执行授权。

待统一执行的 Mac 测试包括 `MacRunCompletionSideEffectTests`（六结果持久化、通知语义和关注筛选）、
`MacRunCoordinatorTests`（六结果身份和代际 fencing）、`MacRunCancellationTests`、
`MacRunRecoveryInspectionTests` 和 `MacAssistantJournalTransportTests` 协议 2 回环。

### 旧完成测试入口的撤除

`turn_transaction_e2e` 不再调用已删除的 `CompleteAssistantTurn`，也不通过手造最终 checkpoint
测试“成功”。新的终态 fixture 使用真实 Core admission、Host delivery lease、模型 attempt、
文本 delta 和 ModelCompleted，产生真实 Answered 并提交 seal。专项检查错误 Host effect、
seal 插入故障、冷重放、篡改重放和迟到正文不能改变已封存结果。
旧 internal-turn fixture 非法复用已封存 assistant 的行为不保留；后继所有权由真实 NextTurn
promotion fixture 检查。真实压缩替换 Part 后终态的专项尚需补充，不能把上述文本回归算作该覆盖。

CLI `failed_agent_spawn_replay_after_reopen_keeps_one_task_and_exact_receipt` 使用隔离 SQLite 的
真实 Task admission/worker startup-failure receipt，检查冷重开后两次 spawn 重放只交付同一
receipt、无 TaskUpdated 终态和无第二 Task。该专项尚未覆盖携 ActionCompleted seal 的子 Run，
也尚未执行；不得据此声称全部六种 bound-Run spawn 重放已经验收。

内存 `run_effect_publication` adapter 现在仅验证非终态 publication；真实终态必须走 SQLite，
不能从 `run_status` 仿造 seal。原内存终态用例保留失败原子性断言，成功与两种取消顺序移到
`delegated_terminal_results_preserve_real_source_and_reopen_for_both_cancel_orders` 的真实 Core→Host→Store
fixture；检查关闭重开、原来源、重复回执及真实 Answered/Cancelled seal。CLI PTY 子代理断言
改为 Task/Run `sealed` 和完整 result 摘要，Shell 独立执行仍为 `completed`；文件产物单独检查，
不能用执行封存替代业务验收。上述新旧用例迁移仅完成源码，尚未编译执行。

### 三端 Task 封存投影

Mac、iOS、Android 接收 `sealed` 并完整保留 durability 的 `run_result`、`result_digest`、
`terminal_receipt_id`。Agent sealed 必须携带对应 child owner、完整结果和回执；不能以
`completed` 代替 Agent 验收。客户端检查结构，包括六结果必填引用、互斥 effect 集合、
reason code、i64 计数上界和空输入前缀约束；不核验真实引用、实际摘要或业务达成。
冷投影保留结果，终态不允许由进度重新激活。UI 中性显示“执行已封存”，不视为业务成功。

新增六结果保真、缺回执、错 owner、缺结果字段、冷投影和终态不可重启回归源码。
失效的独立 `MacCoreTaskProjectionHarnessMain.swift.fixture` 已删除；其脚本改走
`test_mac_layers.sh task-projection`，在真实 App 模块和 Test 配置下执行
`MacRemoteTaskObservationCoordinatorTests`。本批脚本、代码和测试均未运行。
