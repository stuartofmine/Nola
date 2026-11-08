> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 等待与恢复协议：用户输入不是任务终态

2026-10-13 真实模型验收发现回答后恢复重绑遗漏 Attempt：Core 的新 transport binding
不包含 Application 的 sampling 身份，prepare_effects 原先跳过 Rebind，导致 outbox
attempt_id=null，但正式 Attempt 仍是 prepared，派发报 identity mismatch。
重绑必须从同 Run/effect 的现存活动 Attempt 恢复 ID 与 started 标志，并核验所有权和
非终态；不创建新 Attempt、不重写 prompt，不放宽 bind_delivery 的一致性检查。
若当前事务另有 sampling mutation，仍由其覆盖最终绑定。等待本身不创建任务结果；
客户端结束传输的 WaitSuspension 信号不得映射为 Failed。真实复验见 VERIFICATION。

状态：用户问题的 Core/Application/Store 持久化路径已落地；客户端接线与完整验收仍在推进（见 §7）。与 LC-01/02、AUTH-01 和 TERMINAL_PROTOCOL 共用
同一 Run 生命周期；不新建独立调度服务，不将所有问题一律变成权限请求。

## 1. 整改前核对的实际冲突

| 位置 | 整改前行为 | 必须消除的冲突 |
|---|---|---|
| `cli/src/tools/parts/background.rs::prompt_desktop_host_question` | PID + 进程内序号生成 request_id，经 native event 发出后等待最多 24 小时 | 传输身份不是持久 wait 身份；Host 存活不是恢复前提 |
| `core/src/agent/runtime/tool_driver.rs::pending_user_input_message` | unavailable 与 cancelled 都取同一 question/instruction 文本 | UI 不可用和用户放弃不是同一业务决定 |
| 同文件工具汇合分支 | 仅在 all_terminal 时检查本次工具结果的 pending question | 需要验证问题先返回、其他工具后返回时的等待事实是否保留，不能依赖最后回执内容 |
| 同文件 NeedsInput 分支 | 写 completion_outcome=NeedsInput，随后 complete_turn | 等待被转换为 TurnCompleted；没有从该结果取得稳定 resume identity |
| `core/src/agent/runtime/submission.rs` 审查接受分支 | needs_input 候选也经 complete_turn | 审查文本不能代替具体、可回答的持久请求 |
| Application 两条事件提交路径 | TurnCompleted 忽略 outcome，写 completed | 在 UI/Task 收到投影前已丢失等待语义 |
| `core/tests/agent_core_host_contract.rs` | 断言 NeedsInput 随 TurnCompleted 发布 | 这是当前行为的反例，不是目标契约通过证据 |

不能用 NeedsInput→Failed 防止假成功；那仍然丢失等待。也不能只禁止 seal，让 Run 永久 executing。
必须同批接通等待记录、回答命令、恢复、投影和 Task 消费，再替换上述旧断言。

## 2. 唯一所有权及输入输出

Core 决定是否等待及哪些工作受其阻塞；Application 验证和提交；Store 保存权威记录。
Host/客户端只呈现已提交问题和转交答复。窗口、pipe、HTTP 连接关闭只是 transport observation。
不同客户端可以显示同一个 wait；不能各自创建问题副本、修改问题或用本地回答先行解除等待。

逻辑 DTO 演进现有 permission、question 与 Core input 类型，不再添加可任意调用的 JSON 后门：

```text
WaitRequest {
  wait_id, conversation_id, run_id, turn_id, originating_call_id?,
  revision, created_at_revision,
  purpose: UserInput(question_set) | Permission(permission_request_ref)
           | ExternalEvent(expected_event_ref),
  blocked_work_refs[],
  deadline?: { at, expiry_action },
  state: Open | Resolved(response_receipt_ref) | Withdrawn(reason_ref)
}
QuestionSet {
  questions: [{ question_id, prompt, options: [{ option_id, label, description? }],
                allow_freeform, required }],
  response_schema_digest
}
RespondToWait {
  command_id, conversation_id, run_id, wait_id, expected_revision,
  response_schema_digest,
  response: Answers([{question_id, value: Option(option_id) | Text(value)}])
            | Decline(reason?)
}
```

- 身份非空、唯一且归属一致；响应 actor 从可信接入层取得，不接受模型自报 actor。
- QuestionSet 必须非空；question_id 与每题 option_id 唯一，schema digest 来自规范化结构。
  回答携带 option_id，不从数字键、翻译后 label 或自然语言关键词推断语义。
- 必答项必须覆盖；禁止未知题号/选项、重复答案和未允许的自由文本。输入长度按已发布 schema 校验。
  模型文本不能绕过 schema，也不能被默认为用户批准。
- `blocked_work_refs` 绑定同 Run 内实际工作；等待一项不等于取消其余已授权、独立的执行。
- 不预设 24 小时后用户拒绝。没有显式 deadline 的等待可长期保留；有 deadline 必须注明
  expiry_action（如 Withdraw + incomplete），过期不授予工具权限。
- RespondToWait 的提交结果为 Committed(receipt, revision)、Replay(same receipt)、
  Conflict(current wait revision/state) 或 Rejected(typed error)。不能用 false 表示全部四种结果。
- 普通用户消息不是隐式回答；可以排队为 steering/后继输入，不能靠语言匹配消费开放 wait。

## 3. 提交、等待及恢复流程

```text
Core 提出问题 → 同事务写 wait + checkpoint + projection + 已提交通知
          → 任意已授权客户端呈现 → RespondToWait
          → 身份/schema/CAS/取消/权限核对 → 答案 receipt + Core input + 后继资格同事务提交
          → Supervisor claim → 继续执行
```

1. 等待提交失败：不展示可操作的已接受问题，不 ACK 模型/Host 回执；按相同输入身份核对重放。
2. 等待已提交、通知丢失：客户端从会话开放 waits 查询恢复，不依赖 native event 曾经到达。
3. 没有答案：不调用模型轮询，不续占用于 CPU/工具执行的活动槽位；Run/Task 保持可恢复等待。
   释放执行槽不撤销已存在的外部效果。调度实现必须区分等待占位与活动执行，不能靠关闭 Run 释放槽。
4. 答案提交：校验精确身份和请求版本，规范化后作为用户提供的数据写入历史；权限答案走 AUTH-01，
   不允许 QuestionSet 的普通选项直接成为 grant。Core 根据新事实决定下一动作。
5. 回答 ACK 丢失：原 command_id 查询/重发得到同一 receipt；不能再次派发 continuation。
6. 重启：从开放 wait + 已提交答案 + outbox 恢复；没有答案就仍等待，已有答案就只恢复未交付后继。
   无需重建原 PID、内存条件变量或原问题窗口。

准备好的提问展示不需要再次向模型询问。仅恢复 UI、重订阅、切换会话不增加 physical model request。
回答改变的上下文在下一 Attempt 才冻结；已有 Attempt 的配置/请求快照不因 UI 状态被改写。

## 4. 取消、不可用与多路汇合

| 事实 | 合法后继 | 禁止转换 |
|---|---|---|
| 关闭面板/切换会话/Host 断连 | 保持 Open，可在其他客户端恢复 | 自动 Decline、自动 Allow、Completed |
| 用户明确不回答当前问题 | 提交 Decline；Core 根据剩余工作决定 incomplete/blocked 或可继续分支 | 推断用户取消整个 Goal |
| 用户取消 Run | 取消意图与开放 wait 的 Withdraw 同事务协调；见 REC-02 | 接受旧回答重新启动已取消 Run |
| 客户端不能渲染结构化问题 | 保留问题并返回 presentation_unavailable；可用客户端呈现等价表单 | 把“无法弹窗”当业务失败或伪造答案 |
| 请求被修订 | 新 revision/schema；旧答复 Conflict | 用旧选项下标解释新问题 |
| 最后一个开放 wait 被解决 | Core 从持久事实重新计算执行资格 | 客户端凭本地面板数宣布完成 |

取消和回答竞争依 revision/CAS 决定先后。回答先提交也不排除紧随其后的合法取消；
取消后禁止派发，已派发效果按 REC-02 核对。late answer 返回已提交状态，不覆盖 seal。

一个 Run 可以有多个开放 wait。汇合根据全部待处理工作和 wait 集合计算，不能只看最后一个工具回执。
“问题先返回、其他工具后返回”和相反顺序必须产生等价事实；独立工具结果仍保存，但不触发重复提问。
同一问题集合默认一份完整回答原子提交；如产品需要分题保存草稿，草稿是展示状态，不解除业务等待。
外部事件等待只接受匹配已声明事件身份的可信观察，不把用户自由文本当事件已经发生。

## 5. 封存与跨端投影

Open wait 不产生 RunResult，不产生成功 terminal seal，也不完成父 Task/Goal。
助手已经输出问题可以结束该文本 Part，但不能据此把 Run 标成 completed。
Run 处于 waiting 时，投影包含 wait_id/revision、问题、可用操作和受阻工作引用；
输入框动作来自当前 conversation 的投影，不能继承另一个会话的执行/等待状态。

用户明确拒绝/取消、期限到达或 Core 确认不可恢复后，先提交对应 wait disposition，再按
TERMINAL_PROTOCOL 形成 incomplete/blocked/cancelled/failed；不得仅因需要提问而封存成功。
可读的证据不足报告不强制触发提问：确实没有用户能解除的条件时应 blocked，而非无限等待。

## 6. 验收与性能

验收必须同时检查真实 Application/SQLite、Host 派发次数、模型请求数和客户端呈现身份：

- 开放 wait 期间无 terminal seal；Task/Goal 不变为完成，无额外模型轮询。
- 问题/普通工具两种返回顺序、多问题乱序、并发答复：无丢问题、重复 continuation 或跨题回答。
- 提问提交前/后、回答提交前/后、ACK 前/后 kill；重启问题和答案仍准确，副作用不重复。
- 两个客户端答同一题，重复原命令与不同命令竞争；跨 Run/过期 revision/篡改 schema 全部拒绝。
- 关闭面板和明确 Decline 分别验证；断连不可伪造拒绝；等待时取消后旧回答不复活 Run。
- 五端切换会话后只呈现各自问题；无 UI 进程时仍可保持/查询等待，重新接入恢复同一 wait_id。

测量沿用 P01/P03/P06/P07：分别记录 wait commit→可操作首帧、回答→receipt、receipt→后继 start、
重启→开放 waits 可读。人工思考时间单列，不能混入模型 TTFT/工具执行耗时。
样本需包含多 wait、断线和取消；预算待真实基线后定版。本文件不声称上述验收已完成。

## 7. Schema 64 与 Application 落地证据（2026-10-11）

`UserInputWaitChanged` 原始 Core 事件持有问题集合；`user_input_wait_heads` 只保存
owner/revision/state、原事件引用与摘要，`user_input_wait_responses` 保存唯一回答命令、
回答摘要和来源事件引用。没有新增调度器，也不把选项当权限 grant。
查询通过原事件恢复完整问题并核对摘要，不从 UI 的 Run/会话参数推断所属关系。

初始 Run 事件和 `commit_transition` 均在原事务中索引 wait。正式提交路径还验证
canonical Turn/Run/message 绑定、Core session、实际 delivery witness revision/cursor。
同事件重放不重复写入；新回答必须匹配原 submission replay 中的 command/schema/answer
摘要，且从 open revision 做 CAS。不可变问题内容不能在回答时替换。
问题关闭、checkpoint、回答索引、后继 outbox 与 Task 状态投影在同一次提交完成。
取消必须先产生 withdrawn；同事务仍有 open wait 的 terminal 状态被拒绝，迟到回答不复活 Run。

Schema 63→64 只创建空的 wait 索引，不从旧 PID/Host 交互窗口制造问题或回答。
这不修复此前缺来源的 schema 62→63 历史输入迁移限制，也不声称旧进程内问题可以恢复。
已完成的 schema 62/v4 性能样本保持原身份，不用本批功能测试宣称延迟改善。

Taskchild 以持久 Task 身份返回 `resume_policy: "task_scheduler"`；普通会话为
`"conversation_run"`。Applied/Replay 都携带政策，Conflict 不携带。
回答仅提交 pending 模型 outbox；Task 恢复 queued 后仍须公平调度取得新 attempt，
不能由客户端答复直接执行，不能复用已退役 lease。

已测范围：真实 Core 模型工具事件→Application→隔离 SQLite、关闭重开查询、
合法答复、终态后重复命令、不同内容冲突、错误 owner/schema/option、两连接并发 CAS、
取消与迟到回答；Taskchild 从可信 admission 导入边界开始，经真实 fair claim/Run 绑定，
验证等待释放 attempt、答复返回 scheduler 政策、outbox 未派发/未 ACK、fresh claim 仍绑定原 Run。
Task fixture 不覆盖父委派传输认证；没有手写 checkpoint 或直接 SQL 伪造 Task authority。
命令和结果见 [VERIFICATION 的持久用户问题等待验收记录](../../../历史任务/实施与审计/VERIFICATION.md)。

已补问题及答复提交后实际子进程 SIGKILL：重开恢复原 wait，回答原命令重放不重复 continuation。
这不覆盖提交中或外部工具正在执行时的进程终止。
仍未完成：五端实际窗口与会话切换、真实 Provider、其余进程 kill/ACK 丢失注入、
完整父 Task 委派到子 Task 答复后续工具执行链、wait 首帧/答复后继的 30 样本延迟基线。
关闭重开与同进程双连接竞争不代替进程崩溃验收。

## 8. Mac 交互与 Host 退出边界

Mac 通过 Application 的 `ListOpenUserInputWaits` / `RespondToConversationWait`
读取和答复原会话、原 Run 的问题，不再向旧 PID 的控制管道写答案。查询使用会话仓库，
不借用模型配置仓库。回答重试保留同一 command ID 与 typed answer；收到回执先更新
该 wait 的权威状态，再刷新集合，读取失败可显式重试。检查执行状态、重新展开问题和
已知开放问题所属会话的投影更新均刷新持久记录，不以本地空面板推断已经回答。

正常 EOF 不等于完成，也不一律等于断连：Host 退出后重新核对 exact active Run、
assistant message 和持久等待。只有同 Run 的开放 wait，或曾收到其已提交 wait 且
该 Run 仍活动，才进入 `awaitingInput`。后者覆盖答案先提交、旧进程尚未退出的竞态。
没有等待证据仍按异常断连处理；已取消或不同 Run 不取得恢复资格。
恢复要等旧进程注册释放，重新检查开放 waits，再用现有 exact-Run 恢复入口；
Taskchild 的 `task_scheduler` 策略不能由普通 Mac 会话启动器替代。

界面按 conversation 保存问题展示状态。`稍后回答` 只隐藏，`不回答` 提交显式 Decline；
每题使用稳定 ID，选项提交 option ID，必答及文字上限在客户端提示并由 Core 再校验。
表单使用不透明语义背景；长说明不能被原生单行按钮外观裁掉。Mac 测试既检查 AX，
也检查实际选项布局高度，并人工核对测试截图。此验收不是实际 Provider 的端到端问答。

## 9. 父会话读取子 Task 问题

`ListOwnedTaskUserInputWaits { owner_conversation_id, task_id? }` 是只读 Application 查询。
返回 `[{task_id, conversation_id, run_id, wait}]`；后三项仍是原子会话/Run/wait 身份，
task_id 只提供导航来源，不复制父问题，不将查询当答复或通知 ACK。
答复继续指向原 child conversation/run，恢复仍遵守 task_scheduler 政策。

Store 在同一个 pinned SQLite 快照内核对 durable Task 的 owner、kind、取消/终态，
immutable Run binding 的 owner/child/run，以及该 Run 的开放 wait，再从原事件验证摘要。
无 binding 的 prepared/claimed Task 不能把同名 child conversation 的既有问题挂到父会话。
错误 owner、指定不存在 Task、已关闭问题、取消意图或已终结 Run 都不返回问题；
读取过滤不伪造 withdrawn，也不改写 Task/问题状态。空白 owner/task ID 拒绝。

混合工具批次中，开放问题不等于整个 Run 已等待：只有 Core 实际提交 waiting_input，
Task 才释放执行 attempt。独立 read 仍执行时保留原 lease；read 回执提交后再挂起。
此顺序已通过真实 Core→Application→SQLite 的同批 question/read 用例验证，
不能以“数据库存在开放问题”提前退役仍执行工具的任务。

CLI `/questions` 显式汇总当前会话及 owned child 的问题；自动问答只展示当前会话，
子 Task 通知不抢输入。每项答复保留原 child conversation/run，交 TaskScheduler 重领，
不能由父 CLI 直接启动。表单返回恢复候选、错误提示和延期标记，后续问题出错不能抹掉
前面已提交的父会话恢复候选；仍有当前会话开放问题时不恢复。Esc 只延期且不立即重弹。
恢复前查询失败明确报错并暂不启动，不能以读取失败推断问题已全部回答。

取消与回答按持久提交顺序决定：共享 Application prepare 在回答写事务内精确读取
Task owner/child/Run binding 的 cancel_requested。取消意图先提交时，新回答返回
Conflict(reason=cancellation_requested)，不写答案或后继，不伪造 withdrawn；等待 Core Interrupt
协调真实工具取消与 wait 关闭。答案先提交时，后续取消不撤销其事实；原 command/digest
仍 Replay，不重复 continuation，Task 的取消标记阻止重新 claim。
热 Runtime 与冷 stateless dispatcher 共用此规则，不依赖客户端判断。
Conflict 的其他 reason 为 schema_changed、wait_not_open、revision_changed。
CLI 区分四类提示；Mac 当前接受该字段但尚未展示细分原因，不能声称多端文案已完成。
完整 Host 副作用取消、多进程精确交错仍需独立验收，不能由顺序提交测试推断全部竞态通过。
Task 等待问题查询失败必须返回真实错误，不能把 Store/Service 失败改成空问题列表。
