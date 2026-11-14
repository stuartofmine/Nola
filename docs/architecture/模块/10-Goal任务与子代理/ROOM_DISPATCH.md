> **当前范围（2026-11-08 源码核对）**：Room 是统一对话内部的专家执行上下文，产品导航遵循[统一对话与可保存 Agent](../../../design/shared/MULTI_AGENT_NAVIGATION.md)。本文同时说明单成员分派与完整专家集合准入；实现存在，当前文档核对不重新执行全平台分派验收。

> **单版本开发约束：仅保留当前接口和格式，无旧版本兼容或数据迁移；不得自动删除、重建或改写用户数据。当前版本恢复、事务回滚与外部协议适配不属于兼容。**

# Room 提交与主控分派

状态：现行提交/分派合同；2026-11-08 核对类型与准入入口，历史未运行状态不作为当前暂停要求。`AgentRoom` 拥有成员及独立 conversation 映射，不是模型共享上下文。

## 不变量

- 一次用户发送拥有稳定 submission ID，先持久接受再执行分派；客户端立即显示发送中，收到接受回执后显示处理中。
- `@` 候选产生 participant ID；正文原样保存，不能靠字符串关键词猜接收人。明确提及绕过模型选择，但仍验证成员有效性。
- 无接收人时调用主控模型，输入为本次消息、有限房间展示上下文及候选成员能力；不注入成员私有会话全文。
- 主控只输出结构化 participant ID。Core 验证候选集合与房间版本；不得执行主控输出中的任意工具。UI 根据已确认的成员身份显示收件人，不要求额外生成一段路由理由。
- 目标成员使用其自身 conversation 设置、上下文和权限；分派不赋予新权限，不覆盖配置。
- 每个 submission 的状态、尝试、选择和取消都归原 room 所有；切换页面只改变订阅，不改变后台 owner。
- 未知执行结果不能重新发送；分派重试与目标执行重试分开，避免重复工具副作用。

## 生命周期

`accepted → routing → selected → executing → completed/failed/cancelled`。
显式提及可由 accepted 直接进入 selected；routing 失败保留原提交和可重试错误，不自动选择第一个成员。

Core 保存 submission 与房间版本，Host 负责实际模型 I/O，所有物理分派调用复用辅助调用预算/取消/用量账本。主控配置作为可配置场景解析，不在各客户端硬编码模型或提示词。

## 接线要求

持久 Agent 的 `description` 是公开职责说明，加入房间时随 `agent_revision` 捕获到成员快照。主控候选包含此说明，不能只凭显示名字猜能力，也不读取成员私有系统提示、凭据或私有会话。说明是待解释的数据，不是工具授权或覆盖主控规则的指令。各端创建和编辑入口一致提供该字段；允许用户留空，但不伪造能力。

房间消息分页按不可变 `input.at_ms` 和稳定身份排序，不能沿用设置表的 `updated_at_ms` 排序；领取、授权、终态更新不得移动历史消息。相同时间以存储身份确定顺序。

Application 使用 submit、精确 submission、分页 submissions、select_recipient、fail_submission、prepare_execution、claim_execution、observe_execution。精确字段以 `core/nola-application/src/agent_room.rs` 为准。精确读取不领取、不派发；客户端只能使用已实现命令，不能先显示自动分派但仍向某个默认成员发送。

PrepareExecution 持久冻结 execution_id 和精确执行意图；ClaimExecution 在同写事务中校验原意图、上下文、配置及模型版本。已有精确 Run 只观察；未创建 Run 时，仅能按可信本地来源与仍有效的原 Native 意图重试同一 execution_id 的准入，不创建新尝试。两个同意图调用仍受既有 Begin 幂等及 Run/effect lease 约束，dispatch_permitted 不授予任何额外工具权限。本机 Host envelope 可在原 source-local receipt 尚未准入、私有 tool-plan 及原 request 全部精确匹配时恢复同一请求。Host 复用现有 session_run_leases 的独立 source-admission key，在读 receipt 到真实 Begin/worker 结束期间围栏；旧 owner 只有经可信 boot/start/PID 观测证实已死才释放，活 owner 或 Unknown 只观察。该 key 不授予 Run/effect 权限，普通陈旧会话锁清理也不得误清它。远端本机镜像中没有 Run 不能证明源 Host 未执行。ObserveExecution 只接收原 run_id，从 Store 读取终态证明并核对所有权；SSE结束、UI回调、模型分派成功都不是执行成功证据。answered/action_completed、incomplete、blocked、failed、cancelled 分别投影。

Host 主控调用在 `apps/cli/src/room_routing.rs`，复用辅助调用账本和用量。各端房间仍归本机Store，不将本机房间ID假装成远端Host房间；移动辅助调用只传候选数据并回传结构化选择，本机Core再校验。独立执行器按 contextID 持有，不通过切换主窗口 selectedConversationID 来发消息。

成员退出/禁用与结果到达竞态通过 revision 及 active 校验拒绝；同一提交重放返回已有状态。无候选明确反馈，需要用户添加或启用成员。草稿是 room 所有，不随候选收件人切换丢失；实际运行上下文依然成员独立。

## 验收

显式提及、无提及主控、歧义/多语言、无候选、过期选择、重复提交、分派取消、切页迟到回调、目标执行未知结果、真实模型分派与每成员独立上下文。先整合实现再统一运行测试。


## 完整专家集合与编辑重发

专家配置现可选择 Core 角色起点和模型实际声明的精确 reasoning effort。角色目录是只读公共内置定义，工具组合与关联 Skill 仍服从真实可用目录、资源授权及具体审批。配置只保存模板 UUID 和可选 `expertReasoningEffort`，完整集合 preview 在实际模型解析后冻结参数，准入时捕获 Core 角色正文；不会把公开职责简介当成私有系统指令。传输中精确档位使用可选 `reasoning_selection`，旧缺省 payload 保持原三档契约。

`@` 选择保存专家 ID 和观察 revision；空白草稿不因浏览候选而创建会话。专家档案更新不热改已加入成员，只有显式确认的 `rejoin` 才在单事务替换当前 roster、创建新上下文。原上下文与历史发送 manifest 保留；已有活动 Run 或未完成提交阻止替换。终态读取核真实 canonical seal，不按前端已显示完成或 SSE 结束推断可替换。

当前各客户端的会话回溯使用新增 `prepare_in_place_resend`，保留父会话、Room 与专家上下文映射，并原子回溯被替换后文涉及的全部上下文；未结算预留阻止准备。下述 `PrepareEditedExpertResend` 仍是既有的新分支契约，不能将两个接口混用。

统一对话发送选中的多位专家，先调用 `AdmitExpertConversationMessage`：父消息、完整不可变 manifest、全部 child submission 和每个私有上下文占用在同一写事务准入。客户端不得先准入父消息，再独立生成某个成功子集。相同 intent 重放返回原 manifest；selected 到实际 Run 创建前也占用上下文，另一父消息不能抢占。占用只能随合法终态或经过真实 seal 核对的 reconcile 更新，不能按本机缺页推断释放。

实际 child Begin 在同一事务核对 manifest、消息/Run ID、正文和模型/配置，并绑定 Room 回执的 execution_id/state。已有执行意图必须与该真实 Run 一致；没有意图但已有真实 Run 时只能观察，不由 absence 派生执行权限。Web 的 prepared child 不进入普通后台 factory；普通专家 child 采用 Core 预留 ID 和冻结模型声明，不能读当前默认选型替代。

`PrepareEditedExpertResend` 必须获得完整原集合的实际模型和 typed PromptTemplate 见证；任何 child 缺证据都拒绝整个准备，不从旧系统消息、投递 receipt 或当前目录补造。新父会话、Room、私有上下文、权限/工具身份全部新建；仅携编辑后的用户输入，不复制历史前缀或授权。prepared 不表示已发送，完整集合最终确认必须同一快照核对全部真实准入。

退出后尚未启动的计划保留。读取/选择只恢复展示；用户显式“恢复原专家消息”才沿完整原 manifest 和 exact receipts 继续。已有真实 Run 只观察；新 Room 的 Web 子执行使用 `room:<room_id>` 客户端身份，审批 nonce/Host/Run/client 仍核真实源值，不由 Renderer 改写。

这些规则的真实 Store 回归源码与客户端交互源码已补，未运行项仍见当前任务（本地工作记录，不随仓库分发），不构成验收通过声明。

### 已知成功输出的恢复

Room 路由的 operation ID 为 submission UUID 的小写形式。Host 先校验并保存成功输出，再调用共享 `ReconcileRoomRouting`：Core 检查 Room/scenario/取消状态和确切成功 attempt，以保存的 participant_id 幂等选择仍有效的成员。记录成功后、提交成员前进程退出，恢复只读取原输出，不再次调用模型。未知 attempt 不推断成功；旧成功记录缺少输出时明确失败，不补造输出。Apple 本地/远端与 Android 本地/远端使用同一提交用例；远端回执通过既有 source observation 绑定本地 Room。

### 工作区回执与专家 Skill 收尾

宿主能力的 effect 声明必须匹配实际回执。工作区创建与交付回收返回已检查的仓库/目录版本及精确后置条件，声明为 `MutateAndValidate` 后，Core 只按检查中的资源和版本消解对应目录项。回收产生的文件 create/write/delete 效果独立保留，仍需实际文件验证；目录检查不代表业务或测试正确。失败、缺少检查及版本不匹配的回执不能清除待验证项。否则目录项残留会持续阻挡 Skill 收尾，即使原项目的测试已经通过。

关联 Skill 的进度、产物及验证记录使用 Core 提供的 ID 与真实执行证据；产物落盘、子任务结束、最终文本与 Skill 完成是不同事实。压缩后通过 `read_task` 重新取得精确工作区选择，不从摘要或路径重建目录身份。
