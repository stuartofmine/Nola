> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Durable Runtime：持久任务与当前执行权

更新：2026-10-06。初次分析参考官方 `earendil-works/pi` 的实验性
`@earendil-works/pi-durable`，下方历史源码引用固定为
`9fba660cf1caca0ade5bea72269352416e595a19`。本地参考仓库现已下载至 `参考资料/pi`，
HEAD 为 `83692682f095528f8b71652ddacff7075e36e893`；实际复核和新提交边界验收见
[`../../../历史任务/实施与审计/MODULAR_REDESIGN_2026-09-28.md` 第8.13节](../../../历史任务/实施与审计/MODULAR_REDESIGN_2026-09-28.md#813-pi-源码复核与关键提交边界收口2026-10-04)。
Nola 借鉴其状态与提交原则，不引入第二套数据库、任务引擎或兼容执行路径。

## 参考原则与采用边界

Service protocol 42 将运行中远端镜像写入改为显式 Run 合同：输入必须携带
run_id，Store 在同一写事务中验证 Run、会话、remote_run_sources 中的来源 Host
以及未终态条件，再比较镜像游标。未准入、外来 Host、错误 Run 和已终态 Run
全部拒绝，包括游标落后时也不能绕过身份检查。已有镜像的 Host 不得通过更大
游标替换；跨 Host 恢复必须有明确交接合同，当前镜像接口不会猜测。终态内容仍
通过已有精确 Run/Host 校验的 terminal projection 提交，不借运行中镜像接口写入。
验收：Application 273 项库测试及 8 项权限/远端 Port 集成通过；最终 protocol-42
Store 账本与远端内容 22 项通过，Port 2 项复跑通过。测试覆盖外来 Host 的旧/同/新
游标、错误 Run、Run 终态后的镜像拒绝和无写入残留。日志：
`/tmp/nola-mirror-owner-app-20261006.log`、`/tmp/nola-mirror-owner-store-20261006.log`、
`/tmp/nola-mirror-owner-port-final-20261006.log`。Mac Port 已传递 runID 且 Swift
语法检查通过，本批未声称重新完成 Mac 构建或 UI 验收。

远端显示事实不是本机执行授权（Service protocol 41）：远端 session mirror 与
terminal conversation projection 不得从 snapshot.permission_rules 派生本机
permission_grants，也不得为镜像更新删除/重写本机会话授权。两条 Application
转换及 Store 授权写入链路已删除；对应持久化输入不再含 permission_grants，
且拒绝未知字段。远端规则可继续作为展示信息保存在镜像中；本地授权只能通过
独立、带具体主体与资源校验的权限流程产生。旧版本已经写入的授权不能仅凭
grant ID 判断是否合法，本次未自动删除既有授权，也不宣称历史授权已完成审计。
验收：Application 库 273 项通过；最终 protocol-41 的权限账本/远端身份/远端
Port 集成 8 项通过；Store 远端内容与运行投影 9 项通过，账本集成 17 项通过。
新增回归验证镜像和终态提交不能改写本机拒绝规则、重开后仍一致，并拒绝附带
permission_grants 的旧持久化输入。旧测试中“镜像规则变成本机授权”的预期已
改为“可展示、不可执行”；maintenance 终态丢弃调用配置的过期断言也改为保留
准确准入配置。日志见 `/tmp/nola-remote-display-permission-ports-final-20261006.log`、
`/tmp/nola-remote-display-permission-focused-20261006.log`、
`/tmp/nola-remote-display-permission-ledger-final-20261006.log`。

Pi Session 先提交 storage，再 adopt 内存状态，最后 publish；未知提交结果或
adoption 失败使 Session 不可继续使用，要求重开。Nola 同样以已提交事实驱动
呈现；现有事务/receipt/hot overlay 是否覆盖每个生产入口，仍需独立验证，
不能仅因类名相似而宣称等价。

2026-10-06 补充：直接 Application publication 事务的 COMMIT 失败也必须尝试
回滚清理，避免 SQLite 仍打开的事务被下一次调用误认为可加入的外层 UoW。
清理成功不证明此前 COMMIT 没有落盘，错误仍要求重开并核对持久事实；不得返回
publication 成功结果。加入调用者已有 UoW 时，提交和回滚仍只归外层所有者。
真实 SQLite 延迟外键在最外层 COMMIT 注入失败：修复前复现事务泄漏，修复后
当前连接和冷重开均无该次设置及同步 outbox 残留；外层 UoW 所有权测试也通过
（`/tmp/nola-publication-commit-cleanup-after-20261006.log`，2 项）。这只是
事务清理边界，不代表跨进程恢复验收。

随后补上 Store 写入栅栏：外层 COMMIT、外层 ROLLBACK 或命令 mutation
savepoint 的结算失败，会记录不可清除的连接级写入 poison，并启用 SQLite
`query_only`。后续事务入口和绕过事务辅助函数的直接 SQL 写入均被拒绝；
清理回滚和诊断读取保留，但成功回滚也不恢复写权限，必须重新打开 Store。
Application 回归验证旧连接拒绝重试、新连接可正常提交。该栅栏覆盖上述
外层/命令结算入口，不应外推为所有独立 repository 事务失败与内存 adoption
均已完成审计。

同日后续将 repository `WriteTransaction` 的结算也纳入同一连接级栅栏。
此前它直接委托 rusqlite 的 transaction/savepoint，独立 COMMIT 失败即使
自动回滚成功仍能继续写入，Drop 清理失败也不会通知 Store。现在该作用域持有
Store 的独占借用，统一处理 COMMIT/RELEASE、显式 ROLLBACK 和 Drop 回滚；
任何结算失败都保留 poison、清除提交证明并启用 `query_only`，只允许清理和读取。
正常 repository 回滚只撤回自己的保存点，不提交或回滚调用者的外层事务。
Service 协议升级为 43，避免继续复用缺少此语义的旧服务。
真实 SQLite 延迟外键回归修复前失败，修复后验证失败连接拒绝写入、重开后无
失败事务残留且可正常提交；丢失保存点覆盖 commit/rollback/drop 三个出口，
另验证正常显式/自动回滚保留外层写入。Store 全库 317 passed / 1 ignored，见
`/tmp/nola-repository-settlement-store-20261006.log`。
协议 43 下 Application 库 273/273、Service 库 15/15 通过，见
`/tmp/nola-repository-settlement-application-service-20261006.log`。
这项覆盖的是 Store repository 事务结算，不是所有内存 adoption、跨进程故障
或客户端增量切换的完成证明；独立 ContentUploadStore/CanonicalTextStore 的
连接并非 NolaStore，此次没有将它们的事务边界计入上述覆盖。

本次 Store 库测试为 313 passed / 1 ignored（含新增两项写入栅栏测试），
Application publication 专项 2 passed。Application 全库为 269 passed / 2 failed：
`terminal_commands_require_full_witness_and_receipt_failure_restores_authority`
在回执故障注入前因 agent task 缺少 parent owner 被拒绝；
`idle_task_claim_commits_without_external_host_effects` 缺少持久 Turn owner。
这两项仍须补齐合法准入数据并复验，不得放宽所有者校验，也不得将全库标为通过。
日志：`/tmp/nola-store-write-fence-store-20261006.log`、
`/tmp/nola-store-write-fence-application-20261006.log`。

上述两项测试随后补齐准入事实，未放宽生产校验：终态回执测试保存父会话
checkpoint 并设置 task owner，使注入的 committed-receipt 故障真正到达；
idle task 测试通过 BeginTurn 和终态 transition 保存真实历史 Turn/assistant
绑定，再执行 idle claim，断言 journal publication 仍属于该父 Run/assistant。
更新后 Application 全库 271/271 通过，见
`/tmp/nola-owner-fixtures-application-20261006.log`。这取代上面的失败状态，
不代表 UI 增量切换或所有跨进程恢复已完成。
同一代码树的 Service 指导回执重启专项也通过（1/1，
`/tmp/nola-owner-service-restart-20261006.log`）：真实 Service 三次生命周期中
保留原回执，重启后用新观察采纳状态，重复观察不再次注入指导；不涉及外部模型调用。

保存点 RELEASE 失败与 ROLLBACK 失败现在具有一致的恢复语义：Application
禁用旧 Runtime，返回不可原地重试的 `command_savepoint_release_failed`，
Service 将该错误列入销毁旧组合并重新打开的分类，且不自动重执行原命令。
这避免 Store 已启用写入栅栏、Service 却继续复用该连接的循环失败。
真实 SQLite 回归模拟嵌套参与者提前释放保存点，验证外层清理后无未发布数据、
旧 Runtime 拒绝新回执、新 Runtime 可正常执行。Application 回执专项 6/6
通过（`/tmp/nola-savepoint-release-application-20261006.log`）。
Service 库测试 15/15 通过（`/tmp/nola-savepoint-release-service-all-20261006.log`），
包括新增 RELEASE 错误重开分类和原有真实 SQLite COMMIT 不确定性重开/回执核对。
RELEASE 故障本身的注入在 Application 层，不能把分类测试描述为独立 Service
进程的 RELEASE 故障注入验收。
[Pi Session 源码](https://github.com/earendil-works/pi/blob/9fba660cf1caca0ade5bea72269352416e595a19/packages/durable/src/session/session.ts#L404-L443)。

Pi 工具先保存执行意图，恢复要求持久记录及当前工具都声明 replay-safe。
Nola 已有对应的双重 replay 栅栏：保存的安全声明与当前可用能力必须一致。
幂等性必须由实际工具/服务保证；恢复运行不等于任意外部副作用 exactly-once。
[工具恢复源码](https://github.com/earendil-works/pi/blob/9fba660cf1caca0ade5bea72269352416e595a19/packages/durable/src/harness/tool.ts#L54-L110)、
[重复调用、单次应用的真实测试断言](https://github.com/earendil-works/pi/blob/9fba660cf1caca0ade5bea72269352416e595a19/packages/durable/test/harness-tasks-recovery.test.ts#L112-L140)。

Pi 每次任务提交重新检查 invocation、running 状态与 durable abort mark；
有活跃前台子任务时，父任务先 completing 而非直接 terminal。
Nola 采用“每次提交都需要当前执行权”和“完成取决于拥有的工作”的原则，
但不能直接复制 Pi 单进程 scheduler 到多进程 Core/Service/Host。
[提交 gate](https://github.com/earendil-works/pi/blob/9fba660cf1caca0ade5bea72269352416e595a19/packages/durable/src/harness/scheduler.ts#L1190-L1210)、
[子任务等待与完成](https://github.com/earendil-works/pi/blob/9fba660cf1caca0ade5bea72269352416e595a19/packages/durable/src/harness/scheduler.ts#L980-L1025)。

Pi SQLite 使用 WAL/NORMAL，最新提交不保证在断电后保留；同一 storage 由一个
进程拥有，没有跨进程 locking。其 memo 也不是可撤销权限的永久批准记录。
这些不是 Nola 可以沿用的可靠性或授权保证。
[存储限制](https://github.com/earendil-works/pi/blob/9fba660cf1caca0ade5bea72269352416e595a19/packages/durable/README.md#L520-L527)。

## 本批根因

Nola RuntimeTask 的 renew/progress 已校验 attempt、generation、revision 和 expiry，
但旧 Retry/Finish 只接受 task ID 与可复用 token。持久任务不保证“一个 token
永远只有一个 owner”：接手 worker 可使用同 token，而旧 worker 仍可能提交。
terminal 也没有在同事务里注销原 attempt 的执行权。

另一问题在调用端：按 task ID 查询全局 registry 的最新 attempt，再构造新 witness，
会让旧 worker 借到后继 owner 的身份。只改 SQL 不改持有方式，仍不符合合同。

## 唯一执行权合同

`RuntimeTaskLeaseWitness` 必填字段：

| 字段 | 约束 |
| --- | --- |
| `task_id` / `attempt_id` | 本 worker 原 Claim/Resume 返回的任务与 attempt |
| `worker_generation` / `lease_token` | 同一轮 worker 身份，不以 token 单独认领 |
| `expected_attempt_revision` | 本 handle 最近一次接受的 CAS revision |
| `current_expiry_ms` | 本 handle 最近一次接受的 lease expiry；提交时尚未过期 |

Application 的 Retry/Finish 和 Store Port 都使用同一 typed witness，删除旧
token-only 入参，不接受可选 witness 或兼容 fallback。
Application Service协议先提升至27（attempt-bound Retry/Finish），28新增精确冷恢复
读取，31新增Store终态凭据与父任务回传，32新增持久指导准入与精确Run交付，现为33
（提交结果携带精确 assistant journal 游标）；socket身份随之变化，避免新worker复用旧服务
的执行合同。Service 当前协议为42，存储schema为41（增加持久展示顺序目录；远端 journal 的精确来源绑定和
record 级分批内容准备；协议36新增精确record中间正文引用读取，37新增Core持久化下载进度查询，38新增经证明的接收游标查询，39要求源端按实际JSON预算准入日志帧，保留原发布记录）；正在运行的旧App不由本批自动重启。
Claim/Resume 返回时绑定 worker-owned mutable handle；heartbeat、progress、retry、
finish 共用此 handle。成功更新后只替换同 identity 的 revision/expiry。
命令与接受结果之间保持串行，拒绝更新不能伪造成功的新 witness。
registry 可用于发现任务、操作员取消及清理；不是 worker 重新取得权限的来源。

跨进程请求中的attempt只是原身份的传输快照，不是永久执行准入。子worker必须在
命令spawn或Core claim/model启动前，用原handle通过fresh CAS gate；不能等副作用
发生后才报告进度。Shell的terminal sidecar与成功退出码只在Finish实际提交后发布。
旧worker退场也不得按裸task ID删除后继Agent的控制handle。

Store 在同一事务中验证精确 attempt CAS，以及 durable task 仍 running、
worker/token/expiry 一致且未过期。正常 retry/completed/failed 不能跨过持久取消标记；
当前有效 owner 可以提交 cancelled。失败返回不改变 task、attempt 或事件。
成功必须一起完成：注销原 leased attempt、转换 task、写入对应 terminal event。
本批保留已有 `superseded` 状态表示执行权已失效，具体结果以 task/event 为准；
不把仍 leased 的 attempt 当作已结算，也不虚构新的 schema 状态。

operator/recovery 的过期处理是另一事实入口：依据持久 running 状态和真实 expiry
判断，同事务注销旧执行权。它不提供无 witness 的 worker finish，也不能替旧 worker
通过查询“当前 owner”来授权。权限 suspend/resume 的精确 continuation 合同保持不变。

2026-10-06：旧 token-only 权限等待和恢复接口已从 CLI、Application、StorePort
及 Store 整链删除。生产中的存活 worker 使用精确 permission suspension/continuation
挂起，取得新 attempt 后等待父 Core 确认才激活。公平调度也不再读取旧 metadata
权限解决标记；只有精确 continuation 关联的已解决请求可参与权限恢复调度。
Store 专项 24 项通过（包含无 continuation 的旧标记不能阻塞队列）。
冷启动 scheduler 的首次 pending 权限续接已接通（验收边界见下文），不能据此
声称所有权限恢复已完成。

冷恢复接线前的持久层补强（2026-10-06）：Resume 在事务内同时拒绝复用原 token
及 worker generation，避免先提交新 grant、再被 worker 的身份校验拒绝。
最新 suspension 按关联 attempt 的递增序号选择，不用相同毫秒时间戳和随机 UUID
推断先后。两个同毫秒权限周期、反序 ID、关闭重开及拒绝复用 generation 的回归
纳入 Store `runtime_task_e2e`，25/25 通过，日志
`/tmp/nola-permission-cold-store-20261006.log`。这不替代 CLI 完整冷恢复验收。

首次 pending 冷恢复链路：scheduler 读取精确 suspension、原 attempt、原 child Run
及决定；只读检查 session/delivery 所有者已退出（live PID 和未知所有者拒绝）后，
公平恢复提交 fresh generation/token 的新 attempt。显式 `ColdPermissionResume`
随 worker 传输，worker 重新核对持久原身份与 grant，等待父 Core 确认 handoff
才恢复指定 Run。原权限回调核对 Run/Core request/capability/input/instructions 后
消费原决定，不新建权限请求；该路径禁止退回新建 Turn。读取、父确认及传输失败
不得通过普通 retry 借用历史 grant。

验证：CLI `--bin nola permission` 89/89（包括数据库重开、真实 Application/Core
父交接、原请求复用、伪造身份/参数拒绝、live/unknown owner 不消耗 suspension）。
Service 的 COMMIT 故障专项 1/1，新增恢复查询在持续 receipt 故障下仍可读。
日志 `/tmp/nola-permission-cold-owner-cli-20261006.log`、
`/tmp/nola-permission-cold-service-20261006.log`。
此为线程与真实数据库/Application/Core 的组合测试，尚非 OS 子进程或真实模型
端到端验收。grant 已提交后再次崩溃的 successor handoff 已补齐，见下节；
父会话需有活跃 pump 或 idle inbox consumer，不能让子 worker 冒用其配置启动父 Run。

父交接的发送边界也需执行权：worker 在 enqueue Core handoff 前验证 handle 与
本次 grant 的全部不可变身份，并执行 fresh lease CAS；本地已取消或失效的 worker
不得先修改父状态、再在等待循环中发现失权。新增伪造 successor token 和已取消
handoff 不产生父 inbox 命令的回归，CLI 权限专项89/89通过，日志
`/tmp/nola-permission-parent-fence-20261006.log`。

补充真实 Service OS 进程验收：`permission_request_cold_restart_e2e` 连续四次
启动服务，验证 pending 请求、自定义回复、审批后的 grant 跨进程保留；重启后续租
不会改写首次 grant 记录，原 attempt 续租被拒绝。1/1 通过，日志
`/tmp/nola-permission-grant-process-restart-20261006.log`。这仍不是 CLI worker
进程被杀后重启并完成模型任务的验收；successor 协议生产接线与专项另行验证。

### 已提交 grant 后再次崩溃

`RecoverPermissionContinuationFair` 在同一事务中验证原 occurrence、精确 child Run、
过期 predecessor CAS、取消标记、公平顺序及重试预算，撤销 predecessor、发放新
attempt、更新 task 并追加不可变 `permission_recovered` 事件。事件写入失败全部
回滚，同 predecessor 并发只允许一个 winner。原权限决定、continuation 和首次
grant 记录均不改写，不新建请求。查询明确区分 `ExactAttempt` 和仅用于候选发现
的 `RecoveryCandidate`；后者不是 worker 可直接采用的执行权。

冷 worker 由 Store 证明的祖先链生成单条 `RecoverTaskAfterPermission`；Core 原子
检查当前 owner 属于祖先集合，执行 handoff，不再发送容易过时的 Waiting/Resume
命令对。命令队列只允许领取会话队首，已租赁队首不能被其他消费者跳过；这样旧
handoff 与新 recovery 的提交顺序受存储保障。发送前仍需 fresh lease CAS，执行
前仍需父 canonical 确认，不能把 enqueue ACK 当作执行许可。

最终专项：Core Task 合同9/9、CLI权限92/92、Store任务25/25、新successor4/4、
FIFO1/1通过。CLI覆盖父仍为原owner、已为上一grant、旧handoff仍排队三种窗口；
Store覆盖多次恢复/关闭重开、并发单胜、事件回滚及旧身份/取消拒绝。日志
`/tmp/nola-permission-successor-core-20261006.log`、
`/tmp/nola-permission-successor-atomic-cli-rerun-20261006.log`、
`/tmp/nola-permission-successor-store-final-20261006.log`。

后续接线（2026-10-06）：删除普通worker的 `resume_pending -> Ok(None) -> new Turn`
fallback。新增 `BeginTaskAssistantTurn`，在同一Application事务中校验完整lease、
创建canonical Run并保存不可变task→Run绑定；已知task child拒绝generic Begin。
CLI持有的execution authority不序列化到Config，不允许由配置注入，派生子任务时清除；
创建Run期间锁住authority及lease，避免heartbeat令witness过期。
恢复读取精确绑定；无绑定只能经task准入事务证明child从未开始，不能从当前active指针推断。
绑定在Run删除后保留，防止删除历史变成重新执行许可。
worker及scheduler已接原Run终态对账，按持久终态原子收尾task/attempt及事件；
不得覆盖awaiting_reconciliation。普通parent claim也需canonical确认，不以enqueue ACK开工。

初版绑定专项5/5、CLI authority17/17、权限92/92通过，日志分别为
`/tmp/nola-task-run-binding-20261006.log`、`/tmp/nola-task-authority-cli-20261006.log`、
`/tmp/nola-task-binding-permission-cli-20261006.log`。
父Core准入身份已补进Application事务，校验canonical parent中的任务类型、Running、
取消标记、Core session身份、worker/token与input watermark；7种伪造/缺失分支均拒绝。
补强后App专项6/6、CLI权限92/92通过，日志
`/tmp/nola-task-run-binding-parent-20261006.log`、
`/tmp/nola-task-binding-parent-permission-cli-20261006.log`。
worker transport已删除`resume_existing_turn`猜测字段，恢复选择只依赖持久绑定。
缺失Run的可操作恢复、父任务终态传递以及真实worker进程故障注入仍须纵向验收，
不能把这些定向测试计作完整恢复闭环完成。
上述验收不等于完整 OS worker/真实模型 E2E，也不代表 journal 或 checkpoint 热路径完成。

## 已提交事实与同步呈现

### 终态凭据与父任务回传（2026-10-06）

worker finish、租约过期、取消与bound Run收尾同事务生成不可变terminal receipt。
凭据保存确切task/owner/child/input、结果、来源及Store发出的attempt身份；不能使用
普通事件JSON或父任务现任token伪造授权。CLI即时回传和冷恢复统一只交task+receipt ID，
Application据此入父FIFO，Core应用前再次对照Store事实。入队成功不等于父状态已应用；
CLI确认完成需匹配receipt ID、结果引用、身份及完成水位。

拒绝旧UpdateTask终态、嵌套/直接Host TaskUpdated终态以及durable agent retry/sweep
旁路。较晚取消不得覆盖先前已提交的Run/Task终态；未被父采纳的claim不阻止明确取消。
worker错误完成水位回滚；operator使用确切input水位。重复入队保留首次时间，只有
同ID不同payload才冲突。普通子任务不得继承父worker执行凭据。

Application receipt7项、binding7项通过：
`/tmp/nola-terminal-receipt-app-20261006.log`。跨OS进程fixture两次运行真实worker函数，
验证同一个已终态Run不重开、父FIFO仅一条相同命令；这是故障窗口fixture，不是网络模型
或真实模型全链验收。CLI扩大回归曾34通过、1项HTTP MCP host-kill fixture失败；
定位为macOS系统代理接管loopback请求，在测试作用域保存/设置/恢复NO_PROXY后通过，
未修改系统代理或生产网络策略。最终CLI任务域35/35、Core receipt7/7、Store31/31通过：
`/tmp/nola-terminal-receipt-task-runtime-final-20261006.log`、
`/tmp/nola-terminal-receipt-core-20261006.log`、
`/tmp/nola-terminal-receipt-store-rerun-20261006.log`。
其中MCP测试真实kill了隔离Host进程并验证按replay safety恢复；不等于真实模型任务
在任意崩溃窗口都完成验收。所有测试使用隔离数据库。

### 指导准入、交付与回执恢复（2026-10-06，纵向接线）

CLI已删除内存指导队列与AutomatedInput转换。公共入口持久提交message ID、父任务
身份与原child Run，父FIFO准入后同事务入子FIFO；ApplyTaskGuidance只有持有原worker
的当前lease才能实际注入原Run。Submitted/Accepted/Delivered/Rejected逐消息持久化，
队列满明确拒绝，不静默丢弃。父canonical conversation ID与实际Core session ID分别校验。

已交付消息的丢ACK恢复使用显式ObserveTaskGuidanceDelivery，只观察/flush当前状态，
不重新注入；不能依赖有界operation去重记录。原Run先终态时，未交付指导持久拒绝并
回传父队列，不能阻塞terminal receipt。通用active入口拒绝直接ApplyTaskGuidanceInput；
专用命令与普通提交复用同一事务内hot reducer，不再递归分发或作内部JSON往返。
提交后的会话刷新通知按Store绑定解析canonical owner/child，不借Core session ID路由。

初次CLI默认测试栈暴露checkpoint-bearing回执内联与嵌套返回的栈溢出；将两种恢复结果
及CLI锁闭包返回值堆分配后，默认栈专项通过，不以RUST_MIN_STACK作为解决方案。
CLI专项覆盖无本地worker句柄提交40条、数据库重开、32条准入/其余明确拒绝、无lease
不能交付、SessionStore重开后不重复注入及不新建Run：
`/tmp/nola-guidance-cli-default-20261006.log`。Application binding/guidance 9项、terminal
receipt 7项默认栈通过，含后续输入后清空Core去重记录再重放，以及真实ModelCompleted
在指导Submitted/Accepted两种窗口先提交的终态竞争：
`/tmp/nola-guidance-app-20261006.log`。
最终CLI任务域默认栈36/36通过：
`/tmp/nola-guidance-task-runtime-default-20261006.log`；包含上述指导专项，不重复累计。

缓存Host恢复已区分冻结运行设置与动态worker执行权：本次恢复必须显式携带执行权，
检查不可变task→child/Run绑定并对原lease做fresh CAS后，才更新Host配置、SessionStore
和执行器；不得借缓存补充调用方缺失的执行权，模型设置不随此次恢复请求更换。
专项验证错误Run不续租、fresh身份更新revision、冻结模型不变及缺失/撤销身份拒绝；
CLI任务域37/37通过：`/tmp/nola-resumed-authority-task-runtime-20261006.log`。

Service同request ID的回执以及receipt lookup保持不可变、只读，不能将原accepted
checkpoint误认为当前状态。CLI指导专用恢复入口在查明原最终结果后，发新ID的
Apply(None)观察当前状态；新观察若再次失联，则返回错误、不ACK FIFO，不递归采纳
又一份历史回执。真实Service三次进程启动验证后续输入推进后，旧回执不变、新观察
返回当前队列且不重复注入、Run数仍为1：
`/tmp/nola-guidance-service-restart-20261006.log`，1/1通过。这是Service进程恢复测试，
不是网络模型任务或真实worker被杀的全链测试。
CLI socket故障fixture验证恢复后必发不同request ID、清除lease，并在第二次观察也
丢ACK时返回错误而非旧结果；与指导纵向链一同通过默认栈回归：
`/tmp/nola-guidance-receipt-cli-final-20261006.log`（3项，包含1项既有本地化测试）。

无缓存 task child 的恢复准入已补齐：缺少显式 worker authority 时，通过持久子任务
身份拒绝恢复；执行权检查位于 orphan delivery 所有权恢复之前。普通会话不受此限制。
CLI resume 定向 51 项通过（测试进程设置 loopback NO_PROXY，不修改生产代理）。

这不是完整真实模型或所有客户端验收。指导结果 UI 和跨 worker 进程组合仍需扩大
验证；Core 指导结果历史仍带正文，应随私有 checkpoint 拆分收口，不能称为有界恢复
状态。统一 journal 的主要 Application 生产写入已接通，公开传输、客户端消费和
checkpoint 热路径仍未完成。当前准确边界见 `../04-存储与索引/COMMITTED_ASSISTANT_JOURNAL_DESIGN.md`。

每段流式输出仍通过durable command/receipt事务，不为了降低同步开销丢掉提交事实。
真实服务回归发现，2 KiB回复的同步队列累计约2.87 MiB，主要是每段都重复发布相同
Part元数据与`running`状态，不是RSS超预算。整改在canonical mutation写入前逐项
观察，保留原执行顺序，只从replica publication省略确实未变化的事实。

Stream head CAS不合并或省略；metadata的state、JSON或updated_at变化均保留，
终态status的completed_at变化也保留。小metadata通过SQL布尔比较，不加载正文到
Rust；大值保守发布，不能据此声称SQLite引擎永不读取整行。无内容变化只推进本地
运行revision，不伪造已发布base；下一次真实publication引用最后的已发布base。
canonical更新、publication和receipt继续原子提交，失败整组回滚。实际验收结果
统一记录于整体方案8.10，不能以队列变小替代冷重启后的canonical一致性。

## 验收与后续

必验：同 token ABA、旧 generation、续租后的旧 revision、expiry 边界、取消优先与
合法 cancelled、后继 registry 替换后的旧 handle、事件写入失败整组回滚、两个连接
并发提交单胜，以及关闭重开后的 attempt/task/event 一致性。
测试使用隔离数据库，不迁移或重启用户正在运行的 App 数据。

本批只整改 RuntimeTask 执行权，不宣称全部 durable 架构完成。后续仍须完成
owned foreground/background 子任务的完成屏障、未知 commit 的各入口审计、
Core 私有 checkpoint 正文分离及公开 assistant journal 传输/客户端消费链路。
源选择、授权事实和文件内容版本也必须分别建模，不能把 durable memo 视为授权。
实施和实际验收状态见
[整体方案第 8.10 节](../../../历史任务/实施与审计/MODULAR_REDESIGN_2026-09-28.md#810-pi-durable-的采用与执行权整改2026-10-03)。
