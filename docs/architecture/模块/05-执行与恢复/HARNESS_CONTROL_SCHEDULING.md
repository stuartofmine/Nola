> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Harness 控制面调度：不依赖模型输出唤醒

状态：2026-10-12 进入最后统一验收，Driver 接线 12/12、opaque 协议 7/7 已通过；
Core/CLI 完整验证继续暴露并批量修复契约问题。客户端真实进程与真实模型链路仍待
完成，逐次结果见 VERIFICATION，不以这些局部通过声称整体完成。

## 参考与实际反例

Cursor 的[持续改进 harness](https://cursor.com/blog/continually-improving-agent-harness)介绍按实际 agent 行为迭代工具与运行环境；[动态上下文发现](https://cursor.com/blog/dynamic-context-discovery)强调按需提供上下文。这里只参考公开设计，没有 Cursor 闭源运行时源码。下面的调度缺陷及处理方案来自 Nola 代码审计，不归因于 Cursor 的私有实现。

`nola-runtime-driver::dispatch_with_lease_renewal` 原来仅在 Host response 前后消费用户边界输入。无首帧、工具无输出、后台授权等待时，`recv_timeout` 只按照 delivery 心跳等待，默认心跳 100 秒；甚至超时后也只续租而不消费输入。用户补充或控制命令的处理依赖外部系统主动输出，这是控制面与数据面错误耦合，不能通过更频繁地请求模型解决。

## HCTRL-09 Mac 启动与冷恢复来源契约

Mac 写出的冻结启动描述符由 `nola-host-protocol::HostMacLaunchDescriptor` 严格解码，
`input_origin` 为必填闭域，仅允许 `interactive_user`、`scheduled_action`。Native
新输入准入同时核对描述符来源与实际 Host input 来源相同；未知来源、内部恢复／委托来源、
缺字段及两处来源不一致均拒绝，不以默认值或忽略未知字段兼容。

`resume` 没有新输入通道，禁止顶层及 configuration 中注入 input_origin。它只携带
原冻结描述符，按原 conversation／Run／assistant 身份读取持久化 invocation，并在
执行前比较完整 binding。解析器的缺省交互来源不能拿来否定原计划任务来源，也不能
把描述符来源写回为新的用户指令。改变冻结来源必须拒绝且不得推进 checkpoint。

该字段只是原 Run 的恢复事实，不是新指令或权限。Application 保存原 invocation；
冷恢复读取原身份与描述符，重新验证当前凭据、工作区、权限规则。计划任务的技能
目录仍只是发现根，不能由恢复流程扩成可写工作区。UI 当前会话和模型选择不能覆盖
原 Run 的冻结配置，恢复不重新提交原 prompt。

验收分三层：闭域／必填／往返及来源匹配反例；真实 bundled Host 启动与权限冷恢复；
真实 provider 的完整终局。协议单测通过不能代替后两层。此次真实 helper 验收曾发现
Swift 必写且恢复必读字段不在 Rust schema，说明客户端 fixture 与纯 Rust 单测各自
通过仍可能掩盖跨语言契约漂移；必须保留实际客户端编码到 Native 解码的纵向用例。

### 精确读取终局 Run 不等于重新恢复执行

当前 Run 唯一终局存储状态是 `sealed`，业务结果由 canonical result 区分回答、任务
完成、未完成、受阻、取消及失败。精确 invocation 查询必须在同一 pinned snapshot
中取得冻结配置、checkpoint 和原 Run 的正式终局证据，核对三方身份、摘要、完成
时间及 checkpoint decision；不能用旧 completed／failed／cancelled Run 标签判断。
消息、工具、Turn 与子任务的状态域独立，不做全仓机械替换。

读取封存事实不产生新准入或执行权。无 seal、错 owner、配置摘要改变或 result 与
checkpoint 不一致均拒绝。允许读取终局原配置，不代表允许 resume 终局 Run；实际
resume 仍要求原活动 Run 与执行绑定。验收必须包含正常回答、取消和失败封存后冷读，
以及封存记录缺失反例，不能仅验证恢复到最后一条文本。

## HCTRL-01 所有权与流程

| 模块 | 输入 | 输出及责任 |
|---|---|---|
| BoundaryInputSource | 已存在的用户命令/通知队列 | 提供有限一批，保留稳定身份；成功提交后 ACK，失败返回原输入 |
| Runtime Driver | 已提交 Host requests、opaque boundary source、心跳时钟 | 安排轮询和 Host response ACK，不解释输入含义、不生成执行结果 |
| Application/Core | 原始 typed 输入与当前 Run | 决定补充、取消、等待或后继语义，原子提交后发布 acceptance |
| Host | 已 claim 的 effect 和 commit ACK | 协作执行/停止，报告观察证据；停止请求不是停止完成证据 |

静默等待现在取 `min(下次 lease 心跳, 100ms 控制轮询间隔)`。只有存在 source 且当前 dispatch 尚无作用域错误（`scope_error`）时启用控制轮询；单一 effect 的 admission 错误不关闭健康 sibling 的控制源（见 HCTRL-03）。不存在 source 不增加轮询唤醒。超时消费 source 后立即重新检查续租期限。已有持续流的 delta 前后提交顺序不改变，也不向历史或 prompt 插入状态文本。

100ms 是本地调度间隔，不是端到端 SLA：同步的 source I/O、Application 事务、发布回调、OS 调度仍可能延迟。它不是新的请求超时、模型 token 上限或 lease 有效期。最终验收须分别测静默输入准入延迟、空队列查询开销、续租正确性，不能声称已达到 100ms 用户可见响应。

## HCTRL-02 取消、失败与恢复

- 远程客户端身份来自该 Run 的正式 source submission receipt，不来自会话 ID、
  UI 当前选中项或前一个 Run。CLI 本地 NextTurn 产生独立本地 Run，前一轮 remote
  client 不能借 resume/observe 接管它。Web 当前有活动执行时返回 run_in_progress，
  不把本地入队 helper 假装成已支持的远程排队 API。
- 取消中的原压缩 effect 可能晚到 summary；确认其 exact envelope 身份后只完成
  原 effect 的停止收尾，不要求已经撤销的摘要计划重新存在，不应用 summary。
  错 request/attempt/lease 仍拒绝。Host 自述停止不等于用户已取消；没有已提交的
  cancellation ID 时不得生成 Cancelled。
- delegated Completed 要先验证 typed items/usage，再保存原 effect/request/attempt
  来源，最后退休 effect。该来源只证明结果来自哪个已准入请求，不证明业务已完成；
  未评估需求仍是 Incomplete，只有正式证据支撑的 acceptance 才能 ActionCompleted。

- Driver 不按文字或整数值识别取消，也不重排 FIFO。补充输入继续交 Application 决定 CurrentTurn/NextTurn；stop/cancel 的优先级、撤销开放等待和封存规则仍由 Core 的 typed 控制命令决定。
- 收到成功 acceptance 才发布，并在发布之后 ACK source。取消输入被提交并不等于已派发进程停止；后续 claim 只读取 canonical outbox，不从输入构造新模型/工具 effect。
- source/admission/ACK 失败统一属于 `DispatchScope`，无论发生在静默轮询、非 delta 帧提交之前还是 delta 提交之后：原机制重排全部未 ACK 输入（提交后 ACK 丢失由 Application 去重）；Driver 关闭本派发作用域中的全部 effect，逐一发协作 abort，拒绝所有迟到帧且不再消费 source。不合成 cancelled/failed/success 结果，不重放未知外部效果。delta 已提交的事实保留，不回滚成未发生。
- 单一 Host response 的 fencing/接受错误属于 `Effect`，只关闭该 effect；错误范围由内部类型表达，不根据错误文本推断。整个作用域失败同时覆盖未附 delivery 的已派发请求，不仅遍历 lease map。
- 同一个已经关闭的 delivery 收到迟到帧，不重复触发 abort 回调。首个错误保留；不会持续轮询同一失败输入或启动后继工作。
- cooperative abort 不是强杀机制。没有 abort 回调或 Host 不响应，scoped worker 仍可能等待；这属于 Host 取消契约的后续真实验收范围，当前变更不声称解决不可中断外部 I/O。

## HCTRL-05 进程内控制能力与后台转移

执行 owner 的后台转移能力由产品装配显式注入，用户请求只是消耗该能力的操作意图。
请求本身不能将不支持后台转移的句柄升级为支持；子级继承能力和共享的请求，不获得额外权限。
这里的能力仅指进程管理支持，不替代 Core 对命令、资源或副作用的授权。

取消与转移请求并发时，已观察到的取消阻止新的后台 promotion，包括自动宽限时间到期。
取消请求成功也不证明进程已停止：继续等待 canonical terminal 或显式未知效果出口，不能
提前返回 cancelled，更不能返回“后台运行中”把取消伪装成成功移交。原进程和任务身份
保持不变，前台等待释放不得创建第二个 Shell worker。

最终验收覆盖：默认句柄请求后台不自授能力、显式支持下子级一次消费请求、取消与手动
转移同批到达、取消与自动宽限到期、完成先于转移，以及取消提交失败/进程迟迟未退出。
本条是接受契约，正在同步代码；不是这些竞态已经通过测试的证明。

## 验收源码与未覆盖范围

`core/nola-runtime-driver/tests/opaque_protocol.rs` 新增：没有任何 Host 帧时仍消费输入并 ACK；输入拒绝时无 ACK、保留输入、精确 abort 且不接受晚到结果；opaque Application 提交取消后仅查询已提交后继，不自行重新派发。

补充双 effect 反例：source 读取失败和输入准入拒绝分别发生在非 delta 前/delta 后，另一 effect 的后续帧不提交、未 ACK 输入不再次消费，每个 effect 只发一次 abort；已提交 delta 不丢失。

这些是独立协议泵契约测试源码，不是真实 Core 取消、数据库恢复、客户端可见反馈的通过证据。最终统一阶段还需真实 CLI/Mac 静默 provider、durable cancel 与补充输入竞争、取消后进程结束/未知结果处理，以及高频空轮询 I/O 开销的纵向验收。现有 `../15-观测与验收/MEASUREMENT.md` 的时钟及缺样本规则继续适用。

## HCTRL-03 局部执行失败不能关闭公共控制面

进一步审计发现 `first_admission_error` 同时承担“最终应报告的首个失败”和“是否允许控制轮询”两个职责。一个 effect 的 fencing/续租拒绝就会停止静默轮询，但其他 effect 的有输出路径仍会准入输入：取消与补充能否送达错误地取决于健康 sibling 是否产生输出。

现在将两个状态明确分离：`first_admission_error` 仅保留最终诊断；`scope_error` 才关闭共享控制源。Effect 局部错误仍拒绝该 effect 的迟到帧、停止其续租并仅对它发 abort；健康 sibling 的静默轮询和前后帧准入继续。后来若控制源、Application 准入或 ACK 失败，才升级为全作用域关闭，并保留原始首个失败报告。没有从错误文本识别取消，也没有吞掉失败并让 pump 成功返回。

新增反例源码 `effect_failure_preserves_sibling_control_admission_and_later_scope_failure`：先拒绝一个 effect，再分别在静默、非 delta 前、delta 后准入输入；覆盖成功、准入拒绝、source 失败，检查一次 abort、迟到帧拒绝、已提交 delta 保留、未 ACK 输入保留以及首个错误不被覆盖。测试尚未运行。

Cursor 的[云端 agent 经验](https://cursor.com/blog/cloud-agent-lessons)公开强调 durable execution、按任务拆分工作流/活动，以及 agent、机器和会话存储解耦。本节借鉴的是故障与生命周期分界，不声称复用了 Cursor 私有调度代码，也不为本地客户端引入 Temporal。Nola 的实际故障修复仍由以上源码反例驱动。

### 明确保留的边界

- `HostDriver::dispatch_batch` 仍借用 Host 并在 scoped worker 中运行；`join` 不能安全提前返回或遗弃借用。协作 abort 没有强制终止证据时，不得假写 cancelled 或自动重试未知副作用。要消除不可中断 Host 对整个 pump 的等待，需要独立拥有的执行句柄/进程与 durable attempt 终止证据，不能只加 join 超时或 detach。
- Host emit 必须等待逐帧 Application ACK，单 worker 的响应队列不会无限积压；这并不证明边界输入批量处理、公平性或 publish 回调时间有界。同步 boundary/Application/publish 仍可能阻塞心跳，需要最后统一阶段采样，不能把 100ms 轮询间隔当性能预算已达标。

## HCTRL-04 权限决定：事务结果不是更新行数

原 permission transport 的 `bool` 无法区分相同决定重放、不同决定冲突和请求缺失；CLI 还在确认决定前写 task grant，并在客户端遍历其他请求逐个授权。一次冲突或中途失败可能留下与决定不一致的授权事实。

当前输入仍是原请求 ID、typed authorization/intervention 和时间；没有增加客户端任意 grant 字段。Application 返回 `PermissionResolutionOutcome`：

| type | 含义与附带事实 | 写入行为 |
|---|---|---|
| resolved | 首次决定；receipt 带原 request/task/child/Core ID、实际决定和首次时间 | 同事务写决定及其派生授权 |
| already_resolved | 相同决定的另一传输尝试；原 receipt | 不改时间、不复活撤权 grant、不影响新增等待 |
| conflict | 已有不同决定；返回实际原 receipt | 不覆盖、不补授权 |
| missing | 原请求不存在；原 request ID | 无写入 |

Store 的现有写事务拥有读取、比较和首次写入，不用客户端二次读取推测更新失败原因。相同 command receipt 的重放仍返回原先的 `resolved` 回执；新的 command ID 对相同 permission 返回 `already_resolved`。新提交时间不改变决定等价性。自定义 instruction 沿用 trim 后的存储规范；不同 instruction 属于冲突。receipt 不携带工具 input，Debug 不输出 instruction，CLI 事件只记录请求身份/决定类别。

首次 AllowConversation 的 task grant 由 Application 读取原请求派生，Store 在原事务核对 task 当前 child、scope、grant identity/时间和决定。首次 pending 的 scope 决定缺 grant 必须失败，防止预读缺失与并发首次 enqueue 的竞态。grant、原决定、同 task/child/精确 scope 的已有 pending 决定原子提交；含额外 instruction 的 sibling 不自动放行。CLI 删除直接 grant 写入与逐个授权，仅消费 typed receipt、核对 owner 并唤醒等待者。原 task/child/scope 之外的请求继续正常申请权限。

scope_rule 是 Host `session_permission_rule` 生成的规范 JSON 字符串，Store 只比较原始完全相等，不重新解析命令或猜测范围，不尝试扩展到父路径、相似命令或不同序列化文本。多端统一 typed scope 模型仍应由权限模块整体演进，不在存储层加关键词策略。

`permission_request_port_e2e.rs` 已增加源码反例：scope/child/task 排除、额外 instruction 排除、撤权后冷启动重放不新增 grant/等待结果、缺 grant/时间不一致拒绝、故障触发器验证 grant+sibling+原决定无部分写入，以及同 command ID / 新 command ID 的不同幂等边界。消费者签名及 service 冷启动 fixture 已更新。此批尚未编译或运行测试，不能作为实际通过证据。

## HCTRL-06 Shell 提交不确定性与原任务恢复

### HCTRL-06B：执行预算必须来自原 Task 的持久化调度记录

执行前预算裁决（2026-10-12）：Domain 的 `execution_budget_failure` 是
Application 进度更新和 Host 执行准入的共享判定。墙钟达到上限即耗尽，其他资源
超过上限才耗尽；持久化耗尽标记必须有对应限制和测量证据。配置墙钟限制时，缺失、
负数或晚于观察时刻的任务起点不能视为零消耗。Shell/Agent 必须消费初始进度 CAS
返回的 Task；写入成功不等于可以开始新副作用。恢复监控仅持有原起点计算出的剩余
时间，不再重发完整墙钟预算，未配置墙钟限制也不补宿主默认值。

尚未完成的计量契约：CPU 需要按 attempt 保存累计采样水位，事务中只累加新旧水位
差值；重复采样不得重复计费，旧 attempt 不得写入，新 attempt 不得清空 Task 总量。
内存和进程数为跨 attempt 峰值，不求和。崩溃前未提交 CPU、退出子进程和 PID 重用
不能假定为零。当前进程树采样与 CPU 恢复尚未满足这些条件，不应宣称跨恢复预算已
完整验收。权限等待和重启间隔目前计入墙钟；活跃执行时长若需要应另建显式契约。

#### CPU/资源计量边界审计与目标契约（2026-10-12）

当前实现不能作为完整累计计量，具体证据如下；修复解析器不代表下面的账本已经实现。

| 当前边界 | 实现证据 | 后果 |
|---|---|---|
| Host 采样 | `supervision.rs::process_tree_usage` / Windows snapshot | 原 Unix 解析失败补零；Windows 无查询权限也补零，无法区分未知与真实零 |
| Host 汇总 | `aggregate_process_tree_usage` 加总当前存活 PID，执行循环取 max | 已退出子进程消耗可能丢失；PID 重用无法辨别；这只是采样下界，不是累计 CPU |
| Host 作用域 | `current_process_tree_usage` 以宿主自身 PID 为根 | 不能凭“当前进程”推断独占 Task 资源；进程内 fallback/共享子树可能计入别的工作 |
| Application 写入 | `report_runtime_task_progress` 将收到的 usage 整体覆盖 Task 总量，并拒绝下降 | 新 attempt 从零上报会被当作回退；重复加上旧总量则又可能重复计费 |
| 最终提交 | Shell 已合入原子 finish；Agent 的部分 terminal progress 仍忽略结果 | Shell 所提交观察与终态一致；Agent 晚到计量仍需独立账本，业务完成不等于计量完整 |

所有者划分：OS 适配器仅拥有一次采样事实；Host 拥有经准入的物理资源作用域和样本传输；
Application 唯一拥有计量裁决、幂等与总量变更；Store 原子保存 scope 水位与 Task 聚合。
客户端只投影读模型。禁止客户端或 Host 保存另一份可用于授权的 Task CPU 总量。

目标输入（尚未接线）：`ReportScopeObservation` 必须包含 Task、已准入 scope/producer
观察权限、`sample_sequence`、观察时间、计数器来源及指标各自的
`coverage = complete | lower_bound | unavailable`。执行 attempt 通过独立 binding
segment 关联；不能当作物理计量作用域。进程作用域身份必须包含进程出生
标识/OS job 身份，不能只用可重用的 PID。`unavailable` 携带固定错误类别，不携带
伪造零值；`lower_bound` 不得被 UI 或预算判断描述为完整消耗。任务创建时明确资源预算
是硬限制还是观测阈值；平台无法保证硬限制时必须在准入时拒绝或由用户改策略，不能
静默降级。尚未选择或验证各平台的 OS 累计计量实现。

目标事务与数值契约：

1. 新建 scope/binding 必须校验执行准入；观察提交校验独立观察权限和归属。暂停、
   接管或取消后不恢复旧执行权限；原已授权区间的晚到观察按独立契约补证据。
2. 同一个 `(scope, producer epoch, sequence)` 同内容重放返回原 ACK；不同内容冲突。
   更旧序列不能覆盖当前水位，但允许保存已授权历史证据。相同 CPU 水位可更新峰值。
3. 对同一 scope 的累计水位计算 `delta = new - committed`，Task CPU 只加 delta。
   首次水位必须关联 scope 的正式起点/初始采样；不能把共享进程的生命周期总量当成
   本 attempt 消耗。差值为负是生产者连续性错误，不取 abs、不补零、不自动换 scope。
4. 新 attempt 不必产生新物理 scope：同一 scope 延续水位，新增归属区间；新 OS job
   才使用新 scope。Task 按 scope 增量累计，attempt 边界缺样本则保留不可分摊。
   旧 scope 的未知尾部不能自动结清。非重叠 scope 的内存/进程峰值可取 max；并发
   scope 必须按同一时点聚合，无法对齐就保留上下界/未知。溢出是观察错误，不饱和。
5. scope 水位、Task 聚合、coverage/未结清区间、耗尽事实与事件必须在同一事务提交；
   CAS 失败或故障全部回滚。提交成功后才能返回带新 revision 的 ACK。
6. 真实完整/下界消耗已超限，可以形成测量支持的耗尽事实；低于上限的下界不能证明
   “仍在预算内”。执行准入如何处理未知由原准入策略决定，不能由 Host 随意继续。
   观察不可用不是业务失败、不是用户取消，也不是预算耗尽；保留原 Run，必要时沿
   非业务停止通道停止执行，等待独立恢复证据，不要求模型反复修复计量错误。

退出/恢复：退出确认与最后计量 ACK 是不同证据。不能因为进程已退出就把未提交部分
写成零；OS job 的最终累计不可取回时保留未结清区间。重开只重放已提交水位及证据；
不会重新创建一个相同 PID 的“旧作用域”。业务终态回执可存在，但计量完整性须单独
标注，不能篡改成功结果来补计量，也不能宣称完整资源验收。

验收矩阵至少包含：同序号重放/冲突、同水位不同峰值、过期 fence、接管后的迟到样本、
不同 attempt 的增量累计、PID 重用、退出进程、初始 baseline 缺失、采样失败、溢出、
最终 ACK 失败、提交中断重开。例：scope A 水位 10→17→17 只能累计新增 7；scope B
起点已确认后新增 3，Task 在此前总量上只新增 10。此例是契约算例，不是性能基线。
采样频率、采样开销和退出漏计窗口需真实平台测量后制定预算，当前不写未经测量的数字。

本批实现边界：Unix `ps` 快照现在拒绝格式错误、重复 PID、非法 CPU 时间与溢出，
不再返回伪造零/饱和值；正常的真实零仍可表示。Windows 采样未知、Shell 吞掉采样
错误、独占作用域与上述跨 attempt 账本仍未完成，不能据解析器测试宣称计量闭环。

Shell 原子收尾（2026-10-12）：`FinishDetailedWithEvent.final_observation` 携带最终
输出字节、资源水位及退出证据。Application 复用普通 progress 的单调水位、metadata
保留键及预算计算；Store 收到准备好的新 metadata 与原 metadata/update CAS，核对
同一 attempt lease，在一个事务内替换 metadata、撤销 lease、结束 Task、插入事件和
terminal receipt。任何冲突或存储错误全部回滚。没有最终观察的启动前失败仍可结束，
不是兼容旧版数据的回退。CLI 不再先忽略最终 ReportProgress 的失败再单独完成任务；
最终提交失败保留原始 exit-evidence 文件，不写成功 status 文件。
这只保证**所提交观察与终态的原子性**，不保证 OS 采样完整，也未实现未知提交结果的
持久 observation 重放。物理退出结果与预算观察仍是不同事实，不用计量错误改写业务结果。

Agent 的最终观察不能套用 Shell finish：原 Run 可能已合法 seal 并撤销 attempt lease；
同一 worker 的 usage 还可能跨权限恢复后的多个 attempt。完整字段、权限、状态流与
验收以 [资源归属与晚到观察契约](../15-观测与验收/HARNESS_RESOURCE_ACCOUNTING.md) 为准：通过预先
准入的 scope/producer 观察权限补原区间证据，不凭历史 lease 写当前 Task 总量；不恢复
执行租约、不改 terminal receipt、不重跑模型。未知 ACK 核对原 observation/command，
数据库不可写不能宣称已持久化。此独立账本尚未实现，不能以 Shell 回归替代验收。

启动窗口收紧：Shell 在 spawn 前、Agent 在执行入口前再次提交只观察时间的 progress，
消费 fenced ACK 的取消/预算状态，并检查自初始准入起的单调时间。progress 不要求
伪造输出或状态文本：时间流逝本身可能改变持久化预算，因此空业务字段的观察合法，
但必须通过相同的 attempt CAS。该检查不能使数据库与 OS 启动成为一个原子事务；
检查后的耗尽仍由持续监控终止，不能宣称从未发生副作用。
Shell 启动后的第二次 ACK 也检查预算与取消；观察失败时终止已启动的子进程，保留
原 Task 和输出/执行证据，交精确恢复核对，不再伪造成普通的 command-start failure。
跨 attempt CPU 计量仍未完成，不由本批启动检查测试替代验收。
预算停止的收尾流程：拒绝派发不是完整收尾。初始准入和启动前拒绝后读取正式 scheduling，
只有持久化且测量证据有效的 budget_exhaustion 才进入原 Run 的 cancel-only 通道。
该通道核对 Task/父会话/子会话/Run/assistant 身份，不构造正常 Host、不调模型，使用
稳定 cancellation_id `task-budget-stop:<task_id>`，原因不是“用户取消”。Core 等待
独立副作用的停止确认并封存原 Run；Store 从该 seal 生成唯一 Task receipt，其中 child
Cancelled 与 supervising Task 的 typed budget_failure 分开保留。进程退出码不生成终态。

| 观察 | 恢复流程 | 可否重新派发模型 |
|---|---|---|
| 无持久耗尽标记 | 原正常恢复规则 | 仍须预算与租约准入 |
| 耗尽证据损坏/读取失败 | 保留记录并报告观察失败 | 否 |
| 耗尽且原 Run 未封存 | cancel-only，等待副作用停止 | 否 |
| 耗尽且原 Run 已封存 | 精确核对并重放同一 Task receipt | 否 |
| 耗尽但尚无 Run binding | 不伪造子运行或结果；报告缺绑定 | 否，尚缺未启动 Task 的正式收尾契约 |

调度恢复在普通 claim/retry 之前消费已持久耗尽标记，等待停止不计为模型重试；正在活跃
或归属未知的副作用不会被假定停止。此接线不代表所有平台和真实执行进程均已验收。
等待授权预算观察接线（隔离契约测试通过，验收边界见 VERIFICATION）：等待授权期间若跨过墙钟期限而
没有新 progress，由 Application 对原 Task 做无新执行的时钟观察/CAS，再走上述停止
通道；Host 不得直接补造耗尽标记。
接口为 `ObservePermissionWaitBudget(task_id, suspension_id,
expected_suspension_revision, now_ms)`：调用方不能提交耗尽类型、预算或 usage。
Application 读取原 scheduling/start，使用共享裁决；Store 原子核对 Task 的等待状态、
原 metadata/updated_at/started_at、pending suspension 的 ID/revision，以及原
permission_suspended attempt。只提交预算事实和观察事件，不续租、不生成终态。
已有有效耗尽事实应幂等返回；取消、封存或授权恢复抢先提交时必须报告 CAS 冲突并重读，
不能覆盖其状态或将冲突当作“未耗尽”。授权恢复抢先时，由恢复 worker 的启动准入拒绝
超限；预算观察抢先时，恢复不得抹掉 sticky，原 Run 仍走 cancel-only 收尾。
未耗尽或已有有效耗尽事实的观察不写库、不重复创建事件或归档代次。缺记录、损坏策略、
时间倒退、失效等待身份、CAS 冲突及存储故障均显式失败，不能当作“未耗尽”。当前
投递快照必须同时由 SQL baseline 投影与 Application 增量编码提供一致的 active-turn
状态；不能查询旧 checkpoint 正文假装当前取消状态。缺当前 witness 时拒绝写入。
验收覆盖上述竞争、重复观察和原 Run 身份不变，不能用活跃租约的 progress 测试替代；
无 Run binding 任务的收尾、等待用户输入等其他非执行状态仍是独立未完成分支。
预算监控线程若无法启动，Agent 不得忽略错误后无监控执行，应在模型入口前停止并释放
本次本地执行句柄；这不代表原 Run 已经封存。

续租与恢复的约束：权限恢复消费、启动和 heartbeat 共用同一 active attempt 的串行
续租入口。已有 expiry 足够覆盖请求区间时不缩短、不重复提交 RenewLease；需要延长
时仍通过原 token/revision/expiry CAS，过期绝不续活。授权 continuation 的不可变
grant 身份与随后合法延长的当前 lease 水位不是两份执行权，读取时必须区分。
恢复验收需同时覆盖短前任过期接管、长后继租约消费原授权及父投影不拥有执行权。

预算默认值的唯一应用时机是 Task 创建准入：创建者将有效 scheduling 与 Task 一并提交。
启动和恢复不根据宿主当前默认值重新生成预算，也不把缺记录当作旧版本兼容场景。

| 读取结果 | 执行准入 | 处理 |
|---|---|---|
| Task 存在，metadata 为对象，scheduling 完整且校验通过 | 继续检查原租约/取消和其他前置条件 | 使用原预算和累计资源记录 |
| metadata 对象没有 scheduling | 拒绝执行 | 解析层可返回 None，但执行层不能回退默认预算 |
| Task 不存在 | 拒绝执行 | 不创建替代 Task，不启动 Shell 或模型 |
| JSON/结构/预算校验失败 | 拒绝执行 | 保留原记录供修复，不掩盖为未配置 |
| Store 打开或查询失败 | 拒绝执行 | 返回读取失败，不推断任务失败、完成或取消 |

失败发生在新外部副作用之前，并释放本次进程内执行句柄/续租设施；不伪造业务终态或
重新提交模型。原 Task、Run、receipt 和已经发生的副作用保持原身份。
Shell 本次启动对 cancellation 与 scheduling 使用同一条已读取 Task 记录，避免二次
查询失败被吞掉后误当未取消；这不替代持续运行期间的正式取消/租约检查。
解析 `RuntimeTask::scheduling` 的 metadata 必须与 `set_scheduling` 一样要求 JSON 对象，
null、数组、数字不能被解释成“没有配置”。源码和统一验收结果单列于 VERIFICATION。

Agent 反例必须从真实准入的原 Run 和已存在的 pending Host effect 开始，先确认未走
terminal-only replay，再进入 `run_agent_task`。缺预算或损坏预算下同时检查原 Task/
事件不变、Run 数不变、effect 的请求身份/状态/摘要不变、delivery attempt 数不变以及
本地句柄释放。只检查“没有新 effect”不足：旧 pending effect 也不能被派发。
此证据证明共享持久化和生产 worker 入口边界，不等于真实 provider 或进程 kill 验收。

状态：提交出口正在整改，以下是约束实现的验收契约，不是验证通过记录。

源码证据：`apps/cli/src/task_runtime_parts/execution.rs::queue_background_shell_task`
先持久化任务及 scheduling，再调用 `spawn_shell_task_process`。后者还会打开 Store、
claim attempt、登记租约、启动 worker 及交付请求。整条调用的 `Err` 不能证明任务没有
落库，也不能证明外部命令没有执行。`tools/parts/background.rs` 原先在显式后台
排队出错后删除 stdout/stderr/status，并返回没有 task_id 的普通错误；这会丢失观察
入口及证据。前台排队和本地 registry 失败也必须使用同一结果分类。

### 权威、输入与输出

- Application/Store 的 RuntimeTask、attempt 和 fenced receipt 拥有任务状态；Host 的
  registry 只是进程内索引。registry 缺项、锁失败或 UI 已关闭，均不能修改任务终态。
- 排队前生成的 task_id 必须贯穿提交、观察、取消及恢复；owner 和资源范围仍按既有
  权限契约核验。不能仅凭 task_id 可猜测就允许读取其他会话的输出。
- 未知出口携带原 task_id、观察阶段及 `ExecutionReceipt.state = Unknown`。阶段用于
  诊断失败发生在哪里，不用错误文本决定是否重试。`ToolOutcome.error` 表达本次
  交互失败，不能覆盖 receipt 成为“命令执行失败”的证据。
- 在已知尚未进入持久化/执行路径时，参数或授权拒绝仍是确定的前置拒绝；不要伪造
  “任务已存在”。一旦跨过可能提交的边界而没有确定回执，保留未知性及原身份。

| 观察结果 | 可做的后续操作 | 禁止推断 |
|---|---|---|
| 排队成功，尚未 claim | 观察原任务，由调度器 claim | 立即认定执行完成 |
| 排队/启动返回错误，无可靠提交回执 | 保留身份和输出文件，查询原任务 | 删除证据、换 ID 重发同一副作用 |
| 原任务仍活跃或等待授权 | 显示对应状态；授权或取消指向原任务 | 把等待授权当执行失败重试 |
| 取消提交已接受，无停止证据 | 继续观察原任务 | 直接显示已取消或后台转移成功 |
| 观察失败或原任务查询缺失 | 保留未知；进一步核对提交与 attempt 证据 | 单凭缺失认定从未执行 |
| fenced terminal receipt 已提交 | 展示确定结果及其证据 | 将迟到的旧 attempt 帧覆盖终态 |

这里不承诺任意 Shell 副作用的 exactly-once。恢复不得按命令文本去重，也不得让模型
靠措辞猜是否重发；重复 effect 的处理由稳定 command/effect/attempt 身份与持久化
准入契约约束。新发起的不同 effect 是否属于同一业务操作，不能由 Host 的字符串比较
补造。任何自动重试都必须先具备原 effect 可安全重放的证据。

### 纵向验收与测量

最终统一验收注入以下窗口：持久化之前失败、提交之后/claim 之前失败、claim 后启动
失败、worker 已启动但父进程未获 ACK、本地 registry 登记失败、取消 ACK 丢失和
重启后首次查询失败。每例同时核对客户端状态、Core receipt、原 task/attempt、输出
文件以及实际副作用计数；“工具返回 Unknown”这个单元断言本身不足以证明没有重复。
随后在同一会话恢复并观察原任务，检查没有仅因传输失败产生第二个 task/worker。

性能记录分别覆盖 submit→durable ACK、ACK→claim、claim→worker 首个确认、
观察恢复→canonical receipt，以及 receipt→客户端可见更新。分母区分冷启动、
正常排队、容量等待、授权等待和故障恢复；用户授权耗时不混入 Host 开销。
缺少阶段样本标为缺失，不以零代替；按 `../15-观测与验收/MEASUREMENT.md` 建立真实基线后再定预算。

## HCTRL-08：取消是固定身份的命令，不是临时拼装的提示

iOS 生产取消曾只发送 `interrupt` 类型和 reason，遗漏 Core 必填的 cancellation_id，
造成没有 durable receipt 的外层拒绝；客户端正确保留 unknown，但用户无法完成取消。
修复方向是一个 Run 拥有一个不可变取消操作：conversation/Run/assistant owner、
invocation digest、cancellation ID、transition ID、操作时间及原因一起冻结。重试发送
原命令，由 pending journal 保留原 transport envelope；冷启动查询原回执，不换 ID
重建请求。取消结果仍需 Core seal 或执行证据，不将提交 ACK 当成已停止。

操作时间与恢复派发时间不同：前者属于持久命令，不能因重试改变；后者是本次领取
Host effect 的时间，不能复用数分钟前的时间来生成已过期租约。新派发时间不早于
原操作时间，且只能领取 canonical outbox 已准入的取消工作，不重新启动模型。
相关 iOS 定向回归已在第七轮通过，包括实际模型开始后取消及冷启动 pending envelope
精确重放；不声称已覆盖任意不可中断 Host。完整验证范围见 VERIFICATION。

## HCTRL-07：交互可见性与可接收性必须一致

权限决定的 canonical owner 不因 UI 切换而变化，但客户端用于选择可见交互的
conversation owner 也必须参与变化观察。先用不可观察的 owner 提前返回，会让视图
从未订阅真正的 request；后台已进入等待授权，前台却没有按钮。Mac 已发现并修复
这一观察缺口，不能仅用直接读取 model 的 nonce 断言代替真实页面出现和点击验收。

iOS 暴露权限卡片之前必须先安装响应 continuation／handler；不能先 publish 可点击
状态、下一异步步骤才准备接收。同步状态回调立即确认也是合法输入，不得丢失。
决定仍携带原会话、原请求和 nonce；可观察或可点击不提供额外授权。

排队消息被提升为新 Run 后，客户端接受新 Run 的正式 admission 和 journal 身份。
旧 Run 的 seal 保持不变，不能沿用其远程提交回执。历史重新加载和持续订阅是两种
独立验收：前者证明落库可读，后者须在不手动 reload 的情况下通过生产投影订阅观察
新 Run。Mac 两条回归均已落源码；本批尚未复验，不以读回通过推断实时 UI 正常。

冷恢复诊断只补失败阶段，不改变状态机：记录稳定阶段和不含自由文本的错误指纹，
保留原始错误返回；不得把未知 String 错误按关键词分类为可重试，或因唤醒/通知失败
重新执行已封存任务。测试隔离子进程保留有界原始原因，防止只剩退出码 2 而无法
判断失败发生在终局提交、回执读取、父任务通知还是 worker 准入。
## HCTRL-10：无进展恢复与用户等待分离

本节“当前响应处理”记录现有实现，不再作为最终恢复策略。2026-10-13 真实任务
审计证明其缺少不同策略出口；整改目标以下文 HCTRL-11 为准，未接线前不得声称满足。

状态：部分生产接线、Core/Host 模块验证通过；完整恢复预算与真实纵向验收未完成。
历史失败样本与逐次命令见 AUDIT（本地工作记录，不随仓库分发） 和 VERIFICATION（本地工作记录，不随仓库分发），
它们不是当前状态定义。真实模型重复调用仍未证明已解决。

### 所有者与禁止事项

Core 拥有无进展判定、恢复控制和结果决策；Application 原子提交其事件、checkpoint
及 outbox；Host 只执行已准入 effect；客户端只投影。无进展不是权限拒绝、用户
问题或执行失败。禁止 Host/客户端自行清零恢复状态、重试模型或合成用户提问；
模型文字也不能授权重试未知副作用。

当前触发源为结构化重复 batch 或连续无进展纠错轮次，不按自然语言关键词路由。
恢复状态不取代工具执行回执；被拒绝的再次调用没有执行成功或失败的外部事实。

### HCTRL-10 历史实现与输入输出

以下记录 schema 19 阶段，当前 schema 20 已由 HCTRL-11 的部分实现替代：合法替代工具不再整体剥夺，增加独立冻结恢复配额、拒绝调用指纹及新观察来源。尚缺版本化 mutation/validation 见证与完整统一恢复决定，不能视为 HCTRL-11 已全面完成。当前实现与验收状态见 链路审计（本地工作记录，不随仓库分发）。

`AgentEngineeringLoopState.no_progress_recovery` 是唯一恢复控制字段，必填，
使用 `NoProgressRecoveryState`：

| 状态 | 数据与条件 | 允许含义 |
|---|---|---|
| inactive | 不含原因 | 没有活动的无进展恢复控制 |
| pending | 必须有非空白 reason | 需要处理无进展，不表示用户欠缺决定 |

Core checkpoint 仅接受 `AGENT_CHECKPOINT_SCHEMA_VERSION` 定义的当前版本，必填无进展状态、许可复用来源与控制拒绝事实。缺字段、缺原因解码失败；
空白原因在 checkpoint 恢复验证中失败。无旧 bool/可选原因别名、默认补齐或
迁移。用户问题另存于 canonical UserInputWait，权限另存于 permission 状态，
不能从 pending 推导任一种等待。

生产入口：`turn.rs::require_no_progress_recovery` 记录触发；
`model_planner.rs` 读取同一状态生成受限模型请求；
`submission.rs` 接受真实模型响应；
`requirements.rs::conclude_no_progress` 共用未完成收尾。
响应的 provider/delegated context usage 在提前退出之前记录；这不代替完整
physical-attempt 持久账本，也不证明所有协议的缓存计量已覆盖。

### 当前响应处理契约

| 恢复响应 | 当前处理 |
|---|---|
| 非空直接回答 | 走普通正文合成及原结果校验；不合成提问，不自动变成 action_completed |
| 单个 requirements_update | 原控制事务校验；有效结果走原完成流程；拒绝或无结果则 no_progress 收尾 |
| 单个 request_user_input | 先经 Domain QuestionSet 解码及 validate，再走原提问准入与持久等待 |
| 非法问题、空输出、再次普通工具调用 | 不派发，提出 incomplete/execution.no_progress，并交统一完成仲裁 |

Core 不再存在合成 `loop_checkpoint_protocol_call` 的实现。
问题的 schema 有效不代表语义上确实需要用户；后者仍须真实模型行为验收。
`complete_turn/decide_requirement_result` 保留现有取消、权限等待、未知执行结果、
未清执行义务和负面评估的优先级；no_progress 不能把它们伪装为成功。
并发新输入可能要求继续处理，不能用上述收尾丢弃已经接受的新输入。

### 完整目标流程与尚未实现部分

当前 pending 不是完整恢复预算记录。最终必须在现有 Core checkpoint 中绑定
recovery_id、触发 turn/step、结构化 batch 指纹、来源 call/result 引用、恢复机会
的消费见证及退出结果。Application 以同一事务提交机会消费与模型 effect；
冷恢复只重放同一准入，不能重新获得一次免费恢复机会。

```text
正常执行 → 无进展事实 → 待恢复
待恢复 → 原子消费恢复机会并准入模型 → 恢复中
恢复中 → 足够证据的回答 / 明确未完成 / 真实用户问题
恢复中 → 实质不同的合法方案 → 执行并取得进展见证
恢复中 → 无合法出口或机会耗尽 → 未完成
```

实质不同方案的重新准入、独立 recovery_id/消费记录及进展见证尚未接线；
当前 planner 仍限制外部工具，因此不声称上图所有分支已经实现。
更换参数或成功状态码本身不算进展，须关联实际内容/资源版本变化或已完成要求；
重复同一证据不得清零预算。进展见证必须定义来源、失效条件及原子提交时机，
不能另设 Host 私有计数。恢复配额不是整个任务固定轮数上限。

### 验收边界

已执行模块回归：Core 库 486、Host 契约 142 通过，
日志 `<evidence-dir>/nola-typed-recovery-state.log`。覆盖 enum 往返、必填字段、原因验证、
冷恢复后的重复/空/无效控制/非法问题拒绝、直接回答不强制提问，以及四类拒绝
分支的 context input usage 精确增量。显式用户回答后的继续路径仍有回归。

尚缺：独立预算的原子消费/重放/竞争、不同方案及跨 checkpoint 进展、截断续写
与新输入组合、真实数据库 seal/usage 与客户端可见终态的同次构建证据。
必须覆盖多语言自然续问、失败修改、未知副作用、授权等待、取消和恢复；
模块通过不能代替这些纵向验收，未覆盖前本节保持部分完成。

## HCTRL-11：有界策略恢复契约（部分接线，非完整验收）

依据：链路审计（本地工作记录，不随仓库分发）。不添加另一个 Host loop。
Core 的恢复状态拥有者替换 HCTRL-10 pending 的单次强制结束行为，Application
继续原子提交 checkpoint、预算消费和 effect；不建立平行权限或完成判定器。

### 输入与输出

恢复输入必须绑定当前 Run/Turn、触发 Step、失败 call/result 引用、失败类别、
规范化调用指纹、观察到的资源版本/输出引用及剩余原任务预算。来自 Host 的
观察须有已提交回执；模型关于“换了策略”“已经修好”的文字不是状态转移证据。
失败类别至少区分协议拒绝（未执行）、已知执行失败、重复且无新观察、未知副作用；
未知副作用先对账，不能直接准入重复写入。

2026-10-13 的进一步输入审计发现：当前 `engineering_failure_summary` 将结果
字段压成最多 1200 字符的展示文本，`recoverable_protocol_failure` 和
`recovery_tool_for_failure` 再从该文本解析 `error`/`next_action` 并选择工具。
`ToolResult` 已区分业务 data、presentation 和 execution，却没有单独的 typed
恢复失败字段。这条链仍不符合上面的输入契约：展示截断可能丢机器字段，普通
工具内容也不应成为强制切换工具的权威。不能用增加字符串模式表修复。

整改接口必须由原调用准入/执行所有者提交失败事实（code、是否已执行、原
call/effect 身份、具体参数路径或容量单位）；业务文本与 presentation 原样保留
为诊断数据，不能反向提升为控制指令。Core 从该事实产生恢复决定，提示模型
修正或重规划，不把 `write_file` 的完整替换强制改为 `append_file`。若 Host
不能证明未执行，只能记录未知结果并走对账。此 typed 接线尚未完成。

恢复输出是下面的单一决定，不得同时设置相互竞争的阶段标志：

| 决定 | 前置条件 | 后续 |
| --- | --- | --- |
| RetryCorrectedCall | 原调用确定未执行，修正后 schema 合法，调用仍在现有能力/权限范围 | 原准入流程，不给补救动作豁免权限 |
| ReplanWithinBudget | 重复或失败已知，存在剩余任务/恢复预算，无待对账副作用 | 暴露原合法工具集，给出失败事实；只拒绝命中的重复调用，不封死全部调查工具 |
| ReconcileEffect | 外部结果未知 | 先观察/查询原 effect，不能把重试当作首次执行 |
| WaitForDecision | 模型提出合法问题且当前确有需要交给用户的决定 | canonical UserInputWait；循环本身不是用户决定 |
| PermissionPending | 具体新调用缺授权 | 原权限流程；恢复状态不代替 grant |
| FinishIncomplete | 无剩余恢复机会或合法行动，任务仍未满足 | 经统一终态仲裁封存 Incomplete，保留未完成要求和证据 |

恢复模型请求消费一个明确配置的恢复机会，也消费普通任务预算；剩余额度不能由
Host 或每次新模型采样重新创建。额度是产品执行策略输入，不是从题名、语言或
失败词语推导的常数。机会在模型 effect 准入同一事务中消费；重复投递不重复扣除，
冷恢复不补回额度。没有新增配额的证据，不自动扩张整个任务预算。

### 运行时停止的来源事实

v4 后已实现 RecoveryStopped 三类原因及 Runtime 来源，Pending 账本事件保留
Core 前沿，Store 严格绑定同 transition 停止事实与 Incomplete 结果。
Application 冷恢复/重复投递套件 7/7、来源前沿 4/4、Store 停止负例 2/2
已通过；预算 0/3 和最后工具成功/失败是实际落库路径。逐消费序号到独立
请求 ID 的证明、另外两类原因的 Application 正向矩阵仍未完成；完整证据
见 HARNESS_PROMPT_CHAIN_AUDIT.md，不能由这些测试宣称全部验收门槛已满足。

harness-v4 的真实工具完成路径暴露了来源错误：恢复额度耗尽时
`conclude_no_progress` 只设置 proposed Incomplete，没有提交运行时原因事实，
`decide_requirement_result` 因而复用上一次 model_source。Store 正确拒绝把
当前 tool_completed 事务当作那个 model_completed 事务，留下无 seal 的 Run。

必须为运行时预算/策略停止建立明确事实：预算种类、冻结额度、已消费见证、
触发 Turn/Step 和来源调用，随同一 Application 事务原子提交。结果的来源是
引用该事实的 Runtime 决定，不是模型决定。预算耗尽仍是 Incomplete，不能为了
复用现有 Fact 类型伪装为 Error/Failed 或 Blocker/Blocked；已有取消、真实失败、
未知副作用和权限等待的仲裁优先级保持不变。

验收必须穿过 Application → Store：最后一次恢复模型请求产生工具、工具完成
后额度耗尽、额度为零、checkpoint 冷恢复、同一 Host 响应重放、取消竞态。
检查事实先于决定、来源匹配原事务、恰好一次 seal；另有伪造旧模型来源的反例
继续被 Store 拒绝。仅 reducer 返回 Incomplete 不是这条链的验收证据。

### 进展与退出

不同参数仅说明允许评估一个新候选，不证明进展。进展见证可以是新的相关观察、
验证排除了一个具体假设、资源版本发生预期变化或某个明确要求获得满足证据。
每个见证引用其来源回执和被影响的要求/资源；不能因为任意成功 exit code、
工具名字变化、无关临时文件写入而清除恢复原因或验证债务。

重复相同数据不重置配额；读取新位置/新版本不能被“必须先修改”规则统统拒绝。
恢复后的完成仍走统一结果判定：已修正一个协议错误不代表整个任务完成。
新用户输入按原 FIFO 水位消费，取消优先于派发；不得为恢复而丢弃新约束。

### Prompt 与请求体

固定政策只说明行动/证据/权限边界，不要求每个任务都走同一套 SOP 台账。
动态恢复帧携带结构化失败事实、已拒绝指纹、允许的恢复决定和真实剩余预算。
移除 `request_user_decision` 与“重复不需要提问”的矛盾建议。provider 必须保持
动态帧的语义与相对位置，不能把它并入缓存稳定政策。按最终 wire 验证这一点。

### 必须通过的纵向验收

- 已拒绝的相同副作用不执行；不同且合法的调查/修改可被 Core 准入并取得回执。
- 同一失败经 checkpoint 序列化恢复后，原因、预算消费、调用身份和证据保持一致。
- 只读任务通过新的证据继续；无需为恢复制造 mutation 或用户问题。
- 暂停授权不耗费模型恢复重试；批准只恢复原调用，不清除其他未完成要求。
- 未知副作用、并发新输入、SIGINT 与模型结果同时到达时没有双执行或虚假成功。
- 最终数据库 seal、公开状态及 UI/CLI 展示对应同一个 Run 结果；原始模型文本不能覆盖它。
- 自然追问、多语言、未知工具恢复和真实模型四题测试；不得加入题目专属提示或答案。

当前仅为整改契约，尚未替换生产状态或通过以上验收。

`<evidence-dir>` 表示本地验收产物根目录；后面的路径相对该目录，产物不随公开仓库分发。
