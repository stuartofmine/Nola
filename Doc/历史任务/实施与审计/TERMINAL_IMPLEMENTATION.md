> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 结果封存：实现进度与精确来源

状态：来源持久化已接入两条生产 Application 写入口；新结果封存尚未接线。
本页不改变 [TERMINAL_PROTOCOL](../../软件架构设计/模块/05-执行与恢复/TERMINAL_PROTOCOL.md) 的最终契约。

## 当前落实的边界

`committed_event_payload_for_persistence` 为 TurnCompleted / TurnFailed /
TurnCancelled 保存真实 Core envelope：session、turn、revision、cursor。
两条 Application 提交路径复用该函数，事件原有字段与 outcome 不改写。
Store run_events.sequence 是 Run 日志顺序，不能替代 Core cursor；两者故意分开。
已有 Task admission 的 envelope 写入行为保留。

`nola_store_port::run_decision::RunDecisionRepository` 是内部只读来源查询：

| 输入/输出 | 契约 |
|---|---|
| RunDecisionQuery | conversation、run、turn、Core revision、Core event cursor 全部必填 |
| StoredRunDecision | 精确源 transition、Store checkpoint revision、Core session、原事件 JSON |
| None | 精确 event 不存在，或者存在但不是 terminal decision；不代表成功 |
| Conflict | 外围 Run 不属于指定会话/本地来源，不从 JSON 内的自述 Run 推断归属 |
| Invariant / JSON error | 已存在决策的 envelope 缺失、损坏、身份/版本不匹配、缺 Turn owner 或提交来源 |

读在一个 SQLite read snapshot 内完成。通过事件主键查找，再用不可变提交边界定位
原 transition；不会读取当前 checkpoint 当历史证据，也不会用最新 revision 填旧事件。
后续空提交、其他 Turn 的推进和数据库重开均不改变来源。
未添加第二份可写状态表、客户端字段或 schema 默认值；旧事件缺 envelope 返回证据缺失错误，
不迁移/删除用户库，不让旧生产 V1 seal 因新增查询而阻塞。

## 验收与限制

- Store 专项：2 项通过；真实 BeginTurn/CommitTransition、文件库重开、后续空提交、
  错 Run/conversation/turn/revision/cursor、缺 envelope、非终态与缺事件均覆盖。
- Application 事件序列化专项：1 项通过；检查全部五个 completion outcome 以及 failed/cancelled
  保存真实 envelope 后原事件字段保持相同。
- 这些测试不证明完整 Core→Application→Store→客户端链路通过。

## 下一步不得跳过

1. 将 RunResult 决定绑定上述精确来源，补齐准入 input watermark、要求版本与逐项证据身份。
2. Store 同一事务核验所有结果引用、写不可变结果和 V2 seal；V1 seal 目前仍没有 outcome。
3. Wait、blocked、incomplete 必须有合法持久出口；不能仅拒绝旧 TurnCompleted 导致回执永远重试。
4. Task、Goal、Skill 和多端消费者使用同一 result/seal，不把事件“结束”解释成“任务满足”。

当前新查询没有生产 V2 seal 消费者；这是一项必要来源前置，不是结果链路完成。

## 生产切换清单（独立只读核对）

以下按真实调用顺序列出，不能只改前三项便称为完成。路径相对仓库根。

| 边界 | 实际文件/函数 | 必须同批改动 |
|---|---|---|
| 模型结束判断 | `core/nola-kernel/src/model_completion.rs::plan_model_completion`，`core/src/agent/runtime/submission.rs` 的 ModelCompletionFacts / PublishReviewedCandidate 分支 | 从布尔债务升级为有身份的义务和可执行出口；reviewed blocked 目前走 TurnFailed，不能仅改为 complete_turn |
| 工具提问 | `core/src/agent/runtime/tool_driver.rs` 的持久 user-input wait / all_terminal 分支 | 已接入持久 wait，不能退回 complete_turn；结果切换须保留回答 CAS、取消竞争、独立工具完成与恢复语义 |
| Turn 收尾 | `core/src/agent/runtime/turn_lifecycle.rs::complete_turn` | 分离 result 决定、自动 memory、下一输入准入；当前只结束旧 Turn，后继由 Application 显式建立独立 Run，禁止新 Run 覆盖旧结果来源 |
| 事件协议 | `core/src/agent/runtime_event.rs`、`core/src/agent/protocol.rs` 的 TurnCompleted/Failed/Cancelled | 结果决定含 source identity 和完整 RunResult/引用；等待不是终态事件；不能补默认成功字段 |
| 双写入口 | `core/nola-application/src/lib.rs::accept_host_event_in_transaction` / `reduce_and_commit_active_submission_in_transaction` | 两者统一消费有类型的 progress/wait/finalizing/seal 决定，不再丢 outcome 或独立推断状态 |
| 恢复 fallback | 同文件 `checkpoint_terminal_status` / `turn_lifecycle_projection` | 当前只有生命周期三态；恢复必须使用原决定与持久 wait/result 来源，不读取新 active_turn 推断旧结果 |
| 提交/发布 | 同文件 `publish_transition_journal`；`core/nola-store-port/src/run_effect.rs::CommitTransitionInput` | typed transition 替换两个独立 status 字符串；正文最终发布、结果引用核验、seal 在同一 UoW，任一步失败无发布/ACK |
| 原子写入 | `core/nola-store/src/conversation_transition_store.rs::commit_transition` | 写 result 与来源索引、固定 frontier、校验 CAS；不能独立先写 runs.status 再补结果 |
| 本地 seal | `core/nola-store/src/assistant_journal/terminal.rs::freeze/load/page` | 使用原决定引用，核验所有结果来源；V2 digest 绑定 result，保持历史 seal 不随后台 heads 改写 |
| Wire/digest | `core/nola-domain/src/assistant_run_terminal.rs`、`assistant_run_terminal_schema.sql`、`core/nola-store-port/src/assistant_journal.rs` | 更新版本、result canonical digest、SQL 不变性及固定跨端向量；消息 state 与 Run outcome 分开 |
| 远端落库 | `core/nola-store/src/assistant_journal/terminal_receive.rs::begin/stage/commit` | staging 必须收到并核验 result，原子安装终态；当前只把三态复制回 runs/messages，不能把 blocked 映射 completed |
| 查询出口 | `core/nola-store-port/src/remote_session.rs::CanonicalRunTerminal`；`core/nola-store/src/remote_session_port.rs::canonical_run_terminal`；Application `application_command/conversation.rs` | 不允许另一查询出口继续只给三态使调用者误判；查询不制造/修复结果 |
| 子 Task | `core/nola-store/src/runtime_task_terminal.rs::journal_result` / `record_terminal_receipt_with_journal`；`runtime_task_run_binding.rs::reconcile_runtime_task_bound_run_terminal` | receipt 固定 child result/seal；目前复制 status，必须按交付契约判断而非“子 Run completed”提升成功，资源预算结果单独保留 |
| Skill | `core/src/agent/runtime/submission.rs::finalize_skills_from_turn_events` | 当前 TurnCompleted 普通分支发 Skill Complete；改为自己的要求验收与 result 引用；既有 goal pause 特例不能覆盖通用 Wait |
| Goal | `core/src/agent/runtime/goal_work.rs::accept_goal_completion` | 保持父任务独立评估和版本校验，不能只检查 child status/source_verified；绑定可核验 acceptance 身份 |

### 必须同时更新的客户端与旁路消费者

这不是只新增 Swift 字段：以下是读取、验证或据终态采取动作的生产入口。

- CLI：`cli/src/assistant_journal_terminal.rs::catch_up`、`completed_source_with_port`；
  `cli/src/agent_parts/emitter.rs` 的 terminal seal 发布；`cli/src/task_runtime_parts/execution.rs` 的子 Run
  seal 消费；`cli/src/messaging_channel_worker.rs` 的 completed 判断。
- Web：`cli/src/web_parts/assistant_journal.rs` 的 native wire 转发；
  `web/frontend/src/lib/assistantTerminal.ts::readTerminal` 当前硬编码 protocol_version=1 和三态，
  必须更新独立 digest 验证与 result 验证；`runObservation.ts`、`pages/Chat.svelte`、
  `pages/ModelTest.svelte` 不能把终态等价为任务成功。
- macOS：`NolaMac/Services/MacAssistantJournalTransport.swift::MacAssistantRunTerminalSeal`；
  `MacRemoteHostService.swift::synchronizeSealedAssistantFrontier` 及 seal 查询分支；
  `MacApplicationPort.swift` begin/commit receive；`MacRemoteRunObservation.swift` 的 wire。
- iOS：`Nola/Services/IOSAssistantJournalTransport.swift::IOSAssistantRunTerminalSeal`；
  `IOSAssistantJournalConsumer.swift::recoverTerminal` 当前将 seal.status 作为 terminalStatus；
  `IOSCanonicalAgentApplication.swift::committedTerminalSeal`、`IOSRunObservation.swift`、
  `Nola/LLM/WebAgentProvider.swift` 的远端完成消费。
- Android：`AndroidAssistantJournal.kt::receiveTerminal` 保留 Core 原子接收验证；
  `RemoteJournalSummary.kt::terminal` 当前要求 status=completed，必须根据摘要交付契约读取结果，
  不能把所有封存结果作为可用摘要。

无需让每端重新判断业务；五端消费同一已验证 result/wait 投影。上述列表覆盖本轮找到的直接消费者，
正式切换仍需对旧 status 字符串与 protocol_version=1 做全仓枚举门禁。

### 两项不能靠编码默认值解决的契约缺口

**NeedsInput：持久提问已接入，结果切换仍须保留等待契约。** WAIT_PROTOCOL 的 request/schema/CAS/receipt
已用于 Core 提问、Application 回答和 Store 原始事件索引，等待不再依赖 Host PID/内存。
已有重启、回答幂等、取消竞争及 Task 执行槽释放的专项覆盖；这些不是 V2 结果全链验收。
审查模型说 needs_input 不等于存在有效 QuestionSet；缺问题时不能假造 wait 或封存成功。
先让所有终态不变、只在 Store 拒绝 NeedsInput，会把问题转成提交重试死循环，不允许这样发布。

**ActionCompleted：逻辑契约已定版，生产 acceptance 来源尚缺。** TERMINAL_PROTOCOL §4.3 已明确
普通 Run/Goal 共用的要求集合版本、不可变评估、精确来源与封存核验；尚未生产接线。
当前 GoalAuditReport 有报告 ID 与
expected_goal_revision，GoalAuditFinding 有 record_id、assessment_method、evidence_refs，
但 finding 没有独立 acceptance ID 和 requirement revision。普通非 Goal Run 也没有同等要求版本实体。
模型报告的 outcome=action_completed 不可直接变成 acceptance；exit=0、source_verified 或一个完成状态
也不具有这层含义。需要实现“本次准入范围/要求版本 → 接受判断 → 证据版本”的 Core 事件，Store 仅
索引及核验来源，不在事务内运行模型。不能强制每个普通请求建立 Goal，也不能把所有行动降为 Answered。

Failed/Cancelled 同样不是捷径：仍须固定 originating_submission/input watermark、error_ref 或
cancellation_id 和实际 effects 来源；现有字符串错误不足以直接组成完整 RunResult。

### 有顺序的实施与切换闸门

1. **可立即实施的来源基础**：精确决策事件已落地；补准入 input watermark 和 effect/error/cancel
   的明确持久身份。所有引用先能被恢复和精确读取，不虚构 UUID 指向不存在记录。
2. **须先定版契约再实施**：统一无 Goal/有 Goal 的要求及 acceptance 版本、Core wait 事件和回答
   schema；确定四类 pending work（本次执行、等待、取消核对、独立后继），避免集合非空即 cleanup。
3. **在同一集成分支贯通**：Core decision → Application 双路径/恢复 → Store typed transition
   与 result+V2 seal → Task/Skill/Goal → 五端验证/展示。不能把临时无消费者类型作为交付终点。
4. **统一切换而非默认兼容**：协议/checkpoint/schema 版本分别明确；隔离测试库验证不支持版本
   的拒绝路径，不自动清理用户库，不把旧 V1 解读为成功的新 V2。
5. **完整链路闸门**：七类场景（answered/action_completed/incomplete/blocked/cancelled/failed/wait）
   核对实际消息、checkpoint、Run、result/seal、Task receipt、UI 投影；加入未知效果、并发取消、
   双客户端回答、提交前后崩溃、ACK 丢失、重复回执、下一 Turn 同批和真实模型长会话。
   Wait 无 seal/无模型轮询；blocked 不自动升级 failed 或 completed；无证据不允许 action_completed。

以上核对只更新实施清单，没有更改业务流程或声称新的 E2E 已通过。

## 输入准入与 watermark：最小完整契约

状态：已实现 Core 身份队列、接受/应用/撤回事件、schema 63 独立输入事实索引及后继 Run
分离。原应用索引仍是 Turn-local；schema 66 另有同事务派生的 Run-wide 应用序列与来源摘要链。
RunResult watermark 与完整 V2 seal 尚未贯通。
不新增无生产消费者的查询接口。下方“当前丢失点”保留最初审计背景，实施状态以后续小节为准。

### 当前丢失点

1. `assistant_turn.rs::begin_assistant_turn_in_transaction` **已要求** UserInput / GoalUserInput /
   AutomatedInput 携带非空 submission_id，但 BeginTurnInput / BeginInternalTurnInput 没有将该身份
   独立保存为准入事实。身份并非不存在，而是没有与最终 RunResult 所需来源稳定关联。
2. `initial_run_events.rs::plan` 保存 TurnStarted 等有限控制事件，过滤 InputApplied/InputQueued；
   `runs.user_message_id` 只关联消息，不等于 originating_submission_id。
   internal turn 还会引用先前用户消息，故消息所属 Run 也不能被当作新运行的用户准入。
3. `runtime/submission.rs::reduce` 附近的 accepted submission 去重使用 SessionJournal 的
   `accepted_submission_ids/accepted_submission_order`；`session.rs::record_accepted_submission`
   有有界淘汰。它是运行期恢复帮助，不是永久准入身份账本或要求范围版本。
4. `input_queue.rs::AgentInputQueue` 两个队列只保存 AgentItem；
   `turn_lifecycle.rs` 的 InputQueued 只有 turn/delivery/item_count/items，未保留输入批次身份和序号。
   CurrentTurn 在模型边界 drain；NextTurn 在 complete_turn 后 take_next_turn，批次来源因此不能
   从“最新消息”或当前队列长度恢复。
5. `reduce_and_commit_active_submission_in_transaction` 确实持久化 SubmissionReplayReduction，
   但 `host_event_replay_order.submission_id` 是提交的 **transition_id**，内层 AgentSubmission 的
   submission_id 才是输入身份；replay ordinal 是包含 Host 响应的恢复顺序，不是输入 watermark。

### 唯一身份与事件

由 Core 从可信 Application 输入及会话计数分配 accepted identity/序号。客户端提供重试身份，
Core 保留 typed delivery，Application 将 Core 会话/Turn 与已准入 Run 的真实归属绑定并原子提交。
不能让客户端或 Host 自行宣布输入“已经进入模型上下文”。

```text
AcceptedInput {
  admission_id, originating_submission_id, core_session_id,
  accepted_sequence, accepted_at_core_revision, acceptance_event_cursor,
  delivery: InitialTurn | CurrentTurn { turn_id } | NextTurn { after_turn_id },
  content_refs[], payload_digest, origin
}
InputApplied {
  admission_id, destination_turn_id, applied_sequence,
  applied_item_refs[], applied_at_core_revision, application_event_cursor
}
```

- `originating_submission_id` 保留原输入身份；不能用 tool call_id、transport request_id、消息 ID
  或 transition_id 顶替。Application receipt identity 与它有显式映射，不要求二者字符串相等。
- `accepted_sequence` 在 Core session 内单调递增且不重用，拒绝输入不消耗已提交序号；重复同内容
  返回原 admission，重复身份但内容/目标/delivery 不同返回冲突。持久去重不随内存有界集合淘汰。
- `payload_digest` 绑定规范化内容引用、来源及 delivery；权限 actor 从可信接入边界取得，不信任
  模型自报来源。内容原文沿 canonical storage 保存，不为索引再复制一份大历史。
- `InputApplied` 是独立于“已排队”的事实；同 admission 不可重复应用到两个 Run。
  即使多个队列元素合并进一个 model request，也保留逐批次 admission 引用。
- 当前 Core `InputApplication.applied_sequence` 在目的 Turn 内从 1 连续增长。Application 根据
  精确 destination Turn 绑定核验目的 Run；跨 Turn 不能直接沿用这个局部序号作为 Run 水位。
  数据库索引保存 Run、Turn、admission 唯一关系，不能由 UI 修改。

**RunResult.input_watermark 是同 Run 已提交应用记录在决定 frontier 内完整有序前缀的长度。**
前缀按真实 Core 事件 cursor 和事件内 application 顺序建立，同时绑定精确应用集合摘要；
两个 Turn 各有 applied_sequence=1 时，不能取 max 后得到 1。完整定义见
[要求准入与绑定](../../软件架构设计/模块/02-Core与状态机/REQUIREMENT_ADMISSION.md)。schema 66 已提供内部 Run 前缀链头读取；完整来源覆盖、
要求绑定及封存消费尚未实现，不能把有界链头核验当成完整前缀证明。
它也不是 session accepted_sequence 最大值：可能先收到 NextTurn，后收到 CurrentTurn；若仅用会话
最大值会错误地把尚未应用的下一轮输入计入旧 Run。封存核验限定同 Run 的应用关系，拒绝遗漏、重复与未来应用。
RunResult.originating_submission_id 固定为创建该 Run 的准入来源，后续 steering 不覆盖它。
会话序号用于接受顺序，Run 应用序号用于决定范围，两种序号均使用无损持久整数编码，不混用。

### 同事务写入与后继 Run

首次准入：Core AcceptedInput + InputApplied、Run 初始消息/绑定、checkpoint、outbox、receipt
同一 UoW 写入。当前提交失败时不发布 admission、不启动模型；重发原身份只能得到同一记录。

活动输入：AcceptedInput 与队列 checkpoint、submission replay 同事务；只有后续安全边界真正应用时
才写 InputApplied。queued 状态可跨重启保留，不能在恢复时按最新消息猜“已经应用”。

NextTurn：先保存带 after_turn_id 的 successor admission intent，此时不属于旧 Run 的输入范围。
旧 Run 封存后，由 Application 为后继准入建立独立 Run，原 admission_id 不变，使用唯一 successor
绑定防重复；其 InputApplied 从新 Run 序号 1 开始。旧 Core 的 CompleteAndStartNext 同 reduction
行为必须与此一起替换，不能先把新 Turn 归进旧 Run 再事后修消息 ownership。
取消排队输入生成显式 disposition，不删除接纳历史、不生成虚假的 applied 记录。

自动维护输入须保留 Automated origin 与源 result/trigger 引用，有独立 admission；不得把引用的上一条
用户消息当成用户新提交。它是否继续 Goal 或独立维护由 typed admission 决定，不靠文本关键词。

### 迁移与实施顺序

1. Domain/Core 定版 AcceptedInput/InputApplied/disposition、持久序号计数和携 identity 的队列条目；
   更新 checkpoint 版本。`AgentSubmission` 边界仍保留明确输入身份，禁止以缺省生成值掩盖旧来源丢失。
2. Application 首次/活动/自动/后继四条准入路径写同一类型契约；Store 建不可变 admission 和 application
   索引、唯一约束、精确来源读取及 CAS。读取函数只能在这些生产写入接通后增加。
3. RunResult builder 固定 originating_submission 和应用 watermark；Store 核验对应 Run 的连续应用前缀。
   与要求版本/acceptance 绑定合并，不把“收到新输入”一律解释为所有要求已变更。
4. 旧开发数据无法从集合/消息序号无损补出准入账本；明确版本不支持或经受控归档处理，不自动回填假
   watermark，不删除用户库。新 schema/checkpoint 使用隔离库验证，部署切换需和完整 V2 协议一起验收。

### 完整 Run 链路验收矩阵（部分 Core/事件持久化已覆盖，见下）

- 首次提交重复、相同 ID 不同 delivery/content；只有一份 Run/admission/outbox，冲突无部分落库。
- 先 NextTurn 后 CurrentTurn、相反顺序、两个批次合并；旧 Run watermark 只计已应用 current 输入，
  后继 Run 保留原 next submission，不能采用最近一条输入的身份。
- 排队后崩溃、应用提交前/后崩溃、模型请求 ACK 丢失；重开维持原序号/归属，不重复调用或应用。
- 原始准入后多次 active submission 与 checkpoint 推进；源 submission 不变，精确应用范围可重读。
- accepted sequence 超出内存去重窗口；数据库仍拒绝同身份重复副作用。
- internal maintenance 引用历史用户消息；不会误记为新用户准入或继承旧 Run watermark。
- 封存/steering/NextTurn/取消竞争；原结果范围固定、后继独立、迟到输入按明确处置返回。
- 缺 admission、跨 Run 应用、篡改 payload digest、序号缺口、错误 Core revision：事务整体拒绝且不 ACK。

这是下一块完整来源链路的规格，不是新增无消费者接口，也没有据现有消息数据宣称准入账本已存在。

### 已落地的 Core 阶段（不等于完整 Run 账本）

- `input_queue.rs` 以 AdmittedInputBatch 排队，每批带 admission identity、原 submission identity、
  session 接受序号、origin/delivery/anchor Turn、规范化 SHA256 和原 item IDs；不再只有 AgentItem。
- `submit_input/start_turn/model_planner` 发 InputAccepted 和带逐批 InputApplication 的 InputApplied。
  Core 目前计 **Turn-local** applied_sequence；当前 Application 仍可能把下一 Turn 放进旧 Run，
  因而此字段还不能直接冒充最终 RunResult watermark。首次输入、CurrentTurn 和 NextTurn 同一规则。
- 新输入指纹使用规范化 JSON，并绑定内容、delivery、origin/输入种类等原始提交字段。
  成功 dispatch 才登记去重，重放同身份无新事件/请求；同身份不同内容或 delivery 返回冲突。
  有界旧去重集合不负责新输入永久身份。生产 Application 仍要求 submission_id；直接 Core 允许
  unkeyed 输入，明确存 None，不捏造 originating_submission，后续 Run 封存不能把 None 作为合法来源。
- 取消当前 Turn 为未应用 CurrentTurn 批次写 InputWithdrawn，NextTurn 意图保留；Reset 明确撤回
  剩余批次并保留接纳历史与指纹，避免相同 Core session 内的历史输入复活。
- checkpoint 版本 11→12，恢复校验队列来源、接纳序号连续性、应用唯一性和目的 Turn 序号，旧版本
  明确不支持，不增加默认空账本兼容分支。真实用户库没有被修改、重建或清空。
- Application 两条活动提交保存 input events 的真实 Core envelope；initial_run_events 也保存接受/应用
  来源，初始应用事件不重复正文。所有 envelope cursor 统一 `{ sequence }`，不并存两种猜测编码。
  Store 初始控制事实白名单及目的 Turn 检查同步更新；新事件不触发 assistant 文本或 Run 终态变更。

持久体积约束：当前 Core checkpoint 保留本 session 的全部轻量 admission/application/fingerprint，
不保存第二份已应用正文；这是完整性优先的实施中状态，**尚未解决长会话索引写放大**。
不得按条数偷偷淘汰去重事实。下一阶段须将已提交历史下沉至唯一 Store 账本，Core checkpoint 仅保留
序号、开放队列和持久索引 frontier；删除/归档受会话生命周期和显式策略控制，不能在压缩时丢身份。
这项改进须测每输入的 checkpoint 字节增长、恢复耗时和提交耗时后制定预算，不宣称已有性能验收。

验证：Core host contract 125 项、Core lib 451 项通过；新增测试覆盖交错 Next/Current、序列化重启、
重复和冲突、拒绝无污染、取消撤回、篡改恢复和序号溢出。Reset 增补后，两个 input_admission
集成测试再次通过，确认撤回 NextTurn 和重置后旧 submission 不复活。

Application `terminal_event_provenance` 两项通过，其中真实 `begin_assistant_turn` → SQLite → 活动
NextTurn → 关库重开 → CurrentTurn 路径核验原 submission 身份和 accepted_sequence 1/2/3，
初始 InputApplied 只引用真正已应用的首批输入、不重复正文。Store `run_decision` 两项通过，包含
关库重开、后续 checkpoint 不覆盖初始来源、非终态缺失与非法引用的区别。CLI 全依赖编译通过。
测试全部使用隔离 NOLA_HOME/临时数据库和项目缓存脚本。

本阶段边界：以上最初只落地 Core 准入和原始事件事务持久化；后继 Run 和独立索引的后续实施见下一节。
Turn-local applied_sequence 不是生产 RunResult watermark。schema 66 的逐 Run 连续索引见后续实施；
完整 RunResult/V2 seal、全前缀覆盖核验及验收引用绑定仍未实现，不得宣称完整封存链路完成。

### 独立 NextTurn 后继 Run（下一纵向块）

生产切换：

1. Core `complete_turn` 只结束当前 Turn，不在同 reduction 内启动下一 Turn 或发出其 Host effects。
   `start_turn` 不再隐式吸收 Next 队列；普通新输入不能越过已接受的 FIFO Next 输入。
2. Core `PromoteQueuedInput` 显式验证已结束且无未结算 effects 的 predecessor、FIFO admission 和新的
   Turn identity。每个后继只应用一批原输入，不重发 InputAccepted、不改原 anchor/submission。
   后续 Next 仍保留原接受事实；失败或取消后未应用 Current 写撤回事实，Next 不自动撤销。
3. Application `PromoteQueuedAssistantTurn` 只接收 conversation/time；无 active Run 时从严格恢复的
   checkpoint 读取队列，从原 anchor Run 读取冻结调用配置，不能接受 UI 覆盖。task child 拒绝沿用
   旧 Run 的 task lease。通用 active/idle submission 入口拒绝 promotion 绕过独立准入。
4. Application 复用 canonical admission 的 Part/assistant journal/outbox 准备，调用 Store
   `BeginSuccessorTurnInput` 原子写后继。输入含 previous Run、checkpoint CAS、原 admission/submission、
   新 BeginTurnInput 和全部真实用户消息；后继 Run ID 使用原 submission ID。已存在用户消息精确复用，
   不改变接受事件所在旧 Run 的来源；新应用事件归属新 Run。
5. 独立 Store API 及 input 索引由同轮 Store 实施负责：核验实际 seal、无 pending effects、FIFO、
   原批次内容与同 session 归属，并唯一绑定 admission→successor。初始 InputApplied 仍不复制正文，
   完整内容与旧 checkpoint 队列和 canonical user messages 核对。
6. Application 命令分派显式 promotion 成功后才 retain 新 Core；没有后台隐式切换。Runtime/CLI 的
   completed drain、generic cold resume 消费此入口；exact Run 恢复不接管后继，已取消执行 token
   不立即发起后继。客户端执行上下文更新与 Store/Runtime 的对应测试由根协调。

恢复语义：旧 Run 已封存、后继尚未建立时队列仍在同一会话 checkpoint；重开可使用同一 promotion
入口。正在运行后继时重复调用返回无待准入结果，Store 本身还需持久唯一绑定和重复 payload 校验。
这不是重新提交用户文本，不触发默认 Goal 或模型审查，也不是以 call_id/transition_id 替代 submission。

本块验证：全依赖 CLI 编译已通过，Core host contract 全 125 项通过（包含提前/重复 promotion 拒绝
且状态不污染）。真实 Application Host claim→执行开始 ACK→模型开始/完成→旧 Run 封存→关库重开→
后继 Run 准入→跨 Run application 索引核验已通过；确认旧 Run 无遗留可执行 effects，原 admission
与冻结配置不变，重复 Application promotion 不创建新执行。Core lib 全 454 项、Application 来源组
全 3 项通过；另增失败 predecessor 定点测试通过，覆盖未应用 Current 撤回、两批 Next FIFO、重开后
独立应用及原 anchor 不变。所有 live 测试已退出，`git diff --check` 通过。

Runtime 边界复核后的修订（CLI 专项已通过，真实客户端验收未完成）：

- 热 promotion 不是只替换 Core 状态：物理 Host 的实际配置重新生成 HostInvocationBinding，与后继
  冻结 binding 精确相等才准 dispatch。binding 包含路由/凭据身份、生成参数、输出准入及内存/权限/
  工作区等 execution-policy 摘要；不同配置的后继只持久准入，要求 cold exact recovery，不沿用旧 Host。
- 每次后继 Run 调用 Host successor boundary，清理旧 Run 的工具失败、provider exclusion、动态可用
  工具、instruction context、lifecycle 标记及临时 allowed_dirs；保留同一前台 cancellation scope，
  不以新默认 token 躲开用户取消。跨会话/持久权限事实仍由 Core 授权，不因清理扩大权限。
- Runtime 在 dispatch 前从 SessionStore 捕获实际执行 Run，最终 terminal seal 使用该身份，并核对
  terminal snapshot，而不是最初的 submission/旧恢复 Run。非终态等待不读取 terminal owner helper。
- cold generic resume 的 token 已取消时不 promotion；exact Run resume 从不接管后继。取消若在
  promotion 事务期间到达，已提交的新 Run/effects 保留且不 dispatch，返回含新 Run ID 的明确恢复错误，
  不谎报旧 completed snapshot。外层取消分支不得将 Interrupt 投向这个尚未执行的新 owner；旧 Run
  的正常取消仍沿原协议结算。未来恢复必须带新 cancellation scope 并针对确切已提交 Run。
- 此轮测试已补 Host 状态重置/取消共享、hot/cold 实际请求、最终 seal owner 和预取消不消费 Next；
  根正在统一运行完整 CLI recovery 模块及 reset 专项，本段不提前声明通过。

Store 后继专项最终 4/4 通过，含两个独立 SQLite 连接同时抢同一 admission：相同 payload 返回
同一结果，改变 payload 确定冲突。第三连接重开核验 Run、绑定、输入应用、消息和 outbox 无重复。
四项分别覆盖：已有用户及附件 owner 保留/重启重放；CAS 与末端绑定失败全事务回滚；旧 pending
effect 与无关历史用户 owner 拒绝；同进程两线程、独立连接经 Barrier 同时竞争（无共享 Mutex）。
加入该例后 Store 全库 384/0/1，通过与忽略项按 VERIFICATION 记录；不包含双进程 kill 验收。

### Schema 63：Store 输入事实索引（已接生产写入）

实现：`core/nola-store/src/input_fact_schema.sql`、`input_facts.rs`。接受、应用、撤回分别保存
于 `input_admissions`、`input_turn_applications`、`input_dispositions`；同次升级包含后继准入
使用的 `run_successor_admissions` 及其不可变绑定。它们不是新增的旁路端口：后继 Run 准入
直接查询前述接受/应用/撤回事实，核对 FIFO 及是否已经消费。

- 首次、内部与 maintenance 准入由 `initial_run_events::append` 在原事件 INSERT 后索引；
  这些调用仍属于 `turn_admission_store` 的同一个写事务。
- 活动输入和 Host 事件均经 `conversation_transition_store::commit_transition`：本次 checkpoint
  和 Turn ownership 更新完成后、提交之前索引本批事件。索引失败同时回滚 Run、checkpoint、
  原事件及派生索引，不能返回成功 receipt。
- `accepted_sequence` 在 Core session 内连续增长；`turn_applied_sequence` 在目的 Turn 内连续
  增长。接受事件所属 Run 可以不同于应用事件所属 Run，但 conversation/Core session 必须一致，
  目的 Turn 必须有当前 Run 的明确 ownership。同 admission 不得二次应用或撤回后再应用。
- `source_event_id` 与 `source_digest` 固定原事件元数据身份；`payload_digest` 只检查合法形状并
  保留 Core 原值，索引层不重读正文计算，也不据此声称核验了全部内容。item IDs 可为空（合法维护
  输入可能仅有 context），但提供的单个 ID 必须非空且唯一。序号受检转换为持久整数，不经浮点。
- 新写入通过轻量 delivery witness 的真实 `core_revision`、cursor 和 session 核验 frontier；
  Application 与 baseline SQL 投影都写入该真实字段，不用 Store revision 代替 Core revision。
- 原文继续沿 canonical 消息/独立 replay 路径保存。初始与活动持久 `InputApplied` 均仅保存
  turn/applications 元数据，不复制 items 正文；消息物化仍消费原始 Core envelopes。首次控制事实
  的 16KiB 上限保持显式拒绝并回滚，不默默截断多 item 元数据。

迁移边界：62→63 筛选 `core_envelope.session_id` 为文本的原始 input 事件并严格解析，不读取最新 checkpoint
猜测历史，也不创造历史 submission。回填接受/应用行标记 `historical_event_only`，表示只有原事件
来源，**没有重新证明当时的提交 frontier，不能授予 RunResult 封存信用**；本轮新写入标记
`current_frontier`。无 input 事实或未满足 envelope 筛选条件的旧记录仍保留原文但不假装已索引；
选中但字段不完整的 envelope 会解析失败，并不被静默跳过。
如果完整应用事件缺关联 admission、存在身份冲突等，当前迁移明确 fail-closed：事务回滚、旧库
和原 schema 版本保留，可能无法开库。尚未实现持久 gap/隔离迁移，不能声称兼容全部历史数据。

验收（隔离 NOLA_HOME、mbx、`RUST_MIN_STACK=16777216`）：
`nola_cargo test -p nola_store --lib --quiet`：383 passed / 0 failed / 1 ignored，34.36s；
包含 `input_facts::tests` 六项，覆盖真实首次入口及重开、九类非法事实/超限回滚、撤回后应用与
身份冲突、错误 Core revision、同事件 ID 篡改 item 身份，以及真实 62→63 回填的历史信用标记。
`nola_cargo test -p nola_application --lib terminal_event_provenance_tests -- --nocapture`：
3 passed / 0 failed，0.14s；初始准入→排队 Next→关库重开→排队 Current 后，独立索引为
3 条 accepted / 1 条 applied / 3 条 `current_frontier`，并覆盖独立后继跨 Run 来源。
这不是完整 RunResult/V2 seal、真实 Provider、Mac UI 或长历史索引性能验收；schema 62 的
归档 v4 原始性能证据保持原产物归属，不被本轮测试数字覆盖。

### UserInput durable wait：Core 阶段（本轮接线）

- `nola-domain/src/user_input_wait.rs` 定义 QuestionSet、stable option IDs、schema digest、
  RespondToUserInputWait 和 Open/Resolved/Withdrawn。响应摘要按 question_id 排序后计算，
  同 command + 同规范化输入幂等；同命令不同输入、过期 revision、错误 schema、未知题号/选项拒绝。
- `runtime/tool_driver.rs` 在既有能力/输入/权限校验后建立 wait，不发 Host 提问 effect；
  权限回执的独立执行准入路径也在 `finish_transition_after_model` 中转成 wait，不能遗漏后退到
  PID/内存弹窗。其他已授权工具继续，汇合由全部 open wait 和工具状态决定，不检查最后回执文字。
- `runtime/user_input_wait.rs` 把答复作为 typed 工具结果或审查问题的用户数据恢复模型。
  Decline 是明确答复并写 Resolved，不等同权限拒绝或取消 Goal；取消/Reset 写 Withdrawn。
  关闭界面、断线没有自动答案路径。普通 UserInput 在 AwaitingInput 时仍排队，不隐式消费问题。
- 审查 `clarify/needs_input` 必须提供完整 question_set，缺失走原 invalid-review correction。
  合法问题产生 AwaitingInput 而非 TurnCompleted；不能从笼统审查文字制造问题。
- `UserInputWaitChanged` 在 Part 投影后仍保留为原始持久事件；工具显示 WaitingForInput，
  与 WaitingForPermission 分离。snapshot/control 的 open_user_input_waits 是恢复呈现入口。
  checkpoint 升 13，等待账本属于 session 生命周期；Reset 保留 dispositions/response 去重历史。
  当前账本随 session 增长，未实现归档裁剪，不能用有限去重窗口换取历史答案复活风险。

Core 验证：`nola_cargo test -p nola_core --test agent_core_host_contract --quiet` 129/129 通过，
含真实工具提问无 Host effect → checkpoint 严格验证/重开 → CAS 拒绝 → 答案恢复模型 →
重放不重复派发/冲突拒绝、多问题与独立工具两种汇合顺序、取消/Reset late answer、显式 Decline、
审查问题恢复。`nola_cargo test -p nola_core --lib --quiet` 453/453 通过；
`nola_cargo test -p nola_domain user_input_wait --quiet` 2/2 通过。
Application/SQLite 同事务查询/回答、Task 释放槽与恢复、CLI/Mac 呈现由同批其他模块贯通；
本段不把 Core 单项测试等同全链/五端验收，不声称 RunResult/V2 seal 已整体切换。

### Task 用户问题的独立交付身份

StorePort 的 `user_input_notification_occurrence_id` 将 Run、wait ID、revision、schema digest
编码为 `user-input:` 加规范 JSON 数组，不借用选项 label、时间戳或权限 request 身份。
既有 `RuntimeTaskNotificationDelivery` 与 `durable_task_events.notification_delivered` 承载
consumer ACK；没有新建问题副本、第二交付状态库或新的权限授予入口。
Store 在 ACK 事务内核对 Task owner/child/Run binding、非终态且未取消的 Task、非终态 Run、
当前开放 wait 版本与原事件摘要。Task 仍执行独立工具时同样可交付问题，不要求先释放执行槽。
答复/撤回后的旧 occurrence 不能新增 ACK；原 command 的精确重放只是返回未新增，
不会确认后来问题。普通 AppendEvent 不能伪造通知 ACK。

本块新增 Store question_delivery 专项涵盖错 owner/Run/wait/revision/schema、双 consumer、
不可更换 command、普通事件伪造、Task 状态和 unread 不受 ACK 影响、关闭重开、旧问题关闭后
拒绝新 consumer。加入磁盘重开后的最终版本已纳入 `runtime_task_user_input_wait` 统一验证，
5/5 通过（0.15s）；StorePort occurrence 无歧义与显式 null owner 契约 2/2 通过。
此处的 Store leaf fixture 不替代父 Task 委派、真实窗口展示或多进程 kill 验收。

### Schema 65：执行观察的原始来源索引

`execution_evidence_sources` 只索引 Core `ExecutionEvidenceObserved` 的身份、owner、
Core revision/cursor、原事件摘要及原 Host inbox/reduction 引用。完整 receipt 保留在原事件，
没有复制工具正文/Host output，也没有由 receipt 成功生成 requirement acceptance。
单一 Host completion 至多对应一条 evidence（host_event_id 唯一）；相同证据重放不新增记录。
`commit_transition` 仅对该类型事件执行索引，同事务失败回滚；普通 token 不增加此查询成本。
初始准入没有对应 Host completion，保留白名单拒绝，不从旧 transcript/退出码重建观察。

核验包含外围 Run/Turn 所属、Core 信封与真实 checkpoint witness、精确原始 Host 输入，
normalized ToolCompleted 的 call/outcome/原始 typed receipt、原 outbox canonical tool identity，
以及存在时的实际 delivery attempt 身份。可选模型 attempt_id 和 delivery attempt_id 不混用。
并行工具乱序完成时，一个低序号回执可释放多个已持久化的 buffered 回执；每个 observation
按同 Run/effect/request/sequence 找回自己的原始 Host inbox，不借用触发 flush 的回执身份。
每次内部 reduction 分别推进 Core revision，因此最终 witness revision 可以大于单条事件的
revision，不能强制相等。重开清理仅使用与当前 checkpoint 头 revision/cursor 一致的 delivery
witness；仍在 pending effects 的已 ACK 请求保留到实际消费，缺少 current witness 时也不删除。
索引建立时验证原派发请求；后续读取只需持久原事件、Host 回执和 attempt 事实，不依赖可清理 outbox。
查询经 StorePort `ExecutionEvidenceRepository` 重读原事件和 Host reduction、核验摘要及全部
索引字段；缺失与已找到但来源损坏区分，不把反例/未知版本补成验证成功。

当前只在全新空库建立执行证据索引，不从旧库回填；旧版本升级路径已按单版本政策删除。
本批普通 Host ToolCompleted 接线不覆盖委托 Agent 工具观察，也未完成 obligation/requirement
assessment、RunResult 或 V2 seal。已完成的专项与整库回归见 VERIFICATION 的
“原始执行证据与乱序恢复”记录；不据此宣称真实模型或客户端交互通过。

### Schema 66：Run 输入来源前缀

`input_facts::index_event` 在原 InputApplied 索引完成后、同一事务内调用
`run_input_frontier::append_event`。`run_input_applications` 只保存派生身份、原来源引用和摘要，
不复制输入正文、不建立第二份业务准入状态。每个 Run 按原 Core cursor 与事件内数组位置追加
连续序号，Turn-local 序号保留；独立后继 Run 从自己的第一条已应用输入开始。

追加核对原 application、admission、持久 Turn owner、session、revision/cursor 和双来源摘要。
相同身份重放核验原记录和链摘要，不新增序号。不可变触发器禁止修改、删除派生行。
链摘要包含 Run、应用身份及原接受事件身份；数量相同不等于相同前缀。

当前仅初始化当前格式空表，不支持旧库回填。旧来源迁移和 historical_event_only 分支已删除；
每条新输入都必须核对当前真实 delivery witness。已有格式不匹配库原样拒绝，不自动删库。
当前索引也不能单凭存在就取得完整性证明；封存前仍须验证原事件与完整前缀覆盖。

每批原应用事件只解析、计算摘要一次，追加不逐次重扫全历史。内部 `read_prefix` 仅检查
链头原来源及前一摘要，不能授权 seal；全前缀覆盖审计、要求绑定、RunResult/V2 消费仍未完成。
长历史原来源读取、WAL 增长与追加延迟尚无实测预算，功能测试时间不是性能指标。

### 维护压缩的独立终态

维护 Run 只接收带非空 operation_id 的 Compact，不承载普通用户输入、要求 scope 或
RunResult。生产链为 CLI compact_session、Application begin_maintenance_turn、Core Compact、
Host 压缩回执、Application commit_transition；初始准入与后续回执分别在既有事务中持久化。
成功必须在当前维护 Turn 下保留有摘要和 source_fingerprint 的 Compaction 原记录，
或由本次 Core 事件明确记录零归档输入、无摘要的无需压缩结果；
失败与取消保留实际 Core 终态事件及 checkpoint，取消仍要求原始取消指令身份。
ModelAttemptSucceeded 只代表一次模型尝试，不足以终结维护操作。

三种终态均关闭维护生命周期为 sealed，但不写 assistant message 状态、不创建普通
AssistantRunTerminalSeal，也不把维护完成解释成用户任务验收。活动 submission 与 Host 回执
入口都核对维护 checkpoint 的精确 Turn 归属和 scope 隔离。冷恢复仅在终态仍属于原维护
Run 时跳过普通 seal 查询；排空后若已进入不同后继 Run，必须按后继 Run 查询结果。

maintenance_run_lifecycle_cases 覆盖真实准入、压缩采样、完成或失败、取消回执、SQLite
重开，以及没有用户消息和普通 seal 的断言。代码与测试源码已接线，等待整体统一验收。

### 有效审查的未完成结论

模型自选审查发现仍需执行，达到恢复预算后，以 ConcludeIncomplete 进入共同结果仲裁；
有效审查达到候选修订预算同理。原结构化报告保留为结果 reason 的具体说明，来源仍绑定
本次已准入模型回执；不发布未通过候选，也不把业务未完成变成协议 Failed。
实际权限或用户问题等待仍先经过等待门，不封存。只有实际 runtime 阻塞事实支持 Blocked，
模型自报阻塞不能取得该权威。无效审查协议、非法工具通道和重复空响应仍按协议失败处理。
Core 入口回归覆盖三次明确请求执行的审查及中途 checkpoint 恢复；尚待统一验收。
