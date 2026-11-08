> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 结果封存协议：实施规格

状态：当前 V2 契约已接入生产代码，尚未完成统一编译与纵向验收。细化 COMPLETION；不是另建一个结果服务。
实施必须同批修改下列生产边界，不允许先在客户端增加一个默认为成功的 outcome 字段。

实施进度：`nola-domain/src/run_result.rs` 定义第 3 节结果载荷的 Domain 契约，
验证必填身份、六种互斥结果、必需引用、重复引用和持久整数范围；没有 NeedsInput/Wait 终态。
frontier 使用既有 JournalNumber 十进制字符串编码，避免 Web 侧超过 2^53 后失真。
Domain 只做结构检查。Core 已产生原始 scope/assessment/result/fact 事件；Application 随
原 Host 输入、checkpoint 和 effect 提交；Store schema 69 校验来源并产生 V2 seal。
schema 69 另将 Automation occurrence 的 `definition_revision` 固定为必填持久事实；
入队、领取和准入原子校验同一版本，旧 occurrence 不继承新版定义的权限。
Automation 准入完成后由绑定的 canonical Run 管理执行，不能因准入租约到期而重复派发。
字段齐全或 digest 正确本身不能证明引用存在、来源可信或任务已满足。
具体生产约束见 [Store 来源绑定](../04-存储与索引/STORE_RESULT_BINDING.md)；这些实现尚未经过本批运行验收。
现有 TaskCompletionOutcome 是旧 Core 候选判断，不作为第二份封存权威：贯通时由 Core 根据
完整事实形成 RunResult，禁止把缺失旧 outcome 自动映射为 Answered/ActionCompleted。

## 1. 原始缺口与当前整改边界

下表记录整改前的审计反例，不是仍可使用的旧协议。当前入口已统一为原 Core
`RunResultDecided` → Store 来源校验 → 同事务 V2 seal；不支持旧 status-only 封存。
保留反例用于核对测试意图，不保留旧 API 或兼容分支。

| 当前边界 | 精确位置 | 当前字段与行为 |
|---|---|---|
| Core 收尾 | core/src/agent/runtime/turn_lifecycle.rs::complete_turn / promote_queued_input | ContinueCurrent 可继续同轮；NextTurn 已改为旧 Run 封存后由 Application 独立准入，不能再自动 start_turn 到旧 Run |
| 两条 Application 写入口 | core/nola-application/src/lib.rs::accept_host_event_in_transaction / reduce_and_commit_active_submission_in_transaction | TurnCompleted 忽略 outcome，仅记录最后遇到的 terminal_status/terminal_turn_id |
| 恢复 fallback | 同文件 checkpoint_terminal_status，两入口共用 turn_lifecycle_projection | 已统一生命周期映射；仍只按 Completed/Failed/Cancelled 推断状态，没有完整结果 |
| 提交 DTO | core/nola-store-port/src/run_effect.rs::CommitTransitionInput | run_status、assistant_status 为独立 Option<String>；缺 result 及其来源 frontier |
| 清理阶段 | 两条入口的 terminal_cleanup_pending | 剩余 effects 仍推迟 Run 结束，Assistant 可先关闭；独立后继不再混入旧 Run，其他 effect 的职责分类仍须补齐 |
| seal 生产 | core/nola-store/src/assistant_journal/terminal.rs::seal | 从已提交 runs.status 取三态；CheckpointEvidence 读取 task_projection，不读取原任务结果 |
| 本地不变性 | core/nola-store/src/assistant_run_terminal_schema.sql | 封存后禁止更新 seal、manifest、runs.status/completed_at_ms |
| digest | core/nola-domain/src/assistant_run_terminal.rs::computed_digest | V1 包括粗状态与 manifest，不包括任务结果/证据限制 |

旧实现不能只把“发现 TurnCompleted”作为 Run 完成证明：当时同一 reduction 可开始下一 Turn。
该后继所有权问题已修；当前仍不能仅凭 TurnCompleted 证明业务要求已满足或结果已持久封存。
也不能把 successor_effects 非空统称为 cleanup；必须知道每个 effect 属于哪个 Turn、哪个职责。
以上证明有需要消除的语义混合，不证明已发生任意一种特定重复副作用。

实施前置进展：两条写路径的 fallback/execution status 现已统一从 checkpoint 读取，
通过同一个穷尽的 turn_lifecycle_projection 映射；不再另一入口另读 core.turn_status。
新增九种生命周期映射测试，新增 Core 状态时编译器要求更新映射，不用通配分支默默忽略。
这一步只消除生命周期解释的重复。后续生产代码已贯通原 Core 结果事件与 seal，
但不能据静态接线宣称本规格已完成验收，也不新增旧协议缺 outcome 自动成功的兼容分支。

## 2. 身份与四个阶段

1. **生成结束**：provider stop/incomplete 仅描述一次 physical request；不是业务结果。
2. **任务结果决定**：Core 基于本次准入范围、要求与证据提出结果，可同时关闭正文生成。
3. **收尾协调**：派发取消/核对、提交已有回执或安排独立后台工作；不得为“等清理”重复调用模型。
4. **结果封存**：所有封存前置满足，在一个事务写 immutable result 与 seal；这是 Run 终态权威。

UI 可以显示“回答已生成，执行状态待核对”，不能把阶段 2/3 渲染成已经封存成功。
正文 Message 的流结束只影响该 Message；不能反向设置整个 Run 的业务结果。
仍有未知副作用时，按 REC-02 允许 Cancelled/Failed 保留 unknown_effect_refs，或 Blocked
通过原 blocking fact 引用未知 effect 后封存，
不要求永远等不到的取消 ACK；后续观察写独立 effect reconciliation，不改原 seal。

Run 准入范围必须稳定：继续本轮的 steering 输入在本轮接受并记录 input watermark；
排队的 next-turn 用户输入则是独立 submission/Run，保存关联的 previous_run_id/Goal 引用。
不能因为两个用户输入相邻而把后一任务的活动状态覆盖前一任务结果。
原 CompleteAndStartNext 已改为“旧 Run 封存 → Application 准入后继 Run”，
`run_successor_admissions` 固定原 submission 与 admission，重放/CAS 不重建第二个后继。
独立后继边界不免除原 Run 的完整 V2 封存前置。
同一 Run 仍可包含多个 Assistant message/工具阶段；这些是同一准入任务的部分输出，不制造多个 RunResult。

## 3. 唯一结果值与封存前置

演进现有 Domain completion 类型。逻辑结构如下；名称定版时同步 Rust/Swift/Kotlin 协议生成或 fixture：

```text
RunResult {
  result_id, run_id, conversation_id, originating_submission_id,
  input: { watermark, applications_digest }, decided_at_revision, decision_event_cursor,
  requirement_revision, evidence_frontier,
  value: Answered(answer_part_refs)
       | ActionCompleted(requirement_acceptance_refs)
       | Incomplete(reason, remaining_requirement_refs, obligation_refs)
       | Blocked(reason, blocking_fact_refs, resolution_condition)
       | Cancelled(cancellation_id, observed_effect_refs, unknown_effect_refs)
       | Failed(error, observed_effect_refs, unknown_effect_refs)
}
```

- 所有 ID 非空且归属于该会话/Run；时间、revision、cursor 不能由 provider 提供。
- 输入 commitment 固定本次接受范围，同时绑定数量与原来源摘要；Domain 类型已切换，生产
 绑定由 Store 在决定 frontier 内核对双向完整原输入前缀。不能从当前最新消息数推算；迟到输入由其独立准入结果解释。
  数量取目的 Run 内按 Core cursor/事件内位置排序的应用前缀；不是 Turn-local applied_sequence 的最大值。
  NextTurn 先到、CurrentTurn 后到时二者不可混算；准入/应用双事件与切换清单见 TERMINAL_IMPLEMENTATION。
- Answered 至少有一条已提交正文引用；它不证明行动要求满足，不能用于规避未决行动义务。
- ActionCompleted 每项声明引用 acceptance，且与同一 requirement revision 匹配；外部效果仍需真实证据。
- Incomplete 有明确 reason 和剩余工作，Blocked 有具体事实与解除条件，不能只有字符串“失败”。
- Cancelled 必须绑定已提交取消意图；Failed 使用结构化错误分类，不能将业务检查未通过直接映射为 Failed。
  Domain 载荷引用持久 error_ref，错误实体遵守 MODULES 的分类/来源/恢复动作契约。
  observed_effect_refs 表示本结果中已核定效果的 effect 身份，unknown_effect_refs 表示仍不确定的
  effect 身份，两组不重叠；部分发生但整体未知的 effect 放后者，其已有观察回执仍保留、不丢弃。
- Wait 不在 sealed 结果值中；NeedsInput 转为持久 wait + response schema + resume identity。
  具体原子提交和恢复契约见 [WAIT_PROTOCOL](WAIT_PROTOCOL.md)；普通问题回答不授予工具权限。
- 不存在无结果的“completed 默认值”。无结果、跨 Run 引用、封存时仍存在不允许的活动工作均拒绝提交。

封存前 pending effects 必须按所有权/职责分类，而不是只看集合长度：

| 剩余对象 | 决策 |
|---|---|
| 本任务仍有效的模型/工具执行 | 不准成功封存；先完成、取消或核对 |
| 等待用户/授权请求 | 保持 Wait；取消/拒绝必须有明确退出命令 |
| 取消核对有期限且可取得回执 | stopping/recovering，有 deadline 和 owner |
| 效果未知且无安全核对方式 | Blocked 或已取消结果中保留未知 effects，不凭空 settled |
| 自动 memory extraction | 单独维护 admission，引用源 result，不改变原任务 outcome 或无限阻挡 seal |
| 下一用户输入/下一 Goal slice | 独立后继准入，不混进旧 Run cleanup |

结果在封存前是 Core 的可恢复决定；Application 不自行重评业务，只校验一致性并提交。
取消与封存以 expected_revision/CAS 决定先后；有冲突返回事实，不能通过事件数组顺序任意取最后一个。

## 4. 事务载荷与证据固定

CommitTransitionInput 应由 typed transition 区分 Progress / Waiting / Finalizing / Seal，
或采用能表达相同互斥约束的现有类型演进；删除可任意拼接的 run_status/assistant_status 字符串权威。
每条 Assistant 更新仍有明确 message identity，可与 Run lifecycle 不同，不能要求所有消息状态等于 outcome。

Seal 事务内依次校验并提交：输入去重、Core revision、所有 effect 观察、结果引用、要求/证据 frontier、
最终正文、usage、journal frontier、manifest、RunResult、seal，以及父任务可消费的 outbox 引用。
任一校验失败全回滚。外部 blob 先写完校验再引用；业务提交不等待诊断日志。
事件路径与 checkpoint 恢复路径必须调用同一个 terminal projection builder，拒绝双方值不一致。
不能把 checkpoint.active_turn 当历史结果表：它可能已经指向下一 Turn；结果保留明确 source identity。

目标 seal 采用新的协议版本与 domain separator，包含 RunResult 的 canonical digest、
requirement/evidence frontier，并保留现有 admission、journal manifest、Core revision 绑定。
V2 `AssistantRunTerminalSeal` 必填完整 `result: RunResult` 与 `result_digest`，
使用 `NOLA-RUN-TERMINAL-SEAL-V2` 绑定结果摘要及既有 frontier；不先发布一个缺结果的终态。
遵守 LC-01：Run 生命周期结束值为 `sealed`，answered/action_completed/blocked/incomplete/
cancelled/failed 仅由 result 变体表达；删除旧 seal 三态 status 的结果权威，不能往 Run status
再塞 blocked/incomplete，也不能把 lifecycle sealed 复制成 Task 成功。
Assistant message 的 closure 仍是独立展示生命周期，不能共用 Run 结果枚举；manifest 的
ChildReceipt 须绑定精确 child result digest。manifest 编码改变时一并升级其 domain separator、
流式 validator 和跨端固定向量。V2 校验 result owner、结果摘要及决定 frontier 不晚于封存 frontier。
Result 的编码使用固定字段顺序、显式枚举、整数；禁止自由 Map 顺序参与签名。
空引用集合的合法性由结果变体决定；集合按稳定 ID 排序并拒绝重复，列表顺序有意义时必须保留。
只修改 reason/outcome/evidence 引用而保留旧 digest 必须验签失败。

远端分段接收仍是 staging → 全部校验 → 原子发布；未收到完整 result/manifest 不展示终态。
用户导出/导入、远端同步及 Task receipt 都消费相同不可变结果，不从展示文本补字段。
封存后后台 journal 可以推进 live head，但不能重捕获或重签原 terminal frontier。

### 4.1 引用的权威来源与事务核验

结构验证和 digest 不能证明引用存在；以下核验必须在同一封存事务的稳定视图执行。
引用解析失败不调用模型“补一个看起来有效的 ID”，也不使用 latest checkpoint 猜原始决定。

| RunResult 字段 | 已有可复用来源 | 必须核验 / 当前缺口 |
|---|---|---|
| result_id / result value | Domain `run_result.rs`；原 Core `RunResultDecided` envelope | Store 核验原决定与来源；不能从 TurnCompleted 缺失 outcome 推断成功 |
| run/conversation/submission | runs、`input_admissions.submission_id`、后继 `run_successor_admissions.originating_submission_id` | 初始 Run 精确准入来源；后继复用原 admission。拒绝空/未知 origin，不用最新提交或消息 ID 替换 |
| input commitment | 当前 schema 的 `run_input_applications` 保存独立 Run 顺序及双来源链摘要 | `audit_for_decision` 双向核对完整应用前缀；不得取多个 Turn 局部序号的 max |
| decision_event_cursor / decided_at_revision | `run_decision.rs::load_run_decision` 读取原 RunResultDecided 与 source transition | 不识别旧 Turn 终态作为业务结果；Core revision 与 Store checkpoint revision 不同，run_events.sequence 也不是 Core cursor |
| answer_part_refs | message_parts、canonical Part owner、journal manifest | 同 Run、已提交、公开正文、固定内容 frontier；来源字符串存在不等于属于该 Run |
| requirement_revision / remaining_requirement_refs | Core `RequirementScopeChanged` 及 Store 原事件索引 | 核验当前 scope 与完整输入承诺，不借用 Goal 整体 revision、不强制创建 Goal |
| requirement_acceptance_refs | Core `RequirementAssessed`，绑定原模型 control call 与证据 | 只可引用同版本 Satisfied，核查整个范围而非成功子集；不能用 status=completed 或 source_verified 填充 |
| obligation_refs / blocking_fact_refs / evidence_frontier | Core engineering、Host receipt/effects/checks、已提交输入 | 必须先把未知范围、未知版本、检查反例分别变为已提交事实，绑定 effect/receipt/目标版本及 Core frontier；这是 Blocked/Incomplete 的实际阻塞项，不能给计数造虚假引用 |
| cancellation_id | RequestRunCancellation、提交回执与取消 reduction | 核验同 Run 的已提交意图，不用“进程已退出”或任意 request_id 替代 |
| error_ref | 原 Core RuntimeDecisionFactObserved 及其来源 envelope/Host inbox | 绑定稳定错误身份与原事件，不从展示字符串创建完整失败证据；错误分类遵守 MODULES |
| observed/unknown_effect_refs | effect_outbox、delivery attempts、Host inbox | 同 Run 的已派发事实、正确 attempt/代次、决定 frontier 内的观察；不知道结果不能视为未执行 |

上述生产映射已有实现；其正确性仍须通过最终来源替换、回滚、重放和跨端验收证明。
Blocked 现有 Domain 载荷没有独立 unknown_effect_refs；若未知效果构成 blocker，
blocking fact 必须明确引用该 effect 及未知观察来源，不能在封存时把它作为“未执行”丢弃。
要求、验收和义务以 Core 提交事件为语义来源，Store 可建带外键/唯一约束的可重建索引，
不能让 Goal 文件、客户端缓存和新 SQL 表分别接受独立业务写入。

现有执行来源还有一处必须在接线前处理的边界：
`core/src/agent/runtime_event.rs::into_public` 对内部 `ToolCompleted` 不发布对应 AgentEvent；
`runtime/tool_driver.rs` 生成该内部事件时使用经过上下文裁剪的 `model_output`。
因此不能把展示 Part、模型可见工具摘要或最新 checkpoint 当作完整 Host 执行凭据。
新增 evidence/obligation 事实应在工具完成 reducer 仍持有规范化原始 Host receipt
（`_nola_execution`）的位置生成，绑定确切 call、Host request/delivery attempt 与原响应来源，
随同次 Core envelope、checkpoint 和 delivery receipt 原子提交。Store 只索引这些来源，
不从展示文本重建 effects/checks。已有 `EngineeringLoopUpdated` 是聚合状态，
尚缺逐条稳定 obligation 身份、产生事件及消除证据；`validation_required=false` 不可补造验收。

`requirement_revision` 固定本次任务范围：用户新指令改变范围时由 Core 提出新 revision；
旧 acceptance 仍可读，但不能自动满足改写后的要求。要求来自本次实际用户输入与被接受的分解，
不是按自然语言关键词预置一组有限场景。自动验收、模型语义判断及用户明确接受应记录不同来源；
是否需要独立审查仍由模型/明确任务要求决定，不强制每次完成都加一次模型请求。

每个 acceptance 必须绑定要求身份/版本、结果判断来源和证据引用。Store 核验来源/版本/归属，
不在事务中发模型请求或运行检查。Host 的 exit=0 只是一条检查观察，不自动生成任务 acceptance。
未知范围形成具体阻塞事实；已知范围的成功检查不能消除不相关的未知范围事实。

### 4.2 可执行编码契约

Domain `RunResult::canonical_bytes/computed_digest` 已实现：类型字段固定顺序，集合引用排序，
answer_part_refs 保留展示顺序；先拒绝重复/结构错误，再进行编码。
digest = SHA256(`NOLA-RUN-RESULT-V2` + u64 大端字节长度 + canonical JSON bytes)。
所有 frontier 采用 JournalNumber 十进制字符串；文本保留原始 UTF-8，不做隐式 Unicode 归一化。
reason/detail 或 evidence/identity/frontier 改动必须改变 digest；此 digest 不是签名或授权证明。
当前 V2 seal 必填此 digest，并校验完整结果；result 自校验仍不能代替来源和终态验收。
该编码已包含 §3 的 input commitment。RunResult 与 terminal seal 各自有独立 V2 domain separator，
不能混用摘要。旧仅数量载荷不兼容，不沿用旧 digest 将新增摘要字段排除在承诺之外。

固定 answered 向量已由独立 Ruby JSON/Digest 计算并加入 Rust 回归，摘要为
`2b7dfabd0c7ccb443a8253b158f6db50cad8c61bacd477c23537b154c0bccd44`。
跨端接线必须使用同一字节契约及更多结果向量，而不是各端自行序列化 Map 后计算摘要。

### 4.3 要求与接受记录：普通 Run 和 Goal 共用的来源契约

本节定版逻辑字段与所有权，当前生产接线尚待统一验证。不是要求所有聊天先运行规划/审查模型，
也不是预置“写代码、搜索、问答”等场景路由表。任意任务的描述保留为用户输入及模型接受的分解，
Core 负责变更、来源、依赖和证据约束，不能假装通用程序可以证明任意自然语言目标已经满足。

当前落地范围：`nola-domain/src/run_requirements.rs` 已实现要求集合、assessment、方法与来源引用
的结构契约；`RunResult::validate_action_requirement_coverage` 已检查传入集合的版本、水位、
frontier 与全部要求的 Satisfied 引用覆盖，并拒绝用旧判断掩盖已提供的更新判断、同事件来源冲突。
这些 Domain 检查不查询持久来源，也无法发现调用者漏传的新反例或新 assessment。
因此当前 Store 封存会重新读取原 Core 事件和当前 frontier 内的完整判断集合，并核验
原 Host 响应、control call、scope 修改操作与证据。不能把 Domain 结构验证记作
生产行动验收完成；完整链路仍需最终运行测试。

```text
RequirementSetRevision {
  conversation_id, run_id, revision, input: { watermark, applications_digest },
  source_input_refs[], source_decision_ref,
  requirements: [{ requirement_id, description, source_refs[],
                   dependency_ids[], goal_requirement_ref? }]
}
RequirementAssessment {
  assessment_id, run_id, requirement_id, requirement_set_revision,
  assessed_input: { watermark, applications_digest }, evidence_frontier,
  source_decision_ref, method,
  evidence_refs[], verdict: Satisfied | Unsatisfied | InsufficientEvidence,
  rationale
}
```

`source_decision_ref` 是精确 Core 事件位置，不是模型自己填的可信身份；Application 持久化
时绑定 session/turn/revision/cursor。`assessment_id` 由 Core 为新决定分配，原命令重放保持不变。
上面的 canonical DTO 是绑定后的查询值，不是模型输入或原始 Core payload；Core 原事件不预填
尚未分配的自身 cursor，也不自报数据库 owner。具体责任与多 Turn 输入水位见
[要求准入与评估](../02-Core与状态机/REQUIREMENT_ADMISSION.md)。
上述 input/assessed_input 已在 Rust 三个 DTO 及比较中使用，canonical bytes 包含内容摘要；
这只防止传入值不一致，仍须生产消费者证明摘要确实对应完整原来源，不能自行填相同摘要冒充证明。
Satisfied 的 assessment 才能被 `requirement_acceptance_refs` 引用；不存在可单独更新的
“accepted=true”客户端字段。其他 verdict 保留为历史判断，不能伪造成成功 acceptance。

| 操作 | 输入与前置条件 | 同次 Core transition 的结果 |
|---|---|---|
| 接受任务范围 | 已提交输入引用、expected Run revision、描述及分解来源 | 新 RequirementSetRevision；无 Goal 的 Run 不创建长期 Goal |
| 细化/改写要求 | 明确类型化修改、旧集合版本、原输入/新输入来源 | 新集合版本，历史集合不覆盖；依赖 ID 必须存在且无环 |
| 评估要求 | 精确要求版本、当前输入水位、判断来源、证据 frontier | 追加不可变 assessment，拒绝跨 Run/未来/缺失证据 |
| 撤销旧判断 | 新事实或新要求版本及其来源 | 追加新判断/版本，保留旧 assessment，不改其原文或旧 result |
| 封存行动结果 | 当前版本全部要求的 Satisfied 引用及效果事实 | 同事务 result/seal；不能只引用成功子集隐藏未满足项 |

范围的初始来源在准入时固定为实际输入；模型可以在现有响应/工具阶段提出分解，不新增必跑的
“生成要求”请求。没有额外分解的普通回答沿用输入来源，只有正文交付、不含尚未满足的行动承诺时
可以 Answered。不能仅因调用过工具就强制 ActionCompleted，也不能把行动未完成降格成 Answered。
这项语义判断由 Core 接受的模型判断与实际效果/义务共同约束，不靠关键词判定任务类型。

集合 revision 先采用整体递增版本。任何明确的语义范围修改使旧集合的 acceptance 不可用于新封存；
未改变的要求可以在新版本中显式重新评估并引用旧证据，但不能自动复制 Satisfied。
进度通知、心跳、UI 切换不改变集合版本。Goal 的整体 revision 与本集合版本不是同一计数器；
`goal_requirement_ref` 固定 Goal/requirement/语义版本，不能引用“当前 Goal”这种浮动指针。
一个 Goal 可由多次 Run 完成，各 Run 的评估只作为 Goal 的来源，不能自动把父 Goal 标完成。

方法保留能力边界：ModelJudgment 是语义判断，不叫独立验证；EvidenceInspection 必须引用实际
观察/检查及目标版本；用户明确接受若支持，必须来源于认证用户命令，模型不能自报用户接受。
来源可读取与要求满足是两项检查；工具 exit=0 只能支持该检查的观察，不能证明全部用户要求。
现有 `runtime/submission.rs::audit_receipts_valid` 对 `action_completed` 仅核验
`effect_call_ids` 是 `successful_mutation_calls` 的非空子集；它不证明要求完整覆盖、资源版本
匹配或检查谓词满足，不能直接变成新协议的 requirement acceptance。
`runtime/goal_work.rs` 的 `source_verified` 表示引用来源确实被 Core 观测，来源还可以是
Assistant 正文；它不是独立检查或用户验收。当前 Goal completion claim 应保留
`ModelJudgment` 方法，不能因该布尔值为 true 自动升级为 `EvidenceInspection`。
对有外部效果的行动，引用相应 effect/receipt/目标版本；无可核对范围时保留 InsufficientEvidence，
不生成“万能测试通过”的 acceptance，不循环索取同样的检查来假装获得新证据。

要求集与评估的权威是已提交 Core 事件；Store 索引只能由 transition 投影，禁止独立 CRUD 修改。
封存事务对输入水位、要求版本、评估来源、证据及未决工作取同一一致性视图。
旧版本、不同 Run、来源缺失分别返回 conflict/invalid_reference/evidence_unavailable，
不让模型猜数据库 ID，也不让 Store 在事务里进行额外模型审查。
同要求、同范围版本及输入水位的评估按已提交 Core cursor 排序，最新判断才可用于封存；
同 cursor 的多条同要求评估拒绝为歧义。旧否定后出现新的合法满足判断可以接受，反向则不能
继续引用旧满足。数据库插入时间和传入数组顺序不参与决定；完整评估集合必须由 Store 读取。

实施须覆盖：无 Goal 的普通回答；带多项要求的行动；修改范围后的旧验收；同版本重新评估；
仅完成部分要求；已知目标检查不能清除未知范围；子任务成功但父要求不满足；取消与接受竞争；
压缩/重启后精确引用恢复。验收还需检查模型请求计数，证明没有为普通聊天增加强制规划或审查轮次。

## 5. 版本切换与验收闸门

不引入 V1 缺字段自动成功或双写双真相。开发环境使用新建隔离库验收。
既有用户库不得自动清空；遇不支持协议时返回明确版本错误，提供受控导出/归档路线后再另行处理。
协议版本、Store schema 版本、checkpoint 版本分别更新并记录兼容矩阵；仅 bump 一个数字不算切换完成。

同批接线顺序：Domain → Core outcome/事件 → Application 两入口及 fallback → Store/seal/digest →
父 Task 与 Skill/Goal 消费 → 远端协议与各端投影。中间编译成功不是可交付状态。

实施落点及不可拆开的消费者：

1. Core 结果决定、要求/assessment/obligation/error 来源事件及 checkpoint；决定身份重放稳定。
   Domain seal V2 必填结果和摘要，message closure 与 Run 生命周期分型。
2. `nola-store-port/src/run_effect.rs::CommitTransitionInput` 演进 typed 提交阶段；
   Application `accept_host_event_in_transaction`、`reduce_and_commit_active_submission_in_transaction`
   及 checkpoint fallback 使用同一结果投影器，不再各自从 status/数组最后事件猜结果。
3. Store `assistant_journal/terminal.rs::seal` 在同 UoW 核验上述引用并写 result/seal/manifest；
   `terminal_receive.rs` 完整校验 V2 后发布。现 schema 的状态约束、terminal 查询及不可变 trigger
   同批切换为 sealed 生命周期；等待与活动 effect 不因字符串变化被误认为已结束。
4. `runtime_task_run_binding.rs::reconcile_runtime_task_bound_run_terminal` 停止复制 Run status，
   receipt 保留 child result，Task 根据自身交付要求表达结果；Skill terminal 循环不因 TurnCompleted
   自动 Complete，Goal 仅接受精确要求评估。通知/调度不能再用粗三态猜子任务是否成功。
5. `remote_session.rs::CanonicalRunTerminal`、CLI `assistant_journal_terminal.rs`/emitter/恢复入口，
   Swift `MacAssistantJournalTransport`/`IOSAssistantJournalTransport` 与 RunObservation，
   Android `AndroidAssistantJournal.kt`/`RemoteRunObservation.kt`，Web `assistantTerminal.ts`/
   `runObservation.ts` 同批升级完整结果、摘要校验与显示。缺结果明确版本错误，不默认 completed。

前置 1 的来源字段缺失时，不先把 Store 变成拒绝所有旧生产回执的门闩，也不先发布无消费者类型
宣布 V2 完成；应先落可由正式 Core transition 生成及恢复的事实，再在同批切换生产封存。

| 测试输入 | 必须观察的状态/副作用 |
|---|---|
| TurnCompleted 与后继输入同批 | 旧 Run 一份结果；新输入独立准入；旧 seal 不包含新 Run 的 parts/effects |
| 受阻 + 无法核对的已执行效果 | blocked 持久化，未知效果仍有引用，无额外自动模型请求 |
| 决定结果后提交失败 | 无 seal/无发布/无 ACK；重试原 identity 只提交一次 |
| cleanup ACK 丢失 + 重启 | 恢复原 finalizing，deadline/未知效果出口存在；不重写业务产物 |
| Cancel 与 Seal 并发 | 一个确定结果，另一路返回冲突或已封存事实 |
| 正常事件 vs 从 checkpoint 恢复 | 同一 result、同一 journal frontier、同一 digest |
| 改 outcome/reason/证据但不改 digest | 本地验证和每个远端 reader 均拒绝 |
| 子 Run blocked、父 Goal 仍有工作 | Task/Skill 不误报成功；父保留未满足要求或合法替代证据 |
| 自动记忆维护失败 | 原封存结果不变，维护有独立失败记录，不重新打开正文 |

所有用例核对 SQLite、进程实际调用数和客户端重启投影；纯 serde round-trip 不替代纵向验收。
性能按照 P03/P06/P07 测 result validation、transaction、seal、projection 阶段，并附 manifest/证据数量。
当前这些用例未全通过，本文件是后续实施约束，不是完成证明。
