> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 客户端、传输和投影契约

状态：关键接入点已核对，五端实际交互及全部旁路尚未验收。对应 MOD-03/13/14、E2E-06/08/12。
本文件约束数据/控制边界，不声称完成视觉设计、可访问性或帧性能审查。

## 当前接线

| 客户端 | 已核对入口 | 所有权与缺口 |
|---|---|---|
| Mac | `NolaMac/Services/MacApplicationPort.swift`、`MacCanonicalConversationRepository.swift` | service-backed 命令/查询；窗口读取按 generation 和 eventSequence 核对迟到结果 |
| iOS | `Nola/Services/IOSApplicationRuntime.swift`、`IOSCanonicalAgentApplication.swift` | FFI Application submit、pending request 重发/receipt；outcomePending 不等于明确失败 |
| Android | `android/app/src/main/java/com/nola/data/local/AndroidApplicationRuntime.kt` | nativeApplicationSubmit + ApplicationSubmissionJournal；提交结果分 committed/rejected/conflict/pending |
| CLI | `cli/src/application_port.rs` | Service 与 InProcess 两种传输；本轮统一写命令的 receipt 边界，保留只读快速路径 |
| Web journal | `cli/src/web_parts/assistant_journal.rs` | 请求白名单映射 Application 查询，检查 source/Run 身份；不是任意业务 mutation 的代理 |
| Android 投影 | `android/app/src/main/java/com/nola/domain/projection/CanonicalConversationProjector.kt` | canonical 到展示的纯映射，merge 按会话/消息身份；活动 overlay 仍需核对消费者 |
| Apple 远端终态 | `Nola/Services/IOSAssistantJournalTransport.swift`、`NolaMac/Services/MacAssistantJournalTransport.swift` | 解码同一 terminal seal；status 字符串仍缺结构化业务 outcome，需与新 seal 协议同批更新 |

同一数据库不得因为 Service/FFI/InProcess 传输选择不同而使用不同业务语义。
`canonical_run_terminal` 必须在同一读快照中确认恰好一个已提交封存来源：本地
seal 或 committed 的远端 stage。两者同时存在即使内容相同也报不变量错误，
不能按 SQL 返回顺序选赢家。远端还须 manifest_complete；未提交 stage 不算来源。
缺来源、来源歧义和清单未完成均不得伪装成成功或普通“尚未结束”。
来源校验复用 journal 所有者入口：本地必须属于当前 Store epoch 且不是远端 Run；
远端必须匹配已保存的 Host/Run/root/incarnation 绑定，committed、manifest_complete
且 next_entry 等于封存 entry_count。seal 的自校验摘要不证明来源身份，也不能
代替以上读取校验。所有核对在同一个读快照中执行，不引入另一套客户端授权规则。
不同进程形态本身不是重复真相；关键是都通过 Application 的命令、事务、结果和投影。
上表仅证明入口存在及已读路径，不证明客户端所有页面/后台服务都无旁路；自动化反例见 A10。

## CLIENT-01：发送与重发

用户发送先生成稳定 command/submission identity，落入客户端待提交队列；草稿是展示状态。
Application ACK 表示已提交准入事实，不代表模型任务完成。ACK 丢失时用原身份查询/重发，
不能再次生成用户消息、Run 或 Assistant identity。
明确 Rejected 可修正后提交新命令；OutcomePending 必须保留原身份及待核对状态。
本地发送队列不得保存明文 credential，不通过重放用户文字重新推测模型/权限/路由配置。

读写分类由共享 ApplicationCommand 提供，不在五端分别维护命令名称白名单。
写命令统一使用 received-command 回执事务；只读查询可直接走一致性 snapshot，避免轮询产生持久写放大。
会产生租约/快照句柄等内部状态的操作必须由共享层明确分类，客户端不能根据名称中的 get/load 自判只读。
命令参数、client/request identity 与 receipt 必须绑定；同键不同载荷返回冲突，禁止悄悄改写旧结果。
这个规则同样约束客户端组合函数：计数/列表不能先偷偷启动 supervisor 再发只读查询。
恢复、队列推进由显式生命周期入口及独立 supervisor 驱动；刷新 UI 既不创建执行权，也不应成为
任务继续运行的必要条件。测试应分别断言纯查询没有新 attempt/process、显式恢复能推进已准入队列。

### CLIENT-01D：诊断不能替代运行状态

运行生命周期、单次工具结果、任务完成结果与验证诊断是四个不同投影。
客户端可以按稳定身份缓存展示，不得将其中任一个投影写回其它状态的所有者。
外部适配器的附带 validation metadata 必须保留在原事件 payload；不得因此把
tool_finished、permission_request、done/error/cancelled 改成 validation.updated。
独立诊断使用明确的 validation_updated 输入事件；共享 reducer 更新诊断字段，
不据该事件改写运行 status。订阅者必须保留原事件所携带的结束和等待事实。
共享 trace reducer 的运行 status 只采用 Run 生命周期及非终态 phase；单个工具、
hook、usage、记忆或诊断事件的 success/failure 不能决定它。工具权限等待保存在
对应 activity，不从单个等待/批准推导整个并行 Run 已挂起/恢复。观察到 Run 终态后，
晚到遥测可以继续补充独立诊断和 usage，但不能改写终态 status、reason 或 phase。
这是只读展示归约，不代替 Store 的正式 seal，也不授予执行或恢复权限。
Core TurnCompleted 的 trace 映射必须保留 task outcome：answered/action_completed
才显示 Succeeded；needs_input 显示 Waiting，blocked/incomplete 显示 Incomplete，
并保留各自 reason；没有 outcome 显示 Incomplete/outcome_missing，不补造完成证据。
RunCompleted 的生命周期边界与上述展示 status 正交：需要输入或未完成的执行段
同样可以已经结束，之后的遥测不得令其继续运行。等待的具体输入/授权对象仍必须
来自 canonical challenge，不能从 Waiting 文案自动创建。
生产 CLI Presenter 向 Emitter 传递同一 Domain outcome 的完整字符串，不能把
blocked/incomplete/answered/action_completed 合并为 completed。JSON 终态仍保留原
Core envelope；不另造“任务成功”事件。缺 outcome 的诊断名为 outcome_missing。
结果不是权限决定或用户暂停指令，不能据这些名称擅自批准、暂停 Goal 或重试副作用。
共享 `AgentTraceReducer` 在首条已接受事件上绑定 conversation/run/trace 身份；
之后任一不匹配返回 IdentityMismatch，在 event ID 去重、游标推进、字段写入之前
拒绝。拒绝不能占用序号或污染去重集；新 Run 使用新 reducer，不隐式切换所有者。
Turn 字段不作为此绑定键，避免把合法的运行内 Turn 更新当成跨 Run 输入。

**当前传输身份与客户端接线约束：** `AgentTraceContext::begin_turn` 将 trace 的
`run_id` 设置为 Core Turn ID，并从 Turn 派生 trace ID；它不等于 SQLite 中的
canonical Run ID。生产 Emitter 通过外层 `canonical_run_id` 单独携带后者。
客户端不得用 trace.run_id == canonical_run_id 作为准入条件，也不得将 trace 的
completed 当作持久 Run seal。消费作用域由已准入的 conversation、执行 generation、
canonical Run 绑定确定；其内每条 trace 用自己的 conversation/run/trace 三键
及 cursor 去重。恢复只能由正式绑定恢复该作用域，不能从到达事件猜出新运行。

Mac 接线目标为：先核对调用上下文及传输绑定，再执行 trace 准入，然后才能更新
usage、权限挑战展示、浏览器 snapshot、消息诊断及调用方结果标记。被拒绝事件
不应创建消息、推进权限序号、改缓存统计、清除挑战或把 succeeded 改为 false。
无 trace 的 canonical journal、seal、usage 等消息继续使用各自已提交来源契约，
不能强塞入 trace 去重。合法后续 Turn 应新建独立 trace reducer；不重置旧流以
接受迟到事件，也不因一个 Turn 完成而丢掉整个 canonical Run 的恢复信息。
需以真实 consumeTracedAgentEvent 入口验证这些副作用，而不只测试 reducer 返回值。
当前 Mac 已将 conversation/generation 检查及 reducer 准入移至消费入口，拒绝
先于 traced 分支副作用，并在局部值上计算、随后发布非终态 presentation。
Mac 单流 reducer 现在固定 conversation/run/trace 三键，不能重置成另一条流。
`MacConversationTraceReducer` 在已准入会话执行内管理展示流切换：候选流通过
归约准入后才替换当前流，旧 trace ID 退役；其迟到事件返回 retiredTrace，不
更新当前投影。新提交创建新的会话流管理值，恢复保留同一执行作用域。此处的
“退役”只停止旧流影响当前展示，不产生业务终态、不删除已提交 journal，也不
代替 Host 身份认证。真实断线恢复和并行来源仍需纵向验收，不能单凭新 trace ID
推断 canonical Run 已改变。Mac 对解码后 envelope 使用 sortedKeys JSON
的 SHA-256 摘要区分 Duplicate/EventConflict；这不是原网络字节比较，不覆盖
解码时丢弃的未知字段，也不是签名。序号不合法的新 trace 不得先重置当前流。
完整恢复/副作用验收及资源有界性仍未完成，不能据此宣称客户端边界已完成。

批量 `reduce_agent_trace` 返回 Result，gap/乱序/身份错误不得被静默丢弃并包装成
成功的部分投影。该辅助归约接口不证明五端自有订阅实现已采用同一校验。
准入只接受当前 AGENT_TRACE_SCHEMA_VERSION，不猜测旧版。相同 event ID 与相同
序列化载荷摘要才是 Duplicate；载荷改变返回 EventConflict。摘要是去重比较值，
不是来源认证或执行权限。只保留每事件固定长度摘要，不复制完整正文。
终态后新的 RunStarted/RunCompleted/RunFailed/RunCancelled 返回
TerminalAlreadyObserved；原终态的完全重放仍是 Duplicate。所有拒绝均发生在
状态、去重集与游标写入前，不阻止同序号合法遥测随后到达。

| 输入事实 | 展示含义 | 禁止推导 |
|---|---|---|
| Run 的 canonical failure/seal | 该次运行失败或已封存的结果 | 从诊断文字反推同样的 seal |
| 工具 outcome=failed | 该工具尝试失败 | 整个任务失败、允许重跑副作用 |
| validation=incomplete | 验证尚未完成 | 执行失败、正在等待用户授权 |
| validation=blocked | 验证受阻的诊断 | Run 已受阻封存、自动批准或重试 |
| validation=evidence_insufficient | 无法用现有范围/版本证据核对效果 | 再执行一次修改、把未知版本当通配 |
| validation=not_required | 当前没有验证债务 | 验证已成功、用户需求全部完成 |

Mac 当前实现为每条 assistant presentation 的 `validation-diagnostic` 独立条目，
不复用 `run-status`。canonical 内容刷新不得无条件丢弃该诊断；同一有序流的后续
required/not_required/succeeded/completed 只清除它，不清除运行错误。运行结束也
不能把未完成诊断改成成功。重复事件依赖原 trace cursor 去重，不发出新执行命令。
展示态 incomplete 的“已定型”仅表示不再播放进行中动画，不代表 Run 已终态。
这些规则同样适用于 Core Parts 拥有正文的模式；关闭正文 trace 投影不应关闭诊断。

验收须覆盖正常诊断变更、诊断后 Run 失败、Run 失败后诊断、canonical 刷新、
重复事件和冷恢复。当前 Mac reducer 回归不等于重启恢复或五端交互验收；历史诊断
若没有 canonical 查询/已提交事件来源，不得以本地缓存提升为持久事实。

### CLIENT-01R：有风险恢复决定的持久提交

Android 实施：`ApplicationSubmissionJournal.transaction` 在同路径进程锁和文件锁下重新
读取原状态，涵盖 Runtime 的准备、发送/查询、校验、退休；不再长期持有可覆盖其它
实例的内存文件副本。实际编码 envelope 限 16 MiB，单条 journal 的重复 key/client 开销
单独预留；原 ID、命令或协议改变不得复用旧意图。临时私有文件同步后原子替换并同步
目录，写确认失败使当前实例停止继续提交。重建实例重新同步、读取可见状态，不承诺
外部删除/磁盘损坏可自动恢复。新增旧实例覆盖、交易穿插、损坏文件及坏回执反例源码，
尚未运行；Android 文件系统权限与目录同步能力仍须在真实设备统一验收。
`ApplicationCommandEffect` 是共享协议只读分类的保守子集（另含 service 的
command_receipt 查询），不按名称前缀识别。恢复、会话投影与明确模型/能力读取不进入
未确认写入队列；未分类命令仍走持久写路径。新增原生 JNI 隔离库测试源码：先保存
待确认 A08 意图，重开 Runtime 后仍可读原设置且意图不变，新写必须拒绝。这个样本只
验证观察与写入排序，不伪称已验证未知副作用的实际重试；当前未执行。

Mac 实施接口：`MacUnknownEffectRecoveryControl` 通过现有 ApplicationServiceClient 执行
inspect/submit/recoverPending。每个 exact Run 使用独立 SHA256 文件名，持久原 command bytes、
request/resolution/time、assistant、store/client/protocol；同 Run 未确认时拒绝后续新决定，
其它 Run 不受影响。私有目录 0700、文件及跨实例锁 0600，拒绝符号链接；临时文件 fsync 后
原子 rename，再同步目录。最多 256 个 target 的完整协议输入由 8 MiB command / 12 MiB
base64 journal 边界承载，不用任意小上限截断合法决定。
读取同时限制实际读取字节数，不能只核对读取前的文件大小；文件在检查后增长也不得
触发无界读取。超限或损坏记录保留，阻止同 Run 新决定，不能把无法读取解释为没有待确认意图。

重启只按原 Run 读取原意图，先查原 receipt。null 不表示未执行；仅借 Application 幂等
契约重发完全相同的 requestID 和 command bytes。所有 transport 抛错（包括本地 ABI gate
可能抛出的 rejected）保留意图，不能凭异常名字判断业务拒绝。明确回执归属或结果身份不符
亦保留并报错。文件提交/退休失败令当前控制器不可继续新决定，不能暗中生成替代授权。
该文件仅记录待确认传输意图，返回 RetryAuthorized 不开始执行，返回受阻决定不代替读取 seal。
Mac 检查发现 Run 已终态时，仍核对该 Run 的原待确认回执，再消费 canonical seal。
此路径使用仅查询模式：回执缺失保留意图并报告待确认，绝不重发决定或触发 resume；
不能因为业务已封存便跳过传输意图的确认。新增 blocked 回执丢失后只查询退休及
缺失回执不重发两项测试源码，尚未运行。
源测试覆盖丢回复后只查回执、无回执同 payload 重发、本地 rejected 保留、多 Run 隔离、
跨 client 拒绝、持久化先于发送、协议最大 target 数，以及超限记录阻止恢复和替代提交；
尚未运行测试，UI/生命周期接线另验。超限 fixture 覆盖静态超限，不声称已模拟读取期间增长。

待提交记录只是用户已经确认的命令意图，不是第二份 effect ledger。唯一业务事实仍由
Application 的 resolution receipt、attempt 和最终 seal 确认。记录必须绑定实际执行端、
数据库/客户端作用域、conversation/Run/assistant、完整命令和固定 request/resolution 身份。
用户确认后冻结原时间和 fencing；不能在网络重试时重新 inspect 并替换载荷。

提交顺序：只读 inspection → 明确用户决定 → 原子保存待提交意图 → 发送原命令 →
核验同身份终局回执 → 清除待提交意图 → 读取 canonical 投影。持久化失败不得发送。
发送前退出可重新发送原意图；发送后退出先核对原回执，无回执不能解释为未执行。
需要重发时必须仍用原身份与原 payload，交由 Application 幂等边界决定。取消界面等待
不撤销已确认决定；真正的用户撤销需要另有业务命令，不能直接删文件假装从未提交。

只在明确 committed/rejected/conflict 且回执归属验证通过后退休意图；损坏、权限失败、
协议不符、transport pending 都保留并报告。清理本地文件失败也不得改发新决定。
同一 Run 有未确认恢复决定时阻止新决定超越它，不无故阻塞其它会话的普通查询。
原 inspection 的 lease 字段不展示、不记日志；本地意图按私有文件权限保存，不含凭据，
也不自动同步到另一设备以取得那边的执行权。

界面最终状态只消费 canonical 结果：RetryAuthorized 只说明已授权，尚未执行；
UnknownPreservedBlocked 需读取同 Run seal 才显示受阻结束。用户理由、HTTP 200、文件保存
成功或重启后的 UI 缓存都不能显示“操作成功”。远端 Run 的决定必须交给实际执行 Host，
不能向只存正文镜像的本地 Store 发命令。当前 Mac 专用意图接线正在实施，跨端用户出口
和故障验收未完成；本契约不是现有五端均已满足的声明。

### CLIENT-01U：未知结果的用户交互

Mac 的检查/提交中状态按 operation token 关联原 conversation/Run，而非全局 Bool。
切换会话不显示其它 Run 的忙碌状态；原异步操作结束只删除自己的 token，不能清掉
新会话的在途操作。同 Run 的在途操作跨展示 generation 仍阻止重复提交。新增延迟
回执期间切换会话的 WorkspaceModel 测试源码；尚未运行或进行原生 UI 验收。

这个界面的单一任务是让用户决定如何处理无法确认的副作用，而不是把报错隐藏掉。
状态在原会话内显示；正常查询、启动恢复和切换会话不自动弹警告。选择动作时捕获
原 conversation/Run/assistant 与 inspection，确认返回后再次核对当前 owner；不能把
旧弹窗的按钮解释为新选中会话的操作。

- “授权风险重试…”明确说明原操作可能已经生效，再次执行可能重复写入/发送/收费。
  确认取消是纯展示行为；确认提交仅授权原目标，不宣称已执行或成功。
- “保留未知并结束为受阻…”只在原 Run 没有独立 pending 工作时可提交，仍由共享层
  最终核验；不要为了启用按钮而取消其它操作或把未知改成未执行。
- 待核对回执期间禁用该 Run 的新决定，保留查询/返回能力；提示说明正在核对已提交
  决定而非“执行失败”。不同会话不共享此 pending 状态。
- 不提供“确认成功”按钮，不显示 lease token/config digest。需要定位时显示操作标识，
  模型/工具输出作为不可信内容呈现，不能成为隐藏的恢复命令。

Apple 平台沿用系统文字样式、语义前景和背景色、标准可键盘操作按钮，不新增品牌色板或
反复闪烁动画。风险确认保留 Cancel，避免默认 Return 直接触发风险动作；窄窗口按钮可
纵向排列，风险说明不得单行截断。此处不声称未渲染界面的对比度或命中区域已实测达标。
依据 apple-design 已读取的 `feedback.md › Best practices`（状态贴近相关内容）、
`alerts.md › Best practices / Buttons`（避免启动弹窗、可取消且避免误触默认动作）及
`color.md › Inclusive color / System colors`（语义色和非颜色状态提示）。

## CLIENT-02：订阅、分页和会话切换

Mac composer 的发送/停止、权限模式、思考档位 action 都携原 conversationID；选中会话变化后
旧菜单回调不得改写新会话配置。工作区执行容量占用与所选会话的执行/等待/恢复状态分离，
其它会话运行时当前草稿仍为草稿而不是 Stop。权限卡片不得通过祖先 implicit animation
同时动画 transcript 流式重排；正文身份仍使用稳定 messageID，末尾跟随使用实际 latest anchor。
这些是静态边界修复，尚未取得本轮真实帧耗时或五端交互验收。
设计依据：apple-design `feedback.md` Best practices 的 “integrating status feedback into your interface”
及 `motion.md` Providing feedback 的 “avoid adding motion to UI interactions that occur frequently”。

客户端维护的唯一非持久业务展示缓存由 committed snapshot + cursor 构建。
subscription identity 包含 source、conversation、run、generation；迟到回调先核对身份，再应用 cursor。
旧会话运行中切到新会话时，旧活动状态不能控制新输入框；草稿按会话保存，不与 active Run 共用全局布尔值。
断线、查询错误和运行失败分开：连接断开不得覆盖 canonical RunResult。

分页旧消息和当前增量不能相互覆盖；每条 Message/Part 用稳定 ID 及 revision 更新。
缺口重读 snapshot，不把无序 delta 按到达顺序拼接成事实。正文完成是 seal，不生成第二份历史消息。
收尾包含正文、结果、usage、授权面板变化时应在同一已提交 revision 展示，不出现短暂旧状态“回跳”。
滚动锚点、展开、输入焦点是本地状态；数据到达不能无条件把用户从历史拉到底部。

Mac 目前 refreshIfChanged 有 generation/eventSequence 栅栏，并把查询放在 detached task；
这只能证明这条路径的保护，不能证明实际首帧/滚动不卡顿。
现有最多 100ms 的 coalescing 是实现参数，不是经实测批准的 P06 预算。

## CLIENT-03：终态与远端复制

渲染使用统一 RunResult：等待授权显示等待，证据受限显示受阻，未完成保留剩余事项，运行故障才显示失败。
不得根据自然语言“完成”、工具 exit=0、Host 进程退出或正文为空推导结果。
结构化原因用本地化 key；工具 ID/证据引用不翻译，错误原文只出现在脱敏诊断。

本地和远端终态都验证精确 identity、协议版本、manifest/digest、游标后提交。
复制不是执行：远端消息、grant 描述和 seal 不自动赋予本设备工具权限或新 Run 准入权。
删除 Trace 后 canonical 正文、结果、权限、统计与 Goal/Task 仍可恢复；Trace 只能辅助诊断。
seal 协议升级必须同时更新 Rust、Swift/Kotlin DTO、Web bridge、导出/导入和所有摘要校验。
不能客户端“缺 outcome 就当 completed”来兼容未确定的结果。

## CLIENT-04：准入、内容游标和封存不是同一件事

| 事实 | 输入/查询及输出 | 可据此展示 | 不得推断 |
|---|---|---|---|
| Run/Message 已准入 | admission directory / source submission receipt，含稳定归属 | 占位消息、当前运行身份 | 已有模型文字、已有 journal 记录 |
| 初始读取游标 | 已授权 identity 的 LocalAssistantJournalHead 可返回 sequence=0 | 作为从头读取的 after 参数 | 存在已提交内容 |
| 已提交内容 | journal heads / bounded history heads，sequence>0、digest 与精确 owner | 从相应记录/快照构建正文和部件 | 整个 Run 已完成 |
| 最终结果 | terminal seal + manifest/result 完整验证 | 已封存的 outcome 与不可变完成 frontier | 所有外部副作用已回滚或后续后台状态不再变化 |

Store window_heads 只列真实 head，且 sequence>0；没有内容的已准入 Assistant 可以存在，
其 heads 列表为空。此时 UI 使用 admission 及运行状态，不创建假 journal 记录来凑齐占位。
外部导入消息仅带 run_id 也不等于取得该 Run 的源身份，不得据此制造读取/执行权限。
LocalAssistantJournalHead 的零游标和 LocalRunAssistantJournalHeads 的空列表不矛盾：
前者回答已核验 owner 的读起点，后者发现实际已发布的内容流。

订阅队列是易失通知，不是正文权威；丢通知、进程重启或暂时无 presenter 后，
按 exact conversation/run/source 查询已提交的非零 heads，从消费者 last cursor 补读。
同一 frontier 重复通知可以合并；同一 identity 后续更大 cursor 不能因“已经 seed 过 Run”被丢弃。
队列损坏不能靠 provider trace/最新模型 buffer 补正文。查询错误要保留恢复状态，不当成空列表处理。

对应回归必须先经真实 Application 提交 ModelDelta，确认非零 head，然后清空通知队列/移除订阅；
再核对重订阅输出与已提交 cursor 一致。只创建 Run 后就断言有 journal 记录是无效 fixture。
加密 HTTP/浏览器一致性同时覆盖零内容 admission、非零内容读取、跨会话拒绝和未封存状态。
这些是数据契约测试，不代表已测 Mac 首帧、滚动或远端真实网络延迟。

### 内容接收的提交前置条件

记录头通过 digest 校验只证明操作记录完整，不证明引用的正文已在接收端可读。
接收顺序为：核验 source admission → 固定 source snapshot → 查询本地内容缺口 →
按精确 part/content/chunk 身份读取 → 持久化准备前缀 → 提交完整 record → 发布接收游标。

- `NextAssistantJournalContent` 输出缺口及 `expected_prefix`，不是执行权限；源端只读 HTTP
  不接受 `Stage`/`Receive` 之类本地写命令。
- `StageAssistantJournalContent` 绑定 source、record、part、content 和预期前缀；准备持久化
  不代表正文可发布。每个成功分片必须推进前缀，重复/冲突按协议核对，不能无限循环下载。
- `ReceiveAssistantJournalRecord` 只有在所需内容完整且校验通过后才推进游标。
  `ManifestUnavailable` 是内容前置条件不满足，不能忽略错误、伪造空正文或提前 ACK。
- 在准备后、记录提交前重启，必须能重新查询缺口继续；记录提交后重放不得重复应用。
- snapshot 的释放与业务提交分离，读取失败保留原接收游标；测试应验证成功路径释放，
  异常路径的句柄回收仍需单独覆盖，不能从成功释放推断异常不会泄漏。

HTTP 验收必须真的传输引用的 chunk，重开目标库后读取确切正文；只比较两个端点返回的
记录 JSON，或收到 header 后传空 chunks 并期待成功，都不足以证明正文复制完成。

## iOS 会话执行所有权

`ChatManager` 的界面实例维护 conversation→execution 注册表；每个 execution 实例拥有
独立 generation fence、DeviceAgentCoordinator、HostPermissionSession、Host/Broker、
恢复观察器、journal reader 和 trace 投影。选择变化只重绑界面 publisher，不取消隐藏会话。
同会话仍串行准入，不同会话可同时执行；不能用全局禁止发送代替并发隔离。

模型客户端捕获实际 provider，不共享可被模型选择替换的 ModelService facade。
上下文管理器也按 execution 创建，使用该实例的客户端；共享的是 Application/Store
及数据源服务，不从另一会话的 UI 状态读取运行身份。停止与权限答复路由到选中 execution，
文件选择器在展示时另外捕获原会话，返回时不能授权新选中的会话。

ChatView 按会话保留生成 Task 与未发送草稿，发送闭包捕获 execution；旧回调只有在
conversation 和 assistant 身份同时匹配时才能修改当前画布。异步 usage 查询返回后
也重新检查会话。后台结果仍按原会话持久化，回看从 canonical journal 恢复。

新增回归源码覆盖双 HTTP 请求交错完成、切换后的投影/落库隔离、同名权限请求的
会话隔离及停止只影响当前 execution；原冷启动权限恢复继续覆盖持久准入。
当前未运行这些用例，也未进行真机渲染或并发性能验收；不能将静态接线视为已通过。

## 原生 ABI 的状态所有权边界

原生客户端可调用纯值函数（工具策略评估、上下文准备、协议转换），但纯值结果不是已授权
或已执行事实。持有可变 Core handle 的 new/restore/submit/snapshot 接口不得作为生产客户端
的另一条执行入口：没有 Application 的事务、receipt、outbox 和 Run 绑定，不能 ACK 或发布。
Core reducer 的直接 fixture 应位于 Rust 层；原生纵向 fixture 必须走隔离 Application 数据库。

原生可用性检查须检查实际需要的 ABI：Application 执行依赖 Application ABI/协议，纯策略
依赖策略 ABI，不能用裸 Core v2 的版本号代替两者。未提供的能力显式报不可用，不退回裸 Core。
删除裸写入口时必须同时清理 Swift/Kotlin wrapper、JNI/C header、链接导出和原生测试构造器；
不能仅将调用点改名或把旧接口留作 fallback。纯值接口保留各自所有者，不因共享文件名而误删。

Swift 已移除可变 `AgentCoreV2Runtime` 与无消费者裸 dispatch/recovery envelope。纯上下文函数由
`AgentCoreContextEngine` 核 context ABI 2，纯策略由 `AgentCoreToolPolicyResolver` 核 policy ABI 2；
执行入口可用性及打开数据库由 `IOSApplicationRuntime` 同时核 Application ABI 1 与服务协议 49。
PersonalIntelligence 只核自己的 ABI 1，不借裸 Core 或 Application 的版本号。
原 Swift reducer 权限往返 fixture 已改为隔离 Application 数据库，保留同资源会话授权重用、
不同资源重新询问断言，Host 响应使用真实 claim delivery 身份。跨平台导出收口见 AUDIT A18。
统一验收须包括导出表与依赖门禁、原生真实提交/重开、故障回滚后无重复派发，不能只搜索调用数。

## 验收与计量

- SQLite 验证进程内写入有持久 receipt，重开可读取；只读轮询不增加 command_receipts。
- Service/FFI/InProcess 对同一命令序列具有等价的提交/冲突/重发行为，不能仅编译通过。
- ACK 丢失、重复点击、提交途中退出：只有一个准入结果；客户端重启保留待核对身份。
- 五端执行中切会话、翻历史、返回底部、收尾、断线重连：正文不丢、状态不串、不误报完成。
- 远端乱序、重复、不同 source、被篡改 seal：拒绝非法数据，不覆盖本地事实。
- P06 测实际 commit→投影→首帧，记录窗口大小、流速和帧间隔；纯 reducer 时间不替代渲染时间。

当前新增 SQLite 测试只覆盖第一项子集；剩余项目仍在 VERIFICATION 中保留未完成。
