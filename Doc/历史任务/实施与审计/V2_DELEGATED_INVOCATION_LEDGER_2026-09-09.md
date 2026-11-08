> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 独立 DelegatedInvocation 账本（2026-09-09）

本阶段在已验证的逐事件持久化 ACK 上建立外部 invocation 身份。保留原工作区修改，
未 commit、未转换用户数据库；完整 ARCHITECTURE_V2.md 目标仍进行中。

## 身份与实际事务

`ModelAttempt` 表示可观察的物理模型采样；`DelegatedInvocation` 表示一次不透明的
外部 agent 执行；Host request/Effect 表示投递。三者不共享伪造 ID。

Domain 定义 invocation、状态与 CAS mutation；Kernel 决定 Prepared、Running、
WaitingPermission、Interrupted、Cancelling、Succeeded/Failed/Cancelled 生命周期。首次 Accepted
激活 Prepared，session 只绑定一次，终态不可变，Interrupted 不因重投递自动恢复执行。
Application 在原 Begin/CommitTransition 事务内准备/激活/结算账本，校验每条响应的
invocation UUID、Run、Effect、Part 和 backend session。Store 同事务写 ledger、
retained delivery、Part、checkpoint、事件与 receipt。ACK 清理 outbox 后保留归属。

Host envelope 显式回传 delegated_invocation_id；CLI 在实际 spawn 前拒绝缺失/空 ID
或混用 Model Attempt；Core 不消耗不匹配事件的 sequence，首次绑定后不能替换 UUID。
冷 worker 丢失依旧停在证据待补状态，不重新发送旧任务。

## 冷恢复与投影

恢复从 canonical ledger 验证当前请求和 invocation，拒绝悬空非终态账本。
真实非 SQLite 测试发现 SessionStarted 更新旧 Core 配置后与冻结 Run 配置不符；
Application 现在仅将账本证明的 session 观察映射进 Core restore 适配配置，冻结配置
JSON/digest不改，也不放开其它配置差异。

投影 snapshot owner 使用 tagged `ProjectionExecutionOwner`，区分 ModelAttempt 和
DelegatedInvocation。事件保留各自身份字段并拒绝混用；Kernel 按类型和 ID 隔离文本，
相同字符串也不能跨 namespace。Application 同时读两个规范账本，冲突归属拒绝，
未知归属不伪造。Protocol、Service、FFI 与 Mac owner/event decoder 已接通。

Store schema18、Application Service协议15硬切；共享fixtures更新，未修改用户数据库。

## 已验证的关键行为

- Kernel最终89项与Protocol28项通过，包含状态迁移、session不可替换和跨类型owner隔离。
- Store全量261项通过（含最初5项invocation测试）；最终invocation6项复验通过，
  增加同一request不可同时属于model和delegated的约束；最终取消目标恢复/索引专项7项通过。
- SQLite生命周期与receipt专项4项通过（其中2项直接模型原覆盖）：独立owner冷恢复、
  拒绝session/Part错配、receipt最终写入故障全回滚与同request冷重放。
- 真实CLI delegated8项通过，Rejected Accepted仍Prepared，Rejected Session仍Running
  且未绑定session，Rejected Text保留已提交session。父子进程退出，拒绝内容不规范化。
- Mac实际Swift Testing11项、FFI4项、Application投影5项通过。

CLI实际runtime使用普通Application提交，不将该路径冒充receipt测试；receipt故障由
真实Application.execute_received_command专项验证。最初CLI协议fixture依赖本机endpoint，
现显式提供隔离配置。Mac初次测试读取Desktop的#filePath fixture被阻塞，采样确认后
改用测试bundle资源；最终11项实际执行，未以XCTest的0 tests摘要冒充通过。

日志：`/tmp/nola-invocation-receipt-verified.log`、
`/tmp/nola-delegated-store-full.log`、`/tmp/nola-delegated-store-ledger-final.log`、
`/tmp/nola-delegated-ledger-cli-tests.log`、`/tmp/nola-delegated-projection-mac-bundled.log`。
最终全量整合结果见交接末尾。

## 取消意图与实际确认

最终非SQLite5项实际Application测试又揭示提前写Cancelled的问题。现Interrupt只把已启动
invocation写为非终态Cancelling；原请求被规范退休，迟到目标事件不能重新绑定或冒充
控制请求的ACK，必须拒绝且账本/checkpoint不变。只有真实host.cancel确认提交后才写
Cancelled及完成时间。测试同时覆盖控制ACK前后迟到目标事件，以及正常最终回答。
空Completed会合法触发续写，因此fixture使用真正的最终assistant内容验证结束。

冷恢复通过pending effect显式target_effect_id查询原invocation证据；Store给出被pending
投递或取消目标引用的原始账本事实，并使用pending Run索引，避免扫描其它Run的待投递
数据。证据由Application校验，Store不决定是否重执行。

最终非SQLite日志`/tmp/nola-delegated-memory-cancel-verified.log`5/5；
Store目标恢复`/tmp/nola-delegated-store-cancel-index-final.log`7/7；
SQLite生命周期与receipt重验`/tmp/nola-invocation-lifecycle-final2.log`4/4。

## 仍未完成

本账本不观察外部agent内部隐藏的模型采样。permission control的明确target invocation
与已提交决定投递证据、持续Supervisor/lease/结果未知副作用恢复仍需完成。
完整Kernel运行状态、旧Core依赖、CLI/Android领域所有权、配置/Device体系和§20
五端、长任务、真实LLM、10M UI/性能验收均不能由本阶段专项green替代。

最终取消状态修复后重新执行：Application全量285/285通过，
`/tmp/nola-invocation-application-verified.log`退出0；
全workspace all-targets check退出0，`/tmp/nola-invocation-workspace-verified.log`。
本轮测试与编译均已终态；后续权限控制审计只读，不改变这些验证的代码状态。
