> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 要求准入与评估的身份绑定

状态：生产接线正在实施、尚未统一编译或验收。本文细化 `../05-执行与恢复/TERMINAL_PROTOCOL.md` §4.3。
Core 要求控制及原始事件、Store 来源索引和 V2 seal 已有代码改动，跨模块审计尚未完成；
既有 Domain 结构检查或历史测试通过不能替代真实来源及本轮纵向验收。
本规格不要求创建 Goal，不要求独立审查，也不为每次对话增加规划或验收模型请求。

### 普通回答与执行证据

已有上下文足够的回答允许直接输出，不要求调用 `requirements_update`，更不要求
先调用工具制造 observation。`source_refs` 是已接受输入的身份引用，不是召回命令。
模型仅在缺少所需上下文时决定检索；Core/Host 不按用户文本关键词强制选工具。
显式提出 `answered` 同样不要求非空 assessments/observations；最终仍须发布非空
回答并绑定真实 Part。它不证明外部操作成功，也不能清除既有负面评估或未完成执行。
`action_completed` 的当前范围评估及执行证据规则不因这条普通回答路径而放宽。
执行任务的模型控制上下文必须明确另一条路径：登记请求的后置条件，引用当前
observation ID 评估，再以当前 assessment 引用提出 action_completed 和最终正文。
这些控制操作允许同批提交，不要求独立审查或在已有证据充足时重新调用验证工具。
缺失工作/证据应提出 incomplete，不能把 answered 当作执行成功的替代标签。
动态投影携带已有 evidence/call/source/outcome 关联；完整 receipt 仍由不可变
ExecutionEvidenceObserved 事件保存，不复制成 checkpoint 内的第二套证据。
2026-10-12 Mac 授权写入真实样本已证明缺少此明确指引时能完成文件却落成 answered；
新构建 DnK1FC 已真实执行 write/read/requirements_update，主任务结果为
action_completed 且绑定当前 assessment；单个成功样本不证明所有任务均会登记，
整体审计仍受旧 Trace 来源阻塞，详见 VERIFICATION。
控制工具说明与动态控制上下文必须表达同一边界，不能让模型从“来源引用”推断
必须重新检索当前消息。真实 Mac 样本 `jv08dS` 暴露过此类重复检索；本次说明
整改尚需真实模型复验，不能把文案变化视为已经证明因果或解决该失败。

### 当前生产准入范围

一个普通 Run 对应一个 execution scope。CurrentTurn 输入追加到同一 scope，并更新其
已应用输入引用；排队输入晋升 NextRun 时创建新 scope。Maintenance 不冒充普通任务 scope。
Application 的 `BeginInternalTurnInput` 创建有前驱绑定的新 Run，不是同 Run 跨 Turn。
禁止根据 session 相同或 Turn 时间相邻猜测同一执行范围；当前没有声称支持
`ContinueExecutionTurn` 的生产入口。若未来确需同 Run 新 Turn，必须先定义显式 scope
准入及 Store owner 约束，不能由客户端拼接旧 checkpoint 实现。

## 1. 已核实的边界矛盾

| 当前代码/协议 | 实际事实 | 实施时必须消除的混用 |
|---|---|---|
| `nola-domain/src/run_requirements.rs::RequirementSetRevision` | 必填 conversation_id、run_id、source_decision_ref | 它是绑定后的 canonical 值，不应直接成为 Core 命令或模型工具输入 |
| 同文件 `RequirementAssessment` | 必填 run_id、来源 revision/cursor，UserAcceptance 还含 conversation_id | 模型不能填写这些字段来证明身份、用户授权或已持久化 |
| `agent/protocol.rs::AgentSubmission` | Core 使用 session/turn；目前没有普通要求控制 submission | 不得为复用 Domain DTO 把数据库 Run/Conversation 注入 Core 的语义状态 |
| `runtime/submission.rs::finish_transition_after_model` / `runtime/host_response.rs::envelope_events` | finish 时才递增 Core revision，随后逐事件分配 cursor | 事件 payload 不能预猜其 envelope 的精确 cursor，也不能引用当前最大 cursor 代替自身来源 |
| `run_requirements.rs::validate_against` | 检查 run、集合版本、水位、session 和来源先后关系；未核对实际持久 turn owner | 相同 session 不等于相同 Run；Application/Store 必须解析精确 Turn 绑定 |
| `runtime/submission.rs` 的 `goal_update` 路径 | 仅 Goal Turn、单独控制调用，记录模型主张 | 普通要求不能借道创建 Goal；不能把这一单独调用路径变为强制额外模型轮次 |

`TERMINAL_PROTOCOL` 表格的“expected Run revision”应由外层 Application 理解为 Run/checkpoint
并发门槛；传给 Core 的是独立的 expected requirement revision 和 Core revision，三者不能互换。
`RequirementDecisionRef` 保持绑定后位置值；原始事件中不再复制一份可自报的位置。

## 2. 单一事实、两层表示

Core 保存与发出 owner-free 的语义事实；Application 完整持久化事件信封并传递本次
命令的 Run 上下文，Store 在同一写事务解析原事件、精确 Turn owner 和输入前缀，生成
绑定后的 canonical 值。由持有数据库快照的 Store 完成来源绑定，避免 Application 先查
后写或额外提交另一份可互相矛盾的 canonical mutation。绑定不是第二次业务判断，不
重新命名要求、不重算 verdict、不改写 rationale。

```text
认证用户命令 / 已接受模型响应中的类型化控制调用
  → Core 验证引用和 CAS，分配 ID，更新可恢复语义状态
  → RequirementScopeChanged / RequirementAssessed + AgentEventEnvelope
  → Application 完整事件信封及命令 Run 上下文
  → Store 同事务精确 Turn → Run/Conversation 绑定、原事件、checkpoint、receipt、派生索引
  → 按原来源查询 canonical RequirementSetRevision / RequirementAssessment
  → V2 seal 在固定 frontier 校验全部覆盖与未决事实
```

以下为拟实施 DTO 名，不是现存接口：

- `CoreRequirementScope`：`scope_id`、`revision`、`applied_input_refs`、`source_input_refs`、`requirements`。
  不含 conversation/run/source_decision_ref；要求项复用语义字段，但新 ID 仅由 Core 分配。
- `CoreRequirementAssessment`：`scope_id`、`assessment_id`、`requirement_id`、`requirement_set_revision`、
  `applied_input_refs`、`evidence_frontier`、`method`、`evidence_refs`、`verdict`、`rationale`。
  不含 run/source_decision_ref。用户接受方法使用 Core 已准入的命令来源身份，外层绑定成
  `RequirementUserCommandRef`，不能接受模型自报的 conversation/command。
- 事件 `RequirementScopeChanged { turn_id, scope }` 与
  `RequirementAssessed { turn_id, assessment }` 使用现有信封的 session/turn/revision/cursor。
  同一 reduction 可能含多条事件，绑定逐条使用真实 envelope，不统一换成最后一条 frontier。

`scope_id` 由 Core 在逻辑任务首次应用输入时分配；同一任务内部多 Turn 延续该 scope，版本不能
按 Turn 清零。独立 NextTurn admission 才建立新 scope。Core 不需要知道数据库 Run 名称；
Application 只允许一个 scope 绑定一个 Run，跨 Turn 延续必须引用原 scope，而不是隐式用 active。
`applied_input_refs` 每项为精确 `(admission_id, turn_id, applied_sequence)`，附属 Core session
取真实信封；Core 不把 turn-local applied_sequence 命名为 Run input_watermark。

scope 的恢复状态归 `AgentExecutionState`，不归 UI、Host 或单个 `AgentTurnState`。当前
`ContinueCurrent` 仍在同一 Turn 发起模型请求，并不是创建新 Turn。首次应用入口是
`runtime/turn_lifecycle.rs::start_turn`，活动输入入口是 `runtime/model_planner.rs::request_model`；
两处都必须先更新 scope，再生成本次请求的控制上下文。仅接首次入口会漏掉追加需求。
将来跨 Turn 延续须有类型化 `NewScope | ContinueScope { scope_id }` 准入决定；Application
按持久 Run 绑定校验，Core 不按上一活动 Turn 猜测。独立 NextTurn 必须 NewScope；Maintenance
不继承用户 scope。该准入语义及 execution 字段目前尚未实现。
实查 `BeginInternalTurnInput.expected_previous_run_id` 只是前驱 CAS，Store
`turn_admission_store` 仍会 INSERT 新 Run；它不是可直接复用的同 Run ContinueScope 接口。
实现跨 Turn 范围延续时必须新增明确同 Run 准入并核验 owner，不能按 internal 标志继承，
也不能以只支持 NewScope 的实现宣称已覆盖目标多 Turn 协议。

### 2.1 绑定 API 与输入水位

绑定在 Store 提交原事件的同一事务执行，返回现有 canonical Domain 值。
`binding` 由 Store 从本事务内的持久视图构造，至少含：

```text
RequirementBindingContext {
  conversation_id, run_id, scope_id,
  source: RequirementDecisionRef,             // 精确事件信封，不是调用者自报
  source_turn_owner,                          // 该事件 Turn 的持久 Run 归属
  resolved_applied_inputs[],                  // 原应用事件、session/turn/Run 归属与 frontier
  run_input_frontier: { watermark, applications_digest }
}
```

绑定函数检查 payload scope 匹配、来源 Turn 的独立 owner 证明、所有引用及判断 frontier 不超前、
应用集合精确一致，assessment 与绑定后的同一 scope/version 匹配；它不能仅因类型名称含
“resolved”便证明数据库查询真实。Store 仍须核验同事务原事件与 owner。禁止模型或 Core 自行填
conversation/run/cursor，然后调用此函数将其“洗成”可信 canonical 值。

唯一 Run 水位定义：同 Run 的实际 InputApplied **应用记录**，按真实 Core 事件 cursor 及事件内
application 顺序建立连续的 Run 准入序列；`watermark` 是决定 frontier 内完整前缀的长度。
同时绑定这个前缀的精确应用身份集合摘要（规范编码包含 session、turn、admission、turn-local
applied_sequence 及来源 cursor）。不同集合即使长度相同也不能互换。拒绝漏传应用、重复应用、
未来应用及他 Run 的应用；一个 application 携多项输入仍只占一个序号。

`InputAdmission.accepted_sequence` 是 session 级接受序号，而
`InputApplication.applied_sequence` 及 `input_turn_applications` 是 turn-local；NextTurn 尚未应用
会产生 accepted 序列空洞。不能取这些值的 max/count 全库聚合冒充 Run 水位。需从持久原应用
事件和精确 Run owner 派生完整有序前缀索引，不能新建独立业务准入真相。
schema 66 已实现 `run_input_applications`：同事务追加跨 Turn 的 Run 序号，链摘要绑定原接受与
应用来源；当前仅接受当前格式真实来源，不提供历史版本回填。内部 `read_prefix` 只核验链头及前一摘要，不是全历史审计，
尚未由 Application requirement binding 或 V2 seal 消费。目标完整支持同 Run 多 Turn，不能拿
单 Turn 特例替代协议。完整前缀覆盖和结果绑定仍是生产接线缺口；不得默认水位为 0 或最大 Turn 局部值。只实现 Domain
绑定函数不计生产水位、要求准入或 V2 封存完成；本轮也不新增无生产消费者的绑定接口。

### 2.2 内容承诺与完整性是不同约束

Domain 的三个位置已统一为 `RunInputCommitment { watermark, applications_digest }`，分别由
`RequirementSetRevision.input`、`RequirementAssessment.assessed_input`、`RunResult.input` 持有。
替换原数量字段，不保留可独立填写的平行水位。比较整个值，摘要进入 result canonical bytes
和 seal digest；`[A,B]` 与 `[A,C]` 即使同 Run、同版本、同长度也不得互相支持。

Application 从固定决定 frontier 的 Store 解析结果绑定 commitment，模型不计算、不填写
canonical 摘要。Store 内部须同时返回 owner、through revision/cursor、来源强度及覆盖验证结果；
只有实际检查原应用事件与索引完整对应、顺序连续、无未来项后才能认定完整，不能接受调用者
传入 `complete=true`。Application 不额外维护或提交 canonical 绑定对象。
`current_frontier` 只描述来源强度，不证明所有原应用都已计入。
schema66 链摘要可提供非空前缀的编码基础；零水位必须为固定空承诺
`SHA256("NOLA-RUN-INPUT-COMMITMENT-V1" || u64be(2) || "[]")`，非零水位不能使用该值。
空承诺不默认填充，缺摘要拒绝解析。已有 payload hash 也不证明原正文与附件仍可解析，归档读取
应单独报告缺失，不把缺失原文当成功恢复。

封存消费者接线时，内部解析入口定为
`resolve_run_input_commitment_for_decision(transaction, conversation_id, run_id, decision_event_id)`；
通过已提交决定事件取得真实 session/Turn/revision/cursor，不允许外部任意选择自由 frontier。
在同一事务快照枚举此前原 InputApplied，与派生索引双向核对并逐项核验准入来源、链摘要与
累计来源强度；返回 commitment、owner、真实决定位置及来源强度。空索引必须先证明没有原应用
才可返回空承诺。一次完整核验不搬入每次追加；现有 run_events 的 Run/sequence 索引意味着
原事件筛选仍可能扫描 Run 的全部事件，不能声称开销已仅与应用数有关。封存及绑定阶段的
完整查询尚未实现；新增接口必须由实际消费者使用，不为文档提前添加无人调用的 Port。

Core checkpoint 留最新集合、恢复所需评估头及引用、ID 分配器和命令去重状态；完整历史的持久
权威是原事件。压缩只改模型上下文，不删除这些控制状态，也不从压缩摘要还原要求或验收。
归档可移动原事件，但必须保留可解析的原位置和摘要；不能只保留最新集合覆盖历史。

## 3. 生产控制输入和输出

### 3.1 首次与追加输入

首次 `InputApplied` 为普通 Turn 建立原始任务范围，来源使用真实 admission/application/item
引用。未产生模型分解时可以只有一个“原始准入范围”项：描述仅标明它指向已接受输入，原文仍由
source refs 解析；Core 不自动总结任务、不按词汇猜行动类别，更不能自动断言该项 Satisfied。
附件或非文本输入也通过相同来源引用保留，不伪造文本需求。

canonical `input_watermark` 按 §2.1 取本 Run 已提交应用的完整前缀，Core 仅传精确应用引用，
不能取会话接受输入最大值、消息序号或最新 submission。CurrentTurn 新输入应用后，必须推进
范围版本并使旧版本评估不再支持新结果；
原有细化保留为待重新确认的上下文，不自动把新指令合并成已完成要求。
NextTurn 仅接受尚未应用时不改变旧范围；独立后继准入后建立自己的集合，保留原 admission 来源。

### 3.2 模型控制调用

拟新增 Core-owned `requirements_update` 控制能力，模型只能在该次请求实际提供的 schema 下调用。
输入为 `scope_id`、`expected_requirement_revision`、`expected_input_frontier_token`、`operations[]` 及可选
`answer`；调用身份来自
已准入模型 response/attempt/call，不由参数提供。每种 operation 采用互斥结构：

frontier token 由 Core 根据精确应用集合规范摘要生成，并同 session/scope/version 一起校验。
完整应用引用保留在 Core 事件和 Store 核验中，不要求模型逐条重述日益增长的历史。工具 schema
固定；当前 scope/version/token 通过请求尾部的紧凑控制上下文提供，不能每轮改 tools/system
破坏缓存前缀。token 是乐观并发引用，不是权限凭证，也不替代 Store 的完整前缀验证。

| operation | 模型可提供 | Core 派生或强制检查 |
|---|---|---|
| ReviseScope | 描述、已有 requirement_id 或本次局部 key、来源引用、依赖局部 key/已有 ID | 分配新 ID、解析局部 key、版本加一、原始输入覆盖、DAG 无环、来源已存在；不能静默丢弃已有义务 |
| Assess | 精确要求 ID/版本、verdict、rationale、已观测证据引用 | assessment ID、精确应用集合、真实观察 frontier、所有引用归属；首版方法固定 ModelJudgment，不接受模型选择 EvidenceInspection/UserAcceptance |
| ProposeResult | 拟交付结果类型及其要求/评估引用 | 仅在同次接受的范围和全部事实下形成结果候选；不是直接写 seal，不能把未决行动降为 Answered |

局部 key 只是一个调用内解析依赖及引用的别名，不是持久 ID。Core 一次验证整个控制计划后分配 ID，
或在 working clone 中执行后原子采用；拒绝不得消耗 ID/污染去重/提交半份范围。返回结构化的
alias→ID、accepted revision 和 assessment IDs，模型不猜 Store ID。

控制操作不派发 Host 工具、不申请文件权限；它只能记录业务主张，不能替任何外部操作授权。
外部效果证据必须来自实际观察。不能在同一模型响应中引用尚未执行的工具作为已通过检查；这种
引用返回 `evidence_unavailable`，不得排队后默默把原来的成功主张升级成事实。

控制工具必须独占该模型响应的工具调用批次，不与实际 Host 工具混合执行；混合批次整体拒绝，
不能先执行部分外部效果再因控制错误回滚。`operations` 在 Core 内按声明的原子计划解析引用。
独占不等于强制额外模型轮次：工具参数 `answer` 携带最终交付正文，Core 接受合法 ProposeResult
后直接发布正文及结果候选，无须再调用模型说一遍。需要继续工作的控制调用才返回既有模型循环。
没有要求控制的普通回答可以免额外模型请求，但必须经过同一个结果仲裁器，不能继续沿用绕过
scope/义务检查的旧完成通路。schema/完成通路同批接好之前，不宣称生产接线完成。

#### 原子计划与流式边界

一个模型响应最多含一个 requirements_update；必须收齐 ModelCompleted 的工具调用批次后
统一预检，不能在流式收到前一个 Host call 时就派发。多个控制调用、控制与 Host 工具混批、
控制 answer 与另一份最终正文同时出现均返回明确协议错误，不挑一份猜测意图。已展示的流式
文本保持 append-only，但不能据此产生最终答案信用；最终正文归属按结果发布契约决定。

计划最多一次 ReviseScope，且位于 Assess/ProposeResult 之前。expected revision 约束调用前
版本；后续操作统一针对本计划产生的新版本（无 Revise 则原版本）。引用采用互斥的
`ExistingID { id } | LocalKey { key }`；新要求及新 assessment 都可声明调用内局部 key。
ReviseScope 先注册本操作全部 requirement keys，再解析依赖/替代关系并检测循环；同一操作内
合法 DAG 不受数组声明顺序限制。Assess/ProposeResult 只引用已完成操作产生的类型匹配对象，
禁止引用未来 assessment、重复 key、跨调用或错类型引用。
Core 在 working clone 中解析整个计划、分配 ID、检查结果候选，全部有效才采用。
拒绝时 scope、ID 分配器及成功去重状态不变，Host dispatch 数必须为零；原模型响应、usage、
错误回执仍应持久化，同原输入身份重放返回同拒绝，不能重复记账或产生半份范围。

生产接点在 `runtime/submission.rs` ModelCompleted 特殊控制处理、通用 completion planner
之前。移除 pending model effect 前保留其 request/effect/attempt/call 原来源；call_id 单独不是
权威。若错误转换为可恢复 ToolResult，不能依赖 reducer Err 的整次回滚来保护已修改状态。

#### 范围更新不等于删除义务

ReviseScope 对旧集合每个 ID 必须给出且仅给出一种处置：

| 处置 | 必须携带与校验的内容 | 信用变化 |
|---|---|---|
| Retain | 原 ID 与不变语义；改描述/来源/依赖应走 Replace | 新集合版本仍须重新评估，不复制 Satisfied |
| Replace | 原 ID、非空 successor refs、reason、已应用输入来源 | 旧项留历史，替代关系可追溯；新 ID 不继承验收 |
| Withdraw | 原 ID、reason、已应用用户范围变更输入/命令来源 | 不表示旧项完成，不撤销已产生的外部效果 |

遗漏旧 ID、无来源撤回、替代循环一律拒绝。用户自然语言“不要测试了，只解释”可以作为真实
已应用 user admission 的来源，不要求额外专用 UI 按钮。模型对该输入的范围解释明确记录为
ModelJudgment；类型化 scope-change 是内部事件，不冒充 UserAcceptance 或工具权限授权。
Replace 若缩减旧范围也必须引用这类用户来源，不能通过替代操作绕过撤回约束。
原始输入根引用保留，依赖关系在最终集合中闭合。引用覆盖只能证明可追溯性，不能由程序宣称
新描述忠实涵盖旧自然语言要求；该部分明确标记 ModelJudgment，而非独立验证。

效果与 obligation ledger 独立于可细化的要求列表。任务范围义务可以由有真实用户来源的 scope
retirement 结束，但不是 Satisfied，保留退休原因与旧反例；否则合法缩减需求会永远不能完成。
执行安全、未知效果、开放授权或等待不能随范围列表消失，只能由各自的类型化解决/取消事件
关闭。模型自行把“写并测试”改成“写”不能清测试义务。尚未被模型识别的语义遗漏仍属判断限制，
不以引用集合相等掩饰。

#### 共同结果仲裁与证据发现

普通正文 stop、控制 ProposeResult、审查结果、预算出口都提交同一 Core 仲裁器。存在开放
effect/wait 时不能 seal；存在未决行动义务或未知效果时不能 Answered。ActionCompleted 要求
当前完整最新 assessment 和效果规则同时满足。无额外行动承诺时，模型可在本响应判断正文
足以交付；不按工具名、调用次数或用户文本关键词推断任务类型。没有可行继续动作时选择
有事实引用的 Incomplete/Blocked，真正需用户回答才 Wait；不能返回 validation_required 无限重试。

已提交 observation ID 及可用目标版本须以紧凑 metadata 随工具结果提供给模型，正文裁剪不得
裁掉引用。模型只选择可见引用，不能猜 Store ID；控制仍须在 Core/Store 核验完整原事实。
同响应尚未执行的工具没有已提交观察，不接受占位证据或承诺稍后补齐。

EvidenceInspection 不开放给模型作为方法选项。后续若加入自动检查评估入口，必须是类型化、
明确授权的检查处理器，核验实际 predicate、目标版本与精确观察，才能产生该方法；这仍非通用
自然语言证明。模型对真实证据的分析也保持 ModelJudgment，不把其声称包装成独立检查。

### 3.3 用户明确接受

拟增 Application `AcceptRunRequirement`：外层含 conversation_id、run_id、command_id、
expected checkpoint revision；正文含 requirement_id、requirement_set_revision、明确接受动作。
由实际认证请求建立 principal/authority，command_id 本身不是认证。Application 先检查 owner，
再把可信命令来源和精确 Turn 送 Core；Core 生成 UserAcceptance assessment。
模型文字“用户已经接受”、普通后续文本、子 Agent 结果都不能替代这个命令。
不存在 UI 独立修改 accepted=true 的入口；授权面板批准工具也不等于验收任务。

## 4. Application 绑定与 Store 事务

两条现有 Application 写入口 `accept_host_event_in_transaction` 与
`reduce_and_commit_active_submission_in_transaction` 共用同一投影函数：

1. 从 typed Core event 和其真实 envelope 读取语义事实；保留 exact source envelope，拒绝未知字段。
2. 按外围 conversation/run、Core session 和事件 Turn 查询精确持久 owner（初始绑定可在同事务新增）。
   不依赖 RunID == TurnID，也不拿当前 active Turn 替换历史事件；任务或后继不借用前驱 owner。
3. 使用该绑定补齐 canonical Domain 的 conversation/run；使用信封补齐 RequirementDecisionRef。
   Core/Store checkpoint revision 各自保存；不互相填充。
4. 按 §2.1 解析输入水位和来源：每项属于该 scope/Run 中精确目的 Turn 的已应用事实；拒绝仅 queued 未 applied 的输入。
   模型操作来源保留原 response/attempt/call，用户接受绑定原认证 command receipt。
5. 同事务保存原事件、checkpoint、幂等回执、不可变索引；任一步失败全部回滚，不提前 ACK 或通知成功。

Store 拟建 `requirement_scope_sources` 与 `requirement_assessment_sources` 派生索引，只存 owner、
稳定 ID/版本、原事件 ID、source digest、必要查询 frontier；正文、描述与 rationale 只保留原事件。
scope 固定绑定 `(core_session_id, scope_id)` 与 Run，不随多 Turn 延续重建；版本唯一键为
`(run_id, requirement_revision)`；assessment 唯一键为 `(run_id, assessment_id)`；
原事件身份唯一且同 Run/Turn。不能给 assessment 设“每要求只有一条”约束，重新判断需要追加记录。
读取索引后重读原事件校验摘要及所有字段，损坏返回错误，不回退 latest 文本。无独立 CRUD 业务入口。

Store 需额外保留完整评估 frontier 下的集合查询：封存不能仅检查客户端提供的成功子集；读取同版本
截至决定 frontier 的全部评估与反例。较晚 Unsatisfied/InsufficientEvidence 不可被旧 Satisfied 掩盖；
有效评估按同要求、同集合版本、水位及结果 frontier 内的最大真实 Core event cursor 选择，
不是数据库写入顺序、数组顺序或客户端传来的 success 子集。最新评估必须 Satisfied，结果必须
引用它；同一要求同 cursor 多份判断属于歧义，直接拒绝；cursor 对应矛盾信封及 revision 倒退也拒绝。
当前 Domain 已补这一结构检查，但调用者漏传较新记录仍无法被内存函数发现；生产 Store 必须
提供完整集合，不能仅 fetch result 中列出的 IDs。新范围版本不自动复制任何 Satisfied。

执行观察已经有 `ExecutionEvidenceObserved` / Schema 65 的精确来源索引，但只证明观察来源。
它不证明目标版本仍新鲜，不自动产生 assessment；EvidenceInspection 还需目标版本可解析来源，
没有该来源时使用 InsufficientEvidence，而不是靠 tool exit=0 填满 acceptance。

## 5. 失败、幂等与恢复

| 情形 | 确定处理 |
|---|---|
| 同 command/模型 call 身份、规范化载荷相同 | 返回原命令回执及原分配 ID，不增加 scope revision/assessment/事件 |
| 同身份、不同载荷 | `command_conflict`，不修改现状；稳定摘要覆盖操作、来源和 expected 值 |
| scope revision 或输入水位过期 | `scope_conflict`，返回可读当前引用；不自动套用新范围 |
| owner/引用不符 | `invalid_reference`，不将外会话/外 Run 证据移入当前范围 |
| 尚无观察或目标版本来源 | `evidence_unavailable` 或保留 InsufficientEvidence；不凭空成功，也不强制循环检查 |
| 不合法图、重复局部 key、空描述 | `invalid_requirement_operation`，整批原子拒绝 |
| sealed 后新命令 | 拒绝修改；精确重复已成功命令可回放原回执，不重新打开 Run |
| cancel 与控制提交竞争 | 以 expected revision/CAS 定序，失败一方读取确定事实；取消不篡改历史评估 |
| commit 故障/重启/压缩 | 原控制身份重放；不重新分配 ID，不从摘要生成缺失事件 |

异常不是笼统“工具执行失败”：错误携 origin、scope、certainty、恢复动作，避免模型重复执行
已完成的外部操作。新 schema/checkpoint 版本缺必要来源应明确拒绝版本，不自动清用户库或补默认成功。

## 6. 性能与纵向验收

先建立性能基线再定数值预算；当前没有这一链路的实测性能结论。性能报告须附机器、数据规模、
冷/热态和 p50/p95：

- 正常无控制 delta：不查要求表、不扫描历史、不增加模型请求；控制投影对无关事件零额外 SQL。
- 单次范围校验随要求数 R 和依赖数 D 增长；现有有序映射的上界为 O((R + D) log R)，
  不应写成未经验证的线性性能承诺。使用迭代 DAG 检查，不能递归随输入耗尽栈。
- 增量索引追加与封存审计分开计量。追加按本批事实及固定 head 校验，不能每条追加重扫全历史。
  决定/封存必须证明 §2.2 的完整 Run 输入前缀、§4 的完整评估 frontier；因此成本可能随该 Run
  的应用数和历史评估数增长，不能承诺只与本次控制载荷有关。批量读取及 owner/frontier 索引应
  排除无关会话/Run，不能通过只读取结果自报 IDs 来伪造常数成本或跳过反例。
- 基准分别测 1/100/1,000 要求与 0/100/10,000 历史评估，分开记录 Core、绑定、Store transaction、
  seal 时间、SQL 次数、checkpoint/event 字节及 WAL 增量；不要只报告 serde 时间。
- 测量控制协议开销时排除模型和用户等待，并报告基线对照与增长曲线，再据实制定预算；不能通过
  减少要求或丢历史验收缩短时间。对应整体 P03/P06/P07 指标仍单独验收。

必须用 Core→Application→SQLite→重开纵向测试，不能用手填 canonical DTO 代替来源：

1. 无 Goal 普通回答建立输入来源，模型请求数与无要求控制基线相同。
2. 同 session 两个不同 Turn/Run，互换 owner、Core frontier、source input、assessment 均拒绝。
3. 新 ID/局部依赖解析、循环图拒绝及整批回滚；原命令重试 ID/版本/事件数量不变。
4. CurrentTurn 输入改变范围；NextTurn 先排队不改变旧水位，后继应用才建立新范围；同 Run 两个
   Turn 都有 applied_sequence=1 时全 Run 前缀为两条，不能互相覆盖或归零。等长度不同集合拒绝绑定。
5. 多要求只成功一部分拒绝 ActionCompleted；同版本新反例不能被旧成功子集掩盖。
6. 未知目标版本不能冒充 EvidenceInspection；ModelJudgment 永不升级为独立验证。
7. 模型伪造 UserAcceptance 拒绝；真实认证命令成功、跨会话及改 payload 重放拒绝。
8. 独占控制调用携 answer 可一次合法收尾；控制与实际工具混合整批拒绝、无部分副作用，不产生未来成功证据。
9. 事务注入失败无半写/无通知/无 ACK；并发 cancel、sealed 重放及进程重启身份稳定。
10. 上下文压缩/归档恢复后原要求、历史评估和证据精确可查；新参数/页面切换不改语义版本。
11. 范围更新漏掉旧要求拒绝；显式替代保留历史，不清未知效果/安全义务；模型伪造撤回来源拒绝。
    真实已应用自然语言缩范围可退休任务义务、不赋成功信用，不强迫额外 UI 命令。
12. 原义务未解决时普通正文 stop 与 ProposeResult(Answered) 一样拒绝成功，且无无限模型重试。
13. 同一原子计划用局部 key 新建要求、评估并交付正文，无额外总结请求；同一 Revise 的 DAG
    声明顺序无关，重复/未来 assessment/错类型引用整批拒绝，ID 不消耗，usage 只提交一次。
    控制前收到流式 Host call 也不得提前派发。
14. 工具正文被裁剪后 observation 引用仍在；跨 Run、未来及不存在的 observation 不取得证据信用。

实施顺序：Core owner-free DTO/控制/状态及事件 → 两入口绑定 → Store 索引/原来源读取 →
覆盖与反例查询 → V2 result/seal 消费。必须由真实生产路径消费后才勾选对应阶段；本文件本身不计完成。
