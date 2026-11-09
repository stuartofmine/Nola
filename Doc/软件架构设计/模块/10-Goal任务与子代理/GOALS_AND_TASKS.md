> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Goal、Task 与证据接受契约

状态：Core 逐要求版本化接受已接入生产路径；跨端及完整端到端验收尚未执行。对应 MOD-10、LC-04/07、E2E-09。

CLI 已移除名为 `persist_goal_state`、实际直接返回成功的旧镜像占位入口及全部调用。
执行报告、Partial 审计、暂停/未完成与唤醒时间仅由对应 canonical Core 命令提交；没有
durable session 时不得返回本地“已暂停”成功。暂停分支采用 typed 原因，不以未知字符串
默认映射状态。详细证据不足原因保留在前置 Partial 审计，不依赖一个不写数据的 verifier JSON。
此清理及无会话拒绝反例仅完成源码，尚未运行验收。

## CLI Goal 输入与控制（2026-11-09）

- 无目标时 `/goal` 和 `/goal start` 将 `/goal start ` 放回现有 composer，等待用户输入，不能用空目标启动。已有目标时 `/goal` 显示当前目标和状态。
- `/goal [start] <objective>` 保留目标的空格、Unicode 和换行；`start` 可显式输入名为 `status`、`pause` 等保留词的目标。`-c` / `--constraints` 是明确的命令参数，必须同时有目标和约束；引号内的参数示例和 `--` 后面的文字按原文保留。
- CLI 不再生成额外的“默认约束”：当前工作目录不自动成为用户禁止越界的语义约束，权限仍由既有 Core/Host 策略判断。运行指导和约束来源分离，回合 contract 的 objective 使用持久 Goal 原文，不保存整段运行指导。
- `/goal pause`、`/goal resume`、`/goal off`（兼容 `cancel`）通过共享 Goal control intent 提交 observed Goal ID/control token，排队不代表已经应用。无目标时不伪报暂停成功。
- 已选择会话的 `/goal resume` 只恢复该会话；只有尚无会话时才查找项目恢复快照。未完成目标不能被新的输入悄悄覆盖；先明确取消，再创建。终态投影优先于客户端残留变量。
- 输入校验和控制提交错误保留草稿并返回 composer，不退出整个交互会话。普通消息仍是普通 user Turn，不自动视为 Goal continuation。

参考本机 `参考资料/codex-main/codex-rs/tui/src/chatwidget/goal_menu.rs`、`tests/goal_validation.rs` 的独立目标输入、显式操作和多行原文测试；没有复制其实现或新增持久协议字段。具体验证与仍未解决的问题见 `Doc/当前任务/CLI_GOAL_INPUT_AUDIT_20261109.md`。

## 当前所有权与类型

实体、数据/上下文所有权及本轮范围以[会话、Run、Goal 与子任务边界](AGENT_THREAD_ARCHITECTURE.md)为统一入口。`continue` 自动推进，`incomplete` 结束本次尝试并保留未完成 Goal，不自动唤醒；profile 轮次是默认值，Task 显式预算沿既有范围覆盖并冻结。

### 暂停与领取执行权不能互相替代

`Claim` 只分配执行租约，不授权解除用户等待、失败或未完成状态。
普通 `Pause(Paused, None)` 持久化 `Suspended`，不再伪称 `Scheduled`。
只有 `Paused + Scheduled + 已到期 next_wake_at_epoch_ms` 可以由调度器直接领取；
Running 可正常领取或在旧租约过期后换 worker。其他可恢复状态先显式提交 `Resume`，
再领取租约；拒绝领取时整个投影和 revision 不变。
`NeedsUser`、`Failed` 不接受 wake 时间；自动重试必须是显式调度状态转换，不能把
“等待用户”同时写成“稍后自动执行”。`ConcludeIncomplete` 保留 `Incomplete`，无自动唤醒。
`ResumeDue` 同时核对 Scheduled 原因和到期时间，不能只看 Paused 标签。
租约有效区间的右端不包含 expiry：到期时旧 owner 的 heartbeat 拒绝，
新 worker 可领取，二者不再对同一个到期时刻采用不同边界。
已补投影序列化/恢复、拒绝后不变、显式恢复及到期领取的测试源码；尚未执行，
这些源码不代替 Application 落库和真实调度器端到端验收。

| 职责 | 实际入口 | 当前事实 |
|---|---|---|
| Goal 语义 | `core/src/agent/goal.rs`: GoalProjection/GoalState | revision、目标、约束、requirements、contracts、execution_reports、audit_reports |
| 工作记录 | `core/src/agent/goal/work.rs`: GoalWorkState/GoalWorkEntry | 用户原文、来源 Turn、所属 Task、约束/细化/决策/进度/阻塞/下一步；supersedes 保留历史 |
| 更新校验 | 同文件 GoalProjection.update_work | goal identity、expected revision、时间、数量与字段大小、来源校验 |
| 完成接受 | `core/src/agent/runtime/goal_work.rs`: accept_goal_completion | source Turn Completed 且 Answered/ActionCompleted 与当前 Core result decision 一致；子 Task 已终态、逐项 claim 覆盖；构造 audit 后 complete |
| 可引用证据 | 同文件 observed_goal_evidence | 从原始 transcript/ownership 读取，不从压缩摘要；包含成功 ToolResult 与 Assistant 正文 |
| Task 生命周期 | `core/src/agent/task.rs`: TaskProjection/TaskHandle/TaskState | queued/running/waiting/suspended/retry_waiting/completed/failed/cancelled |
| 子运行绑定与 receipt | `core/nola-store/src/runtime_task_run_binding.rs` | 绑定精确子 Run、校验 terminal seal，提交父任务结果；见 COMPLETION 的结果丢失问题 |
| 可读工作文件 | `core/nola-store/src/work_archive.rs`: publish/goal_markdown | 导出 goal.json/md、tasks.json、evidence.json、checkpoint；标明 baseline revision/replay_pending |
| Application 查询 | `core/nola-application/src/session_projection.rs` | 根据恢复后的 canonical session 构建 Goal/Task 投影 |

导出的 Goal 文件当前确实是可读视图，不是可直接改写的第二状态库。
baseline + replay_inputs 不等于已重放的最新 Goal；任何执行前恢复必须走 Application 正式重放。
不能只读取 goal.md 就声称已经恢复最新约束。

## 已发现的模型冲突

1. GoalStatus::Failed 的 is_terminal 为 false，而 TaskState::Failed 为 true。
   两者可以有不同生命周期，但 Failed 的恢复条件必须显式建模，不能靠消费者猜测。
2. accept_goal_completion 原先只检查 source Turn 的 Completed；当前要求明确的
   Answered/ActionCompleted 且与当前 Core result decision 同类，缺失 outcome、缺失结果
   或不一致均拒绝，不能从旧 Completed 状态补推成功。恢复反例源码已补，尚未运行。
   当前 child Task 检查除终态外，还要求失败/未完成子任务具有准确 terminal receipt、result digest 与父级执行替代证据；成功子任务不得伪作替代目标。
   子任务失败不必永久阻止父 Goal，但不能忽略失败。
3. GoalEvidence 原 independently_verified 曾混淆独立检查与引用存在性，包含模型正文也会填 true。
   现改为 source_verified；GoalAuditFinding 必填 assessment_method，普通 AcceptCompletion
   明确为 ModelJudgment，显式检查报告可标 EvidenceInspection。后者是评估方法声明，不授予权限，
   也不自动证明独立评估者身份或检查真实性。来源与要求满足不能再共用一个“独立验证”布尔值。
4. Goal completion 需要 GoalAuditReport，但 AcceptCompletion 会本地构造该 report；
   不能据此宣称每个 Goal 都强制额外模型审查，也不能以 report 名称证明独立验证发生过。

整改不能只是把 independently_verified 改成 false（那会让合法纯文本交付也无法完成），
也不能再增加一个强制 reviewer。需要明确要求的接受方式、证据来源与判断主体。
当前增加逐要求 semantic_revision 与持久 RequirementAcceptance 历史；真实接受入口从
Core 当前 session/Turn 派生 assessor，从未压缩 transcript/ownership 取得原始 item 引用。
结构化 Start 可准入精确 predicate/version 检查；Core 只接受实际 Host observation 中
Completed receipt 的同 resource/version/predicate 且 passed 的检查，不将方法标签当检查真实性。
新 finding 缺 assessment_method 明确解码失败，不加默认或旧字段 alias；新 evidence 使用 source_verified。
旧 Goal 审计数据格式不兼容，本轮没有迁移、删除或改写用户已有数据库；协议切换仍须受控处理。

## GOAL-01：目标与要求的版本化契约

Goal 命令包含 goal_id、command_id、expected_revision、来源身份、时间与操作内容。
目标和用户约束保留原文及来源；模型细化记录为 Model origin，不能覆盖 User origin 授权。
Requirement 必须有稳定 ID、描述、依赖、交付/接受条件与对应证据引用。
条件可以是定性要求，不要求将无限用户需求编码成有限关键词或固定业务场景。

接受方式必须明确：可执行检查、事实观察、模型语义评估或用户确认；组合由任务要求决定。
这不是一份自动权限清单，也不能由模型在收尾时偷偷降低已约定的标准。
改变验收条件是独立带来源的 Goal 更新，保留旧 revision 与理由；用户约束变化需有用户来源。
没有明确数值要求时不能凭空编造“必须达到 99%”等验收目标。

已落地的并发约束：GoalAuditReport 必填 expected_goal_revision，apply_audit 在任何修改前
核对精确 GoalProjection revision。工作细化/用户输入/其他已提交更新使旧报告失效时，
返回明确冲突且状态不变；调用者须重新审视当前事实，不能自动刷新 revision 后原样重交。
普通 Core 接受路径和 CLI partial report 均携带其实际读取的版本，不增加独立模型审查请求。
complete 也要求当前 revision 恰为该报告提交后的版本，阻止“验收后更新、再用旧报告完成”。
已消费报告后发生更新，需要新的执行/评估契约记录重新判断；保留旧证据但不沿用旧判断的版本。
单元回归覆盖这条显式重评后完成路径；尚未验证模型自动规划该恢复路径的真实端到端行为。
整个 Goal revision 仍用于并发 CAS，可能对非语义更新产生保守冲突。除此之外，要求本身有
semantic_revision；完成 claim 必须携当前 requirement_revision，不能仅刷新全局 CAS。
start 由 Core 分配要求版本 1，语义修改按受影响要求及依赖闭包各增一次；溢出整批拒绝。
非语义进度不改要求版本。旧 acceptance 与 audit/evidence 原样保留，不默认补给新版本。
旧报告缺版本会解码失败，没有默认/兼容 alias；不自动修改现存用户 checkpoint。

注册事实/产物的 ID 为不可覆盖身份：重复注册先拒绝，不得先替换旧值再报错。
失败前后整个 GoalProjection（包括 revision、已有证据与记录）必须完全相同；
如需修订必须使用明确的版本化操作，不能复用注册命令覆盖已引用来源。

### Goal operation 的语义重放

`GoalProjection.applied_operations` 是 schema 16 checkpoint 的必填字段。
外层 `AgentThreadState.goal_projection` 同样必填，即使当前没有活动 Goal 也必须保存
空或历史投影；缺整个投影不能默认补空而清除幂等身份。普通 checkpoint 解码与
新 Turn 的历史恢复入口均拒绝缺投影/缺账本，拒绝不删除或改写原文件。
账本条目包含 operation_id、规范化 typed command 的 SHA-256 和首次成功 Goal revision。
该账本属于同一 Core session，
跨 Goal Start 保留，不按时间淘汰；存储量随成功控制命令数量增长，不宣称常量内存。
同 ID 同命令返回当前投影，不重新执行、不生成 GoalUpdated、不增加 Core/Goal revision；
时间、CAS、内容任一变化都是冲突。失败命令不占 ID；预检、变更与成功登记原子提交。
正面 AcceptCompletion 和 ApplyAudit 与普通控制使用同一入口，不可绕过去重。
GoalRuntime 不进入通用 id-only accepted_submission 集合：必须先走内容绑定校验，
也不能与普通输入/其他能力恰好相同的 ID 相互吞掉命令。

它不是 Application transport receipt：CLI 每次重试可能创建新 request_id/commit 时间。
外层 App 因新请求仍可写新 receipt，idle checkpoint 的 Store CAS revision 也仍递增，
但 Core 语义状态和事件序号不变。返回当前投影不声称是历史请求的原回执，也不回灌旧状态。
旧回执查询继续使用原 Application receipt 身份；operation_id 只标识命令，不授予权限。

明确的 Constraint/RequirementDetail 更新会使所指要求及其传递依赖方的旧完成验收失效，
Completed 回到 Pending；无 requirement_ids 的 Constraint 作用于全部要求。
supersedes 同时考虑旧记录和新记录的作用域，不能通过换目标悄悄保留旧条件下的完成判断。
保留历史 audit reports 和 evidence_refs；Pending 下的引用只是可重新评估的来源，不是仍然有效的验收。
Progress/NextAction/普通 Decision 不因标题或自然语言内容自动改变要求状态，也不将任意用户文字
按关键词路由成约束更新。未形成明确类型化修改的用户输入仍由 Core/模型按正常流程解释。
整个 work batch 验证成功后才同时更新记录与失效集合，失败不能部分清除旧验收。
这仍是现有 GoalProjection 内的语义决定，不是新的客户端/SQL 状态机。

评估报告按整批拟提交状态校验依赖与未满足项，不能用更新前的 Completed 状态绕过检查。
若报告撤销前置要求的完成状态，仍标为 Completed 的依赖方会使整批被拒绝；
报告须同步撤销依赖方的旧判断。拒绝保持原投影不变，接受后未满足项使用新状态计算。

## GOAL-02：来源可信不等于要求满足

### A09 当前单版本接受输入

本次必填 policy、typed 观察和 claim 来源历史采用 checkpoint schema 16；缺字段拒绝，
不为旧版本补 ModelJudgment，也不迁移或改写用户现有数据库。

`GoalRequirement.acceptance_policy` 在结构化 Start 准入时固定：`model_judgment`
明确承认语义判断，`execution_checks` 携精确 resource_uri/version/predicate 清单。
普通文本 Goal 入口明确选择前者，不按自然语言关键词生成检查，也不声称前者是实测。
计划与模型 work 更新不能改写 policy；缺少所需检查时只可报告未完成/阻塞。
执行检查来自 Core 实际接收的 ExecutionEvidenceObserved，保留原观察身份、Turn、Host
来源和 typed receipt 检查投影，不从正文、exit 标签或摘要生成 predicate。
完成 claim 分开引用普通来源与 execution_evidence_refs；负面子结果的替代还须明确
task_id、terminal_receipt_id、result_digest 和本次替代执行证据，原子校验后才可接受。
每次真实执行观察在同 transition 分配唯一递增 recorded_goal_revision（Goal 账本顺序，
不是 Core cursor 或物理结束时间）。接受 CAS 固定观察快照；同 resource/version/predicate
只能采用此快照最新观察，最新失败或未完成压过旧通过，后续新通过可恢复。
不同资源版本互不覆盖；重复顺序、未来观察或来源不归属该 Goal 明确拒绝。
这些是同一 Goal checkpoint 的来源投影，不是新的客户端事实库；具体端到端验收尚未执行。

目标证据记录演进现有 GoalEvidence，不平行创建客户端/Host evidence 真相：

```text
Evidence {
  id, source_identity, source_revision, observed_at,
  origin: HostReceipt | CanonicalAssistantPart | UserDecision | ExternalObservation,
  payload_ref, resource_identity?, resource_version?, limitations[]
}
RequirementAcceptance {
  id, audit_id, contract_id, requirement_id, requirement_revision, evidence_refs[],
  method, assessor, judgment, rationale, assessed_at_epoch_ms
}
```

Core 校验归属、存在性、版本、执行状态、来源、接受方式及未决义务；
模型判断语义相关性时必须明示该方法，而不是升级成 independent verification。
纯写作/解释可以由 canonical 正文满足交付；正文“测试通过了”不能替代要求中的真实测试。
read_file 返回内容证明读取观察，不自动证明业务性质；exit=0 也只支持对应检查的声明。
独立 reviewer 是可选的模型策略或显式任务要求，其报告同样需要来源、范围和证据，不能授予权限。
引用同一条证据满足多个要求可以合法，但每项都有单独理由；不要求重复执行工具以凑数。

生产接受路径为 `GoalRuntime::AcceptCompletion → accept_goal_completion → apply_audit →
complete`，以 cloned GoalProjection 完整校验后一次发布。输入是精确 Goal CAS、执行 contract、
逐项版本与证据 claim；输出是 GoalUpdated 同信封内的当前投影及不可覆盖接受记录。
acceptance ID 由 audit ID 与 requirement ID 的无歧义编码生成，重复 audit ID 拒绝覆盖。
complete 检查每项最新接受记录的版本与 judgment，而不只看 requirement.status。
alias 如 response 只作为选择引用的便利入口，持久 evidence.source 保存真实 session、Turn、
原始 item 身份；引用通过所属 audit 解析，不以压缩摘要重建。

外部 `ApplyAudit` 仅可报告未满足状态，不能自行传 Completed 或 EvidenceInspection 获得接受；
其 assessor 由接收到的 operation ID 生成。普通完成属于 ModelJudgment，assessor 是真实源
session/Turn，不新增 reviewer 轮次。特殊检查方法需要另有可信事实入口，当前不得伪造。
子 Task 终态只解除未收敛调度阻塞，不自动创建 acceptance；父方仍逐项提供独立保留来源，
失败子结果不被改写。此实现不宣称模型语义判断能自动证明任意业务性质。
GoalSubtaskContract 单版本字段为 proposed_checks、additional_execution_constraints、
requirement_versions；前两者仅是每轮建议检查与附加执行限制，不替代 Goal 原要求与用户
约束。CLI 从实际 Goal 投影构建要求版本表，Core 准入核对其恰好覆盖 relevant_record_ids
中的要求；启动执行和 Passed 审计再校验当前语义版本。finding 不得验收 contract 外的
记录或新版本要求，acceptance 明确绑定 contract_id。模型上下文同时显示原要求版本与
本轮版本表，计划正文不成为另一套接受条件。

执行中发生明确条件修改时，旧计划可以留下执行/未满足记录、以 Partial 收束后重新规划；
尚未执行的 Planned 合同允许 RuntimeOperation 提交无 finding/无 evidence 的 Partial 记录
后重新规划，不伪造 BeginExecution，也不创建任何 requirement acceptance。
不能只刷新全 Goal CAS 就按旧计划验收新要求。逐项版本变化仍只由明确类型化 work 条件
修改触发，不从计划文本差异猜测授权。检查来自真实 Host receipt；没有对应适配器或观察
时无法满足检查 policy，不能把版本化接受记录等同于所有检查方法均可执行。
Core 拒绝完成后 CLI 提交 Partial 并以 ConcludeIncomplete 结束本次 Goal attempt，
保留 Goal 要求和未完成原因、不设置用户等待或自动唤醒，不无条件再调用模型求同一认证。
后续显式继续可重新规划；已封存子 Run 的原结果不因父 Goal 未满足而改写。

## GOAL-03：Task 不是父 Goal 的另一份副本

创建子 Task 必填父身份、任务目标、相关要求 ID、输入引用、边界约束、权限上限、预算、取消关系。
传入上下文是有来源的任务快照，不能复制父会话全部权限或可写 Goal 状态。
子 agent 可提交进度、证据与结果；不得自行修改父 Goal completed 或用户约束。

Task 生命周期描述调度；子 RunResult 描述这一执行片的业务结果，两者各自保存。
父 Core 消费精确 receipt，按 requirement/evidence 更新父目标；receipt 去重必须跨重启保持。
blocked/incomplete 子结果保留未完成项，不作为成功，父任务可选择补做、澄清或报告限制。
子失败不会自动令父 Goal failed；父接受后也不能篡改已封存子结果以掩盖失败。
等待授权不占用模型重试；等待结果要有 wait_id 和唤醒条件，不靠轮询模型询问进度。

## GOAL-04：进度、取消、恢复与文件

绑定 Run 的 Task 取消分为两次提交：第一次只持久化 `cancel_requested` 与首次请求
时间，保留原状态、attempt、权限决定及已发生副作用证据；重复请求不得刷新原时间。
这不是 cancelled 回执。普通 claim/权限恢复必须拒绝已请求取消的 Task。Supervisor
在恢复时使用原 Task→Run binding 派发 cancel-only，不启动普通模型执行；Core 完成
效果清理并封存后，Application 才将原 seal 对账为 Task 的最终结果。未知或不可读
Run 保持取消待确认，不能由租约过期另造 failed/cancelled 回执。未绑定且尚未派发的
Task 仍可由 Store 直接取消，不能将此规则套到已经准入的 Run。

进度的 ReportedDone 是执行者陈述，不等于 requirement accepted。验收进度来自逐项接受事实。
无进展判断比较可核对的来源 revision、产物/检查、解除的阻塞；不能只比较回答文本或 token 数。
临时等待与没有安全下一步不同：前者恢复同一 wait，后者持久 blocker/解除条件并停止无意义调用。

父取消先提交取消树，再派发子取消。取消确认、已发生的副作用、未知结果独立保留，详见 RECOVERY。
重启按 checkpoint + replay + Task receipt 恢复，校验权限与依赖，不用摘要重建执行权。
恢复不改变原 Goal；后续 Run 引用同一 Goal revision/要求，不把“继续”当成新目标丢失约束。

文件导出必须记录 manifest、canonical revision、是否待重放、内容 digest；导出失败不能回滚已提交业务。
导出不可写回即应明确标注；若支持编辑，必须解析成命令/CAS 再提交并重新导出，禁止双写。
归档包含敏感任务与工具信息，权限和保留策略跟随所属会话；不能因是 Markdown 就公开索引。

当前普通命令 ACK 只承诺数据库提交，不等待派生归档。schema 62 在源写事务中持久化
全库 target generation，后台执行者按一致性快照发布并确认 published generation；唤醒通道
只是提示，重开数据库仍能恢复 backlog。发布失败不撤销业务提交，也不能伪装为原操作失败重试。
文件锁串行化发布，失败退避和待发布版本可查询；显式 `flush_work_archives` 与普通 ACK
是不同承诺。该阶段仍为全库扫描，不宣称逐会话增量发布或五端已呈现归档状态。

归档 worker 的关闭也是持久化生命周期契约：同一数据库的 Store 共享 worker；关闭
其中一个所有者不能停止其余所有者的发布。最后一个所有者关闭时必须断开唤醒通道并
join 线程，确保 worker 自己的 SQLite 连接已经释放后才返回。仅发送异步退出通知
不足以允许立即清理数据库目录，会与延迟生成的 WAL/SHM 文件竞争。后台线程不持有
自己的 Worker Arc，也不注册第二个归档 worker，避免引用环或 self-join。关闭不承诺
全部派生文件已经发布；未完成的 publication frontier 保持落库，由下次打开接续。
历史同步路径的分段实测、恢复验证及新测量见 PERFORMANCE_BASELINE。

## GOAL-05：项目身份与三种结果消费者

项目分组不授予权限。现有 Store 写入会 canonicalize project_root，查询必须使用同一规则；
否则 macOS `/var` 与 `/private/var` 等别名会把已落库 Task 从列表、恢复扫描和通知中隐藏。
本轮抽取同一个规范化函数用于准入及查询，不在 UI 增加路径别名列表。
规范化失败沿用既有精确路径规则，不扩大到父目录，也不扫描其他项目来补齐结果。
这还不是稳定 ProjectId：目录移动、删除、符号链接重定向后的身份恢复仍需独立设计，
不得宣称 canonicalize 可以保证永久身份；所有读取/控制另核对 owner conversation。

Task 的三个消费者互不代替：

| 消费者 | 输入与确认点 | 不能代表什么 |
|---|---|---|
| 父模型 mailbox | 精确 task、owner、terminal receipt、projection revision；Host response 提交后消费 | UI 已展示、父 Goal 已满足 |
| UI 通知 | 不可变 notification occurrence 与所属会话；展示交付成功后单独 ACK | 模型已消费、用户已经读懂 |
| 父 Goal 验收 | requirement revision、证据与 acceptance | 修改子 Task 原结果或吞掉通知 |

当前模型 AcknowledgeRuntimeTaskResult 会核对 immutable receipt 与任务投影，更新 unread/model_notified，
并不设置 ui_notified。这条区分应保留。CLI Agent Task 已将 take_runtime_task_notifications
改为只读准备，逐条交付展示历史与输出帧成功后才提交精确 ACK。
Store 在既有事件日志内验证 owner、terminal receipt / permission occurrence、consumer、command；
UI 交付不更改模型 mailbox。Shell 当前只修正交付时机，仍缺逐 consumer 的精确身份协议。
具体代码、测试及限制见 [实现记录](../../../历史任务/实施与审计/TASK_DELIVERY_IMPLEMENTATION.md)。

目标通知接口为只读 ListPendingNotifications(scope, cursor, limit) 与显式
AckNotification(owner_conversation_id, task_id, occurrence_id, consumer_id, command_id)。
终态 occurrence 绑定 terminal receipt；授权 occurrence 绑定 permission_request_id。
存储以 occurrence/consumer 唯一去重，重放同一 ACK 幂等，旧 occurrence 不确认后来的授权请求。
查询失败不改变消费状态，UI 无法展示不 ACK；ACK 丢失用同一身份重发。
通知正文只引用 canonical 状态，不成为额外完成真相，拒绝/取消通过业务命令而不是通知 ACK。

终端绘制与数据库提交不能原子化，不能承诺用户可见 exactly-once。
持久通知采用至少一次交付；展示后、ACK 前退出可能再次展示，前端按 occurrence 去重。
canonical Task 历史始终可查询。若只进入易失 renderer queue，不能声称已经持久展示或已被用户阅读。
通知背压/分页归入 P06，记录 pending 数、最老等待时间、交付重试数和 ACK 延迟，不用全表轮询掩盖积压。

## 验收要求

- 保留既有 `evidence_survives_summary_only_context_and_rejects_ambiguous_or_foreign_refs`：
  它证明引用来自原文/所属 Turn，不证明语义已经满足。
- 同一要求分别提供仅正文宣称、真实观察、正确版本 check、过期 check；核对 method 和 acceptance。
- 源 Turn 为 blocked/incomplete/needs_input 时，不能因生命周期结束而接受未完成行动。
- 子 Task failed 后无替代证据拒绝完成；父补做并提供正确证据可以完成，不要求伪造子成功。
- 更新约束后旧验收失效；压缩、重启和导出恢复后目标、约束、剩余工作、取消树保持一致。
- 独立审查未请求时不新增 reviewer 调用；显式要求时保留其范围与证据，不绕过工具权限。
- 文件 baseline 存在 replay tail 时，正式恢复值与 Application 最新投影一致；不能仅比较 JSON 文件存在。

以上覆盖 E2E-09，并依赖 COMPLETION 的不可变结果和 RECOVERY 的取消/未知效果契约。
目前未取得全部通过证据。

### 完成声明中的两类证据

`evidence_refs` 引用本 Goal 已提交的 `tool:<call_id>` 普通工具结果，或实际答复交付的 `response`。`execution_evidence_refs` 只接受同 Goal、`outcome=succeeded` 且 `state=completed` 的 typed observation；观察列表中的 `state=null` 不具备该资格。无 typed checks 要求的读取/答复任务可以使用普通证据并让 typed 数组为空，不能将普通成功观察升级为 typed completion。声明仍由既有 Core 验收规则校验。

Application Service 的连接工作线程显式预留 16 MiB 栈，用于当前完整冻结快照的恢复/校验（含未优化 Host）；这是 Host 运行资源，不改变 Goal 预算、重试、权限或验收规则。

前台 Host 续接等待已保留的同一 Host，并在取得后重新校验精确 Run。若其他消费者已将该 Run 封存，适配器返回“无需执行”并立即交回原 StepGoal 消费/验收；不将此情况当恢复失败，也不要求用户再次唤醒。后台 Busy 仍走既有延后路径，不替换活跃 Host；owner、lease、冻结绑定及终态校验不变。

## 客户端目标输入（2026-11-10）

CLI 裸 `/goal` 进入明确的目标输入状态，目标原文通过类型化入口提交，不能重新走 slash 命令解析。输入期间不消费普通聊天历史或排队消息，Esc 只取消表单。图形客户端先说明目标用途与示例，附加约束可选；没有当前会话时先引导建立会话。停止确认绑定显示时的目标 ID 和 control token，切换会话后废弃旧确认。状态和权限仍由共享 Core 决定，客户端不添加隐式目标约束。

Goal 执行端口可在单个发布事务内缓存已解析的目标投影；提交、创建回合和日志回放前使缓存失效。它不是持久化协议或跨请求缓存，未知格式继续明确拒绝。交互细节与本轮验证范围见[检查记录](../../../当前任务/UI_INTERACTION_AUDIT_20261110.md)。
