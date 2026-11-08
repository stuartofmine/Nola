# 失败专家的新尝试：共享实施设计

状态：**设计提案，尚未实现/未编译/未测试**。对应 [完整验收审计 A01](UI_PROTOTYPE_ACCEPTANCE_AUDIT_20261029.md)。root 已确认该方向；本文不把现有投递恢复算作失败重试，也不增加任何已实现接口声明。

现行接口依据：[当前运行接口](../软件架构设计/模块/01-领域与协议/CURRENT_RUNTIME_INTERFACES.md)、[Room 派发](../软件架构设计/模块/10-Goal任务与子代理/ROOM_DISPATCH.md)。实际模块：`core/nola-domain/src/expert_message.rs`、`core/nola-application/src/expert_message.rs`、`core/nola-store/src/expert_message.rs`、`core/nola-application/src/agent_room/submission.rs`，以及已有普通 prepared resend 的可信 Host factory/attest/consume。

## 1. 当前边界

- 完整专家 parent 的 manifest 冻结全部选中 participant/defaults、确定 child 身份；真实 Begin 回填模型和 PromptTemplate 定义见证。
- selected/executing 的显式恢复只能继续同一 durable intent。无 Run 的本地 absence、Host source receipt、lease 和模型证据必须符合现有恢复规则。
- sealed failed 是已有真实执行的确定终态。重放原 submission 不产生新尝试；改原 receipt/execution ID/context 会破坏原 owner/快照。
- `PrepareEditedExpertResend` 创建全新 parent/Room/contexts，适合用户修改正文。它不是“只重试原 parent 下失败的一个专家”。
- RoomParticipant 的 context 绑定现为 exact。不能为了复用旧渲染，把一个原 participant/receipt 的 context 偷换成 retry target。

## 2. 建议的唯一业务入口（待新增）

建议名 `RetryFailedExpertEntry`，由 Application 提供，输入包含：

- 稳定 `operation_id`；来源 parent conversation/message ID 与精确 canonical/message revision；
- 来源完整 manifest 的精确 parent 身份、原 `child_submission_id`；
- 该 entry 当前 attempt frontier 的 expected revision；
- 创建时间。身份及正文从 Core 读取，不接受 renderer 传模型、模板正文、凭据或“原专家”猜测。

同一个 operation 相同输入幂等返回原 durable plan；相同 operation 改 payload 明确拒绝。新 operation 在同 entry 仍有未确认/未完成 attempt 时返回 busy，不能产生第二个并发副本。不同原 entry 可各自重试，但成功 peer 不重新执行。

## 3. 同事务准入与所有权

一次 Store 写事务须完成：

1. 精确读取完整原 manifest，确认来源 parent/message/revision。不可从成功 child 子集或目录当前默认值重建集合。
2. 精确定位原 entry 与当前最新 attempt。确认对应真实 Run 已存在且 canonical seal 是可重试失败终态，Room receipt 与 seal/Run/owner 一致。仅 receipt 文案为 failed、网络断线、lease 到期、未知投递或无 Run 不构成此证明。
3. 待重试 entry 的模型见证和原 typed definition 全集必须齐全。完整 manifest 的选中集合仍须保留，但不把无关成功 peer 再准入；待重试 entry 的任何见证缺失明确拒绝，不迁移旧记录、不自动捕获当前模板。
4. 为此原 entry 原子占用 attempt frontier，分配确定的全新空 private context、Run/user/assistant/submission IDs，持久化 retry relation 和 prepared child。原 parent、原 manifest、原失败 Run/receipt/context 一律不改。
5. retry relation 显式连接 `original parent + manifest entry + original failed attempt -> new context/run`。新 context 不能冒充原 RoomParticipant.context；后续实际 Begin 核该 relation，绑定精确新 owner。

建议 durable relation 保存 operation、来源/目标身份、attempt revision、准备状态、确切准入 Run 和终态引用。它是专家消息现有共享 source/prepare/consume 的扩展，不是客户端队列或新的 A2UI runtime。

## 4. 可信 Host 与恢复

- 复用已有 frozen-model-only 私有材料与凭据校验，从原可信 source witness 重建原 primary/generation/model capability。0/多候选、凭据或模型参数漂移均拒绝；不采用当前选中默认模型。
- 复用原 PromptTemplate ID/body 全集。新 Host 的 System/Skill/UiClientData 可按现 fresh policy 构建；不携旧 history、attachments、memory、资源授权或旧 permission grants。
- 目标为空且只提交原用户正文一次；不复制旧 parent 前缀或旧 failed 子会话历史。
- 由可信 Host 在实际 config 与同一个 submission 构造完成后调用既有 attest helper；新 retry relation 加入准入验证。renderer 只能提交意图，不能制造 proof。
- 同一 Begin 事务核 operation/relation、确定 IDs、正文、模型/定义见证、context digest/input digest，并绑定实际新 Run/receipt。准备完成不叫“重试成功”；只有实际准入才显示运行，真实 seal 才显示完成/失败。
- 在 claim/factory/Begin/ack 任一处崩溃，沿原 durable plan/lease/source ledger 恢复；已存在 Run 仅 observe。没有来源证明不因 absent 或超时新起 attempt。

## 5. 共享只读投影与平台接线

建议扩展现专家消息只读投影：每个原 entry 返回 ordered attempts、latest authoritative attempt ID/revision、`can_retry` 及明确不可重试原因；每个 attempt 含 exact context/run/receipt 状态。Core 负责选择哪个 attempt 是当前显示对象。

客户端不得按时间排序猜 latest，也不得复写原失败 receipt。UI 保持同一专家身份和本地颜色，失败动作置于气泡外；新尝试显示在同一 parent/专家位置，历史失败与执行证据仍可回看。成功 peer 保留。按钮 busy、取消/错误、离开/返回及冷恢复均由原 operation 关联。

四端 caller 统一流程：读可重试事实 → 一次 Core command → 持久 prepared intent → 现可信 fresh factory → 观察共享投影。CLI 采用具名即时操作，不复制 GUI 点击布局。此设计不改变唯一新会话入口；失败重试是原消息的显式动作。

## 6. 必须通过的验收

- 两专家真实执行：A成功、B已 sealed failed；只重试B，A的 Run/文本/receipt完全不变，B新context/newRun成功，原失败证据保留。
- 原模型、目录和模板在来源失败后改变；重试仍使用冻结原定义与模型，或因可信材料漂移明确拒绝，不能静默走新默认。
- 双击同 operation、lost ack、claim后/Begin前崩溃、Begin后/回执前崩溃：始终最多一个实际 Run；所有恢复指向原意图。
- 另一 operation 与同 entry 未完成尝试竞争；old frontier revision、篡改原 entry/正文/模型/target/Host证明均回滚，无半个 plan/receipt。
- unknown delivery、cancel pending、source witness缺失、单sealed成功、不是该manifest child：均不能当失败重试准入。
- 四端真实按钮/命令、实际模型、完整输出、停止、重新进入与进程恢复；shared contract tests不替代平台操作。

这些接口在实现、集中审计及验证完成前，当前任务 A01 继续保持未完成。
