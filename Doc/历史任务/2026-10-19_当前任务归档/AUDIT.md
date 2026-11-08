> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 当前实现审计与整改账本

本文件是可继续更新的工作记录，不是“架构已完成”声明。所有路径相对于仓库根。

## 2026-10-12：恢复身份错位的已确认调用链（入口整改中，旧记录未修复）

真实旧 Run 90b3270e-6b9d-4b6a-b3ca-67824970cc53 的同一 effect 243：
outbox/Core 已为 request 245，outcome_unknown 的执行尝试仍为 request 244。
这不是摘要内容问题，也不能通过清空未知结果或自动批准绕过。

1. `cli/src/core_agent_runtime_host_controls.rs::prepare_pending_effect_recovery`
   在 restored_after_process_loss 时选中所有 pending effects，而非先查询执行尝试。
2. 提交 ResumeHostEffect；Core `runtime/effect_dispatch.rs::resume_host_effect`
   无条件分配新 request_id，后续 Application 提交 checkpoint/outbox。
3. 未知执行尝试不能自动 claim successor，因而仍绑定原 request。
4. Store `run_recovery_inspection.rs` 按 effect/run/request 三者 JOIN；换号后
   返回 targets=[]、independent_pending_effects=1。
5. 不能只删除 JOIN 的 request 限制：Core ResolveUnknownEffects 同样要求
   decision target 与 pending effect 的 request 相等，否则事务会拒绝。

整改顺序：Application 先读取原执行尝试并分类；unknown 保持原 request，等待
明确决策；可恢复执行只有在新 attempt 与请求重绑定可原子提交时才换号。
CLI 不得仅凭“进程重启”提交全量身份更新。权限面板重新呈现与重放执行分开。
验收需覆盖：重启前后 unknown 身份不变、正式 inspection 可见原 fenced target、
未经决策不执行、授权重试后 successor 与请求一致、旧响应不能污染新尝试。
既有错位记录需走明确的修复事务验证原 effect/attempt 链，禁止直接 SQL 改状态。

入口整改：CLI 不再把 restored_after_process_loss 等同于全量 ResumeHostEffect。
冻结 catalog 为 Recoverable 的流式恢复仍可重绑定；Idempotent 重试保留 request，
RequiresRecoveryDecision 和缺失能力策略保持原身份，避免让正式决策目标失联。
权限呈现维持单独路径。更深的 Application 原子换号事务仍需完善，不能称完成。
尝试统一不换号的回归失败后撤回该方案：Store Resume candidate 明确要求新 request，
Retry candidate 要求原 request，二者不能混用。失败日志保留，不改查询来掩盖差异。

最新成功证据补充：新版本真实 mainline-contract 三轮均 action_completed，
其中第三轮 cached_request_fork 命中 97.78%，压缩后继续修改 README 与跑测试。
它证明热分叉与任务续接，不解决上面的旧 Run 恢复断点；详见 VERIFICATION。

主线摘要超预算恢复已接线：最终提交前一次保前缀完整重写，再失败则用 Host
Failed 正常结算，保留上下文/全部请求费用；CLI 压缩相关 40 测试和编译通过。
旧真实失败会话通过 --resume 重开仍没有恢复：原 compact delivery 已落为
outcome_unknown/requires_recovery_decision，不能由进程 exit 0 推导任务完成。
下一步需审视 context.compact 的恢复契约与原 Run 的明确恢复入口；不伪造旧
执行结果，也不以新会话成功代替这个遗留恢复缺口。证据见 VERIFICATION 开头。

热分叉主线根因确认并修复：真实 alignment-hot 调试记录中，首条未匹配工具
结果与物理请求逐字段比较，唯一差别为 `_nola_artifacts`。普通 Host 的媒体处理
移除该私有字段，归档 wire 投影未移除；Chat normalization 现统一移除，保留
正文/调用身份的严格匹配。4 个 compaction_fork 测试通过，含新增 artifact 回归
及正文变化拒绝，日志 `/tmp/nola-mainline-fork-tests.log`；CLI 编译通过。
新增 opt-in 本地 alignment 诊断，不把源正文加入用户可见 metrics。

真实复测不能算通过：修复前 alignment-hot 又暴露摘要超出计划预算，Core
拒绝 CompactionCompleted，CLI exit 1，原 Run 仍持有未完成工作；修复后的
alignment-fixed 新提交正确被旧 Run 所有权拒绝，并未真正执行修复后的摘要。
下一主线必须处理摘要预算不满足时的可恢复反馈/恢复原 Run，再复测热分叉；
不得删除原 Run 或换空会话掩盖这个真实失败。

主线压缩实测追加：同一发票/CSV 任务历史，阈值 22000，真实 Qwen 压缩并完成
回顾，30.31 秒、answered，最终事实与两次测试失败修正都正确。性能暴露明确
主线问题：archive_not_in_snapshot 导致独立摘要，摘要 13.366 秒、cache read 0，
下一普通请求输入从 26048 增到 27769。下一步优先修复归档范围与物理请求的
映射/压缩收益，不再扩张周边权限设计。原始证据仍在 wAD00E，详见 VERIFICATION。

2026-10-12 主线调整：按用户要求停止继续铺陈边角场景，以真实 LLM 完成任务为
优先。当前 CLI 构建调用用户 LAN Qwen，在隔离项目完成“修复汇总→运行测试”
以及重启同会话“追加 CSV→两次失败后修正→测试通过”；42.33/82.52 秒，最终
6 项测试及独立产物断言通过，两条 action_completed 已落库。详见 VERIFICATION。
不是 Mac UI/全功能验收。此前进行中的 CoreSettled 关闭、settled 挂起状态及
Host 无重复响应接线已编译；新增权限专项测试未运行，不视为通过。

Store settlement 来源校验接入 commit_transition：回执契约从 Core 下沉 Domain，
不让 Store 依赖 Core reducer 或复制授权类型。精简 witness 增加 turn/call ID；
提交时在替换 checkpoint 前核对唯一原 pending effect 和不可变原主体，以及
Core frontier。Application 回归篡改原 call/turn 均返回 Rejected，保持原 pending、
delivery 且无 settlement 残留；正常提交/冷重放仍通过。Core 486、Host 143、
Application 10 通过，CLI lib check 通过。原等待行即时关闭与后继资格仍待接线。

许可 settlement 持久事件已实现：Core 从原 receipt 生成独立 PermissionSettled，
Application 保存原 session/turn/revision/cursor，不加入 Trace 或另造 UI 消息。
批调用回归核对事件与回执逐项相等；Application 故障注入确认 command receipt
提交失败时事件回滚，冷重放不重复追加。Core 486、Host 143、Application 10 通过，
CLI lib 编译检查通过。Store 原等待来源核验、非终态即时关闭及后继资格仍未接线。

许可复用公开投影纠偏：原持久 receipt 已区分来源，但内部 PermissionResolved
仍使用 AllowConversation，导致 Tool Part 继续显示另一条用户批准。现改为
ConversationRuleApplied，Core 批调用回归同时检查两种 Part decision。Core 486、
Host 143 全通过。进一步确认 RuntimeEvent 被 projection 消耗，不是持久 Run
事件；LC-05a 明确禁止由显示 Part/pending 消失反推 settlement。独立 canonical
settlement 仍待实现，不能复用临时事件名称假装 Store 已有关闭依据。

许可复用来源已进入 Core 持久回执：同批复用记录 ConversationRuleApplied 并引用
原批准 request ID，而非伪装第二次用户选择；恢复校验要求唯一更早的明确
AllowConversation 回执。checkpoint 升至 18，不迁移或改用户库。真实 Core 批调用
回归验证两次派发、一条选择、一条复用来源及冷恢复拒绝非法来源；Core 486、
Host 契约 143 全通过。事件/Store settlement 仍待接线，此关联不是独立授权凭证。

LC-05a 后续取消链证据：扩展热等待取消回归到 callback 退出后的生产 supervisor，
已验证 canonical Cancelled seal、Task receipt、原等待关闭、无伪造用户决定、
冷重开幂等及迟到批准拒绝，未增 worker/Run；后台权限组 20/20 通过。补齐的是
该取消场景的持久生命周期证据，不是非终态许可复用即时关闭或客户端 UI 验收。
具体日志与范围见 VERIFICATION。生产代码本轮无新增修改。

LC-05a 取消分型已实施：BackgroundPermissionWait 分 Decision/StopRequested；
三个热取消及冷恢复取消统一撤销本地执行资格并记录 stop intent，不返回 Deny。
Host StopRequested 只结束 permission callback，不发决定/失败/取消完成回执。
后台等待 20 项与 Host 传输 1 项测试通过、零失败/忽略；既有链接器体积警告仍在。
本轮不代表非终态 Core settlement、等待关闭、后继资格和 UI 全链路已实现；
这些仍按下面审计记录及 LC-05a 后续契约整改。

2026-10-12 非终态权限关闭审计：Core 会话许可可结束同批匹配的 permission effect，
但 Store 目前仅在 Run seal 时关闭绑定等待；EffectWithdrawn 只有 Domain 类型，
没有生产消费。另确认 CLI 后台等待的三处取消检查会返回 Deny，经 Host 转为用户
拒绝。不能只改 UI 或提前 withdrawn：当前恢复资格要求 resolved decision，会
阻塞原 worker。LIFECYCLE LC-05a 已细化选择、Core settlement、等待关闭、执行
恢复的不同所有者、同事务顺序、并发/冷恢复及六组纵向验收门槛。这是待实施设计，
本轮未声称上述缺口已修复；下一步应先补 Core settlement 的提交来源契约。

权限消费链后续整改：删除后台任务使用 Host 相似规则绕过新 Core wait 的快速批准，
删除 Store 对相同 Host scope_rule 的兄弟请求批量写批准；后台 AllowTask 只传选择，
不自报 grant 范围。Core 仍负责许可复用，Store 保留原选择记录及原子撤销/重放。
增加真实 Application/Core 后台等待回归（预置匹配 Host grant 仍必须等待明确决定）
及跨资源/能力同 scope 的 Store 反例。统一测试结果：Application permission_request_port_e2e
15 通过，CLI permission_wait_tests 19 通过，均零失败/忽略、exit 0；CLI 仍有
既有 __eh_frame 链接器体积警告。A09 typed 生产接线仍未完成，不能因移除这两处
分支宣称整套权限架构或真实 UI 已验收。日志与范围见 VERIFICATION。

权限纵向接线审计：Domain FrozenPermissionOffers 含 canonical Run occurrence，
原注释却要求直接来自 Core effect，违反 Core 不拥有数据库 Run 身份的边界。
已修正类型说明，并在 RECOVERY_AND_AUTHORITY A09 明确 Core 主体/范围决定、
Application 来源绑定、Store 选择事务、Host 身份复核的责任及错误顺序。确认
ResourceLocatorKind 尚未覆盖 process invocation，生产仍使用字符串主体和 Host
task matcher；本轮未声称已接线，也未为文档/注释变更重复运行编译测试。

真实 Mac 压缩完整门禁复测已通过：VmJLJd，runner exit 0、XCTest 1/1、零跳过，
315 字节真实部分摘要经一次续写后提交，原文召回通过，截断计量 incomplete。
新隔离库、同一 Test 构建，evidenceProfile=runtime；不是 UI 或跨端验收。
详细构建身份与日志见 VERIFICATION；完整恢复预算与权限生产接线仍未完成。

真实压缩取得有效样本：SEJqYo 的 XCTest 1/1 通过，实际摘要截断 458 字节后
续写一次成功，原文召回通过。runner 尾部门禁仍读空的旧 Trace 表导致 exit 1；
已改为只读 canonical run_events 并核对计划/请求/续写/完成顺序与来源指纹。
新 verifier 的 2 测试及真实库离线复核通过，但完整 runner 尚未重跑。真实性能
数值与门禁范围见 VERIFICATION；不把一次样本外推成性能预算或全端验收。

无进展恢复状态类型化：删除 loop_user_decision_required 和 loop_interruption_reason
持久字段，统一为必填 NoProgressRecoveryState（Inactive/Pending{reason}）；生产
消费者共同读取该状态，真实用户等待仍由 UserInputWait 独立持有。checkpoint
schema 17 拒绝旧版本；空原因在恢复验证时拒绝。增加往返及必填字段回归，测试
正在执行。独立恢复 checkpoint 身份、进展见证及完整预算状态仍待接线。

恢复出口后续：非法 request_user_input 现在先通过 Domain QuestionSet 契约，
失败不进入修正循环；四类拒绝分支均测试请求 usage 不丢失，直接回答不强制提问。
Core 库 485、Host 契约 142 全通过，日志 `/tmp/nola-loop-recovery-exits-final.log`。
这不是提问语义必要性的证明，也不是完整持久 checkpoint 状态/跨端验收。

恢复出口补齐：抽取唯一 conclude_no_progress，重复/空输出及恢复阶段无效或无
结果的 requirements_update 共用该出口。提前退出前记录 context usage，修正前一
增量可能漏过该记录的问题。Core 库 485、Host 契约 142 通过，exit 0，日志
`/tmp/nola-loop-control-exit.log`。原生问题错误、直接回答/截断及新输入组合仍需
核对；没有运行中测试，完整 typed 恢复状态及真实 Mac 压缩仍未完成。

同批权限失败已定位并校正回归：旧测试依赖响应 JSON 的 `resource_pattern:"*"`
扩大到另一个文件，与已实现的具体 Core subject 授权冲突。生产授权保持不变，
测试改为验证不同文件保留等待、分别批准和分别派发/记回执。最终 Core 库 485、
Host 契约 142 全通过，日志 `/tmp/nola-loop-permission-contract-final.log`，exit 0。
此结果不补齐 typed frozen offers 接线，也不证明真实 Mac 压缩已修复。

循环恢复增量测试结果：Core 库 485 通过；Host 契约 141 通过、1 失败，整体
exit 101，日志 `/tmp/nola-loop-recovery-contract-final.log`。新增冷恢复无进展退出
回归通过，但同批权限请求消费用例实际执行 1、预期 2，需追查，不能直接更改预期。
此轮没有运行中的测试；真实 Mac 复验及完整恢复 checkpoint 接线仍待完成。

循环保护实施增量（2026-10-12，测试运行中）：删除自动合成用户提问；恢复响应
可选择原生提问、要求控制或直接回答，违规重试/空响应以 incomplete/no_progress
收尾且不派发。增加从同一 checkpoint 冷恢复后的两种拒绝路径回归。现有用户
主动回答后的继续路径保留。完整 typed checkpoint/进展见证及不同方案准入仍未
完成，不能把内部旧字段名解释为已发生用户等待；真实 Mac 尚未复验该增量。

新构建复验仍失败（2026-10-12）：`b1uzdR` 真实 Mac 用例 exit 65、0/1、零跳过；
首轮重复召回后抛 MacLocalAgentWaitSuspension，未触发压缩截断。请求 tool_choice
为 auto。进一步确认 Core 重复 batch 保护会强制仅暴露 request_user_input，并在
模型不提问时合成提问，混淆了 no_progress 与 needs_input。已在
HARNESS_CONTROL_SCHEDULING 明确待实施的有界恢复/用户等待分离契约；尚未改生产
状态机，不能把本次引导修改声明为根因已修复。

ACK 轮绕路复查（2026-10-12）：检查 `jv08dS` 的实际持久化请求快照，当前用户
文本仍在 user input 中，未被压缩替换；原文检索也实际成功返回了来源 ID。
Core 的普通 answered 路径本来不要求工具 observation，但动态控制说明没有明确
该边界。本次同步工具说明与控制上下文：普通回答无需控制调用或制造执行证据，
source_refs 不要求重复检索当前消息；保留未完成动作及负面评估约束。增加无
observation/assessment 的 answered 控制回归。此为引导契约整改，不宣称真实失败
已解决；需模块回归与真实 Mac 复验。

真实压缩续写复验未通过（2026-10-12）：产物
`/tmp/nola-mac-real-memory.jv08dS`，外层日志
`/tmp/nola-mac-sampling-compaction-real.log`，exit 65，XCTest 0 通过、1 失败、0 跳过。
使用已验证构建、Responses、40k 窗口及单次 128-token 摘要截断注入；实际未命中
截断注入，不能证明续写或热分叉成功。第二个铺垫轮出现重复 conversation_recall，
三次相同调用无进展后产生真实 user_input_wait_changed，问题描述要求用户决定
继续/换方法/停止。测试辅助函数漏认该持久化投影，将其误报为缺少 Trace。
已补辅助函数分类（含 Web source E2E），并为压缩铺垫轮增加严格 ACK 校验，
避免仅凭 finished 继续放大错误历史；这些测试改动尚未重新编译执行。
模型为什么在 ACK 轮偏离为历史检索尚待查清，不以取消循环保护或自动回答用户
决策来让测试变绿。生产压缩问题仍未关闭。

采样结束原因计量整改（2026-10-12，模块与真实断流复验通过）：StreamedChoice 新增只读
sampling_measurement_outcome，正常 streaming 与 PreparedModelRequest 分叉调用
同一规则，不再用 Ok 等同 Succeeded。完整结束/部分或未知/协议错误/内容拒绝
分开记录；取消和准入失败仍保留各自原出口。新增分类矩阵与原部分流回归断言，
不改业务结果、文本、重试或 seal。运行 `nola_cargo test -p nola_cli --lib llm::`，
日志 `/tmp/nola-sampling-outcome-tests.log`，最终 152 通过、0 失败/忽略、1609 未选中，
35.60 秒、exit 0。随后重新 build-for-testing 的真实 1 KiB 断流验收 exit 0、
1/1 零跳过；实际 1025 字节断流对应 incomplete，完整后续请求对应 succeeded，
最终 UI/answered seal 仍通过。产物 `/tmp/nola-mac-real-memory.ztgswO`，外层日志
`/tmp/nola-mac-sampling-outcome-real.log`，新构建指纹与详细计量见 VERIFICATION。
压缩分叉的真实请求计量尚未单独复验，不能用正常请求样本代替证明。

真实断流后的测量语义缺陷（2026-10-12）：同构建 Mac 流式切换注入 1 KiB
断流，实际 1098 字节后终止，第二请求完整，最终 UI/answered seal 验收通过。
但 measurement 两个 provider_sampling end 都标 succeeded。已定位
`cli/src/llm_parts/streaming.rs` 的 Ok(choice) 分支和
`cli/src/llm_parts/request_snapshot.rs` 的 result.is_ok()：保留部分文本的
stream_incomplete 是合法 Ok，却不应算完整生成成功。下一步统一以结构化结束
原因分类采样 outcome，保持部分结果传递，不修改业务重试/封存规则，并补正常/
截断/缺结束原因的两条调用路径测试。原始证据与范围见 VERIFICATION。

Mac trace 单流所有权（2026-10-12，合同复验通过）：移除单流 reducer 的换身份
重置逻辑，固定 conversation/run/trace；新增会话展示流管理值，在候选准入成功
后切换并保存退役 trace ID。生产消费入口与新提交初始化已使用它，旧 trace
迟到不会再清空新流状态。新增 A→非法 B→A 重放→合法 B→迟到 A→B 后继回归，
实际消费入口也断言旧 Turn 失败不创建消息/改变当前结果。不是第二份业务状态，
不生成 seal 或权限决定；仍依赖上层 generation/传输绑定，不证明未知新流来源。
`./scripts/test_mac_layers.sh contracts` 最终 exit 0，XCTest 997 项（2 跳过、
0 失败）、156.417 秒，另 Swift Testing 29 项通过；日志
`/tmp/nola-mac-trace-stream-owner.log`。随后启动当前构建真实 Mac 流式会话切换
测试，外层日志 `/tmp/nola-mac-trace-owner-real.log`，开启隔离数据、构建指纹、
measurement 和正文流证据门禁。最终 runner exit 0，XCTest 1/1、零跳过，1 个
answered 持久 seal 与生产验证结果一致，正文流门通过；产物目录
`/tmp/nola-mac-real-memory.RmfEKJ`，测量口径见 VERIFICATION。退役 ID 总量、真实重连与
多来源并发仍待验收。本项替代旧条目中“新 trace 自动重置”的当前实现描述。

Mac trace 载荷冲突（2026-10-12，合同复验通过）：去重集合改为 event ID 到
SHA-256 摘要映射；摘要取自解码后 envelope 的 sortedKeys JSON，不是原网络字节，
也不是来源签名。同 ID 同摘要 Duplicate，不同摘要 EventConflict；编码失败
InvalidEvent。新 trace 的序号检查先于重置，零序号拒绝不再清除旧流历史。
新增正文投影开/关两模式回归，检查 payload/sequence 改变冲突、原事件重放、
非法新 trace 后旧去重仍有效、合法后继序号仍可消费。已有生产入口只接受
applied/gap，因而新拒绝类别不会进入下游分支。运行
`./scripts/test_mac_layers.sh contracts` 最终 exit 0，XCTest 996 项（2 跳过、
0 失败），另 Swift Testing 29 项通过，新增冲突用例 0.001 秒通过；日志
`/tmp/nola-mac-trace-conflict.log`。未限制事件条目总数，未证明 unknown JSON 字段冲突或性能预算；
独立 trace 生命周期及完整副作用矩阵仍待完成。

Mac trace 消费顺序整改（2026-10-12，合同复验通过）：
consumeTracedAgentEvent 现在在所有 traced 分支副作用之前检查 generation 与
conversation，并消费 reducer disposition；只有 applied/gap 可以继续。
projection 先在局部值上计算，底部只发布已准入的非终态投影，不再二次归约。
无 trace 消息保留各自 canonical 验证。新增实际消费入口回归，覆盖错误 Run、
会话、重复 ID、过期 generation 不创建新消息/改变调用方结果，并允许合法序号
随后到达。合同测试最终 exit 0，XCTest 995 项（2 跳过、0 失败），另 Swift Testing
29 项通过；日志 `/tmp/nola-mac-trace-consumer-admission.log`。实际入口新增用例与
MacRemotePermissionConsumptionTests 均通过。此项替代下条“apply disposition 被忽略”的当前描述；新 trace
自动重置、同 ID 异载荷冲突、权限/浏览器等完整副作用矩阵仍未全部验收。

Mac trace 身份消费审计（2026-10-12，局部合同验收通过）：MacAgentTraceReducer 原先
仅按 trace ID 重置，不检查同一 trace 的 Run/conversation。现绑定 conversation，
拒绝同 trace 换 Run；检查先于去重和重置，拒绝不污染 projection/cursor。增加
复用 ID、不同 ID、跨会话新 trace、拒绝后合法继续及同会话新 trace 测试。
注意这不是完整 CLIENT-01D 接线：Mac 仍保留新 trace 自动重置，尚未改为按正式
运行身份创建独立 reducer；MacWorkspaceModel.consumeTracedAgentEvent 仍忽略
apply disposition，且部分浏览器/权限状态处理先于 reducer。下一步须统一消费
准入位置并验证拒绝无任何上层副作用，不能将本次局部防护写成整链完成。
`./scripts/test_mac_layers.sh contracts` 已完成，exit 0；XCTest 执行 994 项，
2 项跳过、0 失败，另 Swift Testing 29 项通过。日志
`/tmp/nola-mac-trace-identity.log`，新增身份拒绝测试通过。未跑真实 Host/UI 验收。
进一步源码核对确认 trace.run_id 当前为 Core Turn ID，与外层 canonical_run_id
不同，不能直接比较；消费作用域、拒绝无副作用及后续 Turn 的接线目标已补入
CLIENT_BOUNDARIES，尚未据此改造完整生产消费入口。

终态上层集成复验（2026-10-12）：扩展至 Application 收尾、Task 绑定、冷恢复、
非终态重放和 Store 同步/删除。修正两处同步 fixture 的初始 Core session 身份，
使取消提交具有同一 revision 的准入来源；修正恢复竞争测试的零 busy timeout
作用域，避免准备数据阶段偶发锁冲突。未改变生产拒绝规则。最终 10 个测试目标
69 项全通过；命令与失败/最终日志见 VERIFICATION。本轮不替代真实客户端验收。

正式终态来源绑定复用（2026-10-12）：读取此前仅校验 seal 内部与 Run identity，
未复用 journal 的本地 epoch/远端 Host 来源核对。现通过 journal 单一 helper
调用 local load 或 remote reception state，远端另要求接收条目水位等于封存数。
新增复制合法 seal 但无远端绑定的拒绝反例，以及远端完整提交、冷重开、重放和
后续 journal 追加后的 canonical terminal 正例。此处验证已保存绑定，不把摘要
等同签名或声称完成网络身份认证审计。

正式终态来源唯一性（2026-10-12）：canonical_run_terminal 原先对本地 seal 与
远端 committed stage 的 UNION ALL 使用 query_row，静默取首行，且未检查远端
manifest_complete。现同一读快照要求恰好一个完整已提交来源，缺失/歧义/未完成
返回错误。隔离 fixture 覆盖未提交 stage 不干扰、相同内容双来源拒绝、单一但
不完整远端来源拒绝，回滚故障注入后原查询可用。只修改代码和隔离测试，不清理
用户封存记录；来源认证和 manifest 内容校验仍分别由正式提交/校验路径负责。

生产 CLI terminal reason（2026-10-12）：实际 CoreEventPresenter 曾只保留
NeedsInput，其余 outcome 均向 Emitter 写 completed。现使用 Domain outcome 的
明确 transport spelling，缺结果为 outcome_missing；JSON 仍只发送原 Core terminal
envelope，不增加第二套终态记录。新增 presenter→实际 Emitter 的六 outcome 测试，
检查内部 terminal reason 与 JSON 原 event 一致、不制造 error。现有 CLI 进程退出
仅将 failed 映射非零，自动 Goal 暂停仅识别 cancelled/returned_to_composer；本轮
不将 incomplete/blocked 强行改成失败或用户暂停，也不据诊断修改 Goal 状态。

共享 trace 准入补齐（2026-10-12）：当前 schema 外的事件明确拒绝；event ID
去重改为对照固定 SHA-256 序列化摘要，同 ID 异载荷报 EventConflict；终态后新的
开始/结束报 TerminalAlreadyObserved，原事件完全重放保持 Duplicate。均先校验
再推进 cursor/去重/投影。摘要不充当签名，当前也未限制事件总数或完成真实压力
基线；此处不把固定长度摘要宣称为总内存有界。消费者的错误出口仍须逐端核对。

Turn 完成不等于任务成功（2026-10-12）：发现 shared trace 对全部 TurnCompleted
统一使用 Succeeded，并把 Blocked/Incomplete/None reason 降成 completed。现逐项
保留 outcome，needs_input=Waiting，blocked/incomplete=Incomplete；缺 outcome 明确
outcome_missing。reducer 用已观察的 Run terminal kind 判定结束，不再从展示 status
推导结束，避免合法非成功结果被后续遥测复活。新增六 outcome 分支及晚到 phase
用例；这不改变 canonical seal，也不证明全部客户端已经按 reason 展示正确状态。

共享 trace 身份绑定（2026-10-12）：此前 reducer 每次直接覆盖 conversation/run/
trace，且 batch helper 丢弃所有 apply 错误。现在首次接受绑定后三键不可替换，
IdentityMismatch 先于去重/写入；batch 改为 Result，不保留静默部分成功接口。
新增三类错身份 × 是否复用 event ID、拒绝不变和合法后继仍可接受、batch gap
反例。当前仓库所见 batch 调用在共享测试内，不据此声称真实客户端身份隔离已
全部完成；同身份同 event ID 的载荷冲突、错误 schema 与冲突终态诊断仍待审计。

Run trace 状态所有者（2026-10-12）：reducer 原来除 validation 外仍无条件采用
最新事件 status，工具成功能把 Run 变成功、晚到 usage 能把终态变 running。
现限制 Run 状态来源为 Run 生命周期与非终态 phase，activity 结果独立保留，
终态后冻结 status/reason/phase，独立遥测仍可更新。新增三类终态 × 多类前后事件
回归。此归约不代表正式 seal 校验；跨 Run identity fencing 和冲突终态的显式
诊断仍需审计，不能因冻结了展示终态而声称所有输入错误已可识别。

外部诊断吞掉生命周期（2026-10-12）：shared agent_trace 曾对任何带 validation
metadata 的事件改 kind=ValidationUpdated，并改写 status，导致工具结束、等待
授权及 Run 结束原语义丢失。现附带诊断保留原 kind/status，仅独立 validation_updated
映射诊断状态；reducer 保留附带诊断，并不再用独立诊断覆盖 Run 投影 status。
新增五类源事件 × 四类诊断的回归。此处未完成 String→typed enum 切换，也不证明
所有客户端都消费附带诊断；Run 投影其它 activity 对 status 的覆盖仍须继续审计。

Mac 验证诊断隔离（2026-10-12，契约回归通过）：validation.updated 改写独立
validation-diagnostic，而不是 run-status；incomplete/blocked/evidence_insufficient
为非失败诊断，failed/error 才使用失败外观。Core Parts 管理内容时仍消费诊断，
canonical refresh 保留该条，下一条 required/not_required/succeeded/completed 清除
该条但不得删除 Run 错误。finish 不把 incomplete 涂成 completed。新增 Mac reducer
十组合回归已通过；Test 配置 contracts 最终 993 项、2 跳过、0 失败、exit 0。
仅证明所运行回归，未据此声明完整真实客户端验收通过。
apple-design 的 feedback.md › Best practices（“Consider integrating status feedback
into your interface.”）用于就地、非打断式反馈；当前只审代码语义，未验证截图、
对比度、VoiceOver 或真实 provider。下方 Mac 缺口记录为本轮修改前状态。

验证诊断与执行状态分离（2026-10-12）：EngineeringLoop 的模型提示已能说明未知
scope/version 不可用重复验证清债，但 trace 仍输出 required/run_targeted_validation
或 correct_then_validate。现由 Core 已有事实派生 evidence_insufficient，trace
标 Incomplete 且不输出重复验证动作，不增加客户端/Host 状态源。已知范围/版本的
正常验证路径保持不变。此处只修共享 trace，不等于全部客户端完成接线：
NolaMac/Models/MacAssistantPresentation.swift 的 validation.updated 仍把
incomplete/blocked 与 failed/error 一起映射为 .failed，且不消费新的证据不足状态；
这处展示层混淆需要按非终态诊断契约继续整改，不能由当前 Core 测试证明已修复。

可空回执的 Store 接线（2026-10-12）：新增 Application→Core→SQLite 集成回归
复现提交失败：Store 校验原始 Host inbox 时将 execution:null 强制解析为
ExecutionReceipt。现明确解析 Option，字段缺失仍报错，不合成 Unknown/Completed。
用例同时核对 null 与显式 Unknown、业务 data 伪造回执、冷重开、重复投递不增行、
恢复模型上下文后的保留字段。此前协议/Host 测试未覆盖此提交路径；此修复不是
真实 provider、实际文件副作用或 Mac UI 验收，也不宣称已定位某条用户会话。

回执缺失的协议接线（2026-10-12）：上一轮只改 Host 封装未覆盖 shared protocol
validate_completion，后者仍拒绝 Succeeded+Unknown，是局部测试范围不足。当前
ToolResult.execution 改为必填 nullable，CLI 原样传 Option；null 投影为无保留
证据字段，显式 Unknown 保留。校验接受成功返回但无回执/未知证据，Deferred
必须有明确回执；失败检查和明确未执行仍不能标成功。缺字段仍拒绝，不保留
旧默认。下方“非 Option 差距”是此前状态，本次源码修正不等于真实执行验收。

Host 缺回执不得补成已执行（2026-10-12）：tool_result_contract 曾以 error=false
将缺失 execution 回执补为 Completed。现缺失时只输出 Unknown/空 effects/checks；
HostToolOutcome::Succeeded 仍独立表示工具返回状态，不据缺证据改为失败或重试。
模型/业务 data 中伪造 _nola_execution 和展示文字不能替代 Host receipt。
当前 ToolResult wire 的 execution 仍非 Option，所以“原本没有回执”在该封装后
表现为 Unknown，而非可追踪的 None；完全保留缺失与显式 Unknown 的区别仍待协议
调整。真实工具效果与 Core evidence 持久化的纵向验收不由本回归证明。

无挂起批准授权的终态撤销（2026-10-12）：新增批准→实际 Core ExecuteTool effect→
模拟 Host 成功/失败结果→模型完成/失败→Run seal→Task 对账的 fixture。按精确
permission-request grant ID 检查批准记录、关闭故障撤销回滚、冷重开后撤销历史
保留、原用户决定不变。这里不执行真实文件副作用，不能证明工具效果或 UI；
原“批准后组合未覆盖”现在有事务 fixture 证据，真实 Host/客户端验收仍缺。

无挂起权限的非取消终态（2026-10-12）：新增独立 Ensure→用户拒绝→实际 Core
PermissionResolved→模型完成/不可重试失败→原 Run seal→Task 对账测试，覆盖关闭
故障回滚、冷重开、拒绝决定保留、原来源不变、重复对账和单 Run。模型为 fixture。
此前“正常完成/失败分支缺失”现有拒绝分支证据；批准后真实工具执行完成/失败、
授权撤销以及跨端 UI 的组合仍未覆盖，不能将此用例扩张为全部正常执行验收。

授权数据与配置 pattern 分离（2026-10-12）：Kernel 原先对所有来源统一执行
capability/resource wildcard_matches，完整输入中的星号可能被重新解释为范围。
现在 Conversation 要求能力字面相等，资源相等或经过既有显式目录/命令
范围解析；普通资源不进入 wildcard。配置来源仍支持通配。shell 编码前缀只有
对应 shell 能力可以解释，未知能力的输入不能借前缀启用另一类范围解析器。
此处仍沿用既有目录/命令范围格式，不替代 typed offers、Host 资源证据或来源认证。
Turn 包含 /allow 显式全允许策略，保留通配，并由 live origin 准入约束，不能
当作具体调用授权统一改成字面规则；CLI 回归已发现并纠正此前混淆。

Core 会话授权范围所有权（2026-10-12）：tool_driver 两条会话授权路径原来都允许
Host grant 字符串或 resource_pattern 覆盖 pending resource。现始终保留 Core
原资源，不将 Host 执行数据解释为范围选择；删除 CLI 原资源→目录范围改写函数，
前台 Core 适配器只返回会话批准决定。终端 Core 提示不再将父目录描述为本次范围。
尚未完成：native/custom resolver 的冻结范围展示、task Host grant 独立 matcher、
Kernel 字符串 pattern 与 typed offers 的完整替换。此处不是整体权限闭环声明。

Core 资源信息保全（2026-10-12）：Composite 缺字段或 null 不再返回部分对象，
改为保留完整输入；FirstOf 保留路径原始空格。移动/复制/批量读取/格式化/批量
替换/补丁在 built-in catalog 和 fallback 共用完整输入策略，风险敏感工具也
共用此声明。以下旧审计描述的 FirstOf 多资源丢失在这些内置工具上已修正。
注意：这不是 typed 资源身份或权限范围；CLI 仍可能重新派生字符串资源，Kernel
也仍有 pattern 语义，完整输入不能据此宣称端到端授权安全。

生产范围收紧（2026-10-12）：CLI scope_rule 当前格式改为 3，所有规则必须精确
匹配原 tool（包含 bash 前台/后台能力），拒绝格式 2、缺 tool、缺失/空 resource。
移除按项目标记自动扩张目录范围；文件规则仍保留原目录复用语义。命令范围存在
但调用缺命令时也不能绕过匹配。以下旧审计中“忽略 tool、扩大项目根”描述的是
整改前状态；Host 自行派生和匹配、跨资源覆盖及 Core/Store 接线仍未解决。
这是收窄生产行为，不是 typed offers 完整上线或资源新鲜度验证。

资源解析响应契约（2026-10-12）：Domain 增加 resolution request/facts/binding，
验证完整 occurrence 覆盖、原输入摘要、执行目标、资源种类和 access。它不是
Host 身份认证或证据真实性校验，尚未接入 Core/Store。源码审计确认现有
resource_content_version 会读取内容，validation_snapshot 会遍历目录，禁止为
节省接线工作将它们直接用于授权前解析。生产元数据解析与执行前身份复核仍缺失。

### 资源声明存在但不等于资源身份（2026-10-12，接线审计）

HostInputNormalization.permission_resource 的 Constant/FirstOf/Composite/FullInput
已被 HostCapabilityDescriptor::permission_subject 和 tool_driver 使用。FirstOf
丢弃其他资源、Composite 忽略缺项；built-in move/copy 使用 FirstOf，未保留源/目标
各自 access。与 CLI 的范围派生及旧 Core fallback 并存，不能再叠加一套名称映射。
新 typed subject/offers 接线前必须有资源声明→目标 Host 解析→Core 提交事实的
协议；现设计已补全输入输出、受阻/变化/等待授权的状态、取消竞态、冷恢复、
句柄失效与分段性能测量。生产资源解析 effect 尚未实现；未声称真实越权已复现。

纯需求提取代码现已加入 Domain：JSON Pointer One/Each、全部需求覆盖、每项身份
与 access、缺项/空数组/类型错误，以及原字面值保留；不读取文件系统、不依据
工具名或提示词猜字段。它尚未接入 HostInputNormalization，不能视为生产已替换。
同时移除新 matcher 对 POSIX 反斜杠/换行文件名的错误全平台拒绝，目标 Host 才
负责平台解析。跨端显示这些字面值时的转义仍需验收。

### LLM 请求事实反向依赖编排模块（2026-10-12，已拆分）

Domain 全量门禁发现 contracts::StreamedChoice 的 snapshot 字段引用
request_snapshot::PreparedModelRequest，后者又依赖 contracts；Responses/Streaming
也通过 request_snapshot::retain_request 反向依赖快照编排。现将快照数据结构、
脱敏 Debug 与保留原请求事实的纯函数移至 contracts，构造 fork/恢复/调用模型的方法
仍位于 request_snapshot。协议实现只引用中立契约。没有放宽有向无环依赖测试。
另修正三个 Application 源码测试对旧 Core 的直接引用，使用已有的显式 adapter
边界，不新增兼容层。最终验收记录见 VERIFICATION。

权限范围接线前置（2026-10-12）：Core tool_driver 的 PendingPermissionRequest /
AgentEffect 仍持有字符串 resource，effect_dispatch 原样输出；不能直接转换为
已验证 typed resource。Domain 已实现 FrozenPermissionOffers/Selection，选择仅
引用原 occurrence 和内容绑定的 offer_id，不能提交替换范围。它与纯 matcher
一样尚未接入生产，不是来源签名，仍需真实工具资源解析、Core 提交与 Store 事务。

### 可复用授权范围仍由 Host 派生并解释（2026-10-12，未修复）

本次已沿 CLI background wait → session_permission_rule/resource → Application
resolve_permission → Store resolve → CLI matcher 检查：已提交 Core 来源证明不覆盖
scope_rule。文件范围可能按目录标记扩为项目根；Store 相等校验只能证明字符串
未被替换，不能证明范围合法。CLI 的后续 grant 消费又使用自身匹配器直接返回
AllowTask；非 bash 规则写入的 tool 字段未参与匹配，能力族与精确工具语义不明确。
这是授权范围所有权缺失，不是文案问题，尚无完整真实利用/误授权复现，不据此
声称已发生越权。此前来源/关闭测试通过不能证明此链路安全。

整改契约已写入 PERMISSION_CLOSURE 的“授权范围所有权”章节：Domain typed scope
与唯一 matcher，Core 冻结 subject/offers，Store 只接受选中 offer_id 并原子保存
derived grant，Host 只解析/复核资源、不扩大或自行匹配权限。明确多资源、远端
身份、路径变化、网络 origin、process invocation、错误分类、重放与性能测量。
此为待实现设计，不将文档完成登记为代码已修复。

第一阶段代码：Domain 的 permission_scope 提供具体多资源 subject、显式能力
选择、精确/目录范围及纯 matcher。它没有被接入现有授权消费，不会把新纯函数
的测试通过当作 scope_rule 链路已修复。后续必须接线 subject/offers 的原 effect
证据与 Store 选择事务，并替换 Host matcher；旧权限策略 subject 暂仍存在。

### 权限决定与请求关闭仍混用（2026-10-12，新发现）

原 Run 对账曾在 `runtime_task_run_binding.rs` 将 pending permission 写 resolved/deny，
所以停止入口不再 deny 不能证明整个生命周期已修。当前进展：Store resolve 已在
grant 事务中仲裁取消，返回 StopPending；派生 grant 在停止/终态事务中撤销，
公开批量拒绝旁路已删除。当前 schema 72 将独立 closure 接入 Application 请求投影和
绑定 Run 对账事务，以 withdrawn/Closed 区分系统关闭与用户决定；关闭失败则
Task 终态回执、挂起关闭和授权撤销一同回滚。未绑定任务现从正式 completed/failed/cancelled 回执
原子关闭权限，不再合成 deny；运行中只有取消意图时不提前关闭。迟到新建与停止在
同一写事务仲裁，关闭身份重放不复活。绑定 Ensure 来源已接线，仍缺跨端验收；详细测试记录见 VERIFICATION，不能
将局部通过写成整体完成。

新增来源前置约束：挂起事务已比对既有 Task Run binding 的 Run、child 与输入
水位，拒绝 Host 自报的另一执行身份。现已进一步从当前 Core witness 与 pending
outbox 核验真实权限 effect。schema 72 修正来源依赖 suspension 的模块耦合：
Enqueue/Ensure/Suspend 共用请求所有的不可变来源表，挂起记录只携带回执校验副本。
delivery queue 清理不删除该记录，绑定终态只读来源表。首绑准入现拒绝任何已有
权限请求，预检和实际绑定事务共用检查，不把旧请求升级为新 Run 权限。无挂起
Ensure→取消终态链路已覆盖批准/未批准、故障回滚及冷重放；仍需正常完成/失败
分支、scope_rule 权威来源和 UI 状态替换验收。

### Task 终态多写入口绕过 Run 所有权（2026-10-12，事务保护已修）

后续取消接线：Store 对绑定 Task 仅记录首次取消意图，重复请求不更新时间，不
提前撤销 permission suspension/改写授权决定或制造终态。CLI recovery 的所有
cancel_requested 状态均按原 binding 进入 cancel-only 对账（原来仅 waiting_input），
缺证/错误记录诊断并停止本轮调度，不落入普通恢复。下段“仍需整改”描述此次修正
前的发现；真实进程冷恢复、各客户端入口和取消与授权竞争仍需完整验收。

停止工具入口后续：`stop_task` 原先将 request_cancel 的错误吞成 false，再可能
回答 already_finished；现返回提交错误，不把未知写入当作已结束。绑定任务停止
不再调用 deny_pending_permissions_for_task 冒充用户拒绝，所有状态都尝试原 Run
cancel-only；未绑定任务原有取消规则不变。真实 Application/Core 的权限测试
已改为先验证“意图无终态”，再走 supervisor 原 Run 对账，不再沿用旧 fixture
直接期待 Cancellation 来源的终态回执。首次缺冻结配置拒绝后，补正式准入配置复验
通过；完整 permission_wait_tests 14/14 通过。具体证据范围及未覆盖进程死亡见
VERIFICATION 最新条目。

生产审计：`finish_runtime_task_detailed_with_event` 检查 Run binding，但共用的
`record_terminal_receipt_with_journal` 没有检查；expiry、queued/retry cancellation、
expired cancellation 可绕过 worker finish，产生非 BoundRunTerminal 的永久回执。
现唯一回执提交边界检查持久 binding，拒绝所有非正式 Run seal 来源；错误使外层
同事务的 Task 状态、attempt 撤销、事件和回执全部回滚。绑定 Run 暂时不可读不
解除此约束，不以过期或取消请求推断未知副作用已停止。
Application `task_terminal_receipt_e2e` 12/12 通过、0 忽略（2.90 秒），新增测试
对 6 种 writer 验证拒绝、状态/attempt/event 不变、无回执及重开读回。此反例用
隔离 SQL 保留 binding 而不建立 Run，专门测试“证据不可用仍不能另造结果”，
不是实际 Run 准入或取消全链路证明。日志 `/tmp/nola-bound-terminal-guard.log`。

仍需整改的消费路径：`recover_orphaned_agent_tasks` 仅 waiting_input 的取消走
`cancel_bound_task_execution`，其他 cancel_requested 分支再次调用 Store cancel；
绑定的 waiting_permission/retry 状态直接请求取消现在会拒绝并回滚，而不是伪造
终态。后续须将“持久取消意图”与“由原 Run 收尾”拆开并接到所有状态，复用原
Run cancel-only 恢复，不能撤掉本保护或为了通过重新启动普通模型任务。

### 中央验收终态权威纠偏（2026-10-12）

审计 `real_e2e_judge.py::_judge_event_terminal` 发现它仍由 Trace doneCount 判定
canonical completion，与已修 Mac runner 和 INV-10 矛盾。现中央 gate 复用正式结果
读回/持久 seal 的只读比较，同一步骤只接受一组 transcript/store，不从其他步骤
借数据库拼接证据；Trace 不再证明 Run 终态。sealed 的六种结果分别计数，failed、
blocked、incomplete、cancelled 可证明已封存，但不能证明任务成功。Mac 正向用例
仍要求 answered/action_completed，不放宽成功门禁。
其他平台 harness 若未导出正式结果和配对存储会明确缺证失败，不保留 Trace 兼容
回退。跨端导出接线、完整 manifest 生产校验的执行来源及各场景预期结果约束仍须
继续验收；此 Python 比较器不自称替代 Rust seal 验证。

### 真实 Mac 验收门禁与当前终态不一致（2026-10-12，已整改并实跑）

`run_real_mac_xctests.sh` 仍查询 completed/旧 Parts，并要求 Trace 非空和工具完成，
与正式 sealed/result 契约矛盾。现由 Swift 正式 canonical_run_terminal 读回导出身份
和结果，`verify_mac_ui_terminal.py` 在只读事务比对原 seal，拒绝遗漏 Run/错误身份/
非成功结果。通用 runner 不再把文件存在当作渲染质量，不另建 Trace 终态权威。
5 项比较器测试通过；真实流式切会话 XCTest 两次各 1/1 通过，第二次完整 runner 和
正文流门 exit 0。精确产物、首次 gate 失败和计时证据见 VERIFICATION/MEASUREMENT。
其余产品 UI 工作流、完整性能分母与多端链路仍未验收，不因本次正例标记整体完成。

### HCTRL 资源归属契约纠偏（2026-10-12）

源码证据：`cli/src/task_runtime_parts/execution.rs::run_agent_task` 的 usage/watchdog
跨整个 worker 生命周期；`cli/src/runtime_task_port.rs::prepare_permission_resume_with`
要求恢复生成新 attempt/generation/token。因此“按最后 active_lease 保存整个 worker
usage”存在归属错误，不能仅用可接受过期 lease 的 final observation API 修补。
已有契约“新 attempt 新水位槽”也可能在同一物理 scope 上重计。新增
`../软件架构设计/模块/15-观测与验收/HARNESS_RESOURCE_ACCOUNTING.md` 统一区分 scope、attempt binding、producer 及观察
权限；Task 总量按 scope 去重，attempt 边界缺失则保留不可分摊，不改业务 receipt。
本次完成的是源码审计与规范纠偏，未实现 ledger/OS 累计采样，不新增运行时通过声明。

### 当前源码批补充：准入与客户端恢复（2026-10-12）

- 最新结果覆盖下列历史失败：Mac helper 第四批 **12/12 通过、零失败零跳过**，
  包括 interactive／scheduled 授权等待杀进程后冷恢复、终局精确配置读回、原 digest
  与身份、真实 write/read、取消与权限范围隔离。日志 `host-helper-4.log`。共享层
  已修仅当前 sealed 语义，并从同一 pinned snapshot 验证正式结果与 checkpoint。
  Rust 新三类封存冷读、checkpoint 6/6、inspection 3/3 通过，all-targets 检查通过。
  两项旧 fixture（旧标签断言／删除被不可变约束拒绝）已集中修正，不降低保护。
  最终三个 Application integration 目标 **8+6+3=17 全通过**、exit 0、无忽略，
  日志 `exact-sealed-reader-2.log`；其中只移除 commit witness 的 Repository 反例
  带相同真实记录可读的正控制，不以无关错误冒充归属校验。此轮源码已收口，下一
  阶段仍是实测工具／压缩／UI 时延与缓存基线，整体 goal 不标完成。
- 最新验收后续：Rust 27+2+18 及 workspace all-targets 检查全部通过；Mac 第三批
  12 项中 10 项通过、2 个 cold 用例失败（各含主错误与 watchdog 清理错误，共 4 条）。
  两种来源都已恢复并封存；新增终局精确读取暴露 Application 仍把旧 Run status 当
  terminal，当前 sealed 被误当活动态。下一批保留失败回归，从同一 pinned snapshot
  携带 canonical terminal/result 并校验，删除旧终态分支；watchdog cancel 后明确等待
  退出，不遮蔽主错。Mac 真实模型文本 smoke 已独立 1/1 通过、零跳过，产物
  `/tmp/nola-mac-real-memory.Uk4Oww`；不能据此声称全工具或性能基线通过。
- 最新状态：Mac 启动来源闭域、必填字段及 Native 来源一致性检查已实现，两个独立
  Mac fixture 的完整能力声明已补齐，静态 diff 检查通过。冻结后的 Rust 协议／来源／
  恢复定向 27+1+17 全过；Mac helper 第二批 7 项中 6 过，cold 已执行真实进程终止
  与恢复，唯一失败是终局 answered 与 actionCompleted 断言不一致，待核对完成证据。
  另已确认 scheduled_action 恢复缺口：resume 协议故意禁止新 input_origin，但新
  来源比较错误地用默认 interactive_user 比较原冻结来源。当前进入下一源码批，分离
  fresh admission 与 exact-Run recovery 的校验，补 scheduled cold 纵向反例。
  下列较早“尚未复验／执行中”条目为历史检查点，以本段及 VERIFICATION 最新结果为准。
- 用户随后授权修复默认 config.json：仅删除尾逗号，保持原设置与凭据，安全备份并
  收紧文件权限。实际 nola-agent-host 在隔离 NOLA_HOME／DB 加载修复配置，真实模型
  最小请求 exit 0，11.183 秒；已核对 canonical answer 正文 NOLA_CONFIG_OK 与
  answered 终局。临时凭据配置副本已删除。此证据覆盖实际配置加载及基本模型调用，
  不代表 Mac UI、多工具或恢复完整验收；显式能力的完整场景前提仍需分别核对。
- Mac 真实 helper 第一批六项未过：cancel 两项在独立 fixture 缺 profile 时即失败；
  scope 三项发现生产 wire 不一致，Swift 写/恢复必读 mac_launch.input_origin，Rust
  strict descriptor 却无此字段，Host 执行前拒绝。不能删字段或忽略未知来绕过自动
  任务恢复约束。cold 一项 CancellationError 尚待分辨是否掩盖启动失败。本批重新
  进入集中源码收尾：optimization 统一 descriptor 与输入来源校验；boundary 修两处
  独立 Mac fixtures 并诊断 cold；全部收口后统一复验。CLI 六轮已 1732/1732 通过
  （另 4 忽略），不代表真实 Mac Host 启动已通过。
- 收尾复验：iOS 9 suite / 63 项全部通过（第七轮，定向非全库），Service 四目标
  6/6、Core Host 142/142 通过。固定取消身份/时间与恢复派发时间分离、冷 pending
  精确重放均有通过证据。CLI 第六轮全库及 Mac 六项真实 helper 路径仍在验收，
  后者使用 mock provider，不能代替真实模型和 UI 性能验收。
- 后续统一验收已结束：Mac contracts 第五轮全过；iOS 第六轮完整 618 项，605 通过、
  4 失败、9 跳过，xcresult finalized。四失败中取消请求缺必填 cancellation_id 已
  从生产源码确证，回执诊断 outer_ok=false；另三项为独立测试 profile 缺失/时钟混用。
  现集中收尾取消操作的固定身份与恢复、三个 iOS fixture、两项 Rust fixture；这批
  尚未复验。CLI 上轮冷恢复 exit 2 仍未复现/解释，不能标记根因已修复。
- 已确证 iOS 的 foreign journal owner 不是应放宽的客户端校验：Application 正常
  admission 返回合法 sequence=0 游标，但 Store 窗口只发现已有 journal row 且序号
  大于零的 owner。现在仅凭真实本地 Run 根消息与持久化 admission 绑定发现 genesis，
  继续执行原 owner/epoch 校验；历史导入 run label 不获得执行身份。新增缺准入证据
  反例。源码已落盘，未复验。
- iOS 权限卡片发布前先安装决定 continuation，防止同步点击丢失；取消 outcomePending
  尚未查明原因，只补不含参数、正文、凭据和用户 ID 的回执结构诊断，不改为成功或重试。
- iOS 全局模型选择此前误读 provider/runtime 范围内的 profile.selected；整改复用
  现有 chat.default_model 默认配置及 checked revision 提交，不改变 Store 的范围语义，
  不先取消原默认。此项由 boundary review agent 收口。
- 本批仍为源码与静态审计阶段。上一轮通过数不能证明这些变更已经通过，待全部冻结
  后统一 Rust、Mac、iOS 验收；未知子任务冷恢复与取消回执错误仍须取得真实原因。

### 手动压缩与活动控制隔离（2026-10-12，源码审计）

- 确证路径：公开 `RequestCompaction` 原先可将非终态 Turn 改为 Compacting，摘要提交后
  再请求模型，抢占已有模型／工具／权限／取消状态。外层维护 Run admission 已拒绝重叠，
  但直接 Core submission 未守住相同边界。
- 源码修复：公开手动请求要求不存在活动 Turn 且 effect journal quiescent；否则明确 busy
  拒绝，无排队、失败事件或原 checkpoint 变更。内部自动压缩路径保持不变。
- 回归源码覆盖原模型回执和权限指导回执仍可用、取消收尾不被改写、terminal 标签不足以
  绕过未完成 effect。现有取消压缩晚帧测试继续保留。尚未编译／执行，最终统一验收。
- 消费者核对：Application 在 Store begin 前拒绝，CLI 仅返回命令错误不清 Run；Mac
  当前消费自动压缩投影，未发现独立手动入口。Web 维护入口已移除强制关闭工具／清空
  allowlist，保留原配置供 Host 冻结工具前缀；摘要协议而非删工具来禁止执行。
  未分类 String 错误改 HTTP 500 + outcome_unknown，不声称 compacted:false／无提交，
  不猜测 busy 字符串、不自动重试。当前 Svelte 源未发现维护接口消费者。
  配置保留及未知错误响应均补源码测试，尚未执行。

## 当前续接检查点（2026-10-12，源码阶段）

用户约束：所有实现与静态审计完成后再统一编译、测试及真实验收。
2026-10-12 本批已完成源码收口，现进入最后统一验收阶段；新发现的编译或行为失败
必须修复并复验，未运行的链路仍不算完成。以下源码阶段记录保留其当时状态。
不维护旧接口兼容，不删除或迁移用户数据，不覆盖工作区其他变更。
以下是本次恢复的交接状态，不替代各专题的验收证据。

实时续接：CLI 完整库测试首轮已结束，1,712 通过、17 失败、4 忽略，日志
`/tmp/nola-final-verification.gMkJRJ/cli-lib-tests-1.log`。失败按共享 fixture／终局恢复／
工具输出契约批量修复中，尚未复验。排队输入须独立 Run，不能沿用原提交身份；
取消等待只证明观察中止，不证明进程停止或仍在运行。短预算下重复导航元数据挤空
工具预览属于实际生产问题，正在缩短并核对 typed data、精确续读偏移。
Android 自动化已收口，最新 JVM 402 项中 400 通过、2 跳过，无失败；androidTest
Kotlin 编译通过，但未运行设备 JNI／WorkManager／真实模型验收。已绑定 occurrence
恢复必须在 claim 前，不能受当前定义／第二个自动化租约阻断。
Mac／iOS 首轮均在编译阶段失败、0 测试执行：分别为非法 nonisolated typealias、
ChatView 缺函数声明；源码修复与统一复验由对应代理负责，不算客户端测试已通过。
后续实时结果：CLI 第三轮 1,730 通过、1 失败、4 忽略；唯一失败已静态定位为
原始 JSON 与 typed checkpoint 重编码的 f32 精度差异（0.65），不是恢复越权；改为
原始 checkpoint 字节不变性，待复验。Core 库仍 472 全过；Host contract 第三轮
138 通过、4 失败，剩余精确 schema／维护取消／验证义务断言批量收口中。
Mac contracts 第三轮已编译并执行 989 tests、2 skips，170 条失败断言（103 unexpected），
不能称客户端通过；共享 Journal fixture 缺必填 model profile 导致连锁失败，仍需与
独立签名／本地化／滚动失败分组修复。iOS 第四轮编译失败、0 测试执行：第九处
状态 switch 与纯数据 RecurrenceRule 的 actor 隔离已集中修复，待第五轮完整验收。
后续批量源码已修 Mac Journal 能力声明、签名真实委托链检查及权限卡片初始 1pt
视口风险；后者尚须真实布局回归证明。iOS 运行记录筛选和计数改用统一状态分类，
取消与跳过独立，不再遗漏 canonical 结果。外部工具大输出保存失败补明确不可续读
与未知副作用提示，不以截断冒充操作未执行；新增隔离存储失败回归。Rust 第四轮
完整库复验仍在运行；Host contract 第四轮已 142/142 通过（exit 0，1.12 秒）。
Mac 本地化已补 16 keys / 108 entries，原有译文未覆盖；Mac contracts 第四轮运行中，
结束释放 XCTest guard 后再跑 iOS 第五轮，不能交叉占用测试设备。
最新验收结果：Rust 库第四轮 Core 472 通过，CLI 1,731 通过／1 失败／4 忽略；
原 checkpoint 字节比较与新增外部输出失败用例均通过，但子任务冷终局恢复进程返回 2，
阶段原因尚未暴露，不能用上轮通过或重跑代替定位。Application Goal/task 集成 24/24
通过；真实 service 四目标共 3 通过／3 失败，涉及缺 cancellation_id、归档异步发布
窗口及 admission 尚无正文的 snapshot fixture。Mac 第四轮剩 17 失败用例／32 断言：
另一独立 LocalRun fixture、Steering 的旧 Run 归属假设、权限按钮 AX；其余见详细账本。
初始视口修复回归已过但权限按钮仍缺，不能把 1pt 风险称为该交互故障的已证实根因。
iOS 第五轮最终因收尾停止进展中断（exit 143，非完整验收），保留620条case结果：
590通过／21失败／9跳过，xcresult未finalize但Staging诊断保留。现开始下一集中源码批：
Mac 修正 UI 所依赖的 ignored conversation owner；CLI 补冷恢复阶段安全诊断，不猜
未知失败为可重试；iOS 修原始回执与模型声明，并追跨 Run journal owner。
root 的 service 回归已改 typed Interrupt 并比对冷重启原 payload；Goal 文件验收
分已发布读回与持锁阻止发布→杀进程→重启补齐两条路径，仅只读 SQL 观察 publication
frontier，不另开 Store 帮忙导出。guidance 先通过真实模型回执发布正文，再固定快照
验收，不将用户输入或空 admission 冒充 assistant body。源码阶段，尚未复验。
root 本轮额外修正 CLI 恢复夹具的 RuntimeConfig／HostRuntimeOptions 类型边界，以及
取消／Task reconciliation 反例的真实 seal 证据。不要重做已通过的旧失败定位。

本轮 harness 实证缺口：Automation occurrence 未持久冻结 definition_revision，
worker 只提交“当前版本”的 lease，可能使旧 occurrence 在定义变更后借新配置／权限
准入。仅在 HTTP 层先读后校验不能消除竞争；revision 已贯穿持久记录、事务入队、
领取及 Core admission（schema 69，无迁移），Mac／iOS 调度字段已同步，Android
纵向接线进行中。绑定后的 occurrence 仍只恢复原 Run，不因定义变化重复执行。
Runtime Driver integration 第二轮通过：opaque 协议 7/7、真实 Store＋fixture Host
接线 12/12。续租使用可控 epoch 测试，生产默认及 Instant 调度不变；排队输入验证
前 Run 封存、冷重开及合法后继 Run，不把全部消息塞回旧 Run。日志见 VERIFICATION。

冻结前收口（2026-10-12）：MCP 原 effect 稳定 task 身份及事务主键仲裁已落源码，
公开手动压缩的静止状态准入已落源码；两项均不以停止请求或查询失败冒充执行结果。
测量关联已贯通 command→Run→attempt/effect/request，但首输出只称 observed onset
proxy，并报告前驱缺采样等覆盖缺口，不声称真实首 token。当前只收尾 Web 手动压缩
不应强制删 tools 的调用方接线，以及三项已确证的 CLI fixture/工具说明预算矛盾。
这些收口现已完成并冻结，进入既定统一验收，不再以重复审计旧失败替代执行验收；
冻结不表示构建或测试成功。旧 Application 三目标四失败已有后续历史复验记录，
当前工作树仍需全部重新验收。

统一验收续接：共享 Rust 非 CLI all-targets check 已通过，六包库测试已执行：
Core／Domain／Kernel／Runtime Driver 库测试通过；Application 复验 310 通过／1 忽略，
Store 第 5 轮复验 402 通过／1 忽略，无失败；修复保留终局证据／历史归属／不可变
输入契约，没有以放宽生产验证换通过。
Harness 仍为主线：自动化所有生产创建入口同步显式 executionSpec，核查拒绝准入
不得遗留可执行 occurrence，以及权限等待／长任务租约／原身份恢复。
Web typed automation 与原会话导航接线后 294 tests 通过，测量脚本 27 通过；
共享权限／取消／task+automation 绑定集成 36 项通过，Automation Store 集成 9 项通过。
Android 上一轮 398 tests 通过、2 条真实模型测试跳过，新自动化接线待复验。
Rust 完整 all-targets 第 10 轮已通过，原始
日志集中在 `/tmp/nola-final-verification.gMkJRJ`。第 8 轮暴露 Web Automation 的
旧直接执行/Finish 旁路已接显式 executionSpec 与原 occurrence 身份，走
BeginAutomationTurn→Host→canonical seal→Reconcile，Svelte 提交方同步。Android
仍发现旧 Finish／正文猜成功旁路，正由专人整改，不能以禁用入口或状态字符串伪造完成。
CLI presenter 测试已迁到真实 Terminal adapter，保留预览/完成/归属断言，待完整复验。

最新纵向接线：native stdin 已由独立 launch/request/entry 承接，不进入 CLI Config
或 dispatcher；显式项目路径的冷取消已删除 eager getcwd（注入式反例尚未执行）。
Mac 启动参数删除 `--yes` / `--network-policy`，native parser 明确拒绝旧重复参数，
仅从 stdin `permission_profile` 读取用户权限意图；Mac 三种模式序列化测试源码已补。
同一入口的本地模型凭据不再复制到 `--api-key`，保留既有 stdin 凭据字段；新增
Mac 参数中无密钥且 stdin 仍携带所选本地凭据的测试断言，尚未执行。
这不改变 Core 对具体资源的授权权威，也不代表 Host 独立编译闭包已完成。
Shell status/wait/stop 已改为 canonical task 观察与取消流程，展示字符串不再作为
terminal 判据，排队/等待授权保持非终态；具体故障和并发流程尚待最终执行验收。
root 已同步 `bash_background_status/wait/stop` 工具说明：原说明承诺停止并删除 artifacts，
与持久取消/保留证据契约冲突；现在明确查询原任务、等待非终态、取消 ACK 不等于停止。
三项说明不再要求按用户提问关键词固定选工具，工具选择仍交模型按 schema 决定。
Shell 通知消费者已同步：`BackgroundTaskSummary.canonical_state` 与展示 `status`
分离，notification 状态和 delivery 身份来自同一次 canonical task 读取；CLI 不再
比较 `exited(0)` 文本判成功，七语言分别呈现完成/失败/取消，未知值不伪装终态。
退出码只读已提交 `shell_execution.exit_code`，不从 completed 猜 0。相关反例源码
已补并静态检查，尚未运行真实 worker/取消/终端通知验收。

| 工作 | 已有源码 | 剩余事项及负责人 |
|---|---|---|
| Host 请求边界 | 六块 typed request；独立 native launch/request/entry/control 命令；默认 terminal-ui 与无终端 native-host 构建声明已切分 | 无终端生产编译及 Mac 打包待验收；Server composition 仍显式使用 Config/Web，不声称已全部拆出 |
| Harness 运行控制 | 显式交互端口、权限作用域隔离、Shell Unknown 原身份恢复及 canonical 终态/取消已落代码 | 最终真实 task/attempt、副作用计数、停止与通知竞态验收 |
| Supervisor | 已修显式 owner 缺失借用其他会话、旧绑定复活、失败注册回滚覆盖并发新登记 | 最终统一并发/冷恢复验收 |
| Goal | 暂停/到期恢复、命令 operation ledger、恢复必需字段已整改并补测试源码 | 最终统一持久恢复及幂等验收 |
| 测量与诊断 | Application 提交回执事件、全实验清单与只读回执导出、Mac window/publication 两级 proxy 已落源码 | mobile_web agent 接续 command→Run→provider attempt 关联；真实帧、首 token/压缩基线及开销仍待最终测量，不设虚构预算 |

本次恢复时的缺口是共享工具/任务通过 `interaction` 调用权限提示，用户输入等待依赖终端。
现已拆为显式交互端口；Core 保持授权事实权威，没有交互适配器不得静默放行。
root 负责 `main_parts` / `web_parts` 产品装配，原 harness agent 负责共享接口和终端 adapter；
新 `cursor_harness_interaction_ports` 已中断，不重复派工。2026-10-12 恢复时 root 补齐
bootstrap 的 `HostInvocationRequest` fallible 转换及会话绑定测试的 Result 处理。
已记录的静态检查不等于编译成功；新测试均尚未执行，独立 Host 构建及各端真实验收仍待完成。

本次恢复已续接：新增 `runtime_interaction.rs`，权限/提问数据类型与交互端口移出终端模块；
CLI 装配 `TerminalInteraction`，无适配器返回 transport unavailable 而非授权或拒绝。
`user_input_wait::present_pending` 显式接收交互端口，产品调用已同步。
进程存活检查移入 `process_observation.rs`，Core 恢复/取消不再借用 renderer；Unix
直接探测，未知结果保守保留 owner。`agent_command_port`、`host_output_admission`、
`task_permission_wait_tests` 的旧 Config 测试夹具已按当前请求类型补齐。
这些均为源码实现进度，不能替代最终编译、权限流程和跨平台恢复验收。

继续中的主线：跨模块静态整合、协议修正 Incomplete 的来源/落库契约，以及测量身份闭合。
SOP 查询已改为单次快照内按 workspace / owner / 活跃状态过滤，超出资源上限明确报错，
不先截断全局结果再筛会话；源码测试已补，仍待最终验收。
后台能力/意图分离及取消优先已落源码；取消被接受后不重复提交，失败不冒充终态。
“取消提交失败但原任务可能仍运行”的 ToolOutcome 已补原 task_id、观察阶段及显式
Unknown receipt；root 补 Core 输出转换反例，确认业务 data 中伪造的保留字段不能覆盖
Host receipt。这些测试源码未执行，也不证明模型永远不会提出重复命令。
续接补充：`../软件架构设计/模块/05-执行与恢复/HARNESS_CONTROL_SCHEDULING.md` 的 HCTRL-06 已明确提交不确定性、
原任务恢复、故障注入窗口和分阶段测量契约。root 将 Core 输出转换反例扩展至
queue_submit / registry / observe / cancel_submit / cancel_observe 五个阶段。
optimization agent 已修改排队已可能落库后返回错误却删除输出证据、丢失 task_id 的出口，
源码细节见下方 HCTRL-06；当前接续压缩、续跑与取消/权限等待交界审查。
最终仍须核对真实副作用计数和重启恢复，不能以 Unknown 序列化断言代替纵向验收。

权限 resolver/session/设备目录作用域已统一按绑定身份移除，禁止跨线程释放；同一活跃
会话嵌套沿用绑定，不按过期字符串复活回调。root 修 Web native/resume 两处产品装配，
改用 canonical conversation ID 与 Core 一致；client/run 身份仍在原 resolver 闭包和
观察注册中核验，不把它们拼接成另一套会话 ID。冷恢复 fixture 同步显式绑定会话。

iOS/Android/Web UI 独立审查已返回：iOS/Android 输入区是实际语气配置而非复杂度，
不能改名冒充；Web 窄屏三大区域纵向堆叠、输入焦点环和虚假 tabs 键盘承诺需整改。
Web 窄屏导航/详情已改为原生模态面板并补焦点语义；iOS/Android 输入区已移除语气和
上下文快捷项，保留既有设置入口，新增权限设置入口及错误全文/复制。均为展示层源码，
没有修改授权默认值。iOS 关闭错误提示已改为会话与发生次序限定的本地展示状态，
Android 同步按原错误身份清除，不清除执行状态；源码已补，最终行为待验收。
UI 不替代 harness 主线，未启动构建、预览或真实验收。

## HCTRL-06 续接：Shell 队列 ACK 与原任务证据（2026-10-12，源码未验收）

静态证据：`queue_background_shell_task` 先提交 durable RuntimeTask，再 claim/spawn；
后台 bash 原错误分支却删除 stdout/stderr/status 并丢弃 task_id，因此 launch 错误不能
解释为未提交。现两种 supervised Shell 的 queue 错误保留原文件、task_id，显式返回
Unknown execution receipt 与 `queue_submit` 阶段；进程内 registry 失败同样保留原身份。
恢复提示引用现有 `bash_background_status(task_id)`，不是重放 command 或新建任务。
前置 schema/path/output-contract 校验仍在 queue 边界之前，未伪造 durable 提交成功；
queue 调用内异常只说明提交/启动结果未确认，后续查询无记录也不等价于副作用未发生。
新增 source test 注入 save 后 launcher 错误，重开库核对同任务及文件仍在；registry
注入反例检查原 task_id/Unknown 输出。仍未运行测试，完整矩阵与测量见
`../软件架构设计/模块/05-执行与恢复/HARNESS_CONTROL_SCHEDULING.md` HCTRL-06；未实现按命令文本去重，也不声称模型不会
用新 call ID 提出同一命令。真正传输重放仍由 Application delivery/received-command
和 RuntimeTask attempt lease 仲裁，不以本地索引或展示错误充当幂等账本。

同链后续静态复核发现 `background_child_status` 把 queued/suspended/retry_waiting
显示为非 running，等待权限/读库错误又可能退成 exited(unknown)；status/wait/summary
据此提前认定结束，notification 更在核对 durable 终态前结算输出。已移除这一文本/文件/
PID 推断，统一读取原 RuntimeTask，只有 completed/failed/cancelled 是终态；其他已知
等待态保持 pending，缺失/读库失败/未知状态返回观察错误。settlement 自身也核对终态。
status/wait 保留 owner+roots 可见性检查，观察时再次核 task_id/kind/owner；同一次 wait
完成决策和输出用同一观察快照。stop 删除直接 kill、写 stopped、删日志及移除索引的
旁路，仅向 canonical task 提交取消并观察；取消已接受但未终态返回 Deferred，不声称
Stopped。原命令的执行证据与恢复引用保持不变。新增源码反例覆盖伪造状态文件、queued/
waiting_permission、取消已接受但未终态、缺失/数据库错误和异 owner；所有测试尚未运行。
Shell worker 的 completed 基于语义结果，不必然是进程 exit 0。退出码改为只读 canonical
`metadata.shell_execution.exit_code` 并作为独立 typed 字段传递；缺失保持 null，不由
label、状态文件或 completed 字样合成 0。补无退出证据与非零退出码呈现反例源码。

## 协议修正不等于外部受阻（2026-10-12 源码整改）

`turn.rs` 的 `recovery_required` 在已跳过执行的 recoverable protocol failure 分支设置，
由后续有效修正解除；它不是未知副作用的证明。`runtime/requirements.rs` 之前将此标志
与 pending unknown effects 合并并生成 Blocker。现将无未知 effect 的待协议修正收尾
归为 Incomplete（`protocol.recovery_pending`），保留原未完成要求；取消、Failed、
未知效果仍按原高优先级分支处理，不放宽完成证据，不新增自动重试。
新增 `unresolved_protocol_correction_is_incomplete_not_an_invented_blocker` 源码反例，
验证结果类型及不生成 Blocker 事实；不是完整 Core→Store→UI 纵向通过记录。未运行测试。

来源复核进一步发现：规划后继请求会清除 `model_source`，先前模型来源也不能跨
transition 充当本次终局依据。现 Core 在协议纠正未完成的收尾先发
`RuntimeDecisionFactObserved(Error(protocol.recovery_pending))`，再由 Runtime 来源引用
该事实生成 Incomplete；不合成外部 Blocker，不放宽 Store 的模型来源校验。
新增空来源与陈旧模型来源两条 Core 反例，以及 Store 原始事件绑定、seal 持久化、
空/缺失事实及跨 transition 模型来源拒绝回滚的源码用例。Application 已保留该事实
事件与 Core envelope。这些用例尚未执行，真实协议失败→终局→客户端展示仍待验收。

## A20：CLI 按自然语言改写客户端命令（源码移除，未验收）

静态复核发现 `main_parts/interactive.rs::normalize_interactive_navigation` 将三句中文
改写为 `/cd ..`，并在普通输入进入 slash dispatcher 前调用；原测试还要求该行为。
这使客户端根据句式决定能力路由，而非保留原文让模型按工具 schema 选择并由 Core
校验具体调用。已删除改写函数与调用；显式 `/cd` 命令保留。替换为真实 dispatcher 的
多语言、自然跟进与 search/current 歧义词透传反例源码，不新增排除关键词名单。
尚未运行 CLI 或真实模型验收；此处移除一条确定旁路，不代表所有客户端路由已审查完毕。

## A19：iOS 本地提交错误被当成终局回执（源码整改，未验收）

2026-10-12 静态核对 `Nola/Services/IOSApplicationRuntime.swift`：
`transmitEnvelope` 将 FFI 外层 `ok=false, recoverable=false` 转成自造的
`status=rejected`，而 `submitReceipt` 对这个 status 删除持久 pending。这个字典不是经过
client/request/protocol 身份核验的 Application command receipt；不可恢复的传输或协议
错误不等于业务拒绝，更不能证明原命令没有提交。另一条路径在检查 conflict payload
是否合法之前已退休 pending，损坏终局响应也可能丢失原请求身份。

整改契约：外层错误只影响本次观察，保持原意图；唯有真实、身份完整、结构有效的
committed/rejected/conflict receipt 才能退休。A08 committed 还必须匹配原 Run、resolution
和选择，不能只凭 HTTP/FFI 成功。先核原 receipt，无记录时只能按原 ID/载荷重放。
此整改不允许以新的时间、requestID 或 resolutionID 隐式重试未知副作用。

同一入口的存储还存在独立缺口：`fileExists + Data(contentsOf:)` 是无界读取；
`Data.write(.atomic)` 本身没有表达文件/目录同步、私有文件验证或跨实例事务锁。
这些不能因为名称带 atomic 就被写成已满足持久提交契约。需独立 journal 组件负责
传输意图的受限读取、原子持久写入和事务互斥，Application 仍独占业务事实；读取损坏
或持久化不确定必须保留并停止替代提交。

当前源码复核：新增 `IOSApplicationReceiptValidation.swift`，外层错误不再构造
rejected receipt；状态结构、归属、协议、错误与冲突字段先验证，再进入 pending
退休路径。A08 另核原决定身份。`recoverApplicationEnvelope` 先查原回执，只有明确
null 才原载荷重放。新增 `IOSPendingCommandJournal.swift` 并接四条 pending 操作入口，
事务锁涵盖 load/query/send/save；按实际编码核验 16 MiB envelope，限制整体 journal
大小，使用私有文件、受限读取、文件与目录同步。纯解码/重放与 journal 源 fixture 已补，
尚未编译执行。iOS 风险确认控件及冷启动用户发现已接入源码：按原会话执行 owner
发现待确认决定，跨会话仅提供导航，恢复不创建新 Run；独立查询不被 pending 写队列阻塞。
这不证明用户操作、真实 JNI/FFI、进程恢复与最终 seal 的完整闭环已通过。

验收要求：外层错误、错误身份、缺失 conflict 字段均保留原请求；重启恢复不制造新
授权；写入失败前无发送；两个 journal 实例不能互相覆盖未确认意图。源 fixture 与
真实 Application 故障注入分别记录，不用纯 decoder 测试冒充恢复闭环。

## A18：原生裸 Core 写入口仍可构造第二套运行事实（源码已移除，待整批验收）

2026-10-12 静态核对：Swift `Nola/Services/AgentCoreRuntime.swift` 的公开
`AgentCoreV2Runtime` 仍有 new/restore/submit/snapshot/branchCheckpoint；Android
`core/AgentCoreRuntime.kt` 同名类型仍直接持 native handle，并实现与 canonical runtime
相同的 transition 接口。`core/nola-mobile-bindings/src/lib.rs` 强链接裸 Core 导出，
Android JNI 仍有对应桥接。它们没有 Application 的原子提交、命令回执与持久派发边界。

目前检索到 Swift 显式构造器消费者位于 `AgentCoreContextContractTests`，Android 检索到的
生产引用主要是 isAvailable/toolPolicy；这不能证明所有导出无外部调用。两端生产可用性检查
还依赖裸 Core 的版本号，和实际 canonical Application 能力不等价。纯策略/上下文函数仍有
生产消费者，不能删除整个文件。整改边界见 [原生 ABI 契约](../../软件架构设计/模块/13-客户端投影/CLIENT_BOUNDARIES.md)。

所需纵向整改：分离无状态能力接口与 canonical 执行可用性；将原生裸 reducer fixture 改为
隔离 Application 提交/恢复；删除状态型 wrapper/桥接/导出及旧头文件声明，收紧门禁。
以上是整改前的定位证据。当前已删除 Rust `agent/ffi.rs` 及模块、crate 导出、C header 声明、
mobile 强链接项；保留纯函数和 Application ABI，不保留旧接口 fallback。
原 FFI 唯一错误分类断言迁到 recovery 的分类测试；新增 shipping surface 源码门禁防止
恢复裸接口，但尚未执行测试或二进制符号验收，不能仅凭源码搜索宣布所有产物已更新。

Swift 已删除可变 actor 与未消费的旧 dispatch DTO，纯能力拆为 ContextEngine 和
ToolPolicyResolver。聊天执行检查 Application ABI 及当前命令协议；Intelligence 通过新增
独立 ABI 1 getter 核对自身接口，不借 Agent 可用性。唯一裸构造器权限 fixture 改用隔离
Application 准入、Host 响应与持久派发链，保留同资源授权重用及异资源重新询问的断言。
此批代码、fixture 尚未编译/执行；也未更改或删除任何用户数据库。

Android 实施切片（2026-10-12，源码静态）：已移除 `AgentCoreV2Runtime` 全部可变句柄与
对应 `nativeV2*` Kotlin/JNI 桥接；没有保留旧类型别名或 fallback。纯工具策略独立为
`AgentCoreToolPolicyEvaluator`，聊天执行可用性改查 `AndroidApplicationRuntime`，同时匹配
Application ABI 1 与命令协议 49；`AgentCoreTransitionRuntime` 接口及 canonical 实现保留。
Personal Intelligence 使用独立 ABI 探测，不再借裸 Core 执行版本推断自身能力。Android
源测试没有裸 reducer 构造器 fixture，因此未删除原覆盖；新增边界测试源码防止重新暴露
裸 JNI 及接受错误 Application 协议。尚未运行 Android 构建/测试或二进制导出符号检查；
共享 Rust/header 导出与 Swift 删除由同一整改主干分别接线，不能据本切片宣称全部完成。

## A01：真实 Mac 收尾死循环（代码已整改，历史会话与整链路待验收）

证据：只读检查生产 `nola-v4.db`，Run `67BD3B13-04D1-4481-89B8-C39A5284BF4A`，
会话 `52CB0A1E-EDF2-4C43-B0AD-927A69BA48F3`，2026-10-11 11:12:47–11:29:17。

- 11:15:52 `context.compacted` 成功。四次摘要均 `no_request_snapshot`、cache read=0。
- 工具序列 144 的 Shell 使用 `mutate_and_validate`，没有声明变更范围，仅有验证目标。
- 最后一次 Shell 验证 sequence=188，exit=0；sequence=189 仍是 required。
- 持久 checkpoint 的 engineering 有 `unscoped_mutation_count=1`，已知待验证资源/版本集合为空。
  checkpoint body 是 replay base，不能把其 revision 当作末尾事件 cursor；这里与末尾 trace 的
  7 次变更、5 次验证相互印证，完整最终状态仍需通过正式恢复接口验证。
- 三次模型请求 input_tokens 均 100,970，均正常 stop；最终 run.failed 为 PendingObligations。
  相同 token 数不是 body 字节相同的证明，不能据此宣称已验证前缀逐字节相等。

整改前源码复核：`core/src/agent/turn.rs` 的 `record_unverified_mutation`、`apply_successful_validation`、
`model_instruction`；`core/nola-kernel/src/model_completion.rs` 的 PendingObligations 分支；
`core/nola-domain/src/model_completion.rs` 的 ModelCompletionFacts。

2026-10-11 初始核对：Kernel 普通文本分支当时调用 `retry_or_fail(PendingObligations)`，
计数到 3 时 `DiscardAndFail`；不是仅存在于历史日志的已删除行为。独立 NextTurn 与持久用户等待
已分段整改，但都没有替换该普通证据不足出口，也没有重跑上述生产会话证明修复。
矛盾：指令允许“报告未知范围造成的证据不足”，完成判定仍以 validation_required 拒绝文本结束。
模型无法通过正确说明事实退出。需要独立的证据受限结果与合法停止转换，而不是把 unknown 当 verified。
当前整改：上述普通债务重试与失去生产用途的失败枚举已删除；正文及执行后带债务的空正文
进入统一结果仲裁。缺验收保留 Incomplete，真实恢复／未知效果阻塞保留 Blocked；有效审查
的继续执行／修订预算耗尽同样不再伪造协议 Failed。测试源码已补，尚未重跑上述生产会话，
不能将代码修改写成 A01 验收通过。
新契约：LC-03、LC-06、MOD-06。验收：E2E-01、E2E-02。

## A02：设计文档权威冲突（修订中）

`Doc/历史任务/实施与审计/ARCHITECTURE.md` 禁止 Trace 成为业务事实源；`Doc/README.md` 及
`Doc/历史任务/实施与审计/AGENT_TRACE_ARCHITECTURE.md` 又要求以 Trace 驱动完整聊天恢复。
`../历史任务/实施与审计/ARCHITECTURE_V2_MODULE_CONTRACTS.md` 已写未知范围必须保留、可报告限制，却没有把这一出口
贯通 ModelCompletion、Store、客户端。不是简单“没有枚举”：Domain 已有 Blocked/Incomplete/NeedsInput，
但普通验证分支仍主要消费三个布尔值。
整改：收敛权威索引，Trace 旧文档降为历史设计；模块契约不得以完成状态布尔值替代证据记录。

## A03：热分叉验收范围不足（部分改善，不代表历史会话恢复通过）

实现：`cli/src/llm_parts/request_snapshot.rs`、`cli/src/agent_host_driver_parts/compaction_summary.rs`。
最新生产 Run 没有可用快照，不能引用隔离新会话的 3/3 热分叉覆盖率证明生产会话已修复。
本轮将 CompactionRequestWitness.restore 的 Option 改为分类 Result，区分不存在、I/O、超限、
外层损坏、会话/配置不符和内层请求无效，并接入原有 compaction_request_metrics fallback_reason。
读取本身限制为 32 MiB + 1 判超限，避免仅凭 metadata 后无界读取。
新增分类 fixture 和损坏快照冷恢复的 mock-provider 路径；结果见 VERIFICATION，
内层请求无效、写入失败和真实历史快照生命周期仍需继续核对，不算 A03 全部解决。
真实截断续写专项已通过；不等于长历史、多工具执行、收尾与 UI 的整链路通过。
验收：E2E-03、E2E-04。

## A04：已有可复用基础（存在代码，整链路符合性尚需复验）

| 能力 | 当前依据 | 不应推导出的结论 |
|---|---|---|
| 原子 admission | `core/nola-application/src/assistant_turn.rs` | 所有客户端均无旁路 |
| transition 去重、事务 | `core/nola-store/src/conversation_transition_store.rs` | 外部副作用可以 exactly-once |
| delivery lease | `core/nola-runtime-driver/src/lib.rs` | lease 过期等于操作未执行 |
| 工具 effect/check 回执 | `core/nola-host-protocol/src/tool_result.rs` | Shell 成功等于语义验证成功 |
| 任务结果枚举 | `core/nola-domain/src/model_completion.rs` | 所有受阻分支都能正常封存 |
| canonical 使用量 | `core/nola-store/src/committed_run_context_usage.rs` | 所有协议统计口径均已一致 |
| Mac Application port | `NolaMac/Services/MacApplicationPort.swift` | Mac 每个页面都只消费 canonical 投影 |

## A05：实际依赖与运行入口（已核对声明图，调用图待继续）

MCP 恢复观察边界（源码整改，尚未执行验收）：`McpExecutionTaskLease::acquire`
不再把任务查询错误降为不存在。先按 workspace 内的 owner + parent effect 查找唯一原任务，
再核对 server/tool/read-only 和可重试状态；重复候选、损坏元数据、运行中或终态均失败关闭，
不选“最新任务”或创建替代身份。未来 retry deadline 保持原 task ID，由 fair claim 判定就绪。
claim 期间观察失败、任务消失、状态变化都停止，错误保留 task/effect 身份。
隔离数据库 fixture 已补查询故障注入、延迟 retry、运行/终态、元数据和重复身份反例，未运行。
首次观察与创建之间的竞态由现有 canonical Store 创建事务及主键仲裁：新 MCP task ID
取 canonical workspace、可空 owner、parent effect 的规范 JSON 元组 SHA-256，不包含命令文本，
路径身份解析失败直接拒绝。相同 effect 的并发首次调用落同主键，最多提交一条任务及 created 事件。
参数仅持久化 digest；恢复必须匹配 server/tool/read-only/参数 digest，不补猜测旧记录。
创建失败后读取并核对原归属与载荷，但当前产品端口只返回 String 错误，因此即使完全匹配也保留
原创建错误并停止，不把任意 DB 错误转为成功；后续显式重试可观察并复用尚未执行的只读原任务。
已补双 caller 争用、相同载荷重放、冲突载荷和 owner/元组区分源码用例；无 schema 或外部协议变更，
不宣称远程 MCP 物理副作用 exactly-once。上述源码仍待最后统一验证。

2026-10-11 执行 `cargo metadata --offline --no-deps --format-version 1`，共 19 个 workspace package。
下表保留该次历史快照，仅是 workspace 内的普通依赖（不含 dev-dependency）；不等于执行调用图，
也不能证明模块内部无 I/O。当前声明图以 [DEPENDENCIES](../../软件架构设计/总体设计/DEPENDENCIES.md) 为准：
2026-10-12 源码中 Runtime Driver 已改为仅依赖 Domain、Host Protocol，Core 只在其 dev fixture；
该批尚未编译/测试。不要把下表旧边当作当前应保留的兼容接口。

| 模块 | 实际关键依赖 |
|---|---|
| nola_domain | 无 workspace 依赖 |
| nola_kernel / nola_protocol / nola_store_port | nola_domain |
| nola_host_protocol | nola_domain |
| nola_core | agent_engine、domain、host_protocol、kernel |
| nola_agent_engine | domain、kernel、host_protocol（唯一Agent/checkpoint/trace实现；旧adapter已删除） |
| nola_application | domain、kernel、host_protocol、protocol、store_port、agent_engine、measurement |
| nola_store | domain、kernel、store_port |
| nola_application_bootstrap | application、store |
| nola_runtime_driver | domain、host_protocol（生产）；engine仅dev fixture |
| nola_runtime_supervisor | protocol、service_client |
| nola_service_client | protocol |
| nola_service | application、bootstrap、domain、protocol |
| nola_mobile_bindings | application、bootstrap、core、domain、kernel、model_runtime_host、protocol |
| nola_cli | code-memory、application、bootstrap、core、credential_host、domain、host_protocol、protocol、driver、supervisor、service、service_client、store_port |
| nola-code-memory / nola_credential_host / nola_model_runtime_host | 无 workspace 依赖 |

三个 Rust bin：`core/nola-service/src/main.rs`、`cli/src/main.rs`、`cli/src/nola_agent_host.rs`。
本轮已移除 `nola_agent_host.rs` 的 `include!("main.rs")`：两个 bin 只分别调用
`nola_cli::entry::run_cli()` / `run_agent_host()`，共享模块声明与运行层由 `cli/src/lib.rs` 承载。
内部 `EntryIdentity::Cli/AgentHost` 由编译入口注入，Host 命令限制及进程默认面不再读取 argv[0] 文件名；
重命名、软链接或继承 `NOLA_CLIENT_SURFACE=cli` 不得把 AgentHost 变为 CLI。
调用方的 Mac/Web/iOS/Android provenance 和凭据验证仍是独立契约，不能由这个枚举替代。

仍未完成：共享 library 仍包含 terminal UI、Web、Host adapter 与 `main_parts` 分区，
Host 尚未获得不依赖 CLI 产品模块的干净编译边界。本次是明确入口/链接所有权及命令准入整改，不宣称已完成全部模块解耦。
共享层仅公开两个入口函数，具体模块保持私有；现有 debug `#[global_allocator]` 只在 library 定义一次，
两个 bin 不再各自包含一份。静态检索未发现 `CARGO_BIN_NAME` 假定，原单元测试从 bin 迁移为 `--lib` 目标。
已修改 Windows CI、Worker replica 与 CLI/Web/Mac alignment 脚本选择器；历史基线记录保留旧命令，不冒充新目标已验收。

内部 Application Service、Agent task worker、Shell task worker 仍按原显式协议在产品命令分派前启动，
保留原先先于 Host 环境守卫的顺序；catalog、memory worker、macOS background scheduler、memory admin 的显式入口也保留。
这些协议各自验证请求/身份，不能误称为 CLI 用户命令，也不能以文件名区分授权。
新增 `executable_entry_identity` 真实子进程测试源码：复制重命名 Host 后仍拒绝 run/init/wiki/web，
同时仍进入显式同构建 service 协议。单元契约覆盖 typed Host 的 stdin/server/session 模式。
本轮未执行编译或测试，最终验收应包含 `cargo test -p nola_cli --lib` 与
`cargo test -p nola_cli --test executable_entry_identity --test service_schema_preservation`。
旧legacy adapter已删除；Agent/checkpoint/trace唯一实现实际移至独立engine，Application通过
消费式AgentReducerPort提交候选/恢复/checkpoint。Core仅留原生适配，门禁禁止engine回连Core/Store/Application；
typed parts/protocol等投影读能力尚可进一步细分，源码实施不代表编译和端到端验收完成。
`nola_mobile_bindings/src/lib.rs` 同时导出 Application submit 与多个底层 Core ABI。
仅发现双入口不能断言客户端正在双写，但必须逐端追踪实际调用和可见性，不能只检查 Cargo 无环。

整改决定：先按 MOD-02/03/05/13 建立具体用例的唯一调用路径，再移除过宽 re-export、
Host 的剩余 CLI 编译依赖及不用的直接 ABI；不靠重命名 crate 宣称完成模块化。

## A06：规范没有完整进入版本管理（已发现，收敛中）

`.gitignore` 的 `*.md` 规则忽略了许多现有设计文件，包括旧文档索引与 Trace 架构。
仅工作区文件存在不代表契约能跟随代码发布、审查或恢复。
本次为 `Doc/architecture/*.md`、`Doc/README.md`、`Doc/历史任务/实施与审计/AGENT_TRACE_ARCHITECTURE.md`
添加精确例外，未放开全仓 Markdown。其余被引用的规范仍需按权威索引逐项核查版本管理覆盖，
不能自动将所有历史报告提升为当前规范。

## A07：受阻结果在封存链路中丢失（当前链路已接线，尚待统一验收）

原始反例映射见 [收尾纵向契约](../../软件架构设计/模块/02-Core与状态机/COMPLETION.md)。整改前不只是普通分支没有 Blocked：
`PublishReviewedCandidate` 已得到 blocked，却显式生成 Failed/TurnFailed；
`reviewed_blocker_is_published_without_marking_task_successful` 目前断言的就是 Failed。
Application 两条提交路径忽略 TurnCompleted.outcome；不可变 terminal seal 无任务结果字段。
Task bound-run reconciliation 一般复制 Run status，Skill terminal 循环把 TurnCompleted 视作 Complete。
因此只把 blocked 改成 TurnCompleted 会制造下游假成功，不能作为修复交付。

整改边界：同批贯通 Domain、Core、Application 双路径、Store seal、Task/Skill 和投影；
当前已有 `nola-domain/src/run_result.rs` 的互斥六类结果、结构校验与 canonical digest；
精确决定事件/输入来源索引、独立后继 Run、用户等待也已落地（见 TERMINAL_IMPLEMENTATION、
WAIT_PROTOCOL 和 VERIFICATION 的分项范围）。当前 `PublishReviewedCandidate { blocked: true }`
已进入共同仲裁，不再无条件设置 Failed；仅模型声明受阻但缺少原 blocker 时为 Incomplete。
生产 V2 seal 已绑定完整 RunResult 与 digest，Task/Skill 与客户端正在最后静态审计。
数据库引用校验和结果决定来源见 STORE_RESULT_BINDING；未执行当前版本的统一验证，
不构成 A07 验收完成，也不能自动转换缺失结果。
结果与证据引用进入不可变 seal，重放/远端/父子任务共享同一事实。
现有 checkpoint 中存在 completion_outcome 不足以证明 canonical result 已持久化供各端消费。
验收：E2E-01/02/07/09；当前尚无通过证据。

接线前新增核对发现两项契约冲突，详见 [要求身份绑定](../../软件架构设计/模块/02-Core与状态机/REQUIREMENT_ADMISSION.md)：
canonical Requirement DTO 的 Run/Conversation 与自身事件 cursor 不能由模型或尚未封装事件的 Core
自填；应绑定真实事件信封与 Turn owner。既有应用序号是 Turn-local，同 Run 多 Turn 不得取其
最大值作为完整 Run 水位，需精确应用前缀及摘要。当前已实现
`run_input_frontier::audit_for_decision`，由 `run_requirements` 在 scope、assessment、result
入口调用；封存时 `seal_result` 重读原始来源及评估集合。来源索引不是第二份可改写的结果。
这修正了“该查询尚未实现”的旧记录，但实现存在不代表已经验收：原 Host 响应替换、
缺失来源、晚到输入与未知效果的真实纵向反例仍按 STORE_RESULT_BINDING/VERIFICATION 核验。
Domain 结构覆盖检查仍不能单独替代数据库来源校验，也不关闭 A07。

## A08：未知效果的恢复与 Run 级出口（共享命令与原生接线已改，跨端验收未完成）

`EffectDeliveryAttempt` 已区分 replay safety 与 OutcomeUnknown；Application 恢复不会自动重跑
已开始的非幂等操作。这个保护必须保留，不能当作“卡住原因”删除。
初始审计时 `plan_run_recovery` 只有 pending_effect_count，存在 pending 就 Preserve；
无法表达其究竟是等待权限、可 Resume 还是不可核对的未知效果。
当时 `authorize_unknown_effect_retry` 只提供 ConfirmedNotApplied，未覆盖已生效核对和保留未知的受阻封存。
当前 `run_requirements::seal_result` 已允许结果明确保留未知 effect：
Cancelled/Failed 的 unknown refs，或 Blocked 原始 blocker 的 source refs。
每项必须对应真正开始、已结束且无 settlement 的最新 OutcomeUnknown attempt；
活跃租约、未开始的请求、缺引用或普通 pending 都不能借此封存。未知效果不被 ACK，
sealed Run 不再派发。该 Store 出口与恢复命令/用户核对入口并非同一件事，不能把安全
“不重跑”算成产品已经可恢复/可退出。详见 [恢复契约](../../软件架构设计/模块/05-执行与恢复/RECOVERY_AND_AUTHORITY.md)。

2026-10-12 源码状态：共享 `InspectUnknownEffectRecovery` / `ResolveUnknownEffectRecovery`
已绑定精确 Run、配置摘要、attempt/revision/fence；提供风险重试、保留未知受阻、核对原
成功 Host receipt 三种不同操作。后者不接用户或模型声明伪造成功。核心事务、派发 CAS、
seal 与 resolution receipt 已接同一提交边界，热 Core 回滚恢复仍按本目录的恢复契约。

CLI `/recover`、Mac 会话内确认和 iOS 会话 owner/确认入口已接源码；Android 的 typed
Runtime/journal、exact Run 恢复与 Compose 确认，以及 Web 专用源 Host 控制接口和
IndexedDB 决定记录均已接源码。Web service 请求身份只绑定原 owner/request ID，不能把
决定载荷纳入身份哈希而绕过同 ID 改载荷冲突；查询已有 receipt 也须核对原载荷。
Mac/iOS/Android 的待确认请求都必须保留原身份，坏回执不能退休；查询不被未确认写入
阻断，不代表允许新写超越它。Mac 终态只查原回执，不重发决定。客户端进度不是 seal，
恢复决定传输文件不是第二份 effect ledger。各端证据链和实际 UI 均未统一验收，A08 未关闭。

另外 permission transport 的 resolve 用 bool 表示更新是否发生，已决议和不存在都为 false。
静态追踪 `permission.rs::execute_permission_request_command` → Store
`runtime_task_permission_store.rs::resolve_permission_request` → CLI
`task_runtime_parts/supervision.rs`：调用方将 false 统一显示为 no longer pending；
`permission_repository_e2e` 也明确断言同决定第二次提交返回 false。这个结果不能区分
相同决定重放、相反决定冲突和请求不存在。外层 command receipt 可处理同传输 ID 重放，
不等于新的传输 ID 对同 permission request 的语义重复已具有明确结果。
后续应在原事务内返回绑定原请求的 typed resolution outcome；不能在客户端看到 false
后自行补授权或伪造成功。此证据说明恢复语义不完整，不直接证明越权。
验收：E2E-05/07/09/12。Application MemoryMaintenance 测试不等于真实 SQLite/Host 崩溃验收。

## A09：Goal 来源观察与语义验证混用（版本化接受已接线，真实验收未完成）

2026-10-12 补充入口审计：Goal `operation_id` 目前在 reducer 中仅做非空校验；
`AcceptCompletion` 与外部 `ApplyAudit` 还通过各自专用分支进入投影，不能假定
`GoalProjection::apply` 包含所有命令。CLI idle 路径重新生成 committed_at_ms，
通用 service client 的 `execute` 默认分配新 request ID。因此 **Goal operation ID
不能被当作已经证明的跨重启幂等键**。已有 Application received-command receipt
是 canonical 提交事实，应先审查其实际调用链覆盖范围，不新增客户端 Goal 真相库。
待闭合验收：原身份/载荷冻结、丢 ACK 后 receipt-first、相同 ID 改载荷冲突、
重放不重复增加 Goal revision，且 active/idle 两条路径一致。目前已交由独立模块任务处理，
尚无运行证据，不列为通过。

Goal 已有约束、工作记录、来源 Turn 和 Task 引用；不应再创建另一份“防压缩丢失”的状态。
`observed_goal_evidence` 从原文筛选所属 Turn 的工具结果及 Assistant 正文，
整改前 `accept_goal_completion` 将引用转为 `independently_verified=true`；类型注释却要求独立检查。
当前已改 source_verified 与明确 ModelJudgment；来源存在性不再表述为独立验证。
source Turn 必须 Completed，且明确 Answered/ActionCompleted 与当前 Core result decision 一致；
拒绝缺失或未完成结果，不从生命周期标签猜成功。已补 checkpoint 恢复后拒绝且 Goal
不被修改的反例源码，尚未运行。child Task 仅检查 terminal、逐 requirement 接受身份/版本等
仍需与结果来源贯通，不能把这个入口约束当作完成整条链路。
本地构造 GoalAuditReport 不代表额外调用 reviewer，不能误判为强制模型独立审查。

整改契约见 [Goal 与 Task](../../软件架构设计/模块/10-Goal任务与子代理/GOALS_AND_TASKS.md)：证据来源与 requirement acceptance 分开，
接受方式明示、约束来源不可覆盖，子结果不能直接结束父目标。
后续实际修改：移除 GoalEvidence.independently_verified，改为 source_verified；
GoalAuditFinding 必填 assessment_method。普通接受写 ModelJudgment，不再自称独立验证。
保留未核验来源拒绝；缺方法拒绝解码，无旧字段兼容。Core Goal 单元 9 项、三组契约 123/2/1 通过，
并断言序列化字段、明确评估方式、无额外 Host 审查请求以及 checkpoint 恢复一致。
上述测试数量是此前执行记录，不覆盖当前版本化接受变更。当前 `core/src/agent/goal.rs` 与
`goal/work.rs` 已增加逐要求 semantic_revision、带 assessor/来源/要求版本/合同身份的接受历史，
条件修改原子失效依赖闭包；子计划只提供 proposed_checks 和 additional_execution_constraints，
不得替换用户条件。外部审计不允许自行声明 Completed/EvidenceInspection 获得接受。
当前新增结构化 requirement acceptance_policy、真实 ExecutionEvidenceObserved 的 typed checks
绑定与逐 claim 的子结果替代引用。模型计划不能降低原 policy，普通正文不能满足 execution_checks。
来源投影与接受历史随原 Goal checkpoint 保存；CLI 接受拒绝以 Partial/Incomplete 收束，不无限重试。
本批源码尚未编译或运行；真实子任务失败补做及全链路验收仍未取得通过证据。
方法标签不代替真实 check 绑定；完整 A09 仍未关闭，既有用户数据未迁移。
`work_archive` 已标注 baseline/replay_pending；必须正式重放，不能把导出的 goal.md 当最新真相。

## A10：自动化结果由客户端进程退出码决定（生产入口整改中，验收未完成）

整改前 MacAutomationStore 执行路径以 exitCode==0 赋值 succeeded，再由 finishRun 向 Application 传
succeeded/failed；finishAutomationRun 用 try? 吞掉提交错误，随后仍更新活动记录及发送通知。
Application AutomationRunCommand::Finish 接受该 status，未在此边界引用 canonical RunResult。
这和 Task 必须从绑定 Run seal 收尾的契约不一致，不能仅改 UI 文案或打印错误。
整改要求见 [后台与数据契约](../../软件架构设计/模块/12-记忆数据源与自动化/BACKGROUND_DATA.md)：occurrence 绑定 Run，结果源为 seal，
通知在提交后发布，进程退出/失联只作为恢复观察；E2E-11 必须增加这些故障点。

当前源码已将 Mac 初次执行接入共享 Host runtime，移除 Store 私有 Process 执行器；
精确 occurrence 绑定恢复原 Run，结果读取 canonical execution receipt。iOS 也已改用共享
准入与 exactRun 恢复。详见 [Mac 实施记录](../实施与审计/AUTOMATION_MAC_IMPLEMENTATION.md) 和
[iOS 实施记录](../实施与审计/AUTOMATION_IOS_IMPLEMENTATION.md)。旧验收脚本消费者、隔离凭据 fixture
及等待/通知恢复仍在收敛；未编译、未执行真实验收，不将 A10 记为关闭。

Memory 已有自动候选质量校验与 profile 冲突检测；DataSource 已有结果分类和 read_token 校验。
尚未证明后台来源身份、撤权竞态和所有客户端入口完整，因此不列为全模块通过。

## A11：CLI 进程内写命令回执覆盖不完整（已整改入口，验收范围有限）

CliApplicationPort::execute_observed 的 InProcess 分支原先只对 editor 命令及 Host response
使用 execute_received_command，其余直接 application.execute；Service 路径则统一通过命令边界。
本轮移除客户端 requires_editor_admission 子集列表，使用 ApplicationCommand::is_read_only_query
决定是否走回执：写命令统一事务回执，只读仍直接查询。
补真实 SQLite 测试，检查写入 receipt committed、数据库重开可读取，以及查询不增加回执。
这不证明所有重发/失联场景已通过，也不解决其他专用提交函数和所有端的潜在旁路。
接线与后续验收见 [客户端边界](../../软件架构设计/模块/13-客户端投影/CLIENT_BOUNDARIES.md)。
9 项边界测试通过，但扩大后的 CLI 全量基线有 15 项失败、4 项忽略，另有默认栈溢出；
详见 [CLI 基线](../实施与审计/CLI_BASELINE.md)，必须逐项归因，不能视作整体通过。

## A12：Task 项目身份读写不对称及通知提前确认（已分段整改，整链未验收）

重跑 `single_terminal_result_primitive_preserves_independent_ui_delivery`，仍是 0 != 1。
只读失败测试的隔离 SQLite 得到 completed、unread=0、model_notified=true、ui_notified=false；
实际 project_root 为 `/private/var/...`，测试查询为 `/var/...`。
create_runtime_task_with_event 已规范化路径，list_runtime_tasks 却原样 SQL 精确匹配，
因此这项失败不是模型消费清除了 UI 标记，也未证明由 CLI 回执入口修改引起。
本轮共用 project root 规范化函数，增加真实 symlink + SQLite 重开 + 无关项目隔离测试。
其余调度测试是否同根因，须分别运行后确认，不凭相似现象勾掉失败项。

沿通知链另发现：take_runtime_task_notifications 在返回前持久 ACK，调用者随后才清理编辑器、
处理授权或打印。任一步失败均可能丢通知；terminal_notice 在 renderer 拥有终端时仅 commit_history，
不能把 stdout.flush 等同于帧已展示。目标输入、确认时机及至少一次交付语义见 GOAL-05。
发现时尚未改变该协议。后续已修 Agent 的只读准备与输出交付后精确 ACK，
Shell 仅修交付时机与终态冷恢复，仍缺 consumer 协议；见 TASK_DELIVERY_IMPLEMENTATION。

验证：Store Task 30 项通过；CLI task_runtime::tests 10 通过、1 失败。
UI 独立结果测试通过，跨会话测试进至手工 claim 失败；隔离库已有 cli-launch worker。
running_task_count 先 reconcile_supervised_tasks，再计数，且 supervisor 工作区配置清空 session_id。
这不自动证明越权（派发另有准入校验），但证明展示查询会触发生命周期处理和工作进程启动。
必须将显式 supervisor 驱动与纯查询入口分开验证，不能依赖打开 UI/调用 count 才推进队列。
详细命令与范围见 VERIFICATION 的 Task 复核，完整 CLI 尚未重新验收。

后续整改：删除 count/list/manage list-status/pending context 中的隐式 reconcile，
删除 read_task/render_task_detail 中的通知 ACK。原来跨会话测试的手工 claim 重新通过；
新增查询后 attempt=0、queued、worker_id=None 以及详情读取不改变通知 metadata 的断言。
扩大到 task_runtime:: 全模块共 38 项通过，包含资源 supervisor 与权限恢复测试。
这一轮查询拆分当时尚未修复通知提前 ACK；后续通知修复见上，仍未证明无 UI 的所有 driver 都持续推进。
具体驱动点、deadline wake、错误与背压契约补入 REC-03。
本次完整 CLI 复测为 1615 通过、11 失败、4 忽略，未完成整体整改；
新增完整套件权限恢复 lease expired 与单模块通过的差异仍待定位，详见 CLI_BASELINE。

## A13：Turn 结束、后继工作与 Run seal 边界混合（独立后继已实施，完整结果封存未完成）

初始审计时，complete_turn 在 CompleteAndStartNext 分支会同批发 TurnCompleted 并 start_turn；
Application 两入口各自只保留 terminal_status/terminal_turn_id，且以 successor_effects 非空
标记 terminal_cleanup_pending。successor 不仅可能是 host.cancel，也可能来自下一 Turn。
Assistant status 可先结束，Run status 则推迟；这是两个边界，不能靠新增一个状态字符串修复。
Store seal 从 runs.status 取三态，CheckpointEvidence 只检查 session/cursor/task projection，
没有读取 TaskCompletionOutcome；缺结果不能在 seal 阶段从模型正文补猜。

新实施规格见 TERMINAL_PROTOCOL：生成结束、结果决定、收尾协调、不可变封存分开；
后继用户输入明确独立 admission，结果与 requirement/evidence frontier 纳入新版本 digest，
两条 Application 入口和 checkpoint fallback 共用同一 builder。
当前是源代码证据及目标决策，尚未证明某个生产 Run 因混合发生重复副作用；不得扩大结论。
新协议、所有 reader、Task/Skill 与故障注入仍需同批实现，不能以文档完成代替纵向整改。
前置重构已统一两条写入口的 checkpoint 生命周期判定，去除另一入口独立 core.turn_status 分支；
Application lib 288 项通过。完整业务结果仍未进入 seal，A13/A01 保持未解决。

后续整改已删除同 reduction 自动 start_turn：FIFO 排队输入使用原 admission/submission，
由 Application 独立准入新 Run；Store 校验旧 seal、无未结 effects、checkpoint CAS，并将后继绑定、
消息、输入应用索引与 outbox 原子提交。Core、Application、Store 和 CLI 热执行/冷恢复专项证据
见 TERMINAL_IMPLEMENTATION。此处只解决后继所有权边界，不代表 RunResult/V2 seal 或多端验收完成。

## A14：生产依赖图及自动门禁范围（已核对，迁移未完成）

Cargo metadata 核对工作区 19 个包，生产图与 dev fixture 分开；Application/Runtime Driver
具体 Store 依赖为 dev，不应误报生产旁路。最终源码已删除legacy adapter并实际抽取唯一engine，
其生产依赖无Core/Application/Store；Application已接typed reducer/recovery/checkpoint端口。
CLI/mobile_bindings 仍直接引用 Core，实际用例调用需要继续收敛，不能靠无环图声明职责唯一。

扩展现有 CLI dependency_boundary integration target，为值/决策/接口/Store/Application/Driver
添加依赖方向检查及违规注入；现有 CI 已执行该 target。原 Domain 架构检查仍保留。
本地两组分别 3/14 通过，范围和排除项见 DEPENDENCIES；未证明 std I/O/FFI/所有调用点纯净。

## A15：Journal 测试混淆初始游标与内容提交（专项回归已通过，非全端验收）

window_heads 明确要求实际 assistant_journal_heads 且 sequence>0；准入返回的 sequence=0
是读起点。两项 CLI seed 测试在模型 attempt 开始但正文未提交时就期望 seed committed heads；
Web HTTP 测试也直接使用准入 cursor，随后却断言 records 非空。
这些断言不能证明生产丢正文，也不能通过为占位消息制造假 journal 修复。
现改为真实 claim/start/model delta 后核对非零 cursor，再执行原来的丢队列/重新订阅/HTTP 回放断言，
Web 另断言准入后无正文时 window heads 为空。没有更改 Store 的实际发现规则。
加入真实正文后，旧 HTTP fixture 在接收阶段暴露 ManifestUnavailable：它一直传空 chunks，
没有执行内容准备协议。现经已认证 record_chunk HTTP 读取缺口、Stage 持久化前缀、
准备后重开接收库，再 Receive 提交。最终再次重开，断言重复记录不重复应用且确切正文可读。
这没有放宽 Store 的完整性检查，也不是对生产 ManifestUnavailable 的通用修复声明。
输入输出、准备与发布边界写入 CLIENT-04。

2026-10-11 本轮：隔离 NOLA_HOME、mbx 缓存、RUST_MIN_STACK=16777216，执行
`nola_cargo test -p nola_cli --bin nola journal_ --quiet -- --test-threads=1`，
20 passed / 0 failed / 1610 filtered，测试耗时 9.79s；`git diff --check` 通过。
覆盖通知丢失/订阅恢复及本地 HTTP→独立 SQLite 接收，不覆盖真实 Mac UI、跨设备网络、
默认栈/并发或全 CLI 剩余失败。该耗时不是性能基线。

## A16：用户输入等待被当作完成（持久链路已改，客户端整链待验收）

整改前 `tool_driver.rs::pending_user_input_message` 将 unavailable/cancelled 同路处理；工具汇合时
设置 NeedsInput 后调用 complete_turn，事件是 TurnCompleted。Application 双入口均忽略 outcome。
`agent_core_host_contract.rs` 中 unavailable-question 和审查 clarify 的现有断言确认了这一行为，
因此旧测试通过不能证明等待语义正确。普通成功分支也未普遍设置 completion_outcome，
不能简单要求所有旧 Completed 事件已有成功结果，再给缺值随意补默认。

Mac Host 提问入口 `cli/src/tools/parts/background.rs::prompt_desktop_host_question` 使用 PID/原子序号
和最长 24 小时进程内等待。该段代码没有展示业务 wait 事务；不可据此断言全项目没有其他存储，
但它不是可脱离原 Host 恢复的完整权威链路。`all_terminal` 分支只读取本次工具 pending question，
多工具回执顺序需要新增正反顺序测试核实，当前不宣称已复现丢问题。

当前 `tool_driver` 对 `request_user_input` 直接生成有 schema digest 的 UserInputWait，
工具进入 AwaitingUserInput，而不是先创建进程内 Host 提问等待。Application
`user_input_wait::{prepare,applied,respond}` 承担回答准入、回执与 Core 提交，测试源码
覆盖 waiting_input 与开放 wait 恢复。WAIT_PROTOCOL 记录实际实现与剩余客户端路径。
旧行为和旧断言保留在本节作为反例背景，不能继续描述为现行已核对实现。
多客户端竞争、断连、Decline、多路汇合及真实 kill 点仍需统一验收；
不能只把 NeedsInput 改为 Failed、放宽 completed，或禁止封存后让执行永久悬挂。

## 审计覆盖与下一顺序

Goal 引用稳定性补充（2026-10-11）：apply_audit 原未校验报告观察的 Goal revision，
已增加必填 expected_goal_revision 并同步 Core/CLI 生产构造点。register_artifact/register_fact
原先 insert 后检查旧值，重复请求虽报错却替换了记录；改为先检查身份再插入。
新回归核对工作更新后旧报告经序列化恢复仍拒绝且原值不变，以及重复注册不覆盖任何状态。
这是 Goal 入口约束整改，不代表逐要求版本化或 RunResult 的数据库引用链已完成。

## A17：已提交后的派生文件归档阻塞回执（已解耦，本地分层复测已取得）

整改前 `connection.rs::commit_write_unit_of_work` 在 SQL COMMIT 后同步调用 work archive publisher；
归档失败只保留 dirty/错误输出，原命令仍成功，因此等待归档并不提供双介质原子成功保证。
默认关闭的线程局部分段采样已区分 disclosure、SQL commit、post-commit archive，未改变提交语义。
本地固定 Host/真实 Application+SQLite 的 60 样本实验中，首次 1 KiB 正文回执 median 为
新库 51.066 ms、复用库 53.169 ms；归档子段 median 为 41.925/44.191 ms，SQL 为 0.972/0.834 ms。
该证据支持优先整改同步归档边界，不证明其中具体 fsync 占比，也不是生产 UI/模型首 token 测量。
原始数据、失败/缺失口径见 PERFORMANCE_BASELINE 的分段实验；无优化前后性能提升声明。
异步化须先有持久待发布版本、单一执行所有者、防倒退及崩溃恢复，不能删同步后只依赖内存线程。

schema 62 已加入源事务内 target generation、发布后 published generation、持久失败退避；
普通提交仅唤醒后台 publisher。文件锁先于快照取得，确认精确快照版本，避免旧发布覆盖新版本。
锁占用不阻塞业务回执、无新消息重试、进程退出后重开恢复已有专项通过。v4 同口径 60 样本中，
首次正文回执 median 为新库 6.929 ms、复用库 5.325 ms，p95 为 24.315/36.477 ms；
仍有后台竞争长尾。该结果仅覆盖固定 Host 的本地 Application/SQLite 链路，不代表真实模型或
Mac UI 首 token。完整 Store 最终回归 374 通过、0 失败、1 忽略；不把局部分层测量宣称为端到端验收。

## 整体工作包状态

| 工作包 | 状态 | 完成证据要求 |
|---|---|---|
| 总体边界、现有规范冲突、最新真实失败 | 已取得直接证据 | 本页来源与只读运行数据 |
| 状态/模块契约初稿 | 已编写，待逐模块复核 | 类型、错误、持久化与可达出口完整 |
| 所有 crate 依赖及运行入口 | 声明依赖/三个 Rust bin 已核对，跨端调用图未完成 | 实际 Cargo graph 与各端入口，不凭文件名推断 |
| 权限、工具、模型、上下文生产接线 | 权限/取消/恢复边界已映射，逐端及 Provider 接线待核对 | 顺序/并发/委托/重启四路一致 |
| Goal、Task、subagent、Skill、memory、automation | 关键入口均已映射，自动化重复结果所有权已定位；撤权/取消树及全客户端待验收 | 输入输出、取消树、恢复及权限范围 |
| Mac/CLI/iOS/Android/Web 投影 | 关键接入已映射，CLI 回执路径整改；五端交互与全调用覆盖未完成 | 每端真实提交/订阅/断线/切换路径 |
| 首 token、工具、UI 的性能基线 | 本地 Application/SQLite 子段已测；provider/UI 完整口径缺失 | PERFORMANCE_BASELINE 原始样本；不能把固定 Host 输入当真实首 token |
| 纵向整改及故障回归 | 通知交付等局部链路已整改；完整结果封存与五端矩阵未完成 | E2E-01 起，跨模块修改后整批验证 |

实施顺序：先完成模块审计和状态设计 → 证据/收尾完整纵向切片 → 快照/冷恢复切片 →
取消/重启与客户端一致性 → 性能采样/预算定版 → 全矩阵完成审计。
每个切片必须同时贯通 Domain、Kernel、Host、Application、Store 和客户端投影；
不把六端所有功能同时重写，也不把单模块通过当切片完成。

## 客户端错误关闭不能修改执行状态（2026-10-12，源码整改，待统一验收）

直接证据与调用链：

- iOS `ChatView.errorRecoveryBanner` 的关闭、编辑重试、切换模型均调用
  `dismissChatError`；旧实现同时把 View 的 `processingState` 和
  `ChatManager.processingState` 写为 `.idle`。展示操作因此冒充执行状态转换。
- Android `ChatScreen` 同三条操作调用 `ChatViewModel.dismissError`；旧实现在
  `ProcessingState.Error` 下写 Idle，并清除 active response ID、开始时间、部分正文、
  思考和工具事件。关闭错误会丢掉仍应由执行/恢复链路解释的证据。

整改契约与实际接线：

- iOS 新增纯 `ChatErrorPresentation`，每次错误观察产生独立 occurrence UUID，
  关闭要求 occurrence 和当前 conversation 匹配；仅移除展示，不写 manager 或执行
  状态。同会话相同错误再次出现也重新可见。processing publisher 在切入主队列前捕获
  会话身份，消费前核对当前会话及最新状态，拒绝已过期的异步投影。
- Android 关闭要求原始不可变 `ChatUiState` 快照仍为当前快照；CAS 只更新 `error=null`，
  不清运行字段/正文/权限/工具证据。过期按钮不能关闭新会话或替代错误；编辑草稿和
  打开模型页仅在对应关闭被接受后执行。它是展示确认，不是授权、取消或完成回执。
- 错误详情展开/复制仅是本地 UI 状态；三档复杂度、权限策略、Run 状态机未改动。

新增回归源码：`NolaTests/ChatErrorPresentationTests.swift`、
`android/app/src/test/java/com/nola/ui/screen/chat/ChatErrorDismissalTest.kt`，覆盖错误重复
出现、旧会话/旧 occurrence 或旧快照拒绝、运行与部分输出字段保持。
本批仅静态审查与 `git diff --check`，未编译/运行测试；不把纯投影测试等同于真实 UI
验收。最终仍需真实运行中断→关闭错误→切会话→恢复/取消的完整链路，以及延迟队列
回调、相同错误重发、大字/键盘/长诊断交互验收。

## 独立完成审查与普通 final synthesis 边界（2026-10-12）

已查明生产审查入口并非无条件开启：`AgentTurnState::new` 和
`turn_lifecycle` 默认 `final_answer_audit_required=false`。唯一生产置 true 的路径是
`runtime/submission.rs` 对 `request_completion_review` 的模型 typed tool call：
要求该能力在本次模型请求中实际提供、调用为唯一 tool call、响应未截断，且
`candidate_answer` 非空并满足大小限制。`model_planner` 工具说明明确可直接回答，
由模型判断是否请求独立审查；不是“用了工具就必须审查”。

现有回归源码 `agent_core_host_contract.rs` 中
`independent_review_is_disabled_by_default_even_with_evidence_capabilities`、
`executed_evidence_tool_does_not_enable_independent_review_without_opt_in` 和
`model_can_request_review_with_a_typed_control_call` 分别覆盖普通答案、工具后直接
回答及显式审查入口。本次未运行，不能将其当作当前通过证据。

另发现并修复真实提示契约矛盾：普通协议恢复的 `final_answer_retry_requested`
也可进入 `final_synthesis`，但无 candidate 的指令原先无条件声称“工具执行已完成，
答案会保密直到单独审核”。此时 audit flag 可以仍为 false，实际完成策略不会启动审查。
现在 `final_synthesis_instruction(review_requested)` 明确区分模型已请求的私有审查候选
与直接用户答案，且两者均不把 synthesis 阶段当作执行/验证成功证据。
未修改动作证据守卫、审查选择入口、验证义务或结果仲裁。新增对应指令分支测试源码；
仅 `git diff --check`，编译、测试和真实模型协议恢复仍留最终统一验收。

## Application 准入分母诊断切片（2026-10-12，待统一验收）

原采集从 provider_sampling 开始，缺少所有尚未到采样的失败/取消样本；现在 Application
命令入口有带 typed command 身份的执行 span，SQL COMMIT 后仅新回执发 committed point，
重放不新增 receipt point。Rejected/Conflict 不是成功业务准入；命令 committed 也不等于新 Run 或任务完成。
context 的 command 使用 client/request tuple 长度分帧哈希，避免不同 tuple 的拼接冲突；不记录命令正文。

新增只读回执导出与离线 manifest 核对：实验从发送前登记全部样本，固定构建/快照摘要，
单一 SQLite 读快照查询每个样本，missing 保留 absent，采集缺失不改写 canonical 状态。
报告分别列全部样本、命令 committed 数、拒绝/冲突/待恢复、失败/取消/超时及诊断缺失/歧义。
没有新增 usage 计数、预算或业务授权，未连接假造的端到端首帧。provider attempt 与 sample
已补 canonical 导出链：回执 Run 身份→Run owner→Attempt→effect/delivery，含被后继 delivery
覆盖的历史 Host inbox Attempt 关联。报告四元组精确 join，并区分压缩/maintenance/重试；
同钟唯一 Run 起点可给 Application 调用→adapter 子段，跨钟及同 Run follow-up 不伪造归因。
适配器首输出保留原 observed sampling 分母，全样本报告单列缺失类别。
测试源码包括 tuple/replay 分类、拒绝/冲突、缺样本、只读快照、重复/不匹配；尚未运行。

### Android automation admission/recovery 接线（2026-10-12）

Android scheduled action 先保存 typed `AutomationExecutionSpec`，读取实际 canonical
definition revision 入 occurrence；不再默认 revision 1。首次执行由 shared
`BeginAutomationAssistantTurn` 将 occurrence 绑定原 Run，worker EOF、正文非空或 trace
completed 均不能完成 occurrence，只有 shared `ReconcileAutomationExecution` receipt
可发布结果和以原 notification ID 去重通知。删除旧 Finish 调用。

已有 occurrence 优先查询，不借当前 definition/revision 重新 enqueue；已有 binding
先走原 Run recovery，不再次 claim 已绑定 occurrence。dispatch lease 只负责首次准入，
不作为 Host 执行心跳或取消依据。恢复重建原冻结模型并验证执行策略，无法复原时保留
原身份待恢复，不发新用户输入。新准入核 actual device Host ID、真实加载的 source
及 canonical metadata；未实现的 Skill refs 显式拒绝，不把引用声明当加载成功。

scheduled editor 补任务指令必填输入与草稿保护，沿用 Material 组件及语义颜色；结果
展示映射 canonical result kind，blocked/cancelled 不再误作成功。JVM 与设备验收分开：
JVM 编译及测试不能代替 JNI + WorkManager + 真模型断线恢复验收。
本批最终 Gradle `:app:testDebugUnitTest :app:compileDebugAndroidTestKotlin`
（排除 Rust/native 构建）已通过；设备 instrumentation 与真实模型执行未运行。

### CLI 统一验收发现的 delegated 决策来源丢失（2026-10-12）

真实 delegated process 回传 Completed 时，Host response reducer 先退休 pending effect，
随后转入 ModelCompleted；旧 fallback 已找不到原 effect，导致终局判定报
`result has no admitted decision source`，甚至早于 Application 故障注入点。现在在
已接受 Host envelope、退休 effect 之前保留 effect/request/实际可选 attempt 来源，
与普通模型终局路径一致，不为 opaque delegated 后端伪造 provider Attempt。
Host contract 测试保留原正文/工具断言并新增 exact source 身份断言；本批源码待统一复验。

同批 Host contract 复验进一步发现取消摘要的校验顺序问题：已进入 cancelling 的原
compaction effect 已清理摘要计划，晚到的摘要却先被“无 active plan”拒绝，无法走既有
取消过滤。现只对该 pending effect 的 cancelling 状态跳过摘要内容计划校验，仍严格
执行原 Host envelope 身份/顺序校验；foreign request 反例核验 checkpoint 不变。Host
单独报 cancelled 不等价用户取消，必须有已提交 cancellation identity 后确认取消；
否则保留失败与原上下文，不伪造 Cancelled。终局工具 fixture 改为精确允许 Core 内置
requirements_update，禁止业务执行工具，并核验正式 Incomplete 而非把 Completed 当成功。
