> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 状态模型与完整执行流程

### 持久租约与本地等待的时钟边界（2026-10-17 整改）

持久时间使用 RuntimeDriver 每次观察的真实 `SystemTime` epoch，同进程以 Atomic
下界防止倒退；不能用进程启动锚加 `Instant.elapsed` 推算跨进程 epoch。真实恢复
工件中，旧父进程的锚落后新 Host 约 60 秒，曾使后继领取写出早于创建的更新时间。
claim、heartbeat、普通响应与取消响应准入必须使用统一的持久时间入口。

`Instant` 仅用于进程内等待、采样耗时和 watchdog；唤醒后必须重新检查绝对 lease
及精确所有权。墙钟前跳可能使 lease 在下一次 heartbeat 前过期，此时拒绝旧 owner，
不能人为延长 expiry。墙钟后跳使新进程落后已提交时间时，Store 在同一事务内拒绝
倒退的领取、续租和恢复，保留 Task、attempt 与事件，不用未来 lease 截止时间
冒充 now，也不自动改写用户数据。历史的“首 heartbeat 前不会过期”假设不再适用。

时间不授予跨进程所有权。恢复仍核持久 attempt、generation、owner、token 与最新
expiry；旧 owner 被接管后即使本地计时有效也不得提交，不自动重放未知副作用。
当前源码回归覆盖时间前后跳与零写入拒绝，保留原父/子恢复场景；测试结果以当前任务
的集中验收记录为准，不能用投影单测代替真实接管。失败与未封存记录必须保留。

### 用户中断与 Host 响应的提交顺序

Host 的用户执行控制句柄可立即唤醒/中止网络，但该本地信号不是 Run 终态。
驱动必须在接受相应终止响应前，通过 Application 边界提交 `Interrupt`；Core
接纳取消并提交持久状态后，才可把 Host 的中止回执解释为已授权取消。
CLI 边界源对同一执行信号只排队一次，失败重排保留同一 cancellation_id；该队列
状态归属当前 Turn 驱动，在多次 Host effect 排空之间共享，只在 begin_turn 重置。
Host 响应失败后，必须先恢复已提交执行状态；已经 Cancelling 的 Run 排空现有
cancel effects，不再提交第二次 Interrupt。
队列投递资格不是第二套取消状态，权威仍为 Core 的提交结果。不能等外层调用返回后才提交
中断，否则 Host 的取消响应可能先生成 Failed 并封存。没有用户取消意图的 Host
中止仍不得伪装为 Cancelled。Mac 验收必须通过 canonicalRunTerminal 检查 Cancelled，
不能用 Trace、流断开或进程退出替代。

以下为目标契约。命名是逻辑类型，不声称已有同名 wire DTO；改变持久 schema 前需完成 MODULES 的映射审计。

## LC-01：四种状态不能混为一谈

| 维度 | 值域 | 权威所有者 |
|---|---|---|
| Run 生命周期 | queued / executing / waiting / stopping / recovering / sealed | Core 决定，Application 原子提交 |
| RunResult（仅 sealed 时） | answered / action_completed / incomplete / blocked / cancelled / failed | Core 根据结果与证据决定 |
| WaitReason（仅 waiting 时） | permission / user_input / external_event | Core，绑定请求和解除条件 |
| EvidenceState（逐项） | pending / satisfied / contradicted / unavailable / unknown_effect | Core 根据 Host 观察决定 |

上表生命周期是流程阶段，不是数据库可任意写入的枚举。当前持久 `runs.status` 只允许
`queued / running / waiting_input / sealed`。具体映射如下：

| 流程阶段 | 当前持久值与判定来源 |
|---|---|
| 准入后尚未派发 | `queued`，已提交准入 |
| 模型／工具执行、压缩、等待授权、取消中 | `running`；细分来自当前 Core checkpoint、精确 effect 和权限等待记录 |
| 等待用户回答 | `waiting_input`，绑定原 user-input wait 与回答 schema |
| 恢复核对 | 不另写猜测状态；检查原 checkpoint、effect attempt、租约与核对回执 |
| 最终封存 | `sealed`；普通对话须完整 V2 result/seal，维护任务遵循独立维护提交契约 |

Store transition 拒绝其他生命周期字符串，包括消息的 `streaming/completed`、业务结果的
`incomplete/blocked`。不能仅靠 `runs.status=running` 判断可以调用模型或取消已经完成。
RunResult 是唯一封存结果，`TaskCompletionOutcome` 仅为 Core 内部候选，不是第二份权威。
Run 结束不代表 Goal 完成。blocked/incomplete 允许封存本轮、保留未完工作；后续恢复创建关联的新 Run。
waiting 保持当前 Run，不消耗模型重试次数；取消/拒绝后有明确转换，不能永远等待。
问题身份、答复 schema、多个等待的汇合与重启恢复见 [WAIT_PROTOCOL](../05-执行与恢复/WAIT_PROTOCOL.md)。
failed 只描述运行/协议不可恢复故障，不能拿它替代“任务尚未完成”。

### 无进展恢复不是一种等待

当前 Core 的 `no_progress_recovery`（inactive/pending）属于执行控制维度，
不在 WaitReason 中。它不创建 UserInputWait、不授予权限，也不直接封存 Run。
模型主动提出且通过契约校验的问题才可进入 user_input；重复调用本身不能证明
需要用户决策。无合法恢复出口时提出 incomplete/execution.no_progress，仍由
原完成仲裁保留取消、权限等待、未知副作用及未清义务的优先级。

状态恢复必须读取当前 checkpoint 中的必填恢复状态（版本由 `AGENT_CHECKPOINT_SCHEMA_VERSION` 唯一定义），不能由 UI 文案或
历史 Trace 推断。完整恢复配额与进展见证尚未接线，不能把 pending 当作已经
实现了有界恢复预算。当前实现、转换和验收缺口唯一详述于
[HCTRL-10](../05-执行与恢复/HARNESS_CONTROL_SCHEDULING.md#hctrl-10无进展恢复与用户等待分离)。

## LC-02：主状态流

```text
Submit ──提交──► queued ──claim/admit──► executing ──证据决策──► sealed(outcome)
                                         │ ▲
                    等待授权/用户/外部事实 │ │ 关联事件提交
                                         ▼ │
                                        waiting
queued/executing/waiting ──Cancel──► stopping ──执行结果核对──► sealed(cancelled)
执行所有者丢失 ──► recovering ──有确定回执──► executing / waiting / sealed
                          └──副作用未知、无法安全恢复──► sealed(blocked)
```

| 转换 | 前置事实 | 同一提交包含 | 禁止行为 |
|---|---|---|---|
| Submit → queued | command_id 未使用、会话存在、配置可解析 | 用户消息、Run、Assistant identity、配置快照、初始 effect | 先发模型再补记录 |
| queued → executing | 当前 worker generation 与 admission receipt 有效 | attempt、effect 派发资格 | 由 UI spinner 推断开始 |
| executing → waiting | 具体请求、主体、资源/问题与 response schema 完整 | wait record、开放 obligation、投影 | 空轮询调用模型 |
| waiting → executing | response 匹配 wait_id 和 revision，权限重新验证 | decision receipt、状态、后续 effect | 另一会话的批准解除等待 |
| executing → sealed | 满足 LC-03 | outcome、证据引用、正文 seal、usage、checkpoint | stop 字符串直接标完成 |
| 任意活动态 → stopping | cancel intent 已提交 | cancel epoch、effect/task 取消投递 | 未持久化就取消句柄 |
| stopping → sealed | 已知结果已归档，未知 effect 已显式登记 | cancelled outcome、已完成副作用、未确认结果 | 声称所有副作用已回滚 |
| recovering → 后继 | inbox/outbox/replay 与 Host 可观察事实核对 | 恢复决定、代次栅栏、事件去重 | 重跑结果未知的写操作 |

允许迟到执行回执补充独立 effect 的事实，不允许覆盖已经封存的 Run outcome、正文或重新派发。
并发取消与成功封存由 Store 的 revision/CAS 决定唯一先后；后到命令返回已提交事实，不篡改终态。

手动 Compact 不属于上述活动态转换：公开请求仅在 Turn 已终态／不存在且 Host effect
journal 静止时准入。忙时拒绝而不排队，不覆盖 executing/waiting/stopping，也不产生
压缩失败事件。原执行继续由原请求和回执驱动；内部自动压缩的模型安全边界不变。

## LC-03：收尾决策顺序

1. 校验协议与相关身份；无效工具帧、损坏结果不进入正文或证据。
2. 优先处理已提交取消及失效执行权；不能因模型正好生成答案而忽略取消。
3. 有待处理工具或等待事件时不能封存 completed；保持 executing/waiting。
4. 对每项义务判断可满足性，不只看 `validation_required`。
5. 所有任务交付要求有匹配证据，才允许 completed；纯问答允许 answered，不人为制造文件验证。
6. 存在可执行的下一步、用户仍要求推进且预算允许：派发下一步，而非让模型重复声明完成。
7. 需要用户决定：waiting(user_input)，携带具体问题；用户拒绝或选择停止时可 sealed(incomplete/blocked)。
8. 有可引用的实际阻塞事实与解除条件，且无安全自动动作：sealed(blocked)。仅缺少
   验收、检查尚未运行或审查声称受阻，不足以建立 Blocked，保留为 sealed(incomplete)。
9. 预算耗尽/本次执行片结束：sealed(incomplete)，持久 Goal 保持未完成；不得改称成功。
10. 协议或运行环境不可恢复且 Store 可提交：sealed(failed)，同时保留已发生的效果与可恢复入口。
    Store 提交失败则返回 persistence 边界错误、不 ACK、不发布虚构终态；恢复可读写后再核对
    已提交状态并作决定。客户端只能显示观察/保存失败，不能自行把 Run 改成 sealed(failed)。

说明证据不足的回答可作为 incomplete 或有原阻塞事实的 blocked 报告保存；不得当作
action_completed，也不得反复丢弃并收费重生成。
报告生成失败不能阻止已确定的状态落库；允许客户端用结构化 reason 渲染本地化说明。
被跳过的工具调用留下待修正协议时，`engineering.recovery_required` 仅说明协议修正
尚未完成，不是已执行副作用的未知结果，也不能单独生成 Blocker 事实。没有未知 effect
且没有更高优先级取消/失败时，收尾记录 `incomplete(protocol.recovery_pending)`；
Core 必须先提交同次收尾的协议纠正 Error 事实，由 Runtime decision source 引用，
不能借用前轮模型来源或省略来源。Store 保持原有事实绑定与终局封存校验。
保留未完成要求，不自动把同一错误再发给模型。真正已派发但结果未确认的 effect 仍
走原未知效果恢复与受阻契约，不能因此降级为可安全重跑。
是否需要独立模型审查由模型选择/显式任务要求决定，不是所有任务完成的强制额外请求。
任何审查也不能授予权限、伪造效果、覆盖 Host 证据或绕开终态条件。

## LC-04：逐项义务，不是不可解释的债务计数

```text
Obligation {
  id, run_id, origin_effect_id, kind,
  target: Known(ResourceIdentity, expected_version?) | Unknown(reason),
  required_predicate, evidence_ids[], state,
  resolution: Automatic(action_ref) | NeedsUser(request_ref) | NotAvailable(reason)
}
```

必填来源与 kind。ResourceIdentity 包含执行环境/Host、规范化 URI、资源种类；不能凭路径后缀匹配。
文件版本指实际内容摘要或 absent 身份，不是模型自报值；远端文件与同名本地文件不是同一资源。
满足一项义务需要 check 绑定相同资源、匹配版本、明确 predicate 和有效执行回执。
`before_version=None` 不等于写后版本未知；写后版本缺失与未知范围也必须分开。
版本变化使旧证据失效，不删除其历史。用户接受风险可以生成 waiver，但不把未验证证据改为 satisfied。
安全/权限不变量不得被一般风险接受豁免。原始事实、waiver、任务结果均独立保存。

## LC-05：工具与权限流程

消息生命周期结束不等于任务成功。助手可能尚未产生正文，Run 就因明确取消、
失败或用户选择保留未知结果而封存；这时空 Parts 是合法投影，不能让投影层
另设“completed 必须有正文”的提交门槛。成功、受阻及证据不足由持久 RunResult
裁决；投影仍须校验每个已有 Part 的真实消息/Run 所有权，不可生成虚假成功文本。

模型 ToolCall → schema/能力校验 → 具体资源解析 → Core policy → 必要授权等待 →
提交执行资格 → Host 执行 → receipt/effects/checks → Core 证据核对 → 同事务投影。

工具结束至少区分未执行、成功结束、部分执行、延后与结果未知。权限拒绝不能写成执行失败或成功。
自动批准免除产品确认，不突破 OS 权限、硬性策略、未知资源边界、无效参数。
操作标签是意图，不是证据：`mutate_and_validate` 不能仅凭名称证明先写后验；
当前公开 bash schema 不提供该组合：Host 只能证明检查期间目标版本稳定，不能
证明同一进程内部的修改/检查顺序。模型应拆分修改与 validate 调用；不通过命令
关键词自动改写用途。内部效果类型仍可描述拥有真实有序回执的其他能力。
命令 exit=0 只能支持它对应的检查声明，不能证明所有用户要求都已完成。
普通 Shell 不必谎称能够完整观察所有副作用；能力声明若不能提供范围证明，结果必须体现这一限制，
由 LC-03 选择安全后继，不能事后增加一个永远无法解除的布尔债务。

### LC-05a：授权选择、等待关闭、执行恢复是三个不同事实

状态：2026-10-12 生产审计确认缺口，以下为整改契约，尚未整体实现或验收。
已实现取消等待与用户决定分型，后台等待 20 项、Host 传输 1 项测试通过；
热等待取消回归进一步验证生产 supervisor 封存、原等待关闭及数据库重开幂等；
同批许可复用回执现为 ConversationRuleApplied{source_request_id}，不再创建
第二条 Authorization 用户决定；schema 18 恢复时要求唯一更早的
AllowConversation 来源，拒绝缺失、自引用、倒序及 AllowOnce/Deny 来源。
这是来源关联校验，不是完整 typed resource/offer 或 Store settlement 来源证明。
现已新增 AgentEvent::PermissionSettled{receipt}：由 Core 原回执产生，Application
保留 Core envelope 并与 checkpoint/command receipt 一起提交 Run 日志；回滚和
冷重放验证通过。回执/结果契约现由 Domain 共享，恢复 witness 保留 request、
turn 和 call 身份。Store 提交前已核对前一 checkpoint 的唯一待决 permission
effect、原不可变 payload 中的 session/turn/request/capability/resource，以及
新事件 revision/cursor；原等待行的关闭与 Task 后继调度尚未接入。
不能用这些测试或删除 Host 自动批准分支的 34 项测试证明本节整体完成。

当前反例与实际入口：

- `Core/runtime/tool_driver.rs` 在会话许可覆盖同批调用时，结束对应 permission
  effect 并产生 `PermissionResolved`、`ToolStarted` 和新执行 effect。许可复用
  是 Core 决定，不是另一条用户点击记录。这里的 RuntimeEvent 是 reducer 内部
  临时事件，projection 会将其消耗为 PartUpdated；现在同时按原回执产生独立
  PermissionSettled。前者不直接成为 canonical Run event。新事件保留来源，但
  尚不能替代 Store 对原 effect/等待的关联核验。不得扫描显示 Part 反推授权关闭。
- Store `permission_source.rs::close_from_run` 只在整个 Run 封存后关闭绑定等待。
  `PermissionClosureSource::EffectWithdrawn` 目前只有类型/结构验证，没有生产
  提交入口。不能把该枚举的存在写成“等待已能即时关闭”。
- CLI `wait_for_background_agent_permission` 原三个热等待取消检查直接返回
  `Deny`，冷恢复取消则返回普通错误。本轮四处统一经 stop helper 撤销本地
  execution、持久化 stop intent，再返回 `BackgroundPermissionWait::StopRequested`，
  与 `Decision` 分开。Host 结束本次回调但不发用户决定、UI 失败或取消确认；
  canonical 取消流程仍负责关闭等待和确认 quiescence。该分支不声称 Task 已结束。
- 当前恢复入口要求 permission `status=resolved` 且有 decision。仅提前把该行
  改成 withdrawn，会阻止恢复；只隐藏 UI，则原工作线程仍可能等待。因此不得
  以客户端补丁、轮询超时或批量写批准实现下面状态转换。

#### 状态与唯一写入者

| 事实 | 唯一所有者 | 不得推导出的事实 |
|---|---|---|
| 用户选择（允许、拒绝、返回输入、提供指令） | Application 验证原 occurrence 后由 Store 原子记录 | 不能代表效果已执行、Run 已结束或兄弟请求也被用户批准 |
| Core 已接受选择／应用既有许可 | Core reducer；Application 提交其 transition | 不能补写另一次用户选择；必须关联原许可及本次具体 effect |
| 等待关闭 | Store 在上述 transition 或真实取消/封存事务内投影 | 不能单独授予 Task worker 新 lease 或允许重发旧 permission response |
| 后继执行资格 | 原 Task/Run 的调度事务 | 不能仅凭 UI 不再显示等待而取得；原 attempt 不得自行恢复 |

#### 提交和恢复流程

1. 用户选择先按原 request identity 幂等提交；同 ID 异内容冲突。此时 UI 可显示
   “已选择，等待继续”，但不能声称资源操作成功或删除历史选择。
2. Core 接受精确响应后提交明确 settlement，至少绑定原 permission/effect、
   原调用、选择来源（本次用户选择或既有 Core 许可）、后继 effect 身份。对同批
   复用逐项产生 settlement，不复用 Host scope 文本或客户端自报 grant。
   此输出必须是独立 typed canonical 事实，不能把临时 RuntimeEvent、Tool Part
   的 decision 字符串或快照中 pending 消失当作来源。当前公开 Part 已区分
   allow_conversation 和 conversation_rule_applied，但两者都只是显示数据。
3. Application 将 settlement 与实际 Run、checkpoint revision、event cursor
   绑定。Store 在 `commit_transition` 同一写事务中保存 checkpoint、事件、等待
   关闭证据和后继 effect。回滚必须全部回滚；不能先撤掉 UI 等待再提交执行。
4. `closed` 是等待表面的生命周期，不等同 `denied` 或 Task 终态。选择原值与
   时间保持不变；复用许可关闭的等待没有新的用户 decision。客户端读取这份
   canonical 投影，禁止凭 Trace 标题、正文或某个按钮状态推断关闭。
5. 被覆盖/撤销的旧 permission delivery 只停止传输，不能再返回 Allow/Deny，
   也不能将正常关闭包装成 `permission_ui_failed`。后继 worker 必须依据原
   Task/Run 的提交事实重新取得唯一执行资格，而不是消费被关闭的旧响应。
6. 进程重启分别恢复“选择已提交但 Core 未接受”“Core 已接受但通知丢失”与
   “后继执行已开始但结果未知”。前两者重放原身份；第三者走未知副作用恢复，
   不为消除等待提示重发工具。

#### 取消与并发仲裁

- 本地取消信号只能请求 canonical stop，并撤销本地执行资格；不能生成 Deny。
  停止提交成功但尚无关闭证据时，状态是取消处理中，不伪造 terminal receipt。
- 已提交选择与 stop 竞争由 Store 同一事务边界仲裁。已有选择保留审计历史，
  stop 可以阻止后继执行，但不改写选择。过期 worker、迟到 UI 点击不能复活等待。
- 正常停止、已被 Core 解决、传输断开、来源损坏须是不同 typed outcome。
  `PermissionTransportInterruptionReason` 现有 Timeout/Disconnected/Unavailable
  不承担正常取消；不得用字符串匹配错误消息选择这些状态。
- Core 的 `CancellationConfirmed` 只确认原取消 effect 的目标已停止。不能从
  permission 回调自行构造该响应，也不能在工作线程尚活跃时确认取消完成。

#### 纵向验收门槛

同一个隔离 workflow 必须核对 canonical checkpoint、permission 原选择/关闭
证据、实际 effect 派发次数、Task worker lease 和客户端投影，而不只断言一个
函数返回值。至少覆盖：

1. 同批相同主体复用许可：没有第二次用户批准；原等待关闭，后继恰好执行一次。
2. 同批不同主体：另一等待保持开放，不能借前一选择执行。
3. 选择提交后断进程、Core 接受后断通知：冷恢复不重复提问或执行，保留原选择。
4. 选择前取消、等待调度时取消、取得后继资格后取消：均不写伪造 Deny；分别验证
   无派发、阻止后继、实际停止/未知副作用，不能把三者合成一个成功状态。
5. settlement 事务故障注入、过期 effect/attempt、跨 Run 关闭、重复关闭及异内容
   重放：无半提交、不扩权、不恢复已结束工作。
6. CLI 与 Mac 实际等待 UI 在非终态 Run 内关闭，切换会话与冷启动后结果一致；
   收到关闭投影到下一次 UI 呈现的耗时单独测量，不能用整个 Run 耗时替代。

本节必须与 A09 资源/范围接线共享 occurrence 和来源证明，不新增独立权限真相。
先补 Core settlement 与提交契约，再改 Host typed outcome、Task 后继资格及 UI
投影；禁止先返回成功或仅删除等待行来让界面看起来正常。

## LC-06：两种必须可退出的反例

- **效果已发生、范围未知**：保留效果回执，登记 unavailable/unknown_effect；
  若 Host 有安全核对方式则执行，否则受阻退出并告知用户。禁止再次修改文件只为制造验证信用。
- **验证已执行、发现业务错误**：登记 contradicted，不混同执行器崩溃。
  有修复路径则继续；没有路径或用户要求停止，则带失败证据的 incomplete/blocked 报告。

## LC-07：压缩、恢复与长任务

压缩是 Run 内维护操作，不拥有任务终态。事务边界是“完整摘要 + 来源选择 + 原文引用 + 新 context revision”
一起提交。片段续写不提前替换工作集。维护失败保留原 context；预算不能继续时报告明确阻塞原因。
缺少热快照不是缺少原文；从 canonical 原文冷恢复。热请求不得改变原 tools/system/模型/thinking 前缀。
缓存年龄/窗口/快照缺失是不同 fallback reason。成本包含所有失败、分片与续写请求。

Goal 保存目标、约束、分解、进度与验收证据；Task 保存有作用域的执行与取消关系。
子 agent 不拥有父任务完成权、不继承超出授予范围的权限；完成回执作为父任务证据，不直接终结 Goal。
压缩通过引用重新注入这些事实，不让摘要成为 Goal 或权限规则的第二份权威记录。

## LC-08：零 I/O 终态准入

一个被接受的用户输入不一定需要外部 effect。例如 Core 发现完整必需输入无法
容纳时，可以直接形成有事实依据的 Blocked。Run 是否成立取决于输入准入与
Core 决策，不取决于有没有模型调用。不得为满足非空 outbox 人工制造工具、
模型或 Host no-op 请求。

职责与事务要求：

- Core 输出正常的 InputApplied、RuntimeDecisionFactObserved、RunResultDecided
  与 TurnCompleted，并保存已接受的输入、结果及精确所有权。
- Application 区分“有待执行 effect”和“已有合法终态”。只有两者皆无且没有
  合法等待状态时才拒绝。不能单独删除 nonempty-outbox 校验后直接报成功。
- Store 在同一写事务中创建 Run/消息/输入回执、索引决策事实和结果、提交 Core
  checkpoint、发布 assistant journal，并产生与该准入关联的真实终态回执及 seal。
  当前 seal 绑定 committed_transition，不应靠空 ID 或伪造 Host 响应补齐；需明确
  使用正常的 Core 提交来源或扩展准入提交来源，且消费者仍只认可同一 canonical seal。
- 发布或 seal 任一步失败，整个准入回滚；重试同一 submission 不得复制用户消息或
  结果。客户端只消费已提交的准入/终态，不独立猜测执行已结束。

验收必须覆盖：预算受阻且 outbox 为空的持久结果、正文为空但 Blocked 合法、
取消/未知副作用优先级、消息原文保留、重复准入幂等、封存注入失败原子回滚，
以及真实 Mac 小窗口调用。当前真实证据见 VERIFICATION 的 8zhkR5 失败记录。

已实现首个纵向路径：Application 对无 pending effect 且同时含有 RunResultDecided /
TurnCompleted 的准入，创建正常 Store 终态 transition（无 Host 响应、无额外 Core
reduction），推进 Store 提交时钟；同一外层事务内发布 journal 并生成既有 seal。
Store 初始事件验证要求对应已完成 Core Turn 与先前已索引结果；不再无条件要求
外部 effect。仅无 effect、无消息内容且结果为 Blocked 的终态允许 journal genesis
游标，不制造空正文记录。已有调用或内容仍须有真实发布 frontier。
SQLite 集成验证已覆盖重开数据库的 sealed/blocked、原文保留、outbox 为空，以及
seal 插入故障导致全部准入回滚。最新 Mac 构建复验和重复命令准入仍需补充证据。
