> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 收尾纵向契约：从证据到不可变终态

## 未接纳的控制提案不能被普通答复掩盖

2026-10-13 真实评测 `freight-dispatch-shift cWL5T6d`：模型提交错误 scope 且
缺少 answer 的 incomplete 提案后，运行封存为 answered。源码中控制批次拒绝会
回滚要求状态并生成 is_error ToolResult，但此专用分支未登记待纠正协议状态；
现有回滚测试只证明没有部分修改，不证明随后正文收尾的结果正确。

整改契约：控制调用完成与控制变更获准是两个不同事实。被拒绝的 requirements
批次必须留下 Core 拥有、可 checkpoint 恢复的待纠正状态；普通正文不得将其
消除。后续合法的同类控制调用可以纠正；无后续纠正而收尾则 Incomplete，附
协议拒绝证据，不变成执行 Failed、外部 Blocked 或等待授权。取消/真实执行失败
仍保持既有优先级。不能因为拒绝而强制无限重试，不能根据模型正文的措辞改状态。

验收需覆盖完整模型响应入口：错误 scope、缺失 answer、混合控制批次 →
错误工具回执 → 普通正文 → Incomplete；随后合法修正 → 提案结果；恢复后
拒绝状态仍在；无控制错误的普通只读答复仍允许 Answered。当前已接线独立
`control_recovery` tagged state（checkpoint 19）及模型入口回归，尚待统一测试；
不能以已存在的 `apply_requirement_control` 原子回滚单测宣称全部通过。

状态细化：拒绝记录与 scope/assessment/proposal 分开，由 Core
模型响应准入分支写入，包含被拒绝的调用身份及拒绝原因。不复用工程工具的
`recovery_tool` 单槽：控制调用失败不能覆盖另一个尚未纠正的工具协议问题，
控制调用成功也不能清除它。仅在同类控制变更原子准入成功后清除对应拒绝记录；
正文、工具显示状态、压缩及 Host 重连均没有清除权限。新 Run 独立初始化，
同一 Run 的 checkpoint 恢复必须保留；新增持久字段须显式升级 checkpoint，
不默认补字段、不迁移用户数据库。拒绝后的收尾应在结果前提交 runtime error
fact，并由结果来源引用，而不是用已失效的上一轮 model source 伪装裁决来源。

评测证据边界：`message_parts.content_text` 不保证包含流式正文，流式正文另在
canonical stream heads/manifests/chunks。只导出 message_parts 的报告不能证明
答案为空或丢失。导出器须保留 generation、chunk index、manifest root 与当前
head，禁止把不同 generation 拼接成答案；导出数据本身不等同于 seal 校验或
官方任务通过。此项与控制拒绝状态修复共同验收。

Core 已观测到非临时外部修改（现有工程状态 mutation_count > 0）时，普通
`Answered` 不再是合法完成出口：显式 answered 提案拒绝，未提案的正文收尾为
`Incomplete(requirements.unassessed)`。只有经评估的 ActionCompleted 才证明交付；
这不制造新工具调用或自动重试。只读调查与声明为临时的辅助修改不因此强制动作验收。
这项守卫覆盖现有类型化修改观测，不声称穷尽所有远端副作用或替代任务范围建模。

模型通过 `requirements_update` 提交 `propose_result` 时，必须在同次调用的顶层
`answer` 提供非空用户说明（包括未完成/受阻），因为终态提案获准后不会再生成一轮答复。
缺失说明是可纠正的工具协议错误，整个控制批次回滚，不提前发布 assessment 或提案；
不能以“已有工具输出”替代用户答复。这不要求系统取消/预算停止等非模型终态制造模型文本。

## Shell 文件修改观测边界

声明的输出文件由 Host 在执行前后读取实际状态；`edit` 可以由内容或文件权限位变化
证明，不要求字节内容一定变化。Unix 观测 mode 的权限/特殊位，其他平台观测只读属性；
普通文件属性来自已经打开的文件句柄。时间戳变化本身不算修改证据，不通过识别
`chmod` 等命令名称推断成功。相同状态仍为 unchanged，创建/删除仍要求存在性转换。
此指纹仅用于文件输出变化对比，不是授权凭据，不替代内容版本、验证结果或 Run seal；
观测到文件修改也不等同于用户任务验收成功。目录/ACL/扩展属性不由此契约声称覆盖。

状态：原 Core 结果事件、Store V2 封存和 Task/Skill 消费已接线，当前批次尚未统一
编译、测试或真实验收。日期：2026-10-11。
本文件细化 LC-01～LC-06、MOD-02/03/04/06/10/13；不是另一套状态所有者。
具体 DTO/事务、后继输入、清理阶段和版本切换约束见 [TERMINAL_PROTOCOL](../05-执行与恢复/TERMINAL_PROTOCOL.md)。

诊断 Trace 不是结果裁决入口。2026-10-12 静态审查补正：工程状态没有待验证债务时
输出 `not_required`，不凭一个 false 布尔值宣称 `satisfied`；是否真正执行并通过检查
仍看实际证据与 canonical result。外部 validation metadata 的 failed/error 保留
Failed 分类，incomplete 保留 Incomplete；自由格式 blocked 仅表示未完成诊断，
不得据此制造等待授权或 canonical Blocked 结果。原始 metadata 原因保留。
相关投影反例已补源码，尚未运行；跨端必须继续以 seal/result 与真实 pending request
区分收尾和等待，不能用 Trace 标题改变任务状态。

## 1. 原始审计反例与当前调用链

下表保留整改前的信息损失链，不是当前协议说明，也不是保留旧 API 的理由。
路径均相对仓库根；函数/类型名作为定位锚点，避免依赖容易漂移的行号。

| 边界 | 当前入口/类型 | 观察到的行为 | 必须整改的契约 |
|---|---|---|---|
| 工具事实 → 证据 | `core/src/agent/turn.rs`: `record_unverified_mutation`, `apply_successful_validation` | 未知范围记为计数；已知目标检查不能清除此计数 | 逐项保存来源、目标、版本、检查和可解决性，不伪造通配验证 |
| 证据 → 模型请求 | 同文件 `model_instruction` | 未知范围时要求报告限制，不反复修改/检查 | 这个出口必须有合法的 Core 决策与持久终态 |
| 证据 → Kernel | `core/src/agent/runtime/submission.rs`: `ModelCompletionFacts` 构造 | 汇总为 validation/recovery/skill 三个布尔值 | 不丢失为什么不能完成、是否有下一步以及证据引用 |
| Kernel → 运行层 | `core/nola-kernel/src/model_completion.rs`: `plan_model_completion`, `retry_or_fail` | 普通文本 + 未清 validation/recovery/skill 债务仍丢弃重试，计数到 3 失败；预算最终总结另有 CommitAndFail 分支 | 业务受阻不能作为协议失败重试；有具体可执行动作才继续 |
| 审查 → 运行层 | `PublishReviewedCandidate` 分支 | blocked 仍发 Failed/TurnFailed；needs_input 已要求结构化 question_set 并开启持久 wait，不再完成 Turn | 受阻结果仍须贯通 seal；不得将已修的用户等待与 blocked 混为同一未实现项 |
| 运行层 → 生命周期 | `core/src/agent/runtime/turn_lifecycle.rs`: `complete_turn`, `promote_queued_input` | 开放 wait 保持 AwaitingInput；current 输入先继续；完成后仅记 TurnCompleted 与 memory job，NextTurn 由 Application 在旧 Run 封存后独立准入 | 独立后继所有权已落地；仍需结构化结果及 memory/清理事实进入一致封存契约 |
| Core → Application | `core/nola-application/src/lib.rs`: `accept_host_event_in_transaction`, `reduce_and_commit_active_submission_in_transaction` | 两条路径都用 TurnCompleted 的 `..` 忽略 outcome；fallback 只读取 turn.status | 两条输入路径与 checkpoint 恢复必须产生相同结构化终态 |
| Application → seal | `core/nola-domain/src/assistant_run_terminal.rs`: `AssistantRunTerminalSeal`, `AssistantRunTerminalStatus` | 不可变 seal 只有 completed/failed/cancelled；不含 TaskCompletionOutcome | 任务结果及原因必须纳入 seal digest，不能只存在 checkpoint/诊断 |
| seal → 子 Task | `core/nola-store/src/runtime_task_run_binding.rs`: `reconcile_runtime_task_bound_run_terminal` | 核对 seal/run 身份后，一般把 Run status 复制为 Task status；预算失败另记 failed | 正常结束但 blocked/incomplete 的子运行不能提升为 Task 成功 |
| Run → Skill | `core/src/agent/runtime/submission.rs`: 处理 terminal events 的 Skill 收尾循环 | TurnCompleted → Skill Complete；特殊 paused goal 另行跳过 | Skill 必须根据自己的交付契约与证据决定；不能只跟随 TurnCompleted |
| seal → 远端/客户端 | `core/nola-store-port/src/remote_session.rs`: `CanonicalRunTerminal` | 当前 terminal 类型只表达三个结束状态 | 传输、校验和展示均保留 outcome；各端实际展示仍待审计 |

这些不是十个独立 UI bug，而是同一个信息损失链。
不能仅将 reviewed blocked 改为 `complete_turn`：那会让下游 Task/Skill 更容易把受阻判为成功。
原 `reviewed_blocker_is_published_without_marking_task_successful` 测试曾断言 Failed，
旧测试绿灯不能作为语义正确证明。

当前生产调用链为：原 Host 输入 → Core `complete_turn` 等待检查与
`decide_requirement_result` → 原 `RunResultDecided` envelope → Application 同事务提交
→ Store 完整来源核验与 V2 seal → Task receipt/客户端完整结果投影。
普通正文和带待办的空正文不再隐式请求模型；有效审查明确要求继续执行时可以继续，
其恢复／修订预算耗尽是 Incomplete，损坏的审查协议才是 Failed。
Skill 不能仅因 TurnCompleted 变为成功；父 Goal 也不能仅因子 Run sealed 接受交付。
源码接线详情与未完成验收见 [Store 绑定](../04-存储与索引/STORE_RESULT_BINDING.md)、
[要求准入](REQUIREMENT_ADMISSION.md) 和 [验证记录](../../../历史任务/实施与审计/VERIFICATION.md)。

## 2. 唯一决策接口

### 工程证据不是第二套收尾控制器

2026-10-12 源码审计发现 `AgentEngineeringState.completion_retry_requested` 已无合法
`true` 写入者，但 checkpoint 字段、planner 的仅验证工具过滤与 RequiredTool 覆盖、
tool driver 的 `validation_completion_required` 拒绝仍存在。此残留不能作为兼容接口：
恢复载荷不应重新启用当前状态机不会生成的第二套控制流程。

整改要求是删除该字段及全部消费者，不是将其默认值改为 false。保留实际 mutation/版本/
validation 证据和 `promote_validation_tools`，让模型可选择验证；是否 ActionCompleted、
Incomplete、Blocked 或 Failed 仍由要求/证据仲裁及不可变 seal 决定。没有通过验证不能
冒充完成，但也不能单凭待验证布尔值将正文丢弃并强制另一次模型请求。
`empty_completion_retry_requested` 是独立的空响应协议恢复，不属于此次删除范围。
模型提示也必须遵守同一边界：验证不足限制的是“已完成且已验证”的声明，不是所有回答。
模型可以继续适当的验证，也可以说明未验证内容、剩余要求并提出 Incomplete；只有真实
阻断事实才可提议 Blocked。未知资源/版本不得以重复副作用、伪造范围或无限验证来消除。
批次验收须覆盖：序列化不含旧字段、审查接受但证据不足生成 Incomplete 且不续跑、
审查要求继续时写入工具仍可用、已有验证工具提升不回归。执行结果见 VERIFICATION。

下面是逻辑接口，不要求另建同名 CompletionAssessment DTO。当前实现通过 Core 的
要求、观察、等待与结果仲裁表达它；唯一 wire 字段以 TERMINAL_PROTOCOL 和 Domain
类型为准。不在客户端、Host 或 Application 再实现收尾规则。

```text
CompletionAssessmentInput {
  run_id, expected_revision, cancellation_epoch,
  model_step: ProtocolValid | ProtocolInvalid(reason),
  open_effect_ids[], active_wait_ids[],
  requirements[], obligations[], available_resolution_actions[],
  execution_budget, progress_watermark
}

CompletionDecision =
  Continue { action_refs[], progress_watermark }
  | Wait { wait_id, reason, response_schema_ref, obligation_ids[] }
  | Seal { result }

RunResult =
  Answered { answer_part_ids[] }
  | ActionCompleted { requirement_evidence[] }
  | Incomplete { reason, remaining_requirement_ids[], obligation_ids[] }
  | Blocked { reason, obligation_ids[], resolution_condition }
  | Cancelled { cancellation_epoch, observed_effect_ids[], unknown_effect_ids[] }
  | Failed { error, observed_effect_ids[], unknown_effect_ids[] }
```

ID 数组可以为空的情况必须由变体规定：纯问答不制造文件义务；Blocked 必须含结构化 reason，
若不是证据原因（如外部服务不可用），可无 obligation，但必须有相应事实引用。
`requirement_evidence` 不能只是自然语言说明，每项绑定要求 ID、有效证据 ID 与其版本。
正文、工具摘要和模型自述都不能自行创建已验证证据。

`available_resolution_actions` 由已知能力与权限事实提供，不是靠“用户说测试”等关键词推断。
模型仍选择具体工具；Core 只判定是否允许该调用以及回执能否满足要求。
Core 不应假定通用 Shell 可以完整证明副作用范围。没有可核对范围时报告能力限制，而非增添无限债务。

未知写后版本、未知修改范围、检查失败、检查未运行分别保存。
同一义务可以有多条历史证据，但只有匹配当前目标/版本的证据可以满足它；
旧证据不得因后来的文本说明而改成有效。授权批准不是验证证据。

## 3. 报告与决策分离

先决定结构化结果，再决定如何呈现报告。无论有无模型报告，都能封存一个合法的受阻结果。
未被证据支持的“全部完成”文字不能作为成功正文发布：保留为非答案的候选记录，
以结构化结果生成本地化说明。可选请求模型补充解释，但它不拥有改变 outcome 的权力，
也不能成为落库的必要条件或自动重复执行的理由。

模型请求失败是请求级事实；只有无法继续、恢复或报告的运行级故障才生成 Failed。
业务检查失败是 contradicted 证据，有可行修复路径时继续；没有路径则 incomplete/blocked。
预算耗尽属于 incomplete，不应为获取一句更好看的总结继续无限调用模型。

`NeedsInput` 的可回答分支已改为持久 Wait：必须有待回答问题、绑定身份和恢复规则，
Core/Application/SQLite 与客户端落地范围见 WAIT_PROTOCOL；不得回退为完成或失败。
没有真正需要回答的问题，不能仅凭解释文案设置等待；无法安全推进时选择 Blocked。
用户明确拒绝/停止后结束当前等待，按作用域决定 incomplete/blocked/cancelled，不重复弹窗。

## 4. 原子提交与恢复

同一 Application write unit 必须提交：

1. 输入去重标识、Core revision、逐项 obligation/evidence 更新；
2. RunResult、封存时间、最终正文/结构化报告、usage accounting；
3. 最终 assistant journal frontier、terminal manifest 与 seal；
4. 需要交付给父 Task 的结果事实或可幂等协调的已提交引用。

outcome、reason、evidence frontier 纳入 terminal seal 的版本化 canonical bytes 与 digest。
不得以另一张客户端状态表补上 seal 丢失的信息。Task receipt 必须引用精确 child seal，
同时保留 child result 与 supervising task 的资源预算结果；两者可能不同，禁止覆盖前者。

Task 的 completed 只能表示它的交付要求得到满足。子运行 Answered 也不能自动满足一个行动任务。
父 Goal 不因子 Task completed 自动 completed；父 Core 核对整体要求和约束后独立决定。
Skill 只能在自己的输出、步骤和证据要求满足时 Complete；受阻保留可恢复状态/引用。

事务提交前崩溃：不发布终态，按 inbox/outbox 及真实回执恢复；不得盲目重跑写操作。
提交后 ACK 丢失：同 identity 返回同 result/seal，不重复 usage、父任务通知或正文。
取消与封存竞争以 revision/CAS 决定；迟到结果可补 effect 事实，不改不可变 result。
checkpoint 是恢复材料，不能成为另一套与已提交 seal 不一致的终态。

这是持久协议变更：更新 schema/protocol 版本、所有 reader、远端 digest validator 和测试 fixture。
旧版本不猜测 outcome、不加“默认 completed”兼容分支；明确拒绝不匹配，且不能自动删除用户 DB。
开发重建使用隔离库。既有用户数据的升级/归档方案需要独立说明后才能操作。

## 5. 错误分类与性能约束

| 类别 | 例子 | 后继 |
|---|---|---|
| InvalidInput / ProtocolViolation | 缺字段、非法工具帧 | 拒绝该输入；受控修复协议，不产生副作用信用 |
| Conflict / StaleAuthority | revision/lease/cancel epoch 不符 | 重新读取已提交事实；不直接重跑 effect |
| PermissionPending | 有效授权请求未回答 | Wait，不调用模型轮询 |
| EvidenceUnavailable | 修改范围或写后版本不可观察 | 可核对则 Continue，否则 Blocked |
| EvidenceContradicted | 真实业务检查未通过 | 可修复则 Continue，否则 Incomplete/Blocked |
| ExecutionUnknown | Host 断线且效果未知 | 恢复核对；不能确认则 Blocked |
| RuntimeFailure | 不可恢复的协议/执行故障 | Failed，保留已观察效果 |
| StoreCommitFailure | 事务未提交 | 不 ACK、不发布虚构 Failed；错误从边界返回，恢复后重读事实 |

Store 自身无法提交时，不能承诺已持久化一个 Failed；这是产品展示必须区分的观察错误。
收尾性能归入 P03/P06/P07：分别测 assessment、事务、seal、投影到首帧；
附义务数、证据数、manifest 大小和历史长度，检查是否每次全量扫描历史。
无可执行动作的受阻退出不得再产生自动模型重试；这是可计数的正确性门禁，不是虚构毫秒预算。

## 6. 同批实施与验收

顺序：Domain 类型/决策 → Core 证据与事件 → Application 双路径 → Store/schema/seal →
Task/Skill 消费 → 本地及远端客户端投影。跨边界完整接好后统一测试；中间类型编译不代表验收。

| 场景 | 必须同时核对 |
|---|---|
| 未知范围 + 已知目标检查 exit=0 | 义务仍未满足；blocked 不是 failed/completed；无重复模型请求；产物不重写 |
| 已知目标 + 正确版本检查 | 仅对应义务 satisfied；全部要求满足后 action_completed |
| 已知目标 + 旧版本或未知版本 | 不能借用旧检查；版本原因与范围原因分别持久化 |
| 审查返回 blocked / 普通执行证据不足 | 两条路径都检查原阻塞事实；仅声明或缺验收为 Incomplete，真实 blocker 才是 Blocked；不强迫所有任务触发审查 |
| 当前输入排队 + 收尾 | 不吞新输入，不把下一项任务继承到已封存 Run；身份/事件顺序明确 |
| 受阻子 agent + 活动 Skill | 子 seal/Task receipt/父 Goal/Skill 均不误报成功 |
| 事务前后崩溃、重复回执、迟到取消 | 一份结果、一份 seal；恢复状态一致，无重复效果/usage |
| 删除 Trace 后重启、远端重连、切会话 | 仍展示精确 result/reason；等待只影响归属会话 |

现有 Core 审查测试是起点，不是以上链路的通过证据。E2E-01/02/07/09 必须覆盖真正 SQLite、
父子任务 seal 消费和客户端恢复；真实模型另验普通工具流程，不只验结构化审查 mock。
