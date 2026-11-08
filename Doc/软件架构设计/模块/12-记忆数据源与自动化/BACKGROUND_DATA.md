> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 记忆、数据源与自动化契约

状态：关键入口已审计；完整消费者与真实后台流程尚未验收。对应 MOD-12、E2E-11。

## 当前边界与审计结果

| 模块 | 实际入口/类型 | 已有基础与缺口 |
|---|---|---|
| Memory | `core/nola-application/src/memory.rs`: MemoryCommand/MemoryCandidate/MemoryCommitIntent | 自动候选需质量评估；ExplicitUser/Imported/Synced 只走不同验证策略，入口身份与权限仍需核对 |
| Memory 并发 | 同文件 MemoryCommand::Replace、idle_memory_input_preconditions | 比对 canonical profile/冻结语义输入，冲突要求重读；不能以此推导全部调用者无旁路 |
| DataSource | `core/nola-application/src/data_source.rs`: DataSourceCommand/DataSourceReadResult | 已区分成功、空、拒绝、不可用、失败；RecordRead 校验 Pending/read_token |
| Automation 用例 | `core/nola-application/src/automation.rs`: AutomationRunCommand | enqueue/claim/heartbeat/retry/finish；Finish 接受调用者提供的 status/result/error |
| Automation 去重 | `core/nola-store/src/automation_store.rs`: enqueue_automation_run | 按 automation_id/occurrence_key 唯一；不是副作用 exactly-once 的证明 |
| Mac 唤醒 | `cli/src/background_scheduler.rs` | 唤醒 App，声明自己不拥有业务成功；marker/last-check 与准入的崩溃边界需验收 |
| Mac 执行收尾 | `NolaMac/Services/MacAutomationStore.swift`: finishRun 及其调用方 | 仍用 exitCode==0 决定 succeeded；已修提交拒绝/错误不发通知，精确 Run 结果绑定尚缺 |
| OS 注册 | `NolaMac/Services/MacBackgroundSchedulerService.swift` | 注册 LaunchAgent，不应拥有调度业务事实或任务结果 |

自动化的客户端收尾违反 INV-03/04/09：进程退出不是任务完成证明，未提交不能展示已持久成功。
整改需绑定 occurrence 与 canonical RunResult/terminal seal，而不只是把 try? 换成日志。

本轮先修 Mac 提交边界：finish 返回 false 或抛错时不写成功/失败活动、不发完成通知；
心跳拒绝/错误显示需协调状态并停止盲目续租；缺模型时取消已创建的心跳任务。
这是第一切片的历史问题记录（后续 AUTO-02 已替换共享接口）：runner 原先按定义 ID 使用 `--session automation-{definitionID}`，
claim 仅保存 occurrence ID/token，没有精确 Run/submission 绑定，退出码仍是旧结果来源。
不能查询该 session 的 latest seal 猜结果。后续共享准入必须同事务绑定 occurrence、定义 revision、
claim 身份、Run/submission/input，客户端再调用精确 seal reconciliation，读取已提交结果后通知。
续租失败后的在途进程派发隔离也仍需该权威绑定控制，不能只凭客户端错误标签宣称已停止业务执行。

## DATA-01：来源、授权和新鲜度

数据源定义必填 source_id、kind、location、配置 revision、授权状态、freshness policy。
每次读取准入绑定配置/授权 revision、read_token、Host identity 和 deadline；
Host 返回 content reference/digest、观察时间、schema version、record count 或明确失败变体。
Empty 是一次有效观察，不是错误；Denied 不是 Empty；旧数据也不能冒充本轮新数据。

配置修改、撤权、删除会使在途 token 失效；迟到结果只能登记诊断/历史观察，不能复活失效来源。
Store 必须在提交时再次核对 token/revision，不只由客户端发送前检查。
成功引用的内容需先安全写入，再提交引用；读失败是否保留旧快照由 freshness policy 决定并明确标为旧。
ResolveForConversation 同时检查所属范围、授权、撤销、新鲜度与会话 override，不能只看有无缓存文件。
来源文本是数据，不因进入 prompt 而成为系统指令；它不能授权记忆写入或后台任务。

## MEM-01：记忆候选不等于可持久记忆

候选输入包含来源引用、namespace/scope、类型、保留策略、内容、提出时间和质量评估。
质量评估不提供授权；ValidatedAutomatic 必须同时满足当前 long-term/automatic 策略与来源范围。
ExplicitUser intent 必须来自可信用户命令身份，不能由模型/客户端填写字符串而提升权限。
导入与同步保留原来源/版本，不把外部记录升级成用户确认或跨会话全局可用。

Application 负责候选验证、去重、冲突检查与原子写入；Host 只返回候选或读取结果。
关闭记忆后不得派发新的自动提取；在途结果提交前重查策略，不能因“启动时允许”继续写入。
删除/撤权后 recall 必须立即排除，索引重建不能复活 tombstone；retention 清理和 usage 更新不能重新插入已删除内容。
记忆 usage 仅对实际纳入已提交请求的引用计数，重放不重复计数。
Goal 约束不依赖长期记忆：长期记忆禁用不应丢失当前任务的权威工作记录。

当前会话公开 transcript 的 `conversation_recall` 与跨会话长期记忆分开：前者是恢复本次会话已可见
原文的能力，不新增长期记忆授权。工具 schema 不接受 conversation_id/session_id；Host 从执行
上下文取得 owner，Application/Store 再核对 message/part 所属。模型提供其他消息 ID 不扩大范围。
`memory_search`/`memory_save` 仍遵守独立的敏感权限，不能因 recall 可用一起加入内部免授权列表。
这不允许从全局数据库任意检索，也不允许把工具 JSON、隐藏内容或外部会话伪装成本会话公开文本。

## AUTO-01：调度事实与任务结果分离

```text
OS 唤醒 → durable occurrence → claim → 同一 Application Run admission
    → Core/Host 正常执行、授权等待、取消、恢复
    → canonical RunResult seal → occurrence reconciliation → 已提交通知
```

Occurrence 身份由定义 ID/revision、计划时间与触发身份构成，明确时区/DST/错过执行策略；
重复唤醒返回同一准入事实，不能因为进程非零退出重新造 occurrence。
调度定义更新不追溯改变已准入执行配置；禁用/删除是否取消在途任务必须显式选择并记录。
后台来源不自动继承当前前台用户权限，需授权时持久等待并通知，不能无界弹窗/模型轮询。

Occurrence 与 Run 的绑定应在准入事务中保存；不能进程执行后再猜它属于哪次计划。
Agent 类型自动化的结果必须来自精确 Run seal，像 Task bound-run 一样防止 worker summary 替代结果。
非 Agent 自动化也须有具体 effect receipt，不以退出码证明任意业务要求满足。
客户端仅报告进程/传输观察，不能提供权威 succeeded；进程失联先恢复核对，未知副作用不重跑。

Finished、Succeeded、Blocked、NeedsInput 的展示从已提交结果派生；执行片结束不等于目标达成。
定义停用与权限撤销不删除已发生的效果与运行历史。后台取消树沿用 RECOVERY 契约。
通知只发布已提交游标/结果，重复交付可幂等；提交失败时保留待协调事实，不谎报成功或创建第二执行。
心跳失败必须使 owner 重新核对执行资格，不能忽略续租失败后继续拥有无限派发权。

当前局部整改（尚未测试）：Store 的 retry/finish CAS 与 heartbeat 一样要求
`lease_until_ms > observation_time_ms`，不允许以到期 token 继续更新，即使尚未被另一
worker 认领。冷打开、恰好到期、到期后及重新认领后的旧 token 反例已补。
这只约束当前接口所提供的观察时间；仍需 Application 的可信接收时间与精确 Run
绑定，不能据此宣称恶意回填时间已被完整防止，更不能把 CAS 成功视为业务成功。

## 事务与验收

### AUTO-02：Agent occurrence 绑定接口（生产接线已实现，未统一验收）

定义必须携带有标签的 `execution_spec`：`{kind:agent,spec:AgentSpec}` 或
`{kind:user_task,task:UserTaskSpec}`。普通待办的日期、金额（十进制字符串）、
循环规则、完成状态和通知身份由 typed task 保存，id/name/时间仍由定义外层拥有；
不从 metadata 推断权限或执行类型。UserTask 可以持久化但明确拒绝 Agent enqueue、
claim 与准入（wrong-definition-kind），不得填充虚假 prompt。Core 执行 witness 仍为
纯 AgentSpec，不能将整个定义当作执行配置。iOS scheduled action 模型阶段实际进入
Agent，因此必须绑定 occurrence 的规范 Run，不属于可用 exit/status 自报完成的非 Agent job。
AgentSpec 的 `execution_scope` 显式区分 `local_workspace {root}` 与
`device_local {host_id}`，不再用空 workspace 路径冒充设备执行。Application 对设备范围
核对实际 Core Host catalog 的 host_id，且拒绝同时携带 workspace identity；CLI 对本地
workspace 从实际 project_dir 生成 witness。此范围标识不授予资源权限，工具仍须执行
具体 toolPlan、native Host 与资源授权检查。

**A08 衔接**：共享三路 typed 恢复主干已进入实现，详见 RECOVERY_AND_AUTHORITY 的
“未知副作用的唯一恢复决策”。A10 类型阻断解决后已恢复工作；跨端入口、旧旁路删除及
完整故障验收尚未完成，不能声称 A08 已收口。

原生 Host 准入采用单版本 `automation_execution_spec` 与
`automation_artifacts[{kind,id,path,sha256}]`。CLI 从实际 prompt、project_root、
effective permission profile 重建 spec，和定义声明精确比较；工作区外额外目录、
独立 permission rules、bash prefixes 等授权旁路拒绝，不从文字推断权限。
spec 放在 AgentCoreConfig 准入字段，不把完整正文塞进 TurnIdentity。

来源 ID→文件的关联由受信 Mac 原生入口负责：个人数据/每个网页各自生成 occurrence
快照；Skill 指向实际启用 catalog 的 SKILL.md。CLI 核对 refs 与 artifacts 一一对应、
Skill ID 与实际 catalog 路径，逐文件有界读取（单文件 4 MiB、合计 16 MiB）、核 SHA256，
只把这次读到的 UTF-8 正文加入 typed context。之后冻结正文而不重读可变源文件；Host
物理 Host invocation 只绑定真实执行环境与权限，不绑定一次性租约、prompt 或临时文件路径。
spec 由准入 Run configuration digest 绑定，实际已读正文通过 Core context 输入及 checkpoint
持久化；冷恢复从这份 canonical 状态继续，不重读文件、不重新领取准入租约。
新准入仍核 artifact 摘要与已加载正文一致。上下文内容不授予
工具权限；Skill 文档载入也不伪造 Skill 执行/完成证据。旧聚合 context-file、未绑定的
context_components/附件不属于自动化输入契约。读取/摘要不符在创建 Run 前失败。
个人/网页 artifact 还必须携 typed source（authorization、observed_at、maximum_age_minutes），
来源入口先检查真实授权与数据源开关，Host 再检查允许状态、真实时间与有效期，传给 Core
数据保护策略；不靠正文中的“新鲜/已授权”文字当证明。Skill 引用不需要伪造数据授权。
这些代码与反例测试源码尚待 goal 结束时统一验证，不能据此声称真实自动化已验收。

Mac 初次自动化与恢复共用 `MacAgentHostRuntime`，不再由 Scheduler runner 实现第二套模型执行循环。准入保存同一 canonical `mac_launch` 与实际模型 profile/protocol；重启用 occurrence 的精确 Run/assistant/digest 查询原配置和 checkpoint。恢复移除 admission lease/spec/artifact 路径，不重新 claim、不重新读源；实际源内容继续由 checkpoint 持有。持久等待与未知效果保留，最终结果仅经绑定 Run seal 的 Reconcile 消费。该接线尚待最终统一运行验收。

共享层现使用 `automation_execution_bindings` 将 occurrence 精确绑定 conversation、submission、Run
和 invocation config digest。`AutomationExecutionIdentity::for_occurrence` 对 occurrence ID 做 SHA256，
生成安全字符的四个稳定 identity（conversation/submission/user/assistant）；Mac 通过 Observe 取得。
不读取会话“最近一个 seal”，也不解析日志决定结果。客户端接线与最终验收另行核对。

采用既有 `BeginTaskAssistantTurn { lease, input }` 的同事务准入模式，而不是新增
绕过 Application 的 Store 写命令。已实现类型位于 Domain `automation_execution`、Application 同名模块：

| 用例 | 必填输入 | 提交前核对 | 输出与写入 |
|---|---|---|---|
| BeginAutomationAssistantTurn | occurrence_id、定义 revision、worker/lease 身份、稳定 submission identity、BeginAssistantTurn 输入 | 定义存在且授权未撤销；occurrence 精确归属；接收时租约有效；冻结配置对应该定义版本；没有不同绑定 | 同一 UoW 写正常 Run/message/checkpoint/effects、唯一 occurrence→conversation/submission/Run 绑定、准入回执 |
| ObserveAutomationExecution | occurrence_id | 从绑定读取精确 Run；无绑定明确返回未准入，不猜最新会话 | 只读返回绑定、Run 生命周期、权限/输入等待或封存结果；不启动执行 |
| ReconcileAutomationExecution | occurrence_id、稳定 reconciliation command identity | Store 读取已绑定 Run 的本地或正式接收的 seal；核对 conversation/Run/submission、result digest | 同一 UoW 写结果引用与通知待交付记录；重复协调返回原记录，不接受客户端 status/result JSON |
| ObserveAutomationWorker | occurrence_id、当前 worker 身份、进程/连接观察 | 观察者身份与当前认领一致 | 只记 Host 观察与恢复需求；exit=0/非零都不推导业务结果 |

准入故障全部回滚，不能留下“已绑定但无 Run”或无归属的已派发效果。同 submission
重放返回原绑定；同 occurrence 不同 submission/Run 为 Conflict。租约更替只变观察/
协调所有者，不解绑定、不重发旧效果。已绑定 Run 未封存时拒绝创建第二次执行；
真正需要另一次业务尝试必须是明确的新 occurrence，而不是传输重试。

结果展示以六类 RunResult 为准：Answered 表示答复，ActionCompleted 才表示动作要求
已验收；Incomplete/Blocked/Cancelled/Failed 各自保留原因与证据引用。occurrence 的
调度状态只表述 queued/claimed/bound/reconciled，不再与 succeeded 布尔值混用。
等待授权、等待用户输入和观察断开都不是 reconciled，也不是 Failed。

协调不要求原执行租约仍存活：它只引用不可变已提交 seal，可由重启后的调度器幂等
完成；必须与需要有效租约的派发/过程写入分开。通知订阅提交结果，不订阅进程退出。
提交失败保留待协调状态，不能发完成通知。通知按 occurrence/result digest 去重，
交付确认与业务结果独立；没有平台幂等支持时明确允许崩溃窗口内重复，不宣称 exactly-once。

迁移顺序：Domain/StorePort 绑定与结果引用 → Store 当前 schema/原子守卫 → Application
准入/协调命令与回执 → CLI runner 传精确 occurrence 身份 → Mac 只消费提交结果 →
删除 Agent 自动化旧 Finish(status,result_json) 路径。非 Agent 来源先列具体 effect receipt
契约再迁移，不将所有来源强塞进 Agent Run，也不保留旧成功注入作为兼容出口。

专项故障验收必须覆盖：准入写绑定前后回滚、同 occurrence 并发、definition revision
改变、旧租约迟到、exit=0 但结果 Incomplete/Blocked、seal 后进程非零退出、协调提交
失败、提交后通知前重启、重复协调、旧 occurrence 试图引用新 occurrence 的结果。
当前实现补充：

- `BeginAutomationAssistantTurn { lease, input }` 同事务写正常 Core/Run 与唯一 binding。
  完全相同冻结 input 的重试读取原 AcceptedAssistantTurn 回执，即使旧 lease 已过期也不重新授权。
  返回值是历史准入凭证，不是当前 execution；Host 必须随后读取精确已提交执行状态，不能采用旧 events/checkpoint。
  不同 input digest 为 conflict，不能以重试改变任务；专属 occurrence 会话不允许追加第二 Run，后续任务进入新 occurrence/新用户会话。
- `ObserveAutomationExecution` 返回 identity、binding、Run lifecycle、精确 seal、已协调 receipt 与 notification_delivered。
- `ReconcileAutomationExecution { occurrence_id, reconciled_at_ms }` 不接受 status/result JSON，
  原子写 receipt 与待通知记录，occurrence 状态为 reconciled；业务六态保留在 seal。
- `ObserveAutomationWorker` 只记录当前 worker 的 exited/disconnected/launch_failed 事实。
  旧 `AutomationRunCommand::Finish` / Store finish authority 已删除。
- `ListPendingAutomationExecutions { after_occurrence_id, limit }` 以 ID keyset 分页恢复待协调/未通知记录，
  limit 1...256；`AckAutomationNotification` 保留第一次交付确认时间且不允许早于协调提交。
- 定义新增必填 Domain `AutomationExecutionSpec`，单独存储 execution_spec_json；
  prompt/workspace_root/permission_profile/model_selection/context_refs/skill_refs 随定义 revision CAS。
  私有 metadata 不是授权来源。首次准入核 typed Host witness 与定义一致，并检查唯一实际 User item
  与定义 prompt 一致；配置完整 digest 在 Run 准入后冻结。模型策略明确为 use_current_default_at_admission，
  不宣称定义已经指定某个模型。Host 从实际参数生成 witness 的接线与 context/skill 内容来源证明仍需核验；
  仅相同 refs 不是实际加载内容受授权的证明，不能将当前静态改动声称完成全部 AUTO-02。

- 重复 occurrence、双 worker、lease 替换、定义 revision 更新：只有一个有效准入身份。
- 唤醒后/claim 前、Run admission 后、外部效果后/回执前、终态提交后/通知前分别 kill：
  核对恢复路径、副作用次数、绑定身份与通知去重。
- exit=0 但 Run blocked/incomplete，或 finish 提交失败：自动化不能显示 succeeded。
- 关闭记忆/撤销来源时自动候选在途：不提交越界内容，状态与原因可观察。
- DataSource 空结果、失败保留旧快照、过期、撤权后迟到结果分别验证，不只测试 happy path。
- DB 重启、索引清空重建、客户端重连后，记忆策略、来源状态和自动化结果一致。

用隔离 SQLite、真实进程与受控 effect fixture 验证；生产调度器注册和用户任务不作为故障注入目标。
性能采用 P03/P07/P08：调度延迟、读取/提取时间、提交时间、恢复时延分段，人工授权等待单列。
当前未取得完整纵向通过证据，不能将唯一 occurrence 或通过单测等同于完成本契约。
